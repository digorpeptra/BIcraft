//! Observability of `bicraft-cli serve` over real sockets (docs/kubernetes.md, "Observability"): the
//! JSON request log never carries model data (a query with a distinctive literal, an error that
//! names it, file names, a path), `/metrics` lives on its own listener only and reports requests,
//! commands, queue, workspaces, memory and drain state with bounded labels.

#[path = "../../../testsupport/tempdir.rs"]
mod tempdir;

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bicraft_mcp::http::HTTP_PROTOCOL_VERSION;
use bicraft_mcp::{HttpConfig, HttpServer, LogFormat, LogSink};
use serde_json::{Value, json};
use tempdir::TempDir;

const MODEL: &str = include_str!("../../engine/tests/table_functions/model.bim");
/// Minutes of evaluation: only a cancel ends it.
const LONG: &str =
    "EVALUATE ROW(\"x\", SUMX(GENERATESERIES(1, 100000), SUMX(GENERATESERIES(1, 100000), [Value])))";
/// The distinctive literal: it must never reach the log or the metrics.
const SECRET: &str = "ZQX7731secret";

struct Response {
    status: u16,
    body: Vec<u8>,
}

impl Response {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
    fn message(&self) -> Value {
        let t = self.text();
        match t.lines().find_map(|l| l.strip_prefix("data: ")) {
            Some(d) => serde_json::from_str(d).unwrap(),
            None => serde_json::from_str(&t).unwrap_or_else(|e| panic!("{e}: {t}")),
        }
    }
}

fn connect(addr: SocketAddr) -> TcpStream {
    let s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
    s
}

fn read_response(mut s: TcpStream) -> Response {
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    let split = out.windows(4).position(|w| w == b"\r\n\r\n").unwrap_or(out.len());
    let head = String::from_utf8_lossy(&out[..split]).into_owned();
    Response {
        status: head.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0),
        body: out.get(split + 4..).unwrap_or_default().to_vec(),
    }
}

fn request(addr: SocketAddr, method: &str, path: &str, body: &[u8]) -> Response {
    let mut s = connect(addr);
    write!(
        s,
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )
    .unwrap();
    s.write_all(body).unwrap();
    read_response(s)
}

fn call_bytes(ws: &str, tool: &str, args: Value, sse: bool) -> Vec<u8> {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {
        "name": tool, "arguments": args, "_meta": {
            "io.modelcontextprotocol/protocolVersion": HTTP_PROTOCOL_VERSION,
            "io.modelcontextprotocol/clientCapabilities": {}}}})
    .to_string();
    let accept = if sse {
        "application/json, text/event-stream"
    } else {
        "application/json"
    };
    format!(
        "POST /w/{ws}/mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
         Accept: {accept}\r\nMCP-Protocol-Version: {HTTP_PROTOCOL_VERSION}\r\n\
         Mcp-Method: tools/call\r\nMcp-Name: {tool}\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

fn call(addr: SocketAddr, ws: &str, tool: &str, args: Value, sse: bool) -> Value {
    let mut s = connect(addr);
    s.write_all(&call_bytes(ws, tool, args, sse)).unwrap();
    let r = read_response(s);
    assert_eq!(r.status, 200, "{}", r.text());
    r.message()["result"].clone()
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

/// A server whose log lines are collected, with a metrics listener.
fn start(name: &str, workspace_label: bool) -> (TempDir, HttpServer, Arc<Mutex<Vec<String>>>) {
    let dir = TempDir::new(name);
    for ws in ["a", "b"] {
        std::fs::create_dir_all(dir.path().join(ws)).unwrap();
    }
    let lines = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&lines);
    let config = HttpConfig {
        metrics_listen: Some("127.0.0.1:0".parse().unwrap()),
        metrics_workspace_label: workspace_label,
        log_format: LogFormat::Json,
        log_sink: Some(LogSink(Arc::new(move |l: &str| {
            sink.lock().unwrap().push(l.to_string())
        }))),
        ..HttpConfig::default()
    };
    let server = HttpServer::start("127.0.0.1:0".parse().unwrap(), dir.path(), config).unwrap();
    (dir, server, lines)
}

fn scrape(server: &HttpServer) -> String {
    let r = request(server.metrics_addr().unwrap(), "GET", "/metrics", b"");
    assert_eq!(r.status, 200);
    r.text()
}

/// The value of a series (`name{labels}` exactly as rendered).
fn value(metrics: &str, series: &str) -> Option<f64> {
    metrics
        .lines()
        .find_map(|l| l.strip_prefix(series)?.strip_prefix(' ')?.parse().ok())
}

#[test]
fn the_json_log_never_carries_model_data() {
    let (dir, server, lines) = start("metrics-log", false);
    let a = server.addr();
    std::fs::write(dir.path().join("a").join("model.bim"), MODEL).unwrap();
    let opened = call(a, "a", "open_file", json!({"path": "model.bim"}), false);
    assert_eq!(opened["isError"], false, "{opened}");
    // a query whose text and result carry the literal
    let q = format!("EVALUATE ROW(\"{SECRET} total\", 41 + 1, \"label\", \"{SECRET}\")");
    let r = call(a, "a", "evaluate_dax_query", json!({"query": q}), true);
    assert_eq!(r["isError"], false, "{r}");
    assert!(r.to_string().contains(SECRET), "the answer carries it: {r}");
    // an error that names it (the engine's message echoes the unknown measure)
    let bad = format!("EVALUATE ROW(\"x\", [{SECRET}Measure])");
    let r = call(
        a,
        "a",
        "run_command",
        json!({"command": "dax.evaluate", "params": {"query": bad}}),
        false,
    );
    assert_eq!(r["isError"], true, "{r}");
    // a command id that is not one, carrying it
    let r = call(a, "a", "run_command", json!({"command": SECRET}), false);
    assert_eq!(r["isError"], true, "{r}");
    // file names and paths: an upload, its download, a missing file, the listing
    let path = format!("/w/a/files/{SECRET}.csv");
    assert_eq!(request(a, "PUT", &path, SECRET.as_bytes()).status, 201);
    assert_eq!(request(a, "GET", &path, b"").status, 200);
    assert_eq!(
        request(a, "GET", &format!("/w/a/files/no-{SECRET}.txt"), b"").status,
        404
    );
    assert!(request(a, "GET", "/w/a/files/", b"").text().contains(SECRET));
    assert_eq!(request(a, "GET", &format!("/{SECRET}"), b"").status, 404);
    // the main listener never serves the metrics
    assert_eq!(request(a, "GET", "/metrics", b"").status, 404);

    let metrics = scrape(&server);
    let log = lines.lock().unwrap().clone();
    assert!(log.len() >= 10, "{log:#?}");
    for line in &log {
        assert!(!line.contains(SECRET), "model data in the log: {line}");
        let v: Value = serde_json::from_str(line).unwrap_or_else(|e| panic!("{e}: {line}"));
        assert!(v["ts"].as_str().is_some_and(|t| t.ends_with('Z')), "{line}");
        assert_eq!(v["event"], "request", "{line}");
    }
    assert!(!metrics.contains(SECRET), "{metrics}");
    let find = |pred: &dyn Fn(&Value) -> bool| {
        log.iter()
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .find(|v| pred(v))
    };
    let q = find(&|v| v["tool"] == "evaluate_dax_query").expect("the query's line");
    assert_eq!(q["command"], "dax.evaluate");
    assert_eq!(q["route"], "mcp");
    assert_eq!(q["method"], "POST");
    assert_eq!(q["rpc"], "tools/call");
    assert_eq!(q["workspace"], "a");
    assert_eq!(q["caller"], "local");
    assert_eq!(q["status"], 200);
    assert_eq!(q["outcome"], "ok");
    assert!(q["duration_ms"].as_f64().is_some());
    assert!(q["bytes_in"].as_u64().unwrap() > 0);
    assert!(q["bytes_out"].as_u64().unwrap() > 0);
    let e = find(&|v| v["tool"] == "run_command" && v["command"] == "dax.evaluate").unwrap();
    assert_eq!(
        (e["outcome"].as_str(), e["error"].as_str()),
        (Some("error"), Some("command_error"))
    );
    assert!(find(&|v| v["command"] == "unknown").is_some());
    let up = find(&|v| v["route"] == "files" && v["method"] == "PUT").unwrap();
    assert_eq!(
        (up["status"].as_u64(), up["outcome"].as_str()),
        (Some(201), Some("ok"))
    );
    let miss = find(&|v| v["route"] == "files" && v["status"] == 404).unwrap();
    assert_eq!(miss["error"], "client_error");

    // the metrics: requests, commands, open durations, workspaces, memory, drain state
    assert_eq!(
        value(
            &metrics,
            "bicraft_commands_total{command=\"dax.evaluate\",outcome=\"ok\"}"
        ),
        Some(1.0),
        "{metrics}"
    );
    assert_eq!(
        value(
            &metrics,
            "bicraft_commands_total{command=\"dax.evaluate\",outcome=\"error\"}"
        ),
        Some(1.0)
    );
    assert_eq!(
        value(
            &metrics,
            "bicraft_commands_total{command=\"unknown\",outcome=\"error\"}"
        ),
        Some(1.0)
    );
    assert_eq!(
        value(
            &metrics,
            "bicraft_command_duration_seconds_count{command=\"file.open\"}"
        ),
        Some(1.0)
    );
    assert!(
        metrics.contains("bicraft_command_duration_seconds_bucket{command=\"dax.evaluate\",le=\"+Inf\"} 2")
    );
    assert_eq!(
        value(
            &metrics,
            "bicraft_http_requests_total{route=\"mcp\",code=\"200\"}"
        ),
        Some(4.0)
    );
    assert_eq!(
        value(
            &metrics,
            "bicraft_http_requests_total{route=\"files\",code=\"201\"}"
        ),
        Some(1.0)
    );
    assert_eq!(
        value(
            &metrics,
            "bicraft_http_requests_total{route=\"other\",code=\"404\"}"
        ),
        Some(2.0)
    );
    assert_eq!(value(&metrics, "bicraft_workspaces_open"), Some(1.0));
    assert_eq!(value(&metrics, "bicraft_draining"), Some(0.0));
    assert_eq!(value(&metrics, "bicraft_queue_wait_seconds_count"), Some(4.0));
    for series in [
        "bicraft_memory_limit_bytes",
        "bicraft_memory_reserved_bytes",
        "bicraft_memory_evictions_total",
        "bicraft_spill_bytes",
        "bicraft_aggregation_builds_running",
        "bicraft_tool_calls_queued",
        "bicraft_http_requests_in_flight",
    ] {
        assert!(value(&metrics, series).is_some(), "{series} missing:\n{metrics}");
    }
    // no workspace label unless asked for
    assert!(!metrics.contains("workspace=\""), "{metrics}");
    // the metrics listener serves nothing else
    let m = server.metrics_addr().unwrap();
    assert_eq!(request(m, "GET", "/w/a/mcp", b"").status, 404);
    assert_eq!(request(m, "POST", "/metrics", b"").status, 405);
    let report = server.drain(Duration::from_secs(5));
    assert!(report.clean(), "{report:?}");
}

#[test]
fn metrics_follow_the_drain_and_running_calls() {
    let (dir, server, _lines) = start("metrics-drain", true);
    let a = server.addr();
    let m = server.metrics_addr().unwrap();
    std::fs::write(dir.path().join("b").join("model.bim"), MODEL).unwrap();
    call(a, "b", "open_file", json!({"path": "model.bim"}), false);
    let metrics = scrape(&server);
    // the opt-in label, for a workspace that ran a tool call
    assert_eq!(
        value(&metrics, "bicraft_workspace_tool_calls_total{workspace=\"b\"}"),
        Some(1.0)
    );
    let mut long = connect(a);
    long.write_all(&call_bytes(
        "b",
        "evaluate_dax_query",
        json!({"query": LONG, "timeout_ms": 600_000}),
        false,
    ))
    .unwrap();
    wait_until("the long query runs", || {
        value(&scrape(&server), "bicraft_tool_calls_running") == Some(1.0)
    });
    let drain = std::thread::spawn(move || server.drain(Duration::from_millis(800)));
    // the metrics listener stays up while the server drains
    wait_until("the drain shows", || {
        let r = request(m, "GET", "/metrics", b"");
        r.status == 200 && value(&r.text(), "bicraft_draining") == Some(1.0)
    });
    let report = drain.join().unwrap();
    assert_eq!(report.cancelled, 1, "{report:?}");
    drop(long);
    // after the drain the metrics listener is closed too
    assert!(
        TcpStream::connect_timeout(&m, Duration::from_secs(2)).is_err()
            || request(m, "GET", "/metrics", b"").status != 200
    );
}

#[test]
fn a_refused_reader_call_is_logged_and_counted_as_refused() {
    use bicraft_auth::tokens::{self, entry_json};
    use bicraft_auth::{Access, AuthConfig, Authenticator, Grant};
    let dir = TempDir::new("metrics-reader");
    std::fs::create_dir_all(dir.path().join("a")).unwrap();
    std::fs::write(dir.path().join("a").join("model.bim"), MODEL).unwrap();
    let token = tokens::generate().unwrap();
    let mut auth = AuthConfig::default();
    let doc = json!([entry_json("rd", &tokens::hash_token(&token).unwrap(), &[], &[])]);
    auth.add_tokens(tokens::parse(&doc).unwrap()).unwrap();
    auth.acl
        .grants
        .push(Grant::parse_text_as("token:rd=a", Access::Reader).unwrap());
    let lines = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&lines);
    let config = HttpConfig {
        auth: Some(Arc::new(Authenticator::new(auth).unwrap())),
        metrics_listen: Some("127.0.0.1:0".parse().unwrap()),
        log_format: LogFormat::Json,
        log_sink: Some(LogSink(Arc::new(move |l: &str| {
            sink.lock().unwrap().push(l.to_string())
        }))),
        ..HttpConfig::default()
    };
    let server = HttpServer::start("127.0.0.1:0".parse().unwrap(), dir.path(), config).unwrap();
    let a = server.addr();
    let call_as = |tool: &str, args: Value| {
        let mut bytes = call_bytes("a", tool, args, false);
        let at = bytes.windows(2).position(|w| w == b"\r\n").unwrap() + 2;
        let auth = format!("Authorization: Bearer {token}\r\n");
        bytes.splice(at..at, auth.into_bytes());
        let mut s = connect(a);
        s.write_all(&bytes).unwrap();
        let r = read_response(s);
        assert_eq!(r.status, 200, "{}", r.text());
        r.message()["result"].clone()
    };
    assert_eq!(
        call_as("open_file", json!({"path": "model.bim"}))["isError"],
        false
    );
    // a reader may not edit: refused before anything runs
    let r = call_as(
        "add_measure",
        json!({"table": "Product", "name": SECRET, "expression": "1"}),
    );
    assert_eq!(r["isError"], true, "{r}");
    let metrics = scrape(&server);
    assert_eq!(
        value(
            &metrics,
            "bicraft_commands_total{command=\"measure.add\",outcome=\"refused\"}"
        ),
        Some(1.0),
        "{metrics}"
    );
    assert_eq!(
        value(
            &metrics,
            "bicraft_commands_total{command=\"file.open\",outcome=\"ok\"}"
        ),
        Some(1.0)
    );
    let log = lines.lock().unwrap().clone();
    assert!(
        log.iter().all(|l| !l.contains(SECRET) && !l.contains(&token)),
        "{log:#?}"
    );
    let refused: Value = log
        .iter()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .find(|v| v["command"] == "measure.add")
        .unwrap();
    assert_eq!(refused["caller"], "token");
    assert_eq!(refused["outcome"], "refused");
    assert_eq!(refused["error"], "forbidden");
    assert_eq!(refused["status"], 200);
    let report = server.drain(Duration::from_secs(5));
    assert!(report.clean(), "{report:?}");
}
