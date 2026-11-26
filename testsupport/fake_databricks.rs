//! Test-only fake Databricks SQL warehouse: a local HTTP/1.1 server answering the SQL Statement
//! Execution API (`/api/2.0/sql/statements`) in the shapes Databricks documents publicly
//! (statement states, manifests, inline chunks, external links, errors).
//!
//! Not a crate: tests include it with `#[path = "../../../testsupport/fake_databricks.rs"]`.
//! It understands the statements `bicraft-databricks` writes: `SHOW CATALOGS`, `SHOW SCHEMAS
//! IN`, the `information_schema.tables` listing, `SELECT * … LIMIT 0`, and `SELECT <columns> FROM
//! <table> [WHERE …] [LIMIT n]` (a `WHERE` clause is answered from [`Warehouse::filtered`], keyed
//! by the clause text and its parameter values). Every request is recorded.
#![allow(dead_code)]

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

/// One recorded request.
#[derive(Debug, Clone)]
pub struct Req {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Req {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }
}

/// A table of the fake warehouse: columns `(name, type_name, type_text)` and rows of JSON cells.
#[derive(Debug, Clone)]
pub struct FakeTable {
    pub columns: Vec<(String, String, String)>,
    pub rows: Vec<Vec<Value>>,
    pub kind: String,
}

impl FakeTable {
    pub fn new(columns: &[(&str, &str, &str)], rows: Vec<Vec<Value>>) -> Self {
        Self {
            columns: columns
                .iter()
                .map(|(a, b, c)| (a.to_string(), b.to_string(), c.to_string()))
                .collect(),
            rows,
            kind: "MANAGED".into(),
        }
    }
}

/// What the fake warehouse holds and how it behaves.
#[derive(Debug, Clone, Default)]
pub struct Warehouse {
    /// The accepted bearer token (others get HTTP 401).
    pub token: String,
    pub warehouse_id: String,
    /// `catalog.schema.table` → table.
    pub tables: HashMap<String, FakeTable>,
    /// Polls answered `PENDING`/`RUNNING` before `SUCCEEDED`.
    pub pending_polls: usize,
    /// Rows per result chunk (0: one chunk).
    pub chunk_rows: usize,
    /// The first requests answered HTTP 429.
    pub throttle_first: usize,
    /// A statement containing this text fails with `(error_code, message)`.
    pub fail: Option<(String, String, String)>,
    /// `WHERE` clause (as written, `:pN` markers included) + `|` + parameter values joined by
    /// `|` → the row indexes it keeps.
    pub filtered: HashMap<String, Vec<usize>>,
    /// Statements that never finish (always `RUNNING`).
    pub never_finish: bool,
    /// Report `truncated: true` in manifests.
    pub truncated: bool,
}

struct Stmt {
    polls_left: usize,
    columns: Vec<(String, String, String)>,
    rows: Vec<Vec<Value>>,
    links: bool,
    failed: Option<(String, String)>,
}

struct State {
    wh: Warehouse,
    stmts: HashMap<String, Stmt>,
    next_id: usize,
    served: usize,
}

pub struct FakeServer {
    pub base: String,
    pub requests: Arc<Mutex<Vec<Req>>>,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    port: u16,
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
    }
}

static IDS: AtomicUsize = AtomicUsize::new(1);

impl FakeServer {
    pub fn start(wh: Warehouse) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let state = Arc::new(Mutex::new(State {
            wh,
            stmts: HashMap::new(),
            next_id: IDS.fetch_add(1000, Ordering::SeqCst),
            served: 0,
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let (r, s, st) = (Arc::clone(&requests), Arc::clone(&state), Arc::clone(&stop));
        let base = format!("http://127.0.0.1:{port}");
        let b = base.clone();
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                if st.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(conn) = conn else { continue };
                let (r, s, b) = (Arc::clone(&r), Arc::clone(&s), b.clone());
                std::thread::spawn(move || serve(conn, &r, &s, &b));
            }
        });
        Self {
            base,
            requests,
            state,
            stop,
            port,
        }
    }

    pub fn requests(&self) -> Vec<Req> {
        self.requests.lock().unwrap().clone()
    }

    /// The SQL texts of the statements submitted, in order.
    pub fn statements(&self) -> Vec<String> {
        self.requests()
            .iter()
            .filter(|r| r.method == "POST" && r.path == "/api/2.0/sql/statements/")
            .filter_map(|r| {
                r.json()
                    .get("statement")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect()
    }

    pub fn set(&self, f: impl FnOnce(&mut Warehouse)) {
        f(&mut self.state.lock().unwrap().wh);
    }
}

fn serve(conn: TcpStream, requests: &Mutex<Vec<Req>>, state: &Mutex<State>, base: &str) {
    let mut reader = BufReader::new(conn.try_clone().unwrap());
    let mut line = String::new();
    if reader.read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();
    let mut headers = Vec::new();
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    let len: usize = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; len];
    let _ = reader.read_exact(&mut body);
    let req = Req {
        method,
        path,
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    };
    requests.lock().unwrap().push(req.clone());
    let (status, extra, body) = respond(&req, state, base);
    let reason = match status {
        200 => "OK",
        401 => "Unauthorized",
        404 => "Not Found",
        429 => "Too Many Requests",
        _ => "Status",
    };
    let mut out = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (k, v) in extra {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    out.push_str("\r\n");
    out.push_str(&body);
    let mut conn = conn;
    let _ = conn.write_all(out.as_bytes());
    let _ = conn.flush();
}

fn api_error(status: u16, code: &str, message: &str) -> (u16, Vec<(String, String)>, String) {
    (
        status,
        vec![],
        json!({"error_code": code, "message": message}).to_string(),
    )
}

fn ident_path(s: &str) -> String {
    // `a`.`b`.`c` → a.b.c (backticks doubled inside a name)
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    let mut inside = false;
    while let Some(c) = chars.next() {
        match c {
            '`' if inside && chars.peek() == Some(&'`') => {
                chars.next();
                out.push('`');
            }
            '`' => inside = !inside,
            c => out.push(c),
        }
    }
    out
}

fn respond(req: &Req, state: &Mutex<State>, base: &str) -> (u16, Vec<(String, String)>, String) {
    let mut st = state.lock().unwrap();
    st.served += 1;
    if req.path.starts_with("/ext/") {
        // presigned link: no credentials expected
        let mut it = req.path.trim_start_matches("/ext/").split('/');
        let id = it.next().unwrap_or("").to_string();
        let chunk: usize = it
            .next()
            .and_then(|c| c.split('?').next()?.parse().ok())
            .unwrap_or(0);
        let size = st.wh.chunk_rows;
        let Some(s) = st.stmts.get(&id) else {
            return (404, vec![], "[]".into());
        };
        let rows = chunk_of(&s.rows, size, chunk);
        return (
            200,
            vec![],
            Value::Array(rows.into_iter().map(Value::Array).collect()).to_string(),
        );
    }
    if st.served <= st.wh.throttle_first {
        return (
            429,
            vec![("Retry-After".into(), "0".into())],
            json!({"error_code": "TOO_MANY_REQUESTS", "message": "slow down"}).to_string(),
        );
    }
    let expected = format!("Bearer {}", st.wh.token);
    if req.header("authorization") != Some(expected.as_str()) {
        return api_error(401, "UNAUTHENTICATED", "Invalid access token.");
    }
    let path = req.path.as_str();
    if req.method == "POST" && path == "/api/2.0/sql/statements/" {
        let b = req.json();
        if b["warehouse_id"].as_str() != Some(st.wh.warehouse_id.as_str()) {
            return api_error(404, "RESOURCE_DOES_NOT_EXIST", "No warehouse with this id.");
        }
        let sql = b["statement"].as_str().unwrap_or("").to_string();
        let params: Vec<String> = b["parameters"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|p| p["value"].as_str().unwrap_or("").to_string())
                    .collect()
            })
            .unwrap_or_default();
        let links = b["disposition"].as_str() == Some("EXTERNAL_LINKS");
        let id = format!("stmt-{}", st.next_id);
        st.next_id += 1;
        let (columns, rows, failed) = run_sql(&st.wh, &sql, &params);
        let stmt = Stmt {
            polls_left: if st.wh.never_finish {
                usize::MAX
            } else {
                st.wh.pending_polls
            },
            columns,
            rows,
            links,
            failed,
        };
        st.stmts.insert(id.clone(), stmt);
        let wh = st.wh.clone();
        return (
            200,
            vec![],
            statement_json(&id, st.stmts.get_mut(&id).unwrap(), &wh, base),
        );
    }
    if let Some(rest) = path.strip_prefix("/api/2.0/sql/statements/") {
        let parts: Vec<&str> = rest.split('/').collect();
        let wh = st.wh.clone();
        match (req.method.as_str(), parts.as_slice()) {
            ("GET", [id]) => match st.stmts.get_mut(*id) {
                Some(s) => {
                    s.polls_left = s.polls_left.saturating_sub(1);
                    return (200, vec![], statement_json(id, s, &wh, base));
                }
                None => return api_error(404, "RESOURCE_DOES_NOT_EXIST", "no statement"),
            },
            ("GET", [id, "result", "chunks", n]) => {
                let n: usize = n.parse().unwrap_or(0);
                let Some(s) = st.stmts.get(*id) else {
                    return api_error(404, "RESOURCE_DOES_NOT_EXIST", "no statement");
                };
                return (200, vec![], chunk_json(id, s, &wh, n, base).to_string());
            }
            ("POST", [id, "cancel"]) => {
                if let Some(s) = st.stmts.get_mut(*id) {
                    s.failed = Some(("CANCELED".into(), String::new()));
                }
                return (200, vec![], "{}".into());
            }
            _ => {}
        }
    }
    api_error(404, "ENDPOINT_NOT_FOUND", "no such endpoint")
}

type Cols = Vec<(String, String, String)>;

fn run_sql(
    wh: &Warehouse,
    sql: &str,
    params: &[String],
) -> (Cols, Vec<Vec<Value>>, Option<(String, String)>) {
    if let Some((needle, code, msg)) = &wh.fail
        && sql.contains(needle.as_str())
    {
        return (vec![], vec![], Some((code.clone(), msg.clone())));
    }
    let col = |n: &str, t: &str, x: &str| (n.to_string(), t.to_string(), x.to_string());
    if sql == "SHOW CATALOGS" {
        let mut cats: Vec<String> = wh
            .tables
            .keys()
            .filter_map(|k| k.split('.').next().map(str::to_string))
            .collect();
        cats.sort();
        cats.dedup();
        return (
            vec![col("catalog", "STRING", "string")],
            cats.into_iter().map(|c| vec![json!(c)]).collect(),
            None,
        );
    }
    if let Some(c) = sql.strip_prefix("SHOW SCHEMAS IN ") {
        let c = ident_path(c);
        let mut s: Vec<String> = wh
            .tables
            .keys()
            .filter_map(|k| {
                let mut p = k.split('.');
                (p.next()? == c).then(|| p.next().map(str::to_string))?
            })
            .collect();
        s.sort();
        s.dedup();
        return (
            vec![col("databaseName", "STRING", "string")],
            s.into_iter().map(|x| vec![json!(x)]).collect(),
            None,
        );
    }
    if sql.contains("information_schema.tables") {
        let cat = ident_path(
            sql.split(" FROM ")
                .nth(1)
                .and_then(|r| r.split(".information_schema").next())
                .unwrap_or(""),
        );
        let schema = params.first().cloned().unwrap_or_default();
        let mut rows: Vec<(String, String)> = wh
            .tables
            .iter()
            .filter_map(|(k, t)| {
                let p: Vec<&str> = k.split('.').collect();
                (p.len() == 3 && p[0] == cat && p[1] == schema).then(|| (p[2].to_string(), t.kind.clone()))
            })
            .collect();
        rows.sort();
        return (
            vec![
                col("table_name", "STRING", "string"),
                col("table_type", "STRING", "string"),
            ],
            rows.into_iter().map(|(n, k)| vec![json!(n), json!(k)]).collect(),
            None,
        );
    }
    if let Some(rest) = sql.strip_prefix("SHOW TABLES IN ") {
        let want = ident_path(rest);
        let mut rows: Vec<Vec<Value>> = wh
            .tables
            .keys()
            .filter_map(|k| {
                let (cs, t) = k.rsplit_once('.')?;
                (cs == want).then(|| vec![json!(cs.split('.').nth(1)), json!(t), json!("false")])
            })
            .collect();
        rows.sort_by(|a, b| a[1].as_str().cmp(&b[1].as_str()));
        return (
            vec![
                col("database", "STRING", "string"),
                col("tableName", "STRING", "string"),
                col("isTemporary", "BOOLEAN", "boolean"),
            ],
            rows,
            None,
        );
    }
    if let Some(rest) = sql.strip_prefix("SELECT ") {
        let (cols, rest) = rest.split_once(" FROM ").unwrap_or(("*", ""));
        let (table, mut tail) = match rest.find(' ') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, ""),
        };
        let name = ident_path(table);
        let Some(t) = wh.tables.get(&name) else {
            return (
                vec![],
                vec![],
                Some((
                    "TABLE_OR_VIEW_NOT_FOUND".into(),
                    format!("[TABLE_OR_VIEW_NOT_FOUND] The table or view `{name}` cannot be found."),
                )),
            );
        };
        let mut limit = None;
        if let Some(i) = tail.find(" LIMIT ") {
            limit = tail[i + 7..].trim().parse::<usize>().ok();
            tail = &tail[..i];
        }
        let mut keep: Vec<usize> = (0..t.rows.len()).collect();
        if let Some(w) = tail.strip_prefix(" WHERE ") {
            let key = format!("{w}|{}", params.join("|"));
            match wh.filtered.get(&key) {
                Some(k) => keep = k.clone(),
                None => {
                    return (
                        vec![],
                        vec![],
                        Some((
                            "UNKNOWN_FILTER".into(),
                            format!("fake: no rows scripted for {key}"),
                        )),
                    );
                }
            }
        }
        if let Some(n) = limit {
            keep.truncate(n);
        }
        let idx: Vec<usize> = if cols == "*" {
            (0..t.columns.len()).collect()
        } else {
            let mut v = Vec::new();
            for c in cols.split(", ") {
                let n = ident_path(c);
                match t.columns.iter().position(|(x, _, _)| x.eq_ignore_ascii_case(&n)) {
                    Some(i) => v.push(i),
                    None => {
                        return (
                            vec![],
                            vec![],
                            Some((
                                "UNRESOLVED_COLUMN".into(),
                                format!("[UNRESOLVED_COLUMN] `{n}` cannot be resolved."),
                            )),
                        );
                    }
                }
            }
            v
        };
        let columns = idx.iter().map(|&i| t.columns[i].clone()).collect();
        let rows = keep
            .iter()
            .filter_map(|&r| t.rows.get(r))
            .map(|r| idx.iter().map(|&i| r[i].clone()).collect())
            .collect();
        return (columns, rows, None);
    }
    (
        vec![],
        vec![],
        Some(("PARSE_SYNTAX_ERROR".into(), format!("fake cannot run {sql}"))),
    )
}

fn chunk_of(rows: &[Vec<Value>], size: usize, n: usize) -> Vec<Vec<Value>> {
    if size == 0 {
        return if n == 0 { rows.to_vec() } else { vec![] };
    }
    rows.iter().skip(n * size).take(size).cloned().collect()
}

fn chunk_count(rows: usize, size: usize) -> usize {
    if size == 0 || rows == 0 {
        1
    } else {
        rows.div_ceil(size)
    }
}

fn chunk_json(id: &str, s: &Stmt, wh: &Warehouse, n: usize, base: &str) -> Value {
    let total = chunk_count(s.rows.len(), wh.chunk_rows);
    let rows = chunk_of(&s.rows, wh.chunk_rows, n);
    let offset = if wh.chunk_rows == 0 { 0 } else { n * wh.chunk_rows };
    let mut c = json!({"chunk_index": n, "row_offset": offset, "row_count": rows.len()});
    let next = (n + 1 < total).then_some(n + 1);
    if s.links {
        let mut link = json!({
            "chunk_index": n, "row_offset": offset, "row_count": rows.len(),
            "external_link": format!("{base}/ext/{id}/{n}?X-Amz-Signature=secret-signature"),
            "expiration": "2026-10-23T09:00:00.000Z",
            "http_headers": {"x-ms-blob-type": "BlockBlob"},
        });
        if let Some(nx) = next {
            link["next_chunk_index"] = json!(nx);
            link["next_chunk_internal_link"] =
                json!(format!("/api/2.0/sql/statements/{id}/result/chunks/{nx}"));
        }
        c = json!({"external_links": [link]});
    } else {
        c["data_array"] = Value::Array(rows.into_iter().map(Value::Array).collect());
        if let Some(nx) = next {
            c["next_chunk_index"] = json!(nx);
            c["next_chunk_internal_link"] = json!(format!("/api/2.0/sql/statements/{id}/result/chunks/{nx}"));
        }
    }
    c
}

fn statement_json(id: &str, s: &mut Stmt, wh: &Warehouse, base: &str) -> String {
    if let Some((code, msg)) = &s.failed {
        let state = if code == "CANCELED" { "CANCELED" } else { "FAILED" };
        return json!({"statement_id": id, "status": {"state": state, "error": {"error_code": code, "message": msg}}}).to_string();
    }
    if s.polls_left > 0 {
        let state = if s.polls_left.is_multiple_of(2) {
            "PENDING"
        } else {
            "RUNNING"
        };
        return json!({"statement_id": id, "status": {"state": state}}).to_string();
    }
    let total = chunk_count(s.rows.len(), wh.chunk_rows);
    let columns: Vec<Value> = s
        .columns
        .iter()
        .enumerate()
        .map(|(i, (n, t, x))| json!({"name": n, "type_name": t, "type_text": x, "position": i}))
        .collect();
    let chunks: Vec<Value> = (0..total)
        .map(|n| json!({"chunk_index": n, "row_count": chunk_of(&s.rows, wh.chunk_rows, n).len()}))
        .collect();
    let mut v = json!({
        "statement_id": id,
        "status": {"state": "SUCCEEDED"},
        "manifest": {
            "format": "JSON_ARRAY",
            "schema": {"column_count": columns.len(), "columns": columns},
            "total_chunk_count": total,
            "chunks": chunks,
            "total_row_count": s.rows.len(),
            "truncated": wh.truncated,
        },
    });
    if !s.rows.is_empty() || s.links {
        v["result"] = chunk_json(id, s, wh, 0, base);
    } else {
        v["result"] = json!({"chunk_index": 0, "row_offset": 0, "row_count": 0});
    }
    v.to_string()
}
