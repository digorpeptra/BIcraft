//! The Databricks SQL Statement Execution API (REST, `/api/2.0/sql/statements`), as documented
//! publicly by Databricks ("Statement Execution API", API reference *Statement Execution*).
//!
//! One statement: `POST /api/2.0/sql/statements/` with the SQL, the warehouse id, named
//! parameters, `format: JSON_ARRAY`, `disposition: INLINE` (or `EXTERNAL_LINKS` when allowed),
//! `wait_timeout` and `on_wait_timeout: CONTINUE`; while the state is `PENDING` or `RUNNING`,
//! `GET /api/2.0/sql/statements/{id}` with a growing interval (then `POST …/cancel` when the
//! overall wait is exceeded); on `SUCCEEDED`, the first chunk is in the response and the next
//! ones come from `GET /api/2.0/sql/statements/{id}/result/chunks/{n}` until there is no
//! `next_chunk_index`. `FAILED` carries `status.error {error_code, message}`.
//!
//! Requests go to the workspace host the query names and nowhere else: no proxy, no redirect.
//! The only exception is `EXTERNAL_LINKS`, off by default: its presigned cloud-storage URLs are
//! fetched without the `Authorization` header. The token is read once, only when the first
//! request is made, and appears in no message, log or error.

use std::io::Read as _;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bicraft_data::connect::{ConnectError, ConnectErrorKind, RemoteColumn};
use serde_json::{Value, json};

use crate::sql::Statement;
use crate::{Settings, Token, TokenCell, types};

/// Most chunks read for one statement.
const MAX_CHUNKS: usize = 1_000_000;

fn net(message: impl Into<String>) -> ConnectError {
    ConnectError::new(ConnectErrorKind::Network, message)
}

/// The rows of one statement, as the API returns them (values still JSON).
pub(crate) struct Answer {
    pub columns: Vec<RemoteColumn>,
    pub rows: Vec<Vec<Value>>,
}

/// One executed statement, for the caller's report (SQL text and parameter values, never a
/// credential).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Executed {
    pub host: String,
    pub sql: String,
    pub rows: usize,
    pub millis: u64,
}

pub(crate) struct Client {
    pub host: String,
    base: String,
    warehouse_id: String,
    agent: ureq::Agent,
    links: ureq::Agent,
    /// Plain HTTP to a loopback address is accepted (test endpoints only).
    loopback_http: bool,
    token: TokenCell,
    settings: Settings,
    log: Arc<Mutex<Vec<Executed>>>,
}

enum Method {
    Get,
    Post,
}

fn agent(https_only: bool, settings: &Settings) -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .https_only(https_only)
        .max_redirects(0)
        .proxy(None)
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_global(Some(settings.request_timeout))
        .user_agent(concat!("bicraft/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

impl Client {
    pub(crate) fn new(
        host: String,
        base: String,
        warehouse_id: String,
        loopback_http: bool,
        token: TokenCell,
        settings: Settings,
        log: Arc<Mutex<Vec<Executed>>>,
    ) -> Self {
        Self {
            agent: agent(!loopback_http, &settings),
            links: agent(!loopback_http, &settings),
            host,
            base,
            warehouse_id,
            loopback_http,
            token,
            settings,
            log,
        }
    }

    fn token(&self) -> Result<Token, ConnectError> {
        self.token.get()
    }

    /// `text` with the token, should it ever appear, replaced.
    fn redact(&self, text: String) -> String {
        match self.token.peek() {
            Some(t) if t.0.len() >= 4 && text.contains(&*t.0) => text.replace(&*t.0, "[REDACTED]"),
            _ => text,
        }
    }

    /// One API call with retries (HTTP 429/5xx, transport errors), as JSON.
    fn call(&self, method: Method, path: &str, body: Option<&Value>) -> Result<Value, ConnectError> {
        let url = format!("{}{path}", self.base);
        let token = self.token()?;
        let bearer = format!("Bearer {}", token.0);
        let mut delay = self.settings.retry_delay;
        let mut attempt = 0u32;
        loop {
            attempt = attempt.saturating_add(1);
            let last = attempt > self.settings.retries;
            let sent = match (&method, body) {
                (Method::Post, Some(b)) => self
                    .agent
                    .post(&url)
                    .header("Authorization", &bearer)
                    .header("Content-Type", "application/json")
                    .header("Accept", "application/json")
                    .send(b.to_string()),
                (Method::Post, None) => self
                    .agent
                    .post(&url)
                    .header("Authorization", &bearer)
                    .header("Accept", "application/json")
                    .send_empty(),
                (Method::Get, _) => self
                    .agent
                    .get(&url)
                    .header("Authorization", &bearer)
                    .header("Accept", "application/json")
                    .call(),
            };
            let resp = match sent {
                Ok(r) => r,
                Err(e) if !last => {
                    let _ = e;
                    std::thread::sleep(delay);
                    delay = delay.saturating_mul(2);
                    continue;
                }
                Err(e) => {
                    return Err(net(self.redact(format!(
                        "Databricks {}: request to {url} failed: {e}",
                        self.host
                    ))));
                }
            };
            let status = resp.status().as_u16();
            let retry_after = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<u64>().ok());
            let bytes = resp
                .into_body()
                .into_with_config()
                .limit(self.settings.max_response_bytes)
                .read_to_vec()
                .map_err(|e| {
                    net(self.redact(format!(
                        "Databricks {}: reading the response of {url} failed: {e}",
                        self.host
                    )))
                })?;
            let parsed: Option<Value> = serde_json::from_slice(&bytes).ok();
            let api_message = || {
                let p = parsed.as_ref();
                let code = p
                    .and_then(|v| v.get("error_code"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let msg = p
                    .and_then(|v| v.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                self.redact(match (code.is_empty(), msg.is_empty()) {
                    (true, true) => String::new(),
                    (false, true) => format!(": {code}"),
                    (true, false) => format!(": {msg}"),
                    (false, false) => format!(": {code}: {msg}"),
                })
            };
            match status {
                200..=299 => {
                    return parsed
                        .ok_or_else(|| net(format!("Databricks {}: {url} did not return JSON", self.host)));
                }
                401 => {
                    return Err(ConnectError::new(
                        ConnectErrorKind::Credentials,
                        format!(
                            "Databricks {} refused the credentials (HTTP 401{}); check the token in \
                             BICRAFT_DATABRICKS_TOKEN",
                            self.host,
                            api_message()
                        ),
                    ));
                }
                403 => {
                    return Err(ConnectError::new(
                        ConnectErrorKind::Credentials,
                        format!(
                            "Databricks {}: permission denied (HTTP 403{})",
                            self.host,
                            api_message()
                        ),
                    ));
                }
                429 | 500 | 502 | 503 | 504 if !last => {
                    let wait = retry_after
                        .map(|s| Duration::from_secs(s.min(60)))
                        .unwrap_or(delay)
                        .max(self.settings.retry_delay);
                    std::thread::sleep(wait);
                    delay = delay.saturating_mul(2);
                }
                _ => {
                    return Err(net(format!(
                        "Databricks {}: HTTP {status} from {path}{}",
                        self.host,
                        api_message()
                    )));
                }
            }
        }
    }

    /// Runs `st`; at most `max_rows` rows.
    pub(crate) fn run(&self, st: &Statement, max_rows: usize) -> Result<Answer, ConnectError> {
        let started = Instant::now();
        let disposition = if self.settings.external_links {
            "EXTERNAL_LINKS"
        } else {
            "INLINE"
        };
        let body = json!({
            "statement": st.sql,
            "warehouse_id": self.warehouse_id,
            "parameters": st.parameters,
            "format": "JSON_ARRAY",
            "disposition": disposition,
            "wait_timeout": format!("{}s", self.settings.wait_timeout_s),
            "on_wait_timeout": "CONTINUE",
        });
        let mut resp = self.call(Method::Post, "/api/2.0/sql/statements/", Some(&body))?;
        let id = resp
            .get("statement_id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| {
                net(format!(
                    "Databricks {}: the API returned no statement_id",
                    self.host
                ))
            })?;
        if !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(net(format!("Databricks {}: unexpected statement id", self.host)));
        }
        let deadline = started + self.settings.max_wait;
        let mut poll = self.settings.poll_initial;
        loop {
            let state = resp
                .pointer("/status/state")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            match state.as_str() {
                "SUCCEEDED" => break,
                "PENDING" | "RUNNING" => {
                    if Instant::now() >= deadline {
                        let _ = self.call(
                            Method::Post,
                            &format!("/api/2.0/sql/statements/{id}/cancel"),
                            None,
                        );
                        return Err(net(format!(
                            "Databricks {}: the statement did not finish within {} s (cancelled)",
                            self.host,
                            self.settings.max_wait.as_secs()
                        )));
                    }
                    std::thread::sleep(poll);
                    poll = poll.saturating_mul(2).min(self.settings.poll_max);
                    resp = self.call(Method::Get, &format!("/api/2.0/sql/statements/{id}"), None)?;
                }
                "FAILED" => {
                    let e = resp.pointer("/status/error");
                    let code = e
                        .and_then(|e| e.get("error_code"))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let msg = e
                        .and_then(|e| e.get("message"))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    return Err(ConnectError::new(
                        ConnectErrorKind::DataSource,
                        self.redact(format!(
                            "Databricks: {code}{}{msg}",
                            if code.is_empty() { "" } else { ": " }
                        )),
                    ));
                }
                other => {
                    return Err(net(format!(
                        "Databricks {}: the statement ended in state `{other}`",
                        self.host
                    )));
                }
            }
        }
        let manifest = resp.get("manifest").cloned().unwrap_or(Value::Null);
        if manifest.get("truncated").and_then(Value::as_bool) == Some(true) {
            return Err(ConnectError::new(
                ConnectErrorKind::Limit,
                format!(
                    "Databricks {}: the result was truncated (the inline result limit); allow external \
                     links for large tables",
                    self.host
                ),
            ));
        }
        let columns = manifest_columns(&manifest)?;
        let mut rows: Vec<Vec<Value>> = Vec::new();
        let mut chunk = resp.get("result").cloned().unwrap_or(Value::Null);
        let mut chunks = 0usize;
        loop {
            chunks = chunks.saturating_add(1);
            if chunks > MAX_CHUNKS {
                return Err(net(format!("Databricks {}: too many result chunks", self.host)));
            }
            self.collect(&chunk, &mut rows, max_rows)?;
            // Inline results name the next chunk on the result; external links on each link.
            let next = chunk.get("next_chunk_index").and_then(Value::as_u64).or_else(|| {
                chunk
                    .get("external_links")
                    .and_then(Value::as_array)
                    .and_then(|l| l.last())
                    .and_then(|l| l.get("next_chunk_index"))
                    .and_then(Value::as_u64)
            });
            let Some(next) = next else {
                break;
            };
            chunk = self.call(
                Method::Get,
                &format!("/api/2.0/sql/statements/{id}/result/chunks/{next}"),
                None,
            )?;
        }
        if let Ok(mut log) = self.log.lock() {
            let params: Vec<String> = st
                .parameters
                .iter()
                .filter_map(|p| {
                    Some(format!(
                        "{} = {}",
                        p.get("name")?.as_str()?,
                        serde_json::to_string(p.get("value")?).ok()?
                    ))
                })
                .collect();
            let sql = if params.is_empty() {
                st.sql.clone()
            } else {
                format!("{} -- {}", st.sql, params.join(", "))
            };
            log.push(Executed {
                host: self.host.clone(),
                sql,
                rows: rows.len(),
                millis: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            });
        }
        Ok(Answer { columns, rows })
    }

    /// The rows of one result chunk: inline `data_array`, or the files of `external_links`.
    fn collect(
        &self,
        chunk: &Value,
        rows: &mut Vec<Vec<Value>>,
        max_rows: usize,
    ) -> Result<(), ConnectError> {
        let push = |arr: &[Value], rows: &mut Vec<Vec<Value>>| -> Result<(), ConnectError> {
            for r in arr {
                let Value::Array(cells) = r else {
                    return Err(net(format!(
                        "Databricks {}: a result row is not an array",
                        self.host
                    )));
                };
                if rows.len() >= max_rows {
                    return Err(ConnectError::new(
                        ConnectErrorKind::Limit,
                        format!("Databricks {}: more than {max_rows} rows", self.host),
                    ));
                }
                rows.push(cells.clone());
            }
            Ok(())
        };
        if let Some(Value::Array(data)) = chunk.get("data_array") {
            push(data, rows)?;
        }
        if let Some(Value::Array(links)) = chunk.get("external_links") {
            for l in links {
                let data = self.external(l)?;
                push(&data, rows)?;
            }
        }
        Ok(())
    }

    /// A presigned result file: HTTPS only, never with the token.
    fn external(&self, link: &Value) -> Result<Vec<Value>, ConnectError> {
        if !self.settings.external_links {
            return Err(net(format!(
                "Databricks {}: the API returned external links, which are not allowed",
                self.host
            )));
        }
        let url = link
            .get("external_link")
            .and_then(Value::as_str)
            .ok_or_else(|| net(format!("Databricks {}: an external link has no URL", self.host)))?;
        let lower = url.to_ascii_lowercase();
        let loopback = lower.starts_with("http://127.0.0.1:") || lower.starts_with("http://localhost:");
        if !(lower.starts_with("https://") || (self.loopback_http && loopback)) {
            return Err(net(format!(
                "Databricks {}: an external link is not HTTPS",
                self.host
            )));
        }
        let mut req = self.links.get(url);
        if let Some(Value::Object(h)) = link.get("http_headers") {
            for (k, v) in h {
                if k.eq_ignore_ascii_case("authorization") {
                    continue;
                }
                if let Some(v) = v.as_str() {
                    req = req.header(k.as_str(), v);
                }
            }
        }
        // The URL carries a signature: errors name the host only.
        let shown = url.split('?').next().unwrap_or("");
        let resp = req
            .call()
            .map_err(|e| net(format!("Databricks external link {shown}: {e}")))?;
        let status = resp.status().as_u16();
        let mut bytes = Vec::new();
        resp.into_body()
            .into_reader()
            .take(self.settings.max_response_bytes)
            .read_to_end(&mut bytes)
            .map_err(|e| net(format!("Databricks external link {shown}: {e}")))?;
        if !(200..300).contains(&status) {
            return Err(net(format!("Databricks external link {shown}: HTTP {status}")));
        }
        match serde_json::from_slice(&bytes) {
            Ok(Value::Array(rows)) => Ok(rows),
            _ => Err(net(format!(
                "Databricks external link {shown}: not a JSON array of rows"
            ))),
        }
    }
}

/// The columns of a result manifest, in position order.
fn manifest_columns(manifest: &Value) -> Result<Vec<RemoteColumn>, ConnectError> {
    let Some(Value::Array(cols)) = manifest.pointer("/schema/columns") else {
        return Ok(Vec::new());
    };
    let mut out: Vec<(u64, RemoteColumn)> = Vec::with_capacity(cols.len());
    for (i, c) in cols.iter().enumerate() {
        let name = c
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| net("Databricks: a result column has no name"))?;
        let type_name = c.get("type_name").and_then(Value::as_str).unwrap_or("STRING");
        let type_text = c.get("type_text").and_then(Value::as_str).unwrap_or("");
        let pos = c.get("position").and_then(Value::as_u64).unwrap_or(i as u64);
        out.push((pos, types::column(name, type_name, type_text)));
    }
    out.sort_by_key(|(p, _)| *p);
    Ok(out.into_iter().map(|(_, c)| c).collect())
}
