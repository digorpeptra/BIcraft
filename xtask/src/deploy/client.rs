//! A minimal HTTP/1.1 client for the deployment smoke tests: one request per connection
//! (`bicraft-cli serve` answers `Connection: close`), MCP Streamable HTTP at revision 2026-07-28, and
//! the checks both smoke tests run against a live server.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// The MCP revision `bicraft-cli serve` speaks over HTTP.
pub const PROTOCOL: &str = "2026-07-28";
/// How long one request may take (a DAX query on the tiny fixture takes milliseconds; through the
/// controller, the first request of an idle workspace also waits for its pod to start).
const TIMEOUT: Duration = Duration::from_secs(240);
/// Responses larger than this are refused (the fixture's answers are a few KiB).
const MAX_RESPONSE: u64 = 16 << 20;

/// Where the server is and how to talk to it.
#[derive(Clone, Debug)]
pub struct Endpoint {
    pub addr: SocketAddr,
    /// The `Host` header: one of the names the server answers.
    pub host: String,
    /// An API token (`Authorization: Bearer`).
    pub token: String,
    pub workspace: String,
}

#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// The JSON-RPC message: the body, or the last `data:` event of an SSE stream.
    pub fn message(&self) -> Result<Value, String> {
        let text = self.text();
        let is_sse = self
            .header("content-type")
            .is_some_and(|t| t.starts_with("text/event-stream"));
        let json = if is_sse {
            text.lines()
                .filter_map(|l| l.strip_prefix("data:"))
                .map(str::trim)
                .next_back()
                .ok_or_else(|| format!("SSE stream without a data event: {text}"))?
                .to_owned()
        } else {
            text
        };
        serde_json::from_str(&json).map_err(|e| format!("not JSON ({e}): {json}"))
    }
}

pub fn connect(addr: SocketAddr) -> Result<TcpStream, String> {
    let s = TcpStream::connect_timeout(&addr, Duration::from_secs(10))
        .map_err(|e| format!("connect {addr}: {e}"))?;
    s.set_read_timeout(Some(TIMEOUT)).map_err(|e| e.to_string())?;
    s.set_write_timeout(Some(TIMEOUT)).map_err(|e| e.to_string())?;
    Ok(s)
}

/// Reads a whole response (the server closes the connection after it).
pub fn read_response(mut s: TcpStream) -> Result<Response, String> {
    let mut raw = Vec::new();
    let start = Instant::now();
    let mut buf = [0u8; 16 * 1024];
    loop {
        match s.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                raw.extend_from_slice(buf.get(..n).unwrap_or_default());
                if raw.len() as u64 > MAX_RESPONSE {
                    return Err("response too large".into());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            // A reset after the whole response arrived is fine.
            Err(_) if !raw.is_empty() => break,
            Err(e) => return Err(format!("read: {e}")),
        }
        if start.elapsed() > TIMEOUT {
            return Err("response timed out".into());
        }
    }
    parse_response(&raw)
}

fn parse_response(raw: &[u8]) -> Result<Response, String> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| format!("no response head in {} bytes", raw.len()))?;
    let head = String::from_utf8_lossy(raw.get(..split).unwrap_or_default()).into_owned();
    let body = raw.get(split + 4..).unwrap_or_default().to_vec();
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or_default();
    let status = status_line
        .split(' ')
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| format!("bad status line `{status_line}`"))?;
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .collect();
    Ok(Response {
        status,
        headers,
        body,
    })
}

/// The head of a request (without the blank line ending it).
pub fn head(
    ep: &Endpoint,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    len: usize,
    auth: bool,
) -> String {
    let mut h = format!(
        "{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n",
        ep.host
    );
    if auth {
        h.push_str(&format!("Authorization: Bearer {}\r\n", ep.token));
    }
    for (k, v) in headers {
        h.push_str(&format!("{k}: {v}\r\n"));
    }
    h.push_str(&format!("Content-Length: {len}\r\n"));
    h
}

pub fn request(
    ep: &Endpoint,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
    auth: bool,
) -> Result<Response, String> {
    let mut s = connect(ep.addr)?;
    let h = head(ep, method, path, headers, body.len(), auth);
    s.write_all(format!("{h}\r\n").as_bytes())
        .and_then(|()| s.write_all(body))
        .map_err(|e| format!("{method} {path}: write: {e}"))?;
    read_response(s)
}

/// A JSON-RPC request to the workspace's MCP endpoint with the required `_meta` and mirrored
/// headers.
pub fn mcp(
    ep: &Endpoint,
    method: &str,
    mut params: Value,
    sse: bool,
    auth: bool,
) -> Result<Response, String> {
    if let Some(p) = params.as_object_mut() {
        p.insert(
            "_meta".into(),
            json!({
                "io.modelcontextprotocol/protocolVersion": PROTOCOL,
                "io.modelcontextprotocol/clientCapabilities": {},
                "io.modelcontextprotocol/clientInfo": {"name": "bicraft-deploy-smoke", "version": "1"}
            }),
        );
    }
    let name = params.get("name").and_then(Value::as_str).map(str::to_owned);
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string();
    let accept = if sse {
        "application/json, text/event-stream"
    } else {
        "application/json"
    };
    let mut headers = vec![
        ("Content-Type", "application/json"),
        ("Accept", accept),
        ("MCP-Protocol-Version", PROTOCOL),
        ("Mcp-Method", method),
    ];
    if let Some(n) = &name {
        headers.push(("Mcp-Name", n.as_str()));
    }
    let path = format!("/w/{}/mcp", ep.workspace);
    request(ep, "POST", &path, &headers, body.as_bytes(), auth)
}

/// `tools/call`: the tool's result object (fails on HTTP or JSON-RPC errors, not on `isError`).
pub fn call(ep: &Endpoint, tool: &str, args: Value, sse: bool) -> Result<Value, String> {
    let r = mcp(
        ep,
        "tools/call",
        json!({"name": tool, "arguments": args}),
        sse,
        true,
    )?;
    if r.status != 200 {
        return Err(format!("tools/call {tool}: HTTP {}: {}", r.status, r.text()));
    }
    let m = r.message()?;
    m.get("result")
        .cloned()
        .ok_or_else(|| format!("tools/call {tool}: no result: {m}"))
}

fn tool_text(result: &Value) -> String {
    result
        .pointer("/content/0/text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// What the checks did, one line each.
pub type Log = Vec<String>;

fn expect(cond: bool, what: &str, detail: impl FnOnce() -> String) -> Result<(), String> {
    if cond {
        Ok(())
    } else {
        Err(format!("{what}: {}", detail()))
    }
}

/// The DAX query of the smoke tests and the value it must return on the fixture.
pub const QUERY: &str = "EVALUATE ROW(\"Total\", SUM(Sales[Amount]))";
pub const EXPECTED_TOTAL: f64 = 42.0;

/// The checks both smoke tests run against a live server whose workspace is empty: probes,
/// authentication and Host checks, discovery and the tool list, a zipped project uploaded and
/// listed, `file.open`, one DAX query (over SSE), and a sandbox refusal.
pub fn checks(ep: &Endpoint, fixture_zip: &[u8], fixture_file: &str) -> Result<Log, String> {
    let mut log = Log::new();
    let mut step = |s: String| {
        eprintln!("deploy smoke: ok  {s}");
        log.push(s);
    };
    let t0 = Instant::now();

    for probe in ["/healthz", "/readyz"] {
        let r = request(ep, "GET", probe, &[], b"", false)?;
        expect(r.status == 200 && r.text().trim() == "ok", probe, || {
            format!("HTTP {} {}", r.status, r.text())
        })?;
        step(format!("GET {probe} -> 200 ok (no credentials)"));
    }

    let r = mcp(ep, "server/discover", json!({}), false, false)?;
    expect(
        r.status == 401 && r.header("www-authenticate").is_some(),
        "no token",
        || format!("HTTP {} {}", r.status, r.text()),
    )?;
    step("server/discover without a token -> 401 with WWW-Authenticate".into());

    let mut evil = ep.clone();
    evil.host = "attacker.example".into();
    let r = mcp(&evil, "server/discover", json!({}), false, true)?;
    expect(r.status == 403, "foreign Host", || {
        format!("HTTP {} {}", r.status, r.text())
    })?;
    step("a Host the server does not answer -> 403".into());

    let r = mcp(ep, "server/discover", json!({}), false, true)?;
    expect(r.status == 200, "server/discover", || {
        format!("HTTP {} {}", r.status, r.text())
    })?;
    let m = r.message()?;
    let versions = m
        .pointer("/result/supportedVersions")
        .cloned()
        .unwrap_or_default();
    expect(versions == json!([PROTOCOL]), "supportedVersions", || {
        versions.to_string()
    })?;
    let server = m
        .pointer("/result/_meta/io.modelcontextprotocol~1serverInfo/version")
        .and_then(Value::as_str)
        .unwrap_or("?")
        .to_owned();
    step(format!(
        "server/discover -> 200, supportedVersions [{PROTOCOL}], server {server}"
    ));

    let r = mcp(ep, "tools/list", json!({}), false, true)?;
    expect(r.status == 200, "tools/list", || {
        format!("HTTP {} {}", r.status, r.text())
    })?;
    let m = r.message()?;
    let names: Vec<String> = m
        .pointer("/result/tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|t| t.get("name").and_then(Value::as_str).map(str::to_owned))
        .collect();
    for needed in ["open_file", "evaluate_dax_query", "run_command"] {
        expect(names.iter().any(|n| n == needed), "tools/list", || {
            format!("{needed} missing from {names:?}")
        })?;
    }
    step(format!(
        "tools/list -> {} tools (open_file, evaluate_dax_query, run_command)",
        names.len()
    ));

    let files = format!("/w/{}/files/", ep.workspace);
    let r = request(
        ep,
        "POST",
        &files,
        &[("Content-Type", "application/zip")],
        fixture_zip,
        true,
    )?;
    expect(r.status == 201, "zip upload", || {
        format!("HTTP {} {}", r.status, r.text())
    })?;
    let up: Value = serde_json::from_slice(&r.body).map_err(|e| format!("zip upload: {e}"))?;
    step(format!(
        "POST {files} (zipped PBIP project, {} bytes) -> 201, {} files extracted",
        fixture_zip.len(),
        up.get("files").cloned().unwrap_or_default()
    ));

    let r = request(ep, "GET", &files, &[], b"", true)?;
    expect(
        r.status == 200 && r.text().contains(fixture_file),
        "listing",
        || format!("HTTP {} {}", r.status, r.text()),
    )?;
    step(format!("GET {files} -> 200, lists {fixture_file}"));

    let opened = call(ep, "open_file", json!({"path": fixture_file}), false)?;
    expect(
        opened.get("isError") == Some(&Value::Bool(false)),
        "open_file",
        || opened.to_string(),
    )?;
    step(format!("tools/call open_file {{path: {fixture_file}}} -> opened"));

    let q = call(ep, "evaluate_dax_query", json!({"query": QUERY}), true)?;
    expect(
        q.get("isError") == Some(&Value::Bool(false)),
        "evaluate_dax_query",
        || q.to_string(),
    )?;
    let answer: Value = serde_json::from_str(&tool_text(&q)).map_err(|e| format!("DAX answer: {e}"))?;
    let total = answer
        .pointer("/results/0/rows/0")
        .and_then(Value::as_object)
        .and_then(|row| row.values().find_map(Value::as_f64));
    expect(total == Some(EXPECTED_TOTAL), "DAX answer", || answer.to_string())?;
    step(format!(
        "tools/call evaluate_dax_query over SSE: {QUERY} -> {EXPECTED_TOTAL}"
    ));

    let escaped = call(ep, "open_file", json!({"path": "../outside.pbix"}), false)?;
    expect(
        escaped.get("isError") == Some(&Value::Bool(true)),
        "sandbox",
        || escaped.to_string(),
    )?;
    step("tools/call open_file {path: ../outside.pbix} -> refused by the workspace sandbox".into());

    step(format!("all live checks in {} ms", t0.elapsed().as_millis()));
    Ok(log)
}

/// The page's value for meta tag `name`.
fn meta(html: &str, name: &str) -> Option<String> {
    let needle = format!(r#"<meta name="{name}" content=""#);
    let at = html.find(&needle)? + needle.len();
    html.get(at..)?.split('"').next().map(str::to_owned)
}

/// A viewer page: its private view id and interaction token.
struct Page {
    vid: String,
    token: String,
}

fn view_page(ep: &Endpoint, extra: &[(&str, &str)], auth: bool) -> Result<Page, String> {
    let path = format!("/w/{}/view/", ep.workspace);
    let r = request(ep, "GET", &path, extra, b"", auth)?;
    expect(r.status == 200, "viewer page", || {
        format!("HTTP {} {}", r.status, r.text())
    })?;
    let html = r.text();
    let vid = meta(&html, "bicraft-view-id").filter(|v| v.len() == 32);
    let token = meta(&html, "bicraft-interact-token").filter(|v| v.len() == 32);
    match (vid, token) {
        (Some(vid), Some(token)) => Ok(Page { vid, token }),
        _ => Err("viewer page without a view id or interaction token".into()),
    }
}

/// The first value of visual `visual` in the page's snapshot (waiting out visuals without data),
/// and the session that served it.
fn view_value(
    ep: &Endpoint,
    extra: &[(&str, &str)],
    auth: bool,
    p: &Page,
    visual: &str,
) -> Result<(Value, String), String> {
    let path = format!("/w/{}/view/api/state?v={}", ep.workspace, p.vid);
    let start = Instant::now();
    loop {
        let r = request(ep, "GET", &path, extra, b"", auth)?;
        expect(r.status == 200, "viewer state", || {
            format!("HTTP {} {}", r.status, r.text())
        })?;
        let snap: Value = serde_json::from_slice(&r.body).map_err(|e| format!("viewer state: {e}"))?;
        let v = snap
            .pointer("/report/pages")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .flat_map(|p| {
                p.get("visuals")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
            })
            .find(|v| v.get("name").and_then(Value::as_str) == Some(visual))
            .ok_or_else(|| format!("visual {visual} not in the viewer's snapshot"))?;
        if let Some(x) = v.pointer("/data/rows/0/0") {
            let served = snap
                .pointer("/view/session")
                .and_then(Value::as_str)
                .unwrap_or("?");
            return Ok((x.clone(), served.to_owned()));
        }
        if start.elapsed() > Duration::from_secs(30) {
            return Err(format!("visual {visual} got no data in 30 s"));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn run_ok(ep: &Endpoint, command: &str, params: Value) -> Result<Value, String> {
    let r = call(
        ep,
        "run_command",
        json!({"command": command, "params": params}),
        false,
    )?;
    expect(r.get("isError") == Some(&Value::Bool(false)), command, || {
        r.to_string()
    })?;
    Ok(r.get("structuredContent").cloned().unwrap_or_default())
}

/// The live viewer (docs/kubernetes.md, "Viewer"), after [`checks`] opened the fixture as the
/// editor (`ep`): a page with a slicer and a card is added and saved; the page and one data call
/// as a reader (`reader_token`) and as the editor; a reader's slicer changes that view only (not
/// the reader's second page, not the editor's); the CSRF check; the one-time browser login.
pub fn viewer_checks(ep: &Endpoint, reader_token: &str, login: bool) -> Result<Log, String> {
    let mut log = Log::new();
    let mut step = |s: String| {
        eprintln!("deploy smoke: ok  {s}");
        log.push(s);
    };
    let reader = Endpoint {
        token: reader_token.to_owned(),
        ..ep.clone()
    };
    let has_report = call(ep, "run_command", json!({"command": "report.summary"}), false)?;
    if has_report.get("isError") == Some(&Value::Bool(true)) {
        run_ok(ep, "report.new", json!({}))?;
    }
    run_ok(ep, "page.add", json!({"display_name": "Viewer"}))?;
    run_ok(
        ep,
        "measure.add",
        json!({"table": "Sales", "name": "Viewer Total", "expression": "SUM(Sales[Amount])"}),
    )?;
    let add = |ty: &str, field: &str| -> Result<String, String> {
        let v = run_ok(
            ep,
            "visual.add",
            json!({"page": "Viewer", "visual_type": ty, "x": 0, "y": 0, "width": 200, "height": 120,
                   "fields": [{"role": "Values", "field": field}]}),
        )?;
        v.get("visual")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("visual.add {ty}: {v}"))
    };
    let slicer = add("slicer", "Sales[Region]")?;
    let card = add("card", "[Viewer Total]")?;
    run_ok(ep, "file.save", json!({}))?;
    step("the editor added a page with a slicer and a card, and saved".into());

    let page_path = format!("/w/{}/view/", ep.workspace);
    let r = request(ep, "GET", &page_path, &[], b"", false)?;
    expect(r.status == 401, "viewer without credentials", || {
        format!("HTTP {}", r.status)
    })?;
    let r1 = view_page(&reader, &[], true)?;
    let r2 = view_page(&reader, &[], true)?;
    let e1 = view_page(ep, &[], true)?;
    let (v, served) = view_value(&reader, &[], true, &r1, &card)?;
    expect(v.as_f64() == Some(EXPECTED_TOTAL), "reader card", || {
        v.to_string()
    })?;
    step(format!(
        "GET {page_path} and api/state as a reader -> 200, card {v} (served by the {served} session)"
    ));
    let (v, _) = view_value(ep, &[], true, &e1, &card)?;
    expect(v.as_f64() == Some(EXPECTED_TOTAL), "editor card", || {
        v.to_string()
    })?;
    step(format!(
        "GET {page_path} and api/state as the editor -> 200, card {v}"
    ));

    let interact = format!("/w/{}/view/api/interact?v={}", ep.workspace, r1.vid);
    let body =
        json!({"action": "slicer_toggle", "page": "Viewer", "visual": slicer, "value": "North"}).to_string();
    let r = request(
        &reader,
        "POST",
        &interact,
        &[
            ("Content-Type", "application/json"),
            ("Origin", "https://attacker.example"),
            ("X-Bicraft-Token", &r1.token),
        ],
        body.as_bytes(),
        true,
    )?;
    expect(r.status == 403, "foreign Origin", || {
        format!("HTTP {} {}", r.status, r.text())
    })?;
    let r = request(
        &reader,
        "POST",
        &interact,
        &[
            ("Content-Type", "application/json"),
            ("X-Bicraft-Token", &r1.token),
        ],
        body.as_bytes(),
        true,
    )?;
    expect(r.status == 200, "slicer click", || {
        format!("HTTP {} {}", r.status, r.text())
    })?;
    let (mine, _) = view_value(&reader, &[], true, &r1, &card)?;
    let (other, _) = view_value(&reader, &[], true, &r2, &card)?;
    let (editor, _) = view_value(ep, &[], true, &e1, &card)?;
    expect(mine.as_f64() == Some(10.0), "the clicked view", || {
        mine.to_string()
    })?;
    expect(
        other.as_f64() == Some(EXPECTED_TOTAL),
        "the reader's other view",
        || other.to_string(),
    )?;
    expect(
        editor.as_f64() == Some(EXPECTED_TOTAL),
        "the editor's view",
        || editor.to_string(),
    )?;
    step(format!(
        "a reader's slicer (North) -> that view {mine}; the reader's other page {other}, the editor's \
         {editor}; a foreign Origin -> 403"
    ));

    if !login {
        // behind the controller's gateway, which authenticates every request itself
        return Ok(log);
    }
    let login = format!("/w/{}/view/login", ep.workspace);
    let r = request(&reader, "POST", &login, &[], b"", true)?;
    expect(r.status == 200, "viewer login", || {
        format!("HTTP {} {}", r.status, r.text())
    })?;
    let answer: Value = serde_json::from_slice(&r.body).map_err(|e| format!("viewer login: {e}"))?;
    let url = answer
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let path = url
        .find("/w/")
        .and_then(|i| url.get(i..))
        .ok_or_else(|| format!("viewer login url {url}"))?;
    let r = request(ep, "GET", path, &[], b"", false)?;
    let cookie = r
        .header("set-cookie")
        .and_then(|c| c.split(';').next())
        .map(str::to_owned)
        .unwrap_or_default();
    expect(r.status == 303 && !cookie.is_empty(), "login redeem", || {
        format!("HTTP {}", r.status)
    })?;
    let p = view_page(ep, &[("Cookie", &cookie)], false)?;
    let (v, _) = view_value(ep, &[("Cookie", &cookie)], false, &p, &card)?;
    expect(v.as_f64() == Some(EXPECTED_TOTAL), "cookie card", || {
        v.to_string()
    })?;
    let again = request(ep, "GET", path, &[], b"", false)?;
    expect(again.status == 404, "a used login URL", || {
        format!("HTTP {}", again.status)
    })?;
    step(
        "browser login: one-time URL -> 303 with an HttpOnly cookie -> page and state; the URL works once"
            .into(),
    );
    Ok(log)
}

/// A query that runs for minutes on any model (1e10 row visits): only a cancel ends it.
const LONG_QUERY: &str =
    "EVALUATE ROW(\"x\", SUMX(GENERATESERIES(1, 100000), SUMX(GENERATESERIES(1, 100000), 1)))";

/// Concurrency on one workspace (`docs/web-deployment.md`, "Concurrency on one workspace"),
/// after [`checks`] opened the fixture: a read answers while a minutes-long read runs on the same
/// workspace (reads overlap), and a stale `expected_revision` is a `revision_conflict` that
/// changes nothing while the current one runs.
pub fn concurrency_checks(ep: &Endpoint) -> Result<Log, String> {
    let mut log = Log::new();
    let mut step = |s: String| {
        eprintln!("deploy smoke: ok  {s}");
        log.push(s);
    };
    // the long read, over SSE: its connection stays open until it is dropped (which cancels it)
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {
        "name": "evaluate_dax_query",
        "arguments": {"query": LONG_QUERY, "timeout_ms": 600_000},
        "_meta": {"io.modelcontextprotocol/protocolVersion": PROTOCOL,
                  "io.modelcontextprotocol/clientCapabilities": {}}}})
    .to_string();
    let path = format!("/w/{}/mcp", ep.workspace);
    let h = head(
        ep,
        "POST",
        &path,
        &[
            ("Content-Type", "application/json"),
            ("Accept", "application/json, text/event-stream"),
            ("MCP-Protocol-Version", PROTOCOL),
            ("Mcp-Method", "tools/call"),
            ("Mcp-Name", "evaluate_dax_query"),
        ],
        body.len(),
        true,
    );
    let mut long = connect(ep.addr)?;
    long.write_all(format!("{h}\r\n{body}").as_bytes())
        .map_err(|e| format!("long query: {e}"))?;
    // its SSE head arrives once it holds its queue place
    long.set_read_timeout(Some(TIMEOUT))
        .map_err(|e| format!("long query: {e}"))?;
    let mut first = [0u8; 12];
    long.read_exact(&mut first)
        .map_err(|e| format!("long query: no answer head: {e}"))?;
    expect(&first == b"HTTP/1.1 200", "long query", || {
        String::from_utf8_lossy(&first).into_owned()
    })?;
    std::thread::sleep(Duration::from_millis(300));
    let t = Instant::now();
    let q = call(ep, "evaluate_dax_query", json!({"query": QUERY}), false)?;
    expect(
        q.get("isError") == Some(&Value::Bool(false)),
        "a read beside the long one",
        || q.to_string(),
    )?;
    let took = t.elapsed();
    // the long one has not answered: it still runs (its event would follow the head)
    long.set_nonblocking(true)
        .map_err(|e| format!("long query: {e}"))?;
    let mut buf = [0u8; 4096];
    let mut seen = Vec::new();
    loop {
        match long.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => seen.extend_from_slice(buf.get(..n).unwrap_or_default()),
            Err(_) => break,
        }
    }
    let seen = String::from_utf8_lossy(&seen);
    expect(!seen.contains("data:"), "the long query", || {
        format!("answered already: {seen}")
    })?;
    drop(long);
    step(format!(
        "a read answered in {} ms while a minutes-long read ran on the same workspace (reads overlap)",
        took.as_millis()
    ));

    let rev = |r: &Value| r.pointer("/_meta/io.bicraft~1revision").and_then(Value::as_u64);
    let r = call(ep, "run_command", json!({"command": "session.revision"}), false)?;
    let now = r
        .pointer("/structuredContent/revision")
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("session.revision: {r}"))?;
    expect(rev(&r) == Some(now), "the revision in _meta", || r.to_string())?;
    let add = |expected: u64| {
        call(
            ep,
            "run_command",
            json!({"command": "measure.add", "expected_revision": expected,
                   "params": {"table": "Sales", "name": "Smoke conflict", "expression": "1"}}),
            false,
        )
    };
    let stale = add(now.saturating_sub(1))?;
    expect(
        stale.pointer("/structuredContent/code").and_then(Value::as_str) == Some("revision_conflict")
            && stale
                .pointer("/structuredContent/current_revision")
                .and_then(Value::as_u64)
                == Some(now),
        "a stale expected_revision",
        || stale.to_string(),
    )?;
    let r = call(ep, "run_command", json!({"command": "session.revision"}), false)?;
    expect(rev(&r) == Some(now), "a conflict changes nothing", || {
        r.to_string()
    })?;
    step(format!(
        "run_command measure.add with expected_revision {} -> revision_conflict (current {now}), nothing ran",
        now.saturating_sub(1)
    ));
    let ok = add(now)?;
    expect(
        ok.get("isError") == Some(&Value::Bool(false)) && rev(&ok) == Some(now + 1),
        "the current expected_revision",
        || ok.to_string(),
    )?;
    let removed = call(
        ep,
        "run_command",
        json!({"command": "measure.remove", "expected_revision": now + 1,
               "params": {"name": "Smoke conflict"}}),
        false,
    )?;
    expect(
        removed.get("isError") == Some(&Value::Bool(false)),
        "measure.remove",
        || removed.to_string(),
    )?;
    step(format!(
        "run_command measure.add / measure.remove with expected_revision {now} / {} -> ran (revision {})",
        now + 1,
        now + 2
    ));
    // the model is as before (the measure added and removed): saved, the session clean again
    run_ok(ep, "file.save", json!({}))?;
    Ok(log)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn responses_parse() {
        let r = parse_response(b"HTTP/1.1 201 Created\r\nContent-Type: application/json\r\n\r\n{\"a\":1}")
            .unwrap();
        assert_eq!(r.status, 201);
        assert_eq!(r.header("content-type"), Some("application/json"));
        assert_eq!(r.message().unwrap(), json!({"a": 1}));
        let sse = parse_response(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n: keepalive\n\ndata: {\"id\":1}\n\n",
        )
        .unwrap();
        assert_eq!(sse.message().unwrap(), json!({"id": 1}));
        assert!(parse_response(b"garbage").is_err());
        assert!(parse_response(b"HTTP/1.1 abc\r\n\r\n").is_err());
    }
}
