//! Protocol conformance of the MCP Streamable HTTP endpoint (revision 2026-07-28), over real
//! sockets: discovery, tools over JSON and SSE, notifications, methods, header mirroring, `_meta`,
//! `Origin`, body caps, malformed input, the per-workspace queue, cancellation by closing the
//! stream, and workspace concurrency.

#[path = "../../../testsupport/tempdir.rs"]
mod tempdir;

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use bicraft_mcp::http::{
    HEADER_MISMATCH, HTTP_PROTOCOL_VERSION, UNSUPPORTED_PROTOCOL_VERSION, WORKSPACE_BUSY,
};
use bicraft_mcp::{HttpConfig, HttpServer};
use serde_json::{Value, json};
use tempdir::TempDir;

const MODEL: &str = include_str!("../../engine/tests/table_functions/model.bim");
/// Minutes of evaluation without a budget (1e10 row visits): only a cancel or a timeout ends it.
const LONG: &str =
    "EVALUATE ROW(\"x\", SUMX(GENERATESERIES(1, 100000), SUMX(GENERATESERIES(1, 100000), [Value])))";
const SMALL: &str = "EVALUATE ROW(\"x\", SUMX(GENERATESERIES(1, 1000), [Value]))";

struct Fixture {
    dir: TempDir,
    server: HttpServer,
}

fn fixture(name: &str, config: HttpConfig) -> Fixture {
    let dir = TempDir::new(name);
    for ws in ["a", "b"] {
        std::fs::create_dir_all(dir.path().join(ws)).unwrap();
    }
    std::fs::write(dir.path().join("a").join("model.bim"), MODEL).unwrap();
    std::fs::write(dir.path().join("b").join("model.bim"), MODEL).unwrap();
    let server = HttpServer::start("127.0.0.1:0".parse().unwrap(), dir.path(), config).unwrap();
    Fixture { dir, server }
}

impl Fixture {
    fn addr(&self) -> SocketAddr {
        self.server.addr()
    }

    /// Opens the workspace's model (a JSON reply).
    fn open(&self, ws: &str) {
        // relative to the workspace folder: each workspace is a server-mode session rooted there
        let r = call(self.addr(), ws, "open_file", json!({"path": "model.bim"}), false);
        assert_eq!(r.status, 200, "{}", r.body);
        assert_eq!(r.json()["result"]["isError"], false, "{}", r.body);
    }
}

#[derive(Debug)]
struct Response {
    status: u16,
    head: String,
    body: String,
}

impl Response {
    fn header(&self, name: &str) -> Option<&str> {
        self.head.lines().skip(1).find_map(|l| {
            let (n, v) = l.split_once(':')?;
            n.trim().eq_ignore_ascii_case(name).then(|| v.trim())
        })
    }

    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|e| panic!("{e}: {}", self.body))
    }

    /// The JSON-RPC message of an SSE reply (its one `data:` event).
    fn sse_message(&self) -> Value {
        let data = self
            .body
            .lines()
            .find_map(|l| l.strip_prefix("data: "))
            .unwrap_or_else(|| panic!("no event in {}", self.body));
        serde_json::from_str(data).unwrap()
    }

    fn error_code(&self) -> i64 {
        self.json()["error"]["code"].as_i64().unwrap_or(0)
    }
}

fn raw(addr: SocketAddr, bytes: &[u8]) -> Response {
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
    s.write_all(bytes).unwrap();
    read_response(s)
}

fn read_response(mut s: TcpStream) -> Response {
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    let text = String::from_utf8_lossy(&out).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status = head.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    Response {
        status,
        head: head.to_string(),
        body: body.to_string(),
    }
}

fn request_bytes(ws: &str, headers: &[(&str, String)], body: &str) -> Vec<u8> {
    let mut req = format!(
        "POST /w/{ws}/mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (n, v) in headers {
        req.push_str(&format!("{n}: {v}\r\n"));
    }
    req.push_str("\r\n");
    req.push_str(body);
    req.into_bytes()
}

fn meta() -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": HTTP_PROTOCOL_VERSION,
        "io.modelcontextprotocol/clientInfo": {"name": "test", "version": "1"},
        "io.modelcontextprotocol/clientCapabilities": {},
    })
}

/// The headers a conforming client sends for `method` (`name`: the `Mcp-Name` value).
fn headers(method: &str, name: Option<&str>, sse: bool) -> Vec<(&'static str, String)> {
    let mut h = vec![
        ("Content-Type", "application/json".to_string()),
        (
            "Accept",
            if sse {
                "application/json, text/event-stream".to_string()
            } else {
                "application/json".to_string()
            },
        ),
        ("MCP-Protocol-Version", HTTP_PROTOCOL_VERSION.to_string()),
        ("Mcp-Method", method.to_string()),
    ];
    if let Some(n) = name {
        h.push(("Mcp-Name", n.to_string()));
    }
    h
}

fn rpc(addr: SocketAddr, ws: &str, method: &str, mut params: Value, sse: bool) -> Response {
    params["_meta"] = meta();
    let name = params["name"].as_str().map(str::to_string);
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string();
    raw(
        addr,
        &request_bytes(ws, &headers(method, name.as_deref(), sse), &body),
    )
}

fn call(addr: SocketAddr, ws: &str, tool: &str, args: Value, sse: bool) -> Response {
    rpc(
        addr,
        ws,
        "tools/call",
        json!({"name": tool, "arguments": args}),
        sse,
    )
}

fn call_bytes(ws: &str, tool: &str, args: Value, sse: bool) -> Vec<u8> {
    let body = json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call",
        "params": {"name": tool, "arguments": args, "_meta": meta()}})
    .to_string();
    request_bytes(ws, &headers("tools/call", Some(tool), sse), &body)
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn discover_list_and_call_over_json_and_sse() {
    let f = fixture("mcphttp-basic", HttpConfig::default());
    let d = rpc(f.addr(), "a", "server/discover", json!({}), false);
    assert_eq!(d.status, 200, "{}", d.body);
    assert!(d.header("content-type").unwrap().starts_with("application/json"));
    let r = &d.json()["result"];
    assert_eq!(r["resultType"], "complete");
    assert_eq!(r["supportedVersions"], json!([HTTP_PROTOCOL_VERSION]));
    assert_eq!(r["capabilities"], json!({"tools": {}}));
    assert_eq!(
        r["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "bicraft"
    );
    assert!(r["instructions"].as_str().unwrap().contains("open_file"));

    let l = rpc(f.addr(), "a", "tools/list", json!({}), false);
    let r = &l.json()["result"];
    assert_eq!(r["resultType"], "complete");
    assert_eq!(r["cacheScope"], "private");
    assert!(r["ttlMs"].as_u64().unwrap() > 0);
    // the same tools as stdio
    assert_eq!(r["tools"], json!(bicraft_mcp::tool_definitions()));

    // JSON reply
    let c = call(f.addr(), "a", "list_commands", json!({}), false);
    assert_eq!(c.status, 200, "{}", c.body);
    let m = c.json();
    assert_eq!(m["id"], 1);
    assert_eq!(m["result"]["resultType"], "complete");
    assert_eq!(m["result"]["isError"], false);
    assert!(m["result"]["structuredContent"].is_object());

    // SSE reply for a client that accepts both
    f.open("a");
    let c = call(f.addr(), "a", "evaluate_dax_query", json!({"query": SMALL}), true);
    assert_eq!(c.status, 200, "{}", c.body);
    assert!(c.header("content-type").unwrap().starts_with("text/event-stream"));
    assert_eq!(c.header("x-accel-buffering"), Some("no"));
    assert!(c.header("content-length").is_none());
    let m = c.sse_message();
    assert_eq!(m["id"], 1, "{m}");
    assert_eq!(m["result"]["isError"], false, "{m}");
    let rows = &m["result"]["structuredContent"]["results"][0]["rows"];
    assert_eq!(rows[0]["[x]"], 500500.0, "{m}");
    // the session lives on: the model opened by the JSON call answers the SSE one
}

#[test]
fn notifications_get_202_and_other_methods_405() {
    let f = fixture("mcphttp-methods", HttpConfig::default());
    let body = r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":1}}"#;
    let r = raw(
        f.addr(),
        &request_bytes("a", &[("Content-Type", "application/json".into())], body),
    );
    assert_eq!(r.status, 202, "{r:?}");
    assert!(r.body.is_empty());
    assert_eq!(r.header("content-length"), Some("0"));
    // a mirrored method header must still agree
    let r = raw(
        f.addr(),
        &request_bytes(
            "a",
            &[
                ("Content-Type", "application/json".into()),
                ("Mcp-Method", "notifications/other".into()),
            ],
            body,
        ),
    );
    assert_eq!((r.status, r.error_code()), (400, HEADER_MISMATCH));

    for method in ["GET", "DELETE", "PUT", "OPTIONS"] {
        let r = raw(
            f.addr(),
            format!("{method} /w/a/mcp HTTP/1.1\r\nHost: localhost\r\nMcp-Session-Id: x\r\n\r\n").as_bytes(),
        );
        assert_eq!(r.status, 405, "{method}: {r:?}");
        assert_eq!(r.header("allow"), Some("POST"));
    }
    for path in ["/", "/mcp", "/w/a", "/w/../mcp", "/w/a/b/mcp"] {
        let r = raw(
            f.addr(),
            format!("POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\n\r\n").as_bytes(),
        );
        assert_eq!(r.status, 404, "{path}");
    }
    // a workspace that is not a folder of the root
    let r = rpc(f.addr(), "nope", "tools/list", json!({}), false);
    assert_eq!(r.status, 404, "{r:?}");
    // unknown and removed methods
    for method in ["ping", "resources/list", "logging/setLevel"] {
        let r = rpc(f.addr(), "a", method, json!({}), false);
        assert_eq!((r.status, r.error_code()), (404, -32601), "{method}: {r:?}");
    }
}

#[test]
fn headers_must_mirror_the_body() {
    let f = fixture("mcphttp-headers", HttpConfig::default());
    let body = |method: &str, params: Value| {
        json!({"jsonrpc": "2.0", "id": "r1", "method": method, "params": params}).to_string()
    };
    let mut p = json!({"name": "list_commands", "arguments": {}});
    p["_meta"] = meta();
    let call_body = body("tools/call", p.clone());
    let send = |h: Vec<(&'static str, String)>, b: &str| raw(f.addr(), &request_bytes("a", &h, b));
    let without = |name: &str| {
        let mut h = headers("tools/call", Some("list_commands"), false);
        h.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
        h
    };
    let with = |name: &'static str, value: &str| {
        let mut h = without(name);
        h.push((name, value.to_string()));
        h
    };
    // all good, also with the base64 sentinel for Mcp-Name
    assert_eq!(
        send(headers("tools/call", Some("list_commands"), false), &call_body).status,
        200
    );
    assert_eq!(
        send(with("Mcp-Name", "=?base64?bGlzdF9jb21tYW5kcw==?="), &call_body).status,
        200
    );
    for (h, why) in [
        (without("MCP-Protocol-Version"), "no version header"),
        (without("Mcp-Method"), "no method header"),
        (without("Mcp-Name"), "no name header"),
        (with("Mcp-Method", "tools/list"), "method differs"),
        (with("Mcp-Name", "batch"), "name differs"),
        (with("Mcp-Name", "=?base64?YmF0Y2g=?="), "decoded name differs"),
        (with("Mcp-Name", "=?base64?not base64?="), "broken base64"),
        (with("MCP-Protocol-Version", "2025-11-25"), "version differs"),
    ] {
        let r = send(h, &call_body);
        assert_eq!((r.status, r.error_code()), (400, HEADER_MISMATCH), "{why}: {r:?}");
        assert_eq!(r.json()["id"], "r1", "{why}");
    }
    // `_meta` is required (-32602, 400)
    let no_meta = body("tools/call", json!({"name": "list_commands"}));
    let r = send(headers("tools/call", Some("list_commands"), false), &no_meta);
    assert_eq!((r.status, r.error_code()), (400, -32602), "{r:?}");
    let mut caps = p.clone();
    caps["_meta"]
        .as_object_mut()
        .unwrap()
        .remove("io.modelcontextprotocol/clientCapabilities");
    let r = send(
        headers("tools/call", Some("list_commands"), false),
        &body("tools/call", caps),
    );
    assert_eq!((r.status, r.error_code()), (400, -32602), "{r:?}");
    // an unsupported version, consistent in header and body
    let mut old = p.clone();
    old["_meta"]["io.modelcontextprotocol/protocolVersion"] = json!("2025-06-18");
    let r = send(
        with("MCP-Protocol-Version", "2025-06-18"),
        &body("tools/call", old),
    );
    assert_eq!(
        (r.status, r.error_code()),
        (400, UNSUPPORTED_PROTOCOL_VERSION),
        "{r:?}"
    );
    let data = &r.json()["error"]["data"];
    assert_eq!(data["supported"], json!([HTTP_PROTOCOL_VERSION]));
    assert_eq!(data["requested"], "2025-06-18");
    // a legacy initialize is told which versions are served
    let init = json!({"jsonrpc": "2.0", "id": 0, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "x", "version": "1"}}})
    .to_string();
    let r = send(vec![("Content-Type", "application/json".into())], &init);
    assert_eq!(
        (r.status, r.error_code()),
        (400, UNSUPPORTED_PROTOCOL_VERSION),
        "{r:?}"
    );
    assert_eq!(
        r.json()["error"]["data"]["supported"],
        json!([HTTP_PROTOCOL_VERSION])
    );
    // headers with control or non-ASCII bytes
    let r = send(with("Mcp-Name", "list_commands\u{7f}"), &call_body);
    assert_eq!((r.status, r.error_code()), (400, HEADER_MISMATCH), "{r:?}");
    // an unknown tool (the name matches its header)
    let mut unknown = p.clone();
    unknown["name"] = json!("nope");
    let r = send(with("Mcp-Name", "nope"), &body("tools/call", unknown));
    assert_eq!((r.status, r.error_code()), (400, -32602), "{r:?}");
}

#[test]
fn origin_host_body_and_malformed_requests() {
    let config = HttpConfig {
        max_body_bytes: 4096,
        allowed_origins: vec!["https://app.example".into()],
        ..HttpConfig::default()
    };
    let f = fixture("mcphttp-limits", config);
    let list = |extra: Vec<(&'static str, String)>| {
        let mut h = headers("tools/list", None, false);
        h.extend(extra);
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {"_meta": meta()}})
            .to_string();
        raw(f.addr(), &request_bytes("a", &h, &body))
    };
    assert_eq!(list(vec![("Origin", "https://app.example".into())]).status, 200);
    for origin in [
        "https://evil.example",
        "null",
        "http://app.example",
        "https://app.example:444",
    ] {
        let r = list(vec![("Origin", origin.into())]);
        assert_eq!(r.status, 403, "{origin}");
        assert!(r.json().get("id").is_none(), "{r:?}");
    }
    // DNS rebinding: Host must be loopback; a missing one is a 400
    let r = raw(
        f.addr(),
        b"POST /w/a/mcp HTTP/1.1\r\nHost: evil.example\r\nContent-Length: 0\r\n\r\n",
    );
    assert_eq!(r.status, 403);
    let r = raw(f.addr(), b"POST /w/a/mcp HTTP/1.1\r\nContent-Length: 0\r\n\r\n");
    assert_eq!(r.status, 400);
    // body cap: refused before the body is read
    let r = raw(
        f.addr(),
        b"POST /w/a/mcp HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: 5000\r\n\r\n",
    );
    assert_eq!(r.status, 413, "{r:?}");
    // content type, length, encoding
    let r = raw(
        f.addr(),
        b"POST /w/a/mcp HTTP/1.1\r\nHost: localhost\r\nContent-Type: text/plain\r\nContent-Length: 2\r\n\r\n{}",
    );
    assert_eq!(r.status, 415);
    let r = raw(
        f.addr(),
        b"POST /w/a/mcp HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\n\r\n",
    );
    assert_eq!(r.status, 411);
    let r = raw(
        f.addr(),
        b"POST /w/a/mcp HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n",
    );
    assert_eq!(r.status, 411);
    let r = raw(
        f.addr(),
        b"POST /w/a/mcp HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nAccept: text/html\r\nContent-Length: 2\r\n\r\n{}",
    );
    assert_eq!(r.status, 406);
    let plain = |body: &str| {
        raw(
            f.addr(),
            &request_bytes("a", &[("Content-Type", "application/json".into())], body),
        )
    };
    for (body, code) in [
        ("{not json", -32700),
        ("\u{0}\u{1}", -32700),
        ("[]", -32600),
        (r#"[{"jsonrpc":"2.0","id":1,"method":"tools/list"}]"#, -32600),
        ("42", -32600),
        (r#"{"jsonrpc":"2.0","id":1,"result":{}}"#, -32600),
        (r#"{"jsonrpc":"2.0","id":1}"#, -32600),
        (r#"{"jsonrpc":"2.0","id":null,"method":"tools/list"}"#, -32600),
        (r#"{"jsonrpc":"2.0","id":{},"method":"tools/list"}"#, -32600),
        (
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":[]}"#,
            -32602,
        ),
    ] {
        let r = plain(body);
        assert_eq!((r.status, r.error_code()), (400, code), "{body}: {r:?}");
    }
    // a body shorter than announced times out or is refused, never hangs the server
    let mut s = TcpStream::connect(f.addr()).unwrap();
    s.write_all(b"POST /w/a/mcp HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: 10\r\n\r\n{}")
        .unwrap();
    s.shutdown(std::net::Shutdown::Write).unwrap();
    let r = read_response(s);
    assert_eq!(r.status, 400, "{r:?}");
    // repeated mirrored headers are refused
    let mut h = headers("tools/list", None, false);
    h.push(("Mcp-Method", "tools/list".into()));
    let body =
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {"_meta": meta()}}).to_string();
    assert_eq!(raw(f.addr(), &request_bytes("a", &h, &body)).status, 400);
    // the server still answers
    assert_eq!(list(vec![]).status, 200);
}

/// Mutated requests never take the server down.
#[test]
fn hostile_requests_never_crash() {
    let f = fixture("mcphttp-fuzz", HttpConfig::default());
    let good = call_bytes("a", "list_commands", json!({}), true);
    let mut seed = 0x2545_f491_4f6c_dd1d_u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for _ in 0..150 {
        let mut b = good.clone();
        for _ in 0..(next() % 6 + 1) {
            let i = (next() as usize) % b.len();
            match next() % 3 {
                0 => b[i] = (next() & 0xff) as u8,
                1 => {
                    b.remove(i);
                }
                _ => b.insert(i, b"\r\n:{}\"\0"[(next() % 7) as usize]),
            }
        }
        let mut s = TcpStream::connect(f.addr()).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
        let _ = s.write_all(&b);
        let _ = s.shutdown(std::net::Shutdown::Write);
        let mut sink = Vec::new();
        let _ = s.read_to_end(&mut sink);
    }
    assert_eq!(
        rpc(f.addr(), "a", "server/discover", json!({}), false).status,
        200
    );
    wait_until("connections to close", || f.server.open_connections() == 0);
}

/// Closing the response stream cancels the request: a long DAX query stops, and the workspace
/// serves the next call at once.
#[test]
fn closing_the_stream_cancels_a_long_query() {
    let f = fixture("mcphttp-cancel", HttpConfig::default());
    f.open("a");
    for sse in [true, false] {
        let mut s = TcpStream::connect(f.addr()).unwrap();
        s.write_all(&call_bytes(
            "a",
            "evaluate_dax_query",
            json!({"query": LONG, "timeout_ms": 600_000}),
            sse,
        ))
        .unwrap();
        std::thread::sleep(Duration::from_millis(300));
        let started = Instant::now();
        drop(s);
        let r = call(
            f.addr(),
            "a",
            "evaluate_dax_query",
            json!({"query": SMALL}),
            false,
        );
        assert_eq!(r.status, 200, "{r:?}");
        assert_eq!(r.json()["result"]["isError"], false, "{}", r.body);
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "sse {sse}: the long query kept the workspace for {:?}",
            started.elapsed()
        );
    }
}

#[test]
fn a_full_queue_answers_503() {
    let config = HttpConfig {
        queue_per_workspace: 1,
        ..HttpConfig::default()
    };
    let f = fixture("mcphttp-queue", config);
    f.open("a");
    let mut long = TcpStream::connect(f.addr()).unwrap();
    long.write_all(&call_bytes(
        "a",
        "evaluate_dax_query",
        json!({"query": LONG, "timeout_ms": 600_000}),
        true,
    ))
    .unwrap();
    // the SSE head arrives once the call holds its queue place
    long.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    let mut first = [0u8; 12];
    long.read_exact(&mut first).unwrap();
    assert_eq!(&first, b"HTTP/1.1 200");
    let r = call(f.addr(), "a", "list_commands", json!({}), false);
    assert_eq!((r.status, r.error_code()), (503, WORKSPACE_BUSY), "{r:?}");
    assert_eq!(r.header("retry-after"), Some("1"));
    assert_eq!(r.json()["id"], 1);
    // other workspaces and the stateless methods are not held up
    assert_eq!(call(f.addr(), "b", "list_commands", json!({}), false).status, 200);
    assert_eq!(rpc(f.addr(), "a", "tools/list", json!({}), false).status, 200);
    drop(long);
    wait_until("the queue to drain", || {
        call(f.addr(), "a", "list_commands", json!({}), false).status == 200
    });
}

#[test]
fn workspaces_run_in_parallel_and_reads_of_one_workspace_too() {
    let f = fixture("mcphttp-concurrency", HttpConfig::default());
    f.open("a");
    f.open("b");
    let addr = f.addr();
    let budgeted = |ws: &'static str| {
        std::thread::spawn(move || {
            let r = call(
                addr,
                ws,
                "evaluate_dax_query",
                json!({"query": LONG, "timeout_ms": 1500}),
                false,
            );
            assert_eq!(r.status, 200, "{r:?}");
            // ends on its budget
            assert_eq!(r.json()["result"]["isError"], true, "{}", r.body);
            Instant::now()
        })
    };
    // a long call in `a` does not hold up `b`
    let started = Instant::now();
    let a = budgeted("a");
    std::thread::sleep(Duration::from_millis(200));
    let r = call(addr, "b", "evaluate_dax_query", json!({"query": SMALL}), false);
    assert_eq!(r.json()["result"]["isError"], false, "{}", r.body);
    let b_done = started.elapsed();
    let a_done = a.join().unwrap().duration_since(started);
    assert!(b_done < a_done, "b {b_done:?} waited for a {a_done:?}");
    // two long reads in `a` run side by side (`--max-concurrent-reads`)
    let (x, y) = (budgeted("a"), budgeted("a"));
    wait_until("both reads to run", || {
        f.server.metrics_text().contains("\nbicraft_concurrent_reads 2\n")
    });
    x.join().unwrap();
    y.join().unwrap();
}

#[test]
fn loopback_servers_refuse_other_addresses() {
    let dir = TempDir::new("mcphttp-bind");
    let e = HttpServer::start("0.0.0.0:0".parse().unwrap(), dir.path(), HttpConfig::default())
        .err()
        .unwrap();
    assert_eq!(e.kind(), std::io::ErrorKind::InvalidInput);
    let e = HttpServer::start(
        "127.0.0.1:0".parse().unwrap(),
        dir.path().join("missing"),
        HttpConfig::default(),
    )
    .err()
    .unwrap();
    assert_eq!(e.kind(), std::io::ErrorKind::NotFound);
}

// ---- server mode: each workspace is a sandbox (docs/server-mode.md) ------------------------

/// The tool result of a JSON reply.
fn tool_result(r: &Response) -> Value {
    assert_eq!(r.status, 200, "{}", r.body);
    r.json()["result"].clone()
}

/// Asserts a tool call succeeded.
fn ok(r: &Response, what: &str) -> Value {
    let t = tool_result(r);
    assert_eq!(t["isError"], false, "{what}: {t}");
    t["structuredContent"].clone()
}

/// Asserts a tool call failed and returns its error text.
fn tool_error(r: &Response, what: &str) -> String {
    let t = tool_result(r);
    assert_eq!(t["isError"], true, "{what} should fail: {t}");
    t["structuredContent"]["error"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| t["structuredContent"].to_string())
}

/// Asserts a tool call was refused by the path sandbox.
fn assert_refused(r: &Response, what: &str) {
    let e = tool_error(r, what);
    assert!(
        e.contains("server mode") && e.contains("relative to the workspace root"),
        "{what}: {e}"
    );
}

/// Makes `link` point to the folder `target`; false when this OS or account cannot.
fn link_dir(target: &std::path::Path, link: &std::path::Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).is_ok()
    }
    #[cfg(windows)]
    {
        if std::os::windows::fs::symlink_dir(target, link).is_ok() {
            return true;
        }
        // a junction needs no privilege
        std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .is_ok_and(|o| o.status.success())
            && link.exists()
    }
}

#[test]
fn tool_paths_cannot_leave_the_workspace() {
    let f = fixture("mcphttp-sandbox", HttpConfig::default());
    let outside = TempDir::new("mcphttp-sandbox-outside");
    std::fs::write(outside.path().join("x.bim"), MODEL).unwrap();
    let abs_outside = outside.path().join("x.bim").display().to_string();
    let abs_b = f.dir.path().join("b").join("model.bim").display().to_string();
    let addr = f.addr();
    f.open("a");
    // open: absolute paths (another workspace, outside the root) and `..`
    for p in [
        abs_outside.as_str(),
        abs_b.as_str(),
        "../b/model.bim",
        "sub/../../b/model.bim",
    ] {
        assert_refused(&call(addr, "a", "open_file", json!({"path": p}), false), p);
        let step = json!({"command": "file.open", "params": {"path": p}});
        assert_refused(&call(addr, "a", "run_command", step, false), p);
    }
    // save: nothing written outside
    let abs_new = outside.path().join("new.bim").display().to_string();
    for p in [abs_new.as_str(), "../b/new.bim", "../new.bim"] {
        assert_refused(&call(addr, "a", "save_as", json!({"path": p}), false), p);
    }
    assert!(!outside.path().join("new.bim").exists());
    assert!(!f.dir.path().join("b").join("new.bim").exists());
    assert!(!f.dir.path().join("new.bim").exists());
    // the batch tool goes through the same sandbox
    let r = call(
        addr,
        "a",
        "batch",
        json!({"commands": [{"command": "file.save_as", "params": {"path": "../b/new.bim"}}]}),
        false,
    );
    assert!(tool_error(&r, "batch save").contains("relative to the workspace root"));
    // render: the output path is a path too
    let abs_png = outside.path().join("p.png").display().to_string();
    for p in [abs_png.as_str(), "../p.png"] {
        assert_refused(&call(addr, "a", "render_page", json!({"out": p}), false), p);
    }
    assert!(!outside.path().join("p.png").exists());
    // relative paths inside the workspace work, and land in it
    ok(
        &call(addr, "a", "save_as", json!({"path": "copies/m.bim"}), false),
        "save inside",
    );
    assert!(f.dir.path().join("a").join("copies").join("m.bim").is_file());
    ok(
        &call(addr, "a", "open_file", json!({"path": "copies/m.bim"}), false),
        "open inside",
    );
    // a link inside the workspace that leads out of it
    if link_dir(outside.path(), &f.dir.path().join("a").join("out")) {
        assert_refused(
            &call(addr, "a", "open_file", json!({"path": "out/x.bim"}), false),
            "open through a link",
        );
        assert_refused(
            &call(addr, "a", "save_as", json!({"path": "out/y.bim"}), false),
            "save through a link",
        );
        assert!(!outside.path().join("y.bim").exists());
    } else {
        eprintln!("note: cannot create a directory link here; link escape test skipped");
    }
}

#[test]
fn workspaces_cannot_reach_each_other() {
    let f = fixture("mcphttp-isolation", HttpConfig::default());
    let addr = f.addr();
    f.open("a");
    ok(
        &call(addr, "a", "save_as", json!({"path": "secret.bim"}), false),
        "save in a",
    );
    assert!(f.dir.path().join("a").join("secret.bim").is_file());
    // b sees its own folder only: the name is resolved in b, where nothing has it
    let e = tool_error(
        &call(addr, "b", "open_file", json!({"path": "secret.bim"}), false),
        "the file of a by name in b",
    );
    assert!(!e.contains("relative to the workspace root"), "{e}");
    assert_refused(
        &call(addr, "b", "open_file", json!({"path": "../a/secret.bim"}), false),
        "the file of a through ..",
    );
    let abs = f.dir.path().join("a").join("secret.bim").display().to_string();
    assert_refused(
        &call(addr, "b", "open_file", json!({"path": abs}), false),
        "the file of a by absolute path",
    );
    // a workspace folder that is a link to another workspace is no workspace
    if link_dir(&f.dir.path().join("a"), &f.dir.path().join("alias")) {
        let r = call(addr, "alias", "list_commands", json!({}), false);
        assert_eq!(r.status, 404, "{}", r.body);
    } else {
        eprintln!("note: cannot create a directory link here; workspace alias test skipped");
    }
    // another spelling of a workspace (the same folder on a case-insensitive file system) is not
    // a second session over it
    let r = call(addr, "A", "list_commands", json!({}), false);
    assert_eq!(r.status, 404, "{}", r.body);
}

#[test]
fn commands_denied_on_a_server_are_hidden_and_refused() {
    let f = fixture("mcphttp-denied", HttpConfig::default());
    let addr = f.addr();
    let ids = |filter: Value| -> Vec<String> {
        let list = ok(
            &call(addr, "a", "list_commands", json!({"filter": filter}), false),
            "list_commands",
        );
        list["commands"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["id"].as_str().unwrap().to_string())
            .collect()
    };
    let all = ids(Value::Null);
    assert!(all.iter().any(|i| i == "file.open"), "{all:?}");
    assert!(!all.iter().any(|i| i == "file.open_in_desktop"), "{all:?}");
    assert!(ids(json!("open_in_desktop")).is_empty());
    // tools/list: the generic tools stay, no tool maps to a denied command
    let tools = tool_result(&rpc(addr, "a", "tools/list", json!({}), false));
    let names: Vec<&str> = tools["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert!(
        names.contains(&"run_command") && names.contains(&"open_file"),
        "{names:?}"
    );
    // calling it anyway: the policy error, nothing launched
    f.open("a");
    ok(
        &call(addr, "a", "save_as", json!({"path": "m.bim"}), false),
        "save",
    );
    let direct = json!({"command": "file.open_in_desktop", "params": {"dry_run": true}});
    let batched = json!({"commands": [{"command": "file.open_in_desktop"}]});
    for r in [
        call(addr, "a", "run_command", direct, false),
        call(addr, "a", "batch", batched, false),
    ] {
        let e = tool_error(&r, "file.open_in_desktop");
        assert!(e.contains("not available in server mode"), "{e}");
    }
}

#[test]
fn the_databricks_connector_needs_the_servers_network_setting() {
    for allow in [false, true] {
        let config = HttpConfig {
            allow_network: allow,
            ..HttpConfig::default()
        };
        let f = fixture("mcphttp-network", config);
        let addr = f.addr();
        ok(&call(addr, "a", "new_model", json!({"name": "M"}), false), "new");
        ok(
            &call(addr, "a", "new_project", json!({"dir": "p", "name": "P"}), false),
            "project",
        );
        let args = json!({"allow_connectors": ["databricks"]});
        let t = tool_result(&call(addr, "a", "refresh_model", args, false));
        let text = t["structuredContent"].to_string();
        if allow {
            assert!(!text.contains("needs network access"), "{t}");
        } else {
            assert_eq!(t["isError"], true, "{t}");
            assert!(text.contains("needs network access"), "{t}");
        }
    }
}

/// The structured result of a tool call (JSON reply).
fn structured(r: &Response) -> Value {
    assert_eq!(r.status, 200, "{r:?}");
    r.json()["result"]["structuredContent"].clone()
}

/// Read-only calls share a workspace's session: a short query answers while a long one runs.
#[test]
fn reads_on_one_workspace_overlap() {
    let f = fixture("mcphttp-reads", HttpConfig::default());
    f.open("a");
    let mut long = TcpStream::connect(f.addr()).unwrap();
    long.write_all(&call_bytes(
        "a",
        "evaluate_dax_query",
        json!({"query": LONG, "timeout_ms": 600_000}),
        true,
    ))
    .unwrap();
    wait_until("the long read to run", || {
        f.server.metrics_text().contains("\nbicraft_concurrent_reads 1\n")
    });
    // with reads in order this would wait for minutes
    for _ in 0..3 {
        let r = call(
            f.addr(),
            "a",
            "evaluate_dax_query",
            json!({"query": SMALL}),
            false,
        );
        assert_eq!(r.json()["result"]["isError"], false, "{}", r.body);
        let r = call(f.addr(), "a", "model_summary", json!({}), false);
        assert_eq!(r.json()["result"]["isError"], false, "{}", r.body);
    }
    assert!(
        f.server.metrics_text().contains("\nbicraft_concurrent_reads 1\n"),
        "the long read still runs"
    );
    drop(long);
    wait_until("the long read to stop", || {
        f.server.metrics_text().contains("\nbicraft_concurrent_reads 0\n")
    });
}

/// A call that changes the session waits for the running reads, and reads that come after it
/// wait for it (a waiting writer goes first) and see its change.
///
/// The order is read from the server, never from clocks: the write's result carries the
/// session's revision after it, and the later read sees the new measure and that revision only if
/// it ran after the write. The test waits only for server states (the metrics gauges), never for
/// a delay: `bicraft_tool_calls_queued` counts a call only once it waits at the session's gate,
/// where a waiting write has already announced itself, so a read sent after the write is counted
/// always queues behind it.
#[test]
fn a_write_waits_for_reads_and_later_reads_see_it() {
    use std::sync::mpsc::{TryRecvError, channel};
    let revision = |r: &Response| r.json()["result"]["_meta"]["io.bicraft/revision"].as_u64();
    let f = fixture("mcphttp-write-waits", HttpConfig::default());
    f.open("a");
    let addr = f.addr();
    let r = call(
        addr,
        "a",
        "run_command",
        json!({"command": "session.revision"}),
        false,
    );
    let before = structured(&r)["revision"].as_u64().unwrap();
    // a long read holds the session's read side until its stream closes
    let mut long = TcpStream::connect(addr).unwrap();
    long.write_all(&call_bytes(
        "a",
        "evaluate_dax_query",
        json!({"query": LONG, "timeout_ms": 600_000}),
        true,
    ))
    .unwrap();
    wait_until("the long read to run", || {
        f.server.metrics_text().contains("\nbicraft_concurrent_reads 1\n")
    });
    let (write_tx, write_rx) = channel();
    std::thread::spawn(move || {
        let r = call(
            addr,
            "a",
            "run_command",
            json!({"command": "measure.add", "params": {"table": "Product", "name": "Later", "expression": "42"}}),
            false,
        );
        let _ = write_tx.send(r);
    });
    wait_until("the write to wait", || {
        f.server
            .metrics_text()
            .contains("\nbicraft_tool_calls_queued 1\n")
    });
    // a read arriving now queues behind the waiting write, and sees the new measure
    let (read_tx, read_rx) = channel();
    std::thread::spawn(move || {
        let r = call(
            addr,
            "a",
            "evaluate_dax_query",
            json!({"query": "EVALUATE ROW(\"x\", [Later])"}),
            false,
        );
        let _ = read_tx.send(r);
    });
    wait_until("the read to queue", || {
        if let Ok(r) = read_rx.try_recv() {
            panic!("the read passed the waiting write: {}", r.body);
        }
        f.server
            .metrics_text()
            .contains("\nbicraft_tool_calls_queued 2\n")
    });
    // a write never passes a running read
    assert!(
        matches!(write_rx.try_recv(), Err(TryRecvError::Empty)),
        "the write ran while a read held the session"
    );
    drop(long); // the long read is cancelled: the write runs, then the read
    let w = write_rx.recv().expect("the write's answer");
    assert_eq!(w.json()["result"]["isError"], false, "{}", w.body);
    let after = revision(&w).unwrap_or_else(|| panic!("no revision in {}", w.body));
    assert!(
        after > before,
        "the write changed the session: {before} -> {after}"
    );
    let r = read_rx.recv().expect("the read's answer");
    assert_eq!(r.json()["result"]["isError"], false, "{}", r.body);
    assert_eq!(
        revision(&r),
        Some(after),
        "the read ran after the write: {}",
        r.body
    );
    let x = structured(&r);
    assert_eq!(x["results"][0]["rows"][0]["[x]"], 42, "{x}");
}

/// `expected_revision`: the revision comes back in each result's `_meta`; a stale one is a
/// `revision_conflict` error that changes nothing (a batch checks it once, before any step).
#[test]
fn stale_expected_revisions_are_conflicts() {
    let f = fixture("mcphttp-conflict", HttpConfig::default());
    f.open("a");
    let addr = f.addr();
    let rev = |r: &Response| {
        r.json()["result"]["_meta"]["io.bicraft/revision"]
            .as_u64()
            .unwrap()
    };
    let r = call(
        addr,
        "a",
        "run_command",
        json!({"command": "session.revision"}),
        false,
    );
    let start = structured(&r)["revision"].as_u64().unwrap();
    assert_eq!(rev(&r), start);
    // the same revision: runs, and the revision moves on
    let r = call(
        addr,
        "a",
        "run_command",
        json!({"command": "measure.add", "expected_revision": start,
               "params": {"table": "Product", "name": "One", "expression": "1"}}),
        false,
    );
    assert_eq!(r.json()["result"]["isError"], false, "{}", r.body);
    let now = rev(&r);
    assert_eq!(now, start + 1);
    // a stale one: refused, nothing changed
    let r = call(
        addr,
        "a",
        "run_command",
        json!({"command": "measure.add", "expected_revision": start,
               "params": {"table": "Product", "name": "Two", "expression": "2"}}),
        false,
    );
    let s = structured(&r);
    assert_eq!(r.json()["result"]["isError"], true);
    assert_eq!(s["code"], "revision_conflict", "{s}");
    assert_eq!(s["current_revision"], now);
    assert!(s["error"].as_str().unwrap().contains(&now.to_string()), "{s}");
    assert_eq!(rev(&r), now);
    let r = call(
        addr,
        "a",
        "batch",
        json!({"expected_revision": start, "commands": [
            {"command": "measure.add", "params": {"table": "Product", "name": "Three", "expression": "3"}},
            {"command": "measure.add", "params": {"table": "Product", "name": "Four", "expression": "4"}}]}),
        false,
    );
    assert_eq!(structured(&r)["code"], "revision_conflict");
    let names = structured(&call(addr, "a", "model_summary", json!({}), false)).to_string();
    assert!(
        names.contains("One") && !names.contains("Two") && !names.contains("Three"),
        "{names}"
    );
    // a batch with the current revision runs whole
    let r = call(
        addr,
        "a",
        "batch",
        json!({"expected_revision": now, "commands": [
            {"command": "measure.add", "params": {"table": "Product", "name": "Three", "expression": "3"}},
            {"command": "measure.add", "params": {"table": "Product", "name": "Four", "expression": "4"}}]}),
        false,
    );
    assert_eq!(r.json()["result"]["isError"], false, "{}", r.body);
    assert_eq!(rev(&r), now + 2);
    // without it, the last writer wins (as before); a malformed one is an error
    let r = call(
        addr,
        "a",
        "run_command",
        json!({"command": "measure.add", "expected_revision": "x",
               "params": {"table": "Product", "name": "Five", "expression": "5"}}),
        false,
    );
    assert_eq!(r.json()["result"]["isError"], true);
    assert!(
        f.server
            .metrics_text()
            .contains("\nbicraft_revision_conflicts_total 2\n")
    );
}
