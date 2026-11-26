//! Reader and editor access over real sockets (docs/auth.md, "Roles"): what a reader sees in
//! `tools/list` and `list_commands`, every refusal path (convenience tools, `run_command`,
//! `batch`, writing parameters, the file routes), a reader's private `file.open`, reads that
//! still work (after a restart and after an idle close too), and an editor still doing
//! everything.

#[path = "../../../testsupport/tempdir.rs"]
mod tempdir;

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bicraft_auth::tokens::{self, entry_json};
use bicraft_auth::{Access, AuthConfig, Authenticator, Grant};
use bicraft_mcp::http::HTTP_PROTOCOL_VERSION;
use bicraft_mcp::{HttpConfig, HttpServer};
use serde_json::{Value, json};
use tempdir::TempDir;

const MODEL: &str = include_str!("../../engine/tests/table_functions/model.bim");

struct Tokens {
    editor: String,
    reader: String,
    /// In group `team` (editor grant) and named by a reader grant: an editor.
    both: String,
}

fn config(t: &Tokens, idle: Option<Duration>) -> HttpConfig {
    let mut c = AuthConfig::default();
    let entries: Vec<Value> = [
        ("ed", &t.editor, vec![]),
        ("rd", &t.reader, vec![]),
        ("both", &t.both, vec!["team".to_string()]),
    ]
    .iter()
    .map(|(name, token, groups)| entry_json(name, &tokens::hash_token(token).unwrap(), &[], groups))
    .collect();
    c.add_tokens(tokens::parse(&Value::Array(entries)).unwrap())
        .unwrap();
    for (g, access) in [
        ("token:ed=a", Access::Editor),
        ("group:team=a", Access::Editor),
        ("token:rd=a", Access::Reader),
        ("token:both=a", Access::Reader),
    ] {
        c.acl.grants.push(Grant::parse_text_as(g, access).unwrap());
    }
    HttpConfig {
        auth: Some(Arc::new(Authenticator::new(c).unwrap())),
        idle_timeout: idle,
        ..HttpConfig::default()
    }
}

fn tokens() -> Tokens {
    Tokens {
        editor: tokens::generate().unwrap(),
        reader: tokens::generate().unwrap(),
        both: tokens::generate().unwrap(),
    }
}

fn start(dir: &TempDir, config: HttpConfig) -> HttpServer {
    HttpServer::start("127.0.0.1:0".parse().unwrap(), dir.path(), config).unwrap()
}

#[derive(Debug)]
struct Response {
    status: u16,
    body: Vec<u8>,
}

impl Response {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|e| panic!("{e}: {}", self.text()))
    }
}

fn send(addr: SocketAddr, bytes: &[u8]) -> Response {
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
    s.write_all(bytes).unwrap();
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    let split = out.windows(4).position(|w| w == b"\r\n\r\n").unwrap_or(out.len());
    let head = String::from_utf8_lossy(&out[..split]).into_owned();
    Response {
        status: head.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0),
        body: out.get(split + 4..).unwrap_or_default().to_vec(),
    }
}

fn rpc(addr: SocketAddr, token: &str, method: &str, name: Option<&str>, mut params: Value) -> Value {
    params["_meta"] = json!({
        "io.modelcontextprotocol/protocolVersion": HTTP_PROTOCOL_VERSION,
        "io.modelcontextprotocol/clientCapabilities": {}});
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string();
    let name_header = name.map(|n| format!("Mcp-Name: {n}\r\n")).unwrap_or_default();
    let req = format!(
        "POST /w/a/mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
         Accept: application/json\r\nMCP-Protocol-Version: {HTTP_PROTOCOL_VERSION}\r\n\
         Mcp-Method: {method}\r\n{name_header}Authorization: Bearer {token}\r\n\
         Content-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let r = send(addr, req.as_bytes());
    assert_eq!(r.status, 200, "{}", r.text());
    r.json()["result"].clone()
}

/// A tool's result object.
fn call(addr: SocketAddr, token: &str, tool: &str, args: Value) -> Value {
    rpc(
        addr,
        token,
        "tools/call",
        Some(tool),
        json!({"name": tool, "arguments": args}),
    )
}

fn ok(addr: SocketAddr, token: &str, tool: &str, args: Value) -> Value {
    let r = call(addr, token, tool, args);
    assert_eq!(r["isError"], false, "{tool}: {r}");
    r["structuredContent"].clone()
}

/// A refused call: a tool error (the shape of a server-denied command), with its message.
fn refused(addr: SocketAddr, token: &str, tool: &str, args: Value) -> Value {
    let r = call(addr, token, tool, args);
    assert_eq!(r["isError"], true, "{tool}: {r}");
    r["structuredContent"].clone()
}

fn tool_names(addr: SocketAddr, token: &str) -> Vec<String> {
    rpc(addr, token, "tools/list", None, json!({}))["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect()
}

fn command_ids(addr: SocketAddr, token: &str) -> Vec<String> {
    ok(addr, token, "list_commands", json!({}))["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap().to_string())
        .collect()
}

fn measure_names(addr: SocketAddr, token: &str) -> String {
    ok(addr, token, "model_summary", json!({})).to_string()
}

fn file_request(addr: SocketAddr, token: &str, method: &str, path: &str, body: &[u8]) -> Response {
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\n\
         Content-Length: {}\r\n\r\n",
        body.len()
    );
    let mut bytes = req.into_bytes();
    bytes.extend_from_slice(body);
    send(addr, &bytes)
}

fn workspace(name: &str) -> TempDir {
    let dir = TempDir::new(name);
    std::fs::create_dir_all(dir.path().join("a")).unwrap();
    std::fs::write(dir.path().join("a").join("model.bim"), MODEL).unwrap();
    dir
}

#[test]
fn readers_see_and_run_only_read_only_commands() {
    let t = tokens();
    let dir = workspace("roles-mcp");
    let server = start(&dir, config(&t, None));
    let a = server.addr();
    let (rd, ed) = (t.reader.as_str(), t.editor.as_str());

    // tools/list: readers do not see the tools they may never run
    let reader_tools = tool_names(a, rd);
    for visible in [
        "list_commands",
        "run_command",
        "batch",
        "open_file",
        "inspect_file",
        "model_summary",
        "validate_dax",
        "validate_model",
        "model_usage",
        "model_unused",
        "evaluate_dax_query",
        "render_page",
    ] {
        assert!(
            reader_tools.iter().any(|n| n == visible),
            "{visible}: {reader_tools:?}"
        );
    }
    for hidden in [
        "add_measure",
        "save",
        "save_as",
        "new_model",
        "convert",
        "refresh_model",
        "add_page",
        "remove_unused",
    ] {
        assert!(
            !reader_tools.iter().any(|n| n == hidden),
            "{hidden}: {reader_tools:?}"
        );
    }
    let editor_tools = tool_names(a, ed);
    assert!(editor_tools.iter().any(|n| n == "add_measure"));
    assert!(editor_tools.len() > reader_tools.len());

    // list_commands: only what a reader may run
    let reader_cmds = command_ids(a, rd);
    assert!(reader_cmds.iter().any(|c| c == "model.summary"));
    assert!(reader_cmds.iter().any(|c| c == "file.open"));
    for hidden in [
        "measure.add",
        "file.save",
        "model.refresh",
        "theme.export",
        "view.interact",
    ] {
        assert!(!reader_cmds.iter().any(|c| c == hidden), "{hidden}");
    }
    let filtered = ok(a, rd, "list_commands", json!({"filter": "measure"}));
    let filtered = filtered["commands"].as_array().unwrap();
    assert!(filtered.iter().all(|c| c["id"] != "measure.add"), "{filtered:?}");
    assert!(command_ids(a, ed).iter().any(|c| c == "measure.add"));

    // a reader opens the file on a fresh session (nothing open, nothing unsaved) and reads it
    ok(a, rd, "open_file", json!({"path": "model.bim"}));
    assert!(measure_names(a, rd).contains("Avg Price"));
    ok(a, rd, "validate_model", json!({}));
    let rows = ok(
        a,
        rd,
        "evaluate_dax_query",
        json!({"query": "EVALUATE ROW(\"x\", 1 + 1)"}),
    );
    assert_eq!(rows["results"][0]["rows"][0]["[x]"], 2, "{rows}");
    ok(
        a,
        rd,
        "run_command",
        json!({"command": "measure.get", "params": {"name": "Avg Price"}}),
    );

    // refusals: convenience tools, run_command, writing parameters, mutating commands
    let e = refused(
        a,
        rd,
        "add_measure",
        json!({"table": "Product", "name": "X", "expression": "1"}),
    );
    assert!(
        e["error"].as_str().unwrap().contains("not available to a reader"),
        "{e}"
    );
    refused(a, rd, "save", json!({}));
    refused(a, rd, "save_as", json!({"path": "copy.bim"}));
    refused(a, rd, "new_model", json!({"name": "N"}));
    for (command, params) in [
        ("file.save", json!({})),
        (
            "measure.add",
            json!({"table": "Product", "name": "X", "expression": "1"}),
        ),
        ("model.refresh", json!({})),
        ("model.export_vpax", json!({"path": "m.vpax"})),
        ("theme.export", json!({"path": "t.json"})),
        ("file.convert", json!({"from": "model.bim", "to": "c.bim"})),
        ("view.interact", json!({"action": "reset"})),
        ("view.set_language", json!({"ui_language": "fr-FR"})),
        ("dax.clear_cache", json!({})),
        ("dax.memory", json!({"budget": "10M"})),
        ("dax.aggregations", json!({"enabled": false})),
        ("edit.undo", json!({})),
    ] {
        let e = refused(
            a,
            rd,
            "run_command",
            json!({"command": command, "params": params}),
        );
        assert!(e["error"].as_str().unwrap().contains("reader"), "{command}: {e}");
    }
    let e = refused(a, rd, "render_page", json!({"out": "page.png"}));
    assert!(e["error"].as_str().unwrap().contains("`out`"), "{e}");
    ok(a, rd, "run_command", json!({"command": "dax.memory"}));

    // batch: one refused step refuses the whole batch, nothing runs
    let e = refused(
        a,
        rd,
        "batch",
        json!({"atomic": false, "commands": [
            {"command": "model.summary"},
            {"command": "measure.add", "params": {"table": "Product", "name": "Sneaky", "expression": "1"}},
        ]}),
    );
    assert_eq!(e["failed_index"], 1, "{e}");
    assert_eq!(e["failed_command"], "measure.add");
    assert_eq!(e["results"], json!([]));
    assert!(!measure_names(a, rd).contains("Sneaky"));
    let all_read = ok(
        a,
        rd,
        "batch",
        json!({"commands": [{"command": "model.summary"}, {"command": "dax.validate", "params": {"expression": "1"}}]}),
    );
    assert_eq!(all_read["results"].as_array().unwrap().len(), 2);

    // nothing a reader did wrote a file
    let mut files: Vec<String> = std::fs::read_dir(dir.path().join("a"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| !n.starts_with(".bicraft"))
        .collect();
    files.sort();
    assert_eq!(files, ["model.bim"]);

    // a reader's file.open is private (docs/auth.md, "Roles"): while the editor holds unsaved
    // changes it is allowed, the reader reads the saved file, and the editor's draft survives
    ok(a, ed, "open_file", json!({"path": "model.bim"}));
    ok(
        a,
        ed,
        "add_measure",
        json!({"table": "Product", "name": "Draft", "expression": "1"}),
    );
    ok(a, rd, "open_file", json!({"path": "model.bim"}));
    assert!(
        !measure_names(a, rd).contains("Draft"),
        "readers see the saved file"
    );
    let e = refused(
        a,
        rd,
        "batch",
        json!({"commands": [{"command": "file.open", "params": {"path": "model.bim"}}]}),
    );
    assert!(e["error"].as_str().unwrap().contains("call of its own"), "{e}");
    assert!(
        measure_names(a, ed).contains("Draft"),
        "the draft survives in the editor's session"
    );

    // an editor still does everything: edit, save, open, export
    ok(a, ed, "save", json!({}));
    assert!(measure_names(a, rd).contains("Draft"), "saved: readers see it");
    ok(a, ed, "save_as", json!({"path": "copy.bim"}));
    ok(
        a,
        ed,
        "run_command",
        json!({"command": "view.set_language", "params": {"ui_language": "fr-FR"}}),
    );
    ok(
        a,
        ed,
        "batch",
        json!({"commands": [
            {"command": "measure.add", "params": {"table": "Product", "name": "B1", "expression": "1"}},
            {"command": "file.save"},
        ]}),
    );
    assert!(
        std::fs::read_to_string(dir.path().join("a").join("copy.bim"))
            .unwrap()
            .contains("B1")
    );

    // precedence: a reader grant and an editor grant (by group) on the same workspace: editor
    ok(
        a,
        &t.both,
        "add_measure",
        json!({"table": "Product", "name": "ByGroup", "expression": "2"}),
    );
    assert!(tool_names(a, &t.both).iter().any(|n| n == "add_measure"));
}

#[test]
fn readers_on_the_file_routes_get_only() {
    let t = tokens();
    let dir = workspace("roles-files");
    let server = start(&dir, config(&t, None));
    let a = server.addr();
    let (rd, ed) = (t.reader.as_str(), t.editor.as_str());
    let list = file_request(a, rd, "GET", "/w/a/files/", b"");
    assert_eq!(list.status, 200, "{}", list.text());
    assert!(list.text().contains("model.bim"));
    let down = file_request(a, rd, "GET", "/w/a/files/model.bim", b"");
    assert_eq!(down.status, 200);
    assert_eq!(down.body, MODEL.as_bytes());
    assert_eq!(
        file_request(a, rd, "HEAD", "/w/a/files/model.bim", b"").status,
        200
    );
    for (method, path) in [
        ("PUT", "/w/a/files/new.txt"),
        ("PUT", "/w/a/files/model.bim"),
        ("POST", "/w/a/files/"),
        ("DELETE", "/w/a/files/model.bim"),
    ] {
        let r = file_request(a, rd, method, path, b"x");
        assert_eq!(r.status, 403, "{method} {path}: {}", r.text());
    }
    assert!(!dir.path().join("a").join("new.txt").exists());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a").join("model.bim")).unwrap(),
        MODEL
    );
    // the editor writes and deletes
    assert_eq!(file_request(a, ed, "PUT", "/w/a/files/new.txt", b"x").status, 201);
    assert_eq!(
        file_request(a, ed, "DELETE", "/w/a/files/new.txt", b"").status,
        204
    );
}

#[test]
fn readers_after_a_restart_and_an_idle_close() {
    let t = tokens();
    let dir = workspace("roles-restart");
    // an editor opens the file, then the server restarts: the new session has nothing open, and a
    // reader opens the file itself
    let server = start(&dir, config(&t, None));
    ok(
        server.addr(),
        &t.editor,
        "open_file",
        json!({"path": "model.bim"}),
    );
    drop(server.drain(Duration::from_secs(1)));
    let server = start(&dir, config(&t, Some(Duration::from_millis(300))));
    let a = server.addr();
    let e = refused(a, &t.reader, "model_summary", json!({}));
    assert!(
        !e["error"].as_str().unwrap().contains("reader"),
        "nothing open: {e}"
    );
    ok(a, &t.reader, "open_file", json!({"path": "model.bim"}));
    assert!(measure_names(a, &t.reader).contains("Avg Price"));
    // the idle close drops the session; the reader's next call reopens the same file lazily
    let start_wait = Instant::now();
    while server.open_workspaces() > 0 {
        assert!(
            start_wait.elapsed() < Duration::from_secs(30),
            "the workspace never closed"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(measure_names(a, &t.reader).contains("Avg Price"));
    assert_eq!(server.open_workspaces(), 1);
}

/// Minutes of evaluation: only a cancel ends it.
const LONG: &str =
    "EVALUATE ROW(\"x\", SUMX(GENERATESERIES(1, 100000), SUMX(GENERATESERIES(1, 100000), [Value])))";

/// Readers and editors on one workspace: a reader's read runs beside an editor's long read on
/// the clean editor session; an editor's write waits for the reads; after it (unsaved) the
/// reader reads the saved state from a copy; a reader cannot name `expected_revision`.
#[test]
fn readers_and_editors_share_a_workspace() {
    let t = tokens();
    let dir = workspace("roles-concurrency");
    let server = start(&dir, config(&t, None));
    let a = server.addr();
    let (rd, ed) = (t.reader.as_str(), t.editor.as_str());
    ok(a, ed, "open_file", json!({"path": "model.bim"}));
    // the editor's long read, as an SSE call that a closed connection cancels
    let body = json!({"jsonrpc": "2.0", "id": 9, "method": "tools/call", "params": {
        "name": "evaluate_dax_query", "arguments": {"query": LONG, "timeout_ms": 600_000},
        "_meta": {"io.modelcontextprotocol/protocolVersion": HTTP_PROTOCOL_VERSION,
                  "io.modelcontextprotocol/clientCapabilities": {}}}})
    .to_string();
    let req = format!(
        "POST /w/a/mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
         Accept: application/json, text/event-stream\r\nMCP-Protocol-Version: {HTTP_PROTOCOL_VERSION}\r\n\
         Mcp-Method: tools/call\r\nMcp-Name: evaluate_dax_query\r\nAuthorization: Bearer {ed}\r\n\
         Content-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let mut long = TcpStream::connect(a).unwrap();
    long.write_all(req.as_bytes()).unwrap();
    let start = Instant::now();
    while !server.metrics_text().contains("\nbicraft_concurrent_reads 1\n") {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "the long read never ran"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    // the reader reads the clean editor session beside it
    let r = ok(
        a,
        rd,
        "evaluate_dax_query",
        json!({"query": "EVALUATE ROW(\"x\", 1 + 1)"}),
    );
    assert_eq!(r["results"][0]["rows"][0]["[x]"], 2, "{r}");
    ok(a, rd, "model_summary", json!({}));
    // a reader cannot make a call conditional
    let e = refused(
        a,
        rd,
        "run_command",
        json!({"command": "model.summary", "expected_revision": 1}),
    );
    assert!(e["error"].as_str().unwrap().contains("expected_revision"), "{e}");
    // the editor's write waits for the long read
    let writer = std::thread::spawn({
        let ed = ed.to_string();
        move || {
            ok(
                a,
                &ed,
                "run_command",
                json!({"command": "measure.add", "params": {"table": "Product", "name": "Unsaved", "expression": "7"}}),
            )
        }
    });
    let start = Instant::now();
    while !server.metrics_text().contains("\nbicraft_tool_calls_queued 1\n") {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "the write never queued"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!writer.is_finished(), "the write passed a running read");
    drop(long);
    writer.join().unwrap();
    // the session has unsaved edits now: the reader reads the saved file, without the measure
    assert!(!measure_names(a, rd).contains("Unsaved"));
    assert!(measure_names(a, ed).contains("Unsaved"));
}
