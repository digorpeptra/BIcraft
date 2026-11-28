//! L3: the Databricks connector of bicraft's Power Query evaluator.
//!
//! Registered in [`bicraft_data::SourceOptions::connectors`] only when the caller explicitly
//! allows it (`bicraft-cli refresh --allow-connector databricks`, `model.refresh
//! {allow_connectors: ["databricks"]}`), it runs the M functions Power BI Desktop writes for
//! Databricks, `DatabricksMultiCloud.Catalogs(host, httpPath, options)` and
//! `Databricks.Catalogs(…)`, through the **SQL Statement Execution API** of a SQL warehouse
//! (the warehouse id is the last segment of the HTTP path `/sql/1.0/warehouses/<id>`):
//!
//! - navigation: catalogs (`Kind = "Database"`), schemas (`"Schema"`), tables and views
//!   (`"Table"`, `"View"`); a key lookup by name and kind needs no query;
//! - the folded query of a table ([`bicraft_data::connect::RemoteQuery`]) as one `SELECT`
//!   with named parameters ([`sql`]);
//! - Databricks types mapped to M types ([`types`]).
//!
//! Network access is limited to `https://<host>` of the query (no proxy, no redirect); large
//! results through presigned `EXTERNAL_LINKS` only when [`Settings::external_links`] allows it.
//! Authentication is a personal access token (or any bearer token) the caller supplies through a
//! [`TokenProvider`], read once, at the first request; it is never logged, displayed or written.
//! See `docs/formats/data-sources.md`, *Databricks connector*.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

mod api;
pub mod sql;
mod types;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub use api::Executed;
use bicraft_data::connect::{
    Arg, ConnectError, ConnectErrorKind, Connector, NavEntry, RemoteColumn, RemoteQuery, RemoteRows, Source,
};

/// The environment variable the CLI and the MCP server read the token from.
pub const TOKEN_ENV: &str = "BICRAFT_DATABRICKS_TOKEN";

/// A bearer token. Its `Debug` output hides it; it has no `Display`.
#[derive(Clone)]
pub struct Token(Arc<str>);

impl Token {
    /// A token from its text (surrounding blanks removed); `None` when empty.
    pub fn new(text: &str) -> Option<Self> {
        let t = text.trim();
        (!t.is_empty()).then(|| Self(t.into()))
    }

    /// The token in environment variable `var`, when set and not empty.
    pub fn from_env(var: &str) -> Option<Self> {
        bicraft_env::var(var).ok().as_deref().and_then(Self::new)
    }
}

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token(***)")
    }
}

/// Where the token comes from, asked once, at the first request: an environment variable, a
/// masked console prompt, … (`Err`: why there is none, shown to the user).
pub type TokenProvider = Arc<dyn Fn() -> Result<Token, String> + Send + Sync>;

/// The token, fetched once from its provider and shared by every client of a connector.
#[derive(Clone)]
pub(crate) struct TokenCell {
    provider: TokenProvider,
    value: Arc<Mutex<Option<Result<Token, String>>>>,
}

impl TokenCell {
    fn get(&self) -> Result<Token, ConnectError> {
        let mut slot = self
            .value
            .lock()
            .map_err(|_| ConnectError::new(ConnectErrorKind::Credentials, "the token store is unusable"))?;
        if slot.is_none() {
            *slot = Some((self.provider)());
        }
        match slot.as_ref() {
            Some(Ok(t)) => Ok(t.clone()),
            Some(Err(why)) => Err(ConnectError::new(ConnectErrorKind::Credentials, why.clone())),
            None => Err(ConnectError::new(ConnectErrorKind::Credentials, "no token")),
        }
    }

    /// The token if it was already read (for redaction).
    fn peek(&self) -> Option<Token> {
        self.value.lock().ok()?.as_ref()?.as_ref().ok().cloned()
    }
}

/// How the connector talks to the API.
#[derive(Debug, Clone)]
pub struct Settings {
    /// Fetch large results through presigned cloud-storage links (`EXTERNAL_LINKS`): requests
    /// then also go to the cloud storage host the API names (without the token). Off: results
    /// are inline, which the API caps at 25 MiB per statement.
    pub external_links: bool,
    /// `wait_timeout` of a statement, in seconds (the API accepts 5–50).
    pub wait_timeout_s: u32,
    /// Longest time one statement may take before it is cancelled.
    pub max_wait: Duration,
    /// First and longest interval between two status polls.
    pub poll_initial: Duration,
    pub poll_max: Duration,
    /// Retries of a request answered HTTP 429/5xx or failing in transport, and the first delay.
    pub retries: u32,
    pub retry_delay: Duration,
    /// Timeout of one HTTP request and the largest response read.
    pub request_timeout: Duration,
    pub max_response_bytes: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            external_links: false,
            wait_timeout_s: 30,
            max_wait: Duration::from_secs(15 * 60),
            poll_initial: Duration::from_millis(500),
            poll_max: Duration::from_secs(5),
            retries: 4,
            retry_delay: Duration::from_millis(500),
            request_timeout: Duration::from_secs(120),
            max_response_bytes: 512 << 20,
        }
    }
}

/// A test endpoint: requests for workspace `host` go to `base` (plain HTTP on a loopback
/// address only).
#[derive(Debug, Clone)]
struct Endpoint {
    host: String,
    base: String,
}

/// Column lists by (host, table path), shared by the tables of one refresh.
type ColumnCache = Arc<Mutex<HashMap<(String, Vec<String>), Vec<RemoteColumn>>>>;

/// The Databricks connector ([`Connector`] named `databricks`).
pub struct DatabricksConnector {
    token: TokenCell,
    settings: Settings,
    endpoint: Option<Endpoint>,
    log: Arc<Mutex<Vec<Executed>>>,
    clients: Mutex<HashMap<(String, String), Arc<api::Client>>>,
    columns: ColumnCache,
}

impl DatabricksConnector {
    pub fn new(token: TokenProvider, settings: Settings) -> Self {
        Self {
            token: TokenCell {
                provider: token,
                value: Arc::new(Mutex::new(None)),
            },
            settings,
            endpoint: None,
            log: Arc::new(Mutex::new(Vec::new())),
            clients: Mutex::new(HashMap::new()),
            columns: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Sends the requests for workspace `host` to `base` instead (`http://127.0.0.1:<port>` or
    /// `http://localhost:<port>`): for tests against a local fake server only.
    pub fn with_test_endpoint(mut self, host: &str, base: &str) -> Result<Self, String> {
        let b = base.trim_end_matches('/').to_ascii_lowercase();
        let rest = b
            .strip_prefix("http://127.0.0.1:")
            .or_else(|| b.strip_prefix("http://localhost:"))
            .ok_or("a test endpoint must be http://127.0.0.1:<port> or http://localhost:<port>")?;
        if rest.is_empty() || !rest.bytes().all(|c| c.is_ascii_digit()) {
            return Err("a test endpoint must be http://127.0.0.1:<port> or http://localhost:<port>".into());
        }
        self.endpoint = Some(Endpoint {
            host: host.to_ascii_lowercase(),
            base: b,
        });
        Ok(self)
    }

    /// The statements run so far (and clears the list).
    pub fn take_log(&self) -> Vec<Executed> {
        self.log
            .lock()
            .map(|mut l| std::mem::take(&mut *l))
            .unwrap_or_default()
    }

    fn client(&self, host: &str, warehouse: &str) -> Arc<api::Client> {
        let key = (host.to_string(), warehouse.to_string());
        let mut clients = match self.clients.lock() {
            Ok(c) => c,
            Err(p) => p.into_inner(),
        };
        if let Some(c) = clients.get(&key) {
            return Arc::clone(c);
        }
        let (base, loopback) = match &self.endpoint {
            Some(e) if e.host == host => (e.base.clone(), true),
            _ => (format!("https://{host}"), false),
        };
        let c = Arc::new(api::Client::new(
            host.to_string(),
            base,
            warehouse.to_string(),
            loopback,
            self.token.clone(),
            self.settings.clone(),
            Arc::clone(&self.log),
        ));
        clients.insert(key, Arc::clone(&c));
        c
    }
}

fn unsupported(message: impl Into<String>) -> ConnectError {
    ConnectError::new(ConnectErrorKind::Unsupported, message)
}

/// The workspace host of the first argument: a DNS name (with an optional port), with or
/// without `https://` and a trailing `/`.
pub fn parse_host(text: &str) -> Result<String, String> {
    let t = text.trim();
    let t = t
        .strip_prefix("https://")
        .or_else(|| t.strip_prefix("HTTPS://"))
        .unwrap_or(t)
        .trim_end_matches('/');
    let (name, port) = match t.rsplit_once(':') {
        Some((n, p)) => (n, Some(p)),
        None => (t, None),
    };
    let label_ok = |l: &str| {
        !l.is_empty()
            && l.len() <= 63
            && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            && !l.starts_with('-')
            && !l.ends_with('-')
    };
    let name_ok = !name.is_empty() && name.len() <= 253 && name.split('.').all(label_ok);
    let port_ok = port.is_none_or(|p| !p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit()));
    if !name_ok || !port_ok {
        return Err(format!(
            "`{text}` is not a Databricks workspace host (expected a name such as \
             dbc-1234abcd-5678.cloud.databricks.com, without a path)"
        ));
    }
    Ok(t.to_ascii_lowercase())
}

/// The SQL warehouse id of an HTTP path (`/sql/1.0/warehouses/<id>`, or the older
/// `/sql/1.0/endpoints/<id>`).
pub fn parse_warehouse(http_path: &str) -> Result<String, String> {
    let p = http_path.trim().trim_end_matches('/');
    let id = p
        .strip_prefix("/sql/1.0/warehouses/")
        .or_else(|| p.strip_prefix("/sql/1.0/endpoints/"))
        .or_else(|| p.strip_prefix("sql/1.0/warehouses/"))
        .filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric()));
    match id {
        Some(id) => Ok(id.to_string()),
        None if p.contains("protocolv1") => Err(format!(
            "`{http_path}` is a cluster HTTP path; bicraft reaches Databricks through the SQL \
             Statement Execution API, which runs on a SQL warehouse (/sql/1.0/warehouses/<id>)"
        )),
        None => Err(format!(
            "`{http_path}` is not a SQL warehouse HTTP path (/sql/1.0/warehouses/<id>)"
        )),
    }
}

impl Connector for DatabricksConnector {
    fn name(&self) -> &'static str {
        "databricks"
    }

    fn handles(&self, function: &str) -> bool {
        matches!(function, "DatabricksMultiCloud.Catalogs" | "Databricks.Catalogs")
    }

    fn open(&self, function: &str, args: &[Arg]) -> Result<Box<dyn Source>, ConnectError> {
        let text = |i: usize, what: &str| match args.get(i) {
            Some(Arg::Text(t)) => Ok(t.clone()),
            _ => Err(unsupported(format!("{function}: the {what} must be text"))),
        };
        let host = parse_host(&text(0, "host")?).map_err(unsupported)?;
        let warehouse = parse_warehouse(&text(1, "HTTP path")?).map_err(unsupported)?;
        match args.get(2) {
            None | Some(Arg::Null | Arg::Record(_)) => {}
            Some(_) => return Err(unsupported(format!("{function}: the options must be a record"))),
        }
        if args.len() > 3 {
            return Err(unsupported(format!("{function} takes at most 3 arguments")));
        }
        Ok(Box::new(DbxSource {
            client: self.client(&host, &warehouse),
            columns: Arc::clone(&self.columns),
            listings: HashMap::new(),
        }))
    }
}

/// The navigation tree and queries of one connector call.
struct DbxSource {
    client: Arc<api::Client>,
    columns: ColumnCache,
    listings: HashMap<Vec<String>, Vec<NavEntry>>,
}

fn entry(name: String, kind: &str, leaf: bool) -> NavEntry {
    NavEntry {
        name,
        kind: kind.to_string(),
        leaf,
        fields: Vec::new(),
    }
}

/// Text in column `i` of every row (navigation listings).
fn texts(a: &api::Answer, i: usize) -> Vec<String> {
    a.rows
        .iter()
        .filter_map(|r| r.get(i).and_then(|v| v.as_str()).map(str::to_string))
        .collect()
}

const MAX_LISTING: usize = 1_000_000;

impl Source for DbxSource {
    fn lookup(
        &mut self,
        path: &[String],
        name: &str,
        kind: Option<&str>,
    ) -> Result<Option<NavEntry>, ConnectError> {
        Ok(match (path.len(), kind) {
            (0, Some("Database")) => Some(entry(name.to_string(), "Database", false)),
            (1, Some("Schema")) => Some(entry(name.to_string(), "Schema", false)),
            (2, Some(k @ ("Table" | "View"))) => Some(entry(name.to_string(), k, true)),
            _ => None,
        })
    }

    fn list(&mut self, path: &[String]) -> Result<Vec<NavEntry>, ConnectError> {
        if let Some(l) = self.listings.get(path) {
            return Ok(l.clone());
        }
        let out: Vec<NavEntry> = match path {
            [] => {
                let a = self.client.run(&sql::catalogs(), MAX_LISTING)?;
                texts(&a, 0)
                    .into_iter()
                    .map(|n| entry(n, "Database", false))
                    .collect()
            }
            [c] => {
                let a = self.client.run(&sql::schemas(c), MAX_LISTING)?;
                texts(&a, 0)
                    .into_iter()
                    .map(|n| entry(n, "Schema", false))
                    .collect()
            }
            [c, s] => match self.client.run(&sql::tables(c, s), MAX_LISTING) {
                Ok(a) => a
                    .rows
                    .iter()
                    .filter_map(|r| {
                        let name = r.first()?.as_str()?.to_string();
                        let ty = r
                            .get(1)
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_ascii_uppercase();
                        Some(entry(
                            name,
                            if ty.contains("VIEW") { "View" } else { "Table" },
                            true,
                        ))
                    })
                    .collect(),
                // No information_schema (hive_metastore): SHOW TABLES, without the kind.
                Err(e) if e.kind == ConnectErrorKind::DataSource => {
                    let a = self.client.run(&sql::show_tables(c, s), MAX_LISTING)?;
                    texts(&a, 1)
                        .into_iter()
                        .map(|n| entry(n, "Table", true))
                        .collect()
                }
                Err(e) => return Err(e),
            },
            _ => {
                return Err(unsupported(
                    "Databricks navigation goes three levels deep (catalog, schema, table)",
                ));
            }
        };
        self.listings.insert(path.to_vec(), out.clone());
        Ok(out)
    }

    fn columns(&mut self, path: &[String]) -> Result<Vec<RemoteColumn>, ConnectError> {
        let key = (self.client.host.clone(), path.to_vec());
        if let Some(c) = self.columns.lock().ok().and_then(|m| m.get(&key).cloned()) {
            return Ok(c);
        }
        let a = self.client.run(&sql::columns(path), 0)?;
        if let Ok(mut m) = self.columns.lock() {
            m.insert(key, a.columns.clone());
        }
        Ok(a.columns)
    }

    fn fetch(&mut self, q: &RemoteQuery, max_rows: usize) -> Result<RemoteRows, ConnectError> {
        if q.table.len() != 3 {
            return Err(unsupported("a Databricks table is catalog.schema.table"));
        }
        let a = self.client.run(&sql::select(q), max_rows)?;
        let width = a.columns.len();
        let mut rows = Vec::with_capacity(a.rows.len());
        for (ri, r) in a.rows.into_iter().enumerate() {
            if r.len() != width {
                return Err(ConnectError::new(
                    ConnectErrorKind::Network,
                    format!(
                        "Databricks: row {} has {} values for {width} columns",
                        ri + 1,
                        r.len()
                    ),
                ));
            }
            let mut out = Vec::with_capacity(width);
            for (v, c) in r.iter().zip(&a.columns) {
                out.push(types::value(v, c.ty).map_err(|e| {
                    ConnectError::new(
                        ConnectErrorKind::DataSource,
                        format!("Databricks: column `{}`, row {}: {e}", c.name, ri + 1),
                    )
                })?);
            }
            rows.push(out);
        }
        Ok(RemoteRows {
            columns: a.columns,
            rows,
        })
    }

    fn native(&mut self, q: &RemoteQuery) -> Result<Option<String>, ConnectError> {
        if q.table.len() != 3 {
            return Ok(None);
        }
        Ok(Some(sql::native_text(&sql::select(q))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_and_paths() {
        assert_eq!(
            parse_host("dbc-1a2b.cloud.databricks.com").unwrap(),
            "dbc-1a2b.cloud.databricks.com"
        );
        assert_eq!(
            parse_host(" https://ADB-1.2.azuredatabricks.net/ ").unwrap(),
            "adb-1.2.azuredatabricks.net"
        );
        assert_eq!(parse_host("host.example:8443").unwrap(), "host.example:8443");
        for bad in [
            "",
            "a/b",
            "user@host",
            "http://host",
            "h..x",
            "-a.b",
            "a.b:",
            "a.b:123456",
            "a b",
        ] {
            assert!(parse_host(bad).is_err(), "{bad}");
        }
        assert_eq!(
            parse_warehouse("/sql/1.0/warehouses/abc123def").unwrap(),
            "abc123def"
        );
        assert_eq!(parse_warehouse("/sql/1.0/endpoints/abc").unwrap(), "abc");
        let e = parse_warehouse("/sql/protocolv1/o/123/0123-456-abc").unwrap_err();
        assert!(e.contains("cluster"), "{e}");
        assert!(parse_warehouse("/sql/1.0/warehouses/../x").is_err());
    }

    #[test]
    fn token_is_hidden() {
        let t = Token::new(" dapi-secret ").unwrap();
        assert_eq!(format!("{t:?}"), "Token(***)");
        assert!(Token::new("  ").is_none());
    }
}
