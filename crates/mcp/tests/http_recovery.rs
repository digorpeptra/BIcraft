//! Auto-recovery on `bicraft-cli serve` (docs/recovery.md): with `recovery_dir`, a document view
//! that expires with unsaved edits, a server restart, and the workspace's own session leave a
//! journal; reopening the file offers it back, to the same identity only.

#[path = "../../../testsupport/tempdir.rs"]
mod tempdir;

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bicraft_auth::tokens::{self, entry_json};
use bicraft_auth::{Access, AuthConfig, Authenticator, Grant};
use bicraft_mcp::http::HTTP_PROTOCOL_VERSION;
use bicraft_mcp::{HttpConfig, HttpServer};
use serde_json::{Value, json};
use tempdir::TempDir;

const MODEL: &str = include_str!("../../engine/tests/table_functions/model.bim");

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

fn request(addr: SocketAddr, method: &str, path: &str, headers: &str, body: &[u8]) -> Response {
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n{headers}Content-Length: {}\r\n\r\n",
        body.len()
    );
    s.write_all(req.as_bytes()).unwrap();
    s.write_all(body).unwrap();
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    let split = out.windows(4).position(|w| w == b"\r\n\r\n").unwrap_or(out.len());
    let head = String::from_utf8_lossy(&out[..split]).into_owned();
    Response {
        status: head.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0),
        body: out.get(split + 4..).unwrap_or_default().to_vec(),
    }
}

#[derive(Clone)]
struct View {
    vid: String,
    token: String,
}

fn view(addr: SocketAddr, auth: &str, file: &str) -> View {
    let body = json!({"file": file}).to_string();
    let headers = format!("{auth}Content-Type: application/json\r\n");
    let r = request(addr, "POST", "/w/a/view/api/session", &headers, body.as_bytes());
    assert_eq!(r.status, 200, "{}", r.text());
    let j = r.json();
    View {
        vid: j["view"].as_str().unwrap().to_string(),
        token: j["token"].as_str().unwrap().to_string(),
    }
}

fn call_at(addr: SocketAddr, path: &str, extra: &str, command: &str, params: Value) -> Value {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {
        "name": "run_command", "arguments": {"command": command, "params": params}, "_meta": {
            "io.modelcontextprotocol/protocolVersion": HTTP_PROTOCOL_VERSION,
            "io.modelcontextprotocol/clientCapabilities": {}}}})
    .to_string();
    let headers = format!(
        "Content-Type: application/json\r\nAccept: application/json\r\n\
         MCP-Protocol-Version: {HTTP_PROTOCOL_VERSION}\r\nMcp-Method: tools/call\r\nMcp-Name: run_command\r\n{extra}"
    );
    let r = request(addr, "POST", path, &headers, body.as_bytes());
    assert_eq!(r.status, 200, "{}", r.text());
    r.json()["result"].clone()
}

fn on_view(addr: SocketAddr, auth: &str, v: &View, command: &str, params: Value) -> Value {
    call_at(
        addr,
        &format!("/w/a/mcp/view/{}", v.vid),
        &format!("{auth}X-Bicraft-Token: {}\r\n", v.token),
        command,
        params,
    )
}

fn on_workspace(addr: SocketAddr, auth: &str, command: &str, params: Value) -> Value {
    call_at(addr, "/w/a/mcp", auth, command, params)
}

fn ok(v: Value) -> Value {
    assert_eq!(v["isError"], false, "{v}");
    v["structuredContent"].clone()
}

fn add_measure(addr: SocketAddr, auth: &str, v: &View, name: &str) {
    ok(on_view(
        addr,
        auth,
        v,
        "measure.add",
        json!({"table": "Product", "name": name, "expression": "1"}),
    ));
}

fn state(addr: SocketAddr, auth: &str, v: &View) -> Response {
    request(
        addr,
        "GET",
        &format!("/w/a/view/api/state?v={}", v.vid),
        auth,
        b"",
    )
}

/// A workspace `a` with two models, and the recovery folder beside the server root.
fn workspace(name: &str) -> (TempDir, PathBuf) {
    let dir = TempDir::new(name);
    let root = dir.path().join("root");
    let a = root.join("a");
    std::fs::create_dir_all(&a).unwrap();
    for f in ["one.bim", "two.bim"] {
        std::fs::write(a.join(f), MODEL).unwrap();
    }
    (dir, root)
}

fn config(dir: &TempDir) -> HttpConfig {
    HttpConfig {
        recovery_dir: Some(dir.path().join("recovery")),
        ..HttpConfig::default()
    }
}

fn start(root: &std::path::Path, config: HttpConfig) -> HttpServer {
    HttpServer::start("127.0.0.1:0".parse().unwrap(), root, config).unwrap()
}

fn measures(snapshot_text: &str, name: &str) -> bool {
    snapshot_text.contains(&format!("\"{name}\""))
}

/// Waits (a server's sessions go with its last connection thread) until `f` holds.
fn eventually(what: &str, f: impl Fn() -> bool) {
    let until = Instant::now() + Duration::from_secs(20);
    while !f() {
        assert!(Instant::now() < until, "timed out: {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn an_expired_view_keeps_its_edits_and_the_file_offers_them_back() {
    let (dir, root) = workspace("recovery-view-expiry");
    let server = start(
        &root,
        HttpConfig {
            view_idle: Duration::from_millis(200),
            ..config(&dir)
        },
    );
    let a = server.addr();
    let v = view(a, "", "two.bim");
    add_measure(a, "", &v, "Unsaved");
    let snap = state(a, "", &v).json();
    assert_eq!(snap["recovery"]["edits"], 1, "{}", snap["recovery"]);
    assert_eq!(snap["view"]["limits"]["recovery"], true);
    assert_eq!(snap["view"]["limits"]["autosave"], false);
    // nothing written next to the workspace's files
    assert!(std::fs::read_dir(root.join("a")).unwrap().count() == 2);
    // four times the idle time later, the view is gone
    std::thread::sleep(Duration::from_millis(1200));
    eventually("the dirty view expires", || state(a, "", &v).status == 404);
    // the user opens the file again: the edits are offered
    let again = view(a, "", "two.bim");
    let list = ok(on_view(a, "", &again, "recovery.list", json!({})));
    let offers = list["offers"].as_array().unwrap();
    assert_eq!(offers.len(), 1, "{list}");
    assert_eq!(offers[0]["path"], "two.bim");
    assert_eq!(offers[0]["base"], "same");
    let out = ok(on_view(a, "", &again, "recovery.apply", json!({})));
    assert_eq!(out["complete"], true, "{out}");
    assert!(measures(&state(a, "", &again).text(), "Unsaved"));
    // the file on disk is still the saved one
    assert!(
        !std::fs::read_to_string(root.join("a/two.bim"))
            .unwrap()
            .contains("Unsaved")
    );
}

#[test]
fn a_server_restart_loses_nothing() {
    let (dir, root) = workspace("recovery-restart");
    let server = start(&root, config(&dir));
    let a = server.addr();
    let v = view(a, "", "one.bim");
    add_measure(a, "", &v, "InView");
    // the workspace's own session (an agent's MCP client) too
    ok(on_workspace(a, "", "file.open", json!({"path": "two.bim"})));
    ok(on_workspace(
        a,
        "",
        "measure.add",
        json!({"table": "Product", "name": "InWorkspace", "expression": "2"}),
    ));
    drop(server);
    let server = start(&root, config(&dir));
    let a = server.addr();
    let v = view(a, "", "one.bim");
    eventually("the old sessions released their journals", || {
        let l = ok(on_view(a, "", &v, "recovery.list", json!({"all": true})));
        // the view's own journal: the workspace session's is not this identity's
        l["offers"].as_array().is_some_and(|o| o.len() == 1)
    });
    let out = ok(on_view(a, "", &v, "recovery.apply", json!({})));
    assert_eq!(out["complete"], true, "{out}");
    assert!(measures(&state(a, "", &v).text(), "InView"));
    ok(on_workspace(a, "", "file.open", json!({"path": "two.bim"})));
    let out = ok(on_workspace(a, "", "recovery.apply", json!({})));
    assert_eq!(out["applied"], 1, "{out}");
    let s = ok(on_workspace(
        a,
        "",
        "measure.get",
        json!({"table": "Product", "name": "InWorkspace"}),
    ));
    assert!(s.to_string().contains("InWorkspace"));
}

#[test]
fn journals_are_offered_to_their_own_identity_only() {
    let (dir, root) = workspace("recovery-identity");
    let t = (tokens::generate().unwrap(), tokens::generate().unwrap());
    let mut c = AuthConfig::default();
    let entries: Vec<Value> = [("ed", &t.0), ("ed2", &t.1)]
        .iter()
        .map(|(name, token)| entry_json(name, &tokens::hash_token(token).unwrap(), &[], &[]))
        .collect();
    c.add_tokens(tokens::parse(&Value::Array(entries)).unwrap())
        .unwrap();
    for g in ["token:ed=a", "token:ed2=a"] {
        c.acl
            .grants
            .push(Grant::parse_text_as(g, Access::Editor).unwrap());
    }
    let cfg = HttpConfig {
        auth: Some(Arc::new(Authenticator::new(c).unwrap())),
        ..config(&dir)
    };
    let (ed, ed2) = (
        format!("Authorization: Bearer {}\r\n", t.0),
        format!("Authorization: Bearer {}\r\n", t.1),
    );
    let server = start(&root, cfg.clone());
    let a = server.addr();
    let v = view(a, &ed, "one.bim");
    add_measure(a, &ed, &v, "Private");
    drop(server);
    let server = start(&root, cfg);
    let a = server.addr();
    let other = view(a, &ed2, "one.bim");
    let mine = view(a, &ed, "one.bim");
    eventually("the journal is offered to its owner", || {
        let l = ok(on_view(a, &ed, &mine, "recovery.list", json!({"all": true})));
        l["offers"].as_array().is_some_and(|o| o.len() == 1)
    });
    let l = ok(on_view(a, &ed2, &other, "recovery.list", json!({"all": true})));
    assert_eq!(l["offers"], json!([]), "another identity sees the journal");
    assert_eq!(
        on_view(a, &ed2, &other, "recovery.apply", json!({}))["isError"],
        true
    );
    // the journal names no user: its owner is a hash
    let owners: Vec<String> = std::fs::read_dir(dir.path().join("recovery").join("a"))
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
        .filter_map(|e| {
            let text = std::fs::read_to_string(e.path()).ok()?;
            let head: Value = serde_json::from_str(text.lines().next()?).ok()?;
            head["owner"].as_str().map(str::to_string)
        })
        .collect();
    assert!(!owners.is_empty());
    assert!(
        owners.iter().all(|o| o.len() == 17 && o.starts_with('u')),
        "{owners:?}"
    );
    let out = ok(on_view(a, &ed, &mine, "recovery.apply", json!({})));
    assert_eq!(out["complete"], true);
}
