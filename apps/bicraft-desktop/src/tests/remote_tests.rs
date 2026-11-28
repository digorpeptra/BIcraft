//! Remote mode against an in-process `bicraft-cli serve` with API tokens: a reader and an editor
//! connect, see the saved report and click on it (their views only); a wrong token is a clear
//! 401; a server restart is survived.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use bicraft_auth::tokens::{self, entry_json};
use bicraft_auth::{Access, AuthConfig, Authenticator, Grant};
use bicraft_mcp::http::HTTP_PROTOCOL_VERSION;
use bicraft_mcp::{HttpConfig, HttpServer};
use serde_json::{Value, json};

use super::{Temp, click_hit, harness, ready, value, wait};
use crate::App;
use bicraft_ui::source::Server;

struct Tokens {
    editor: String,
    reader: String,
}

fn config(t: &Tokens) -> HttpConfig {
    let mut c = AuthConfig::default();
    let entries: Vec<Value> = [("ed", &t.editor), ("rd", &t.reader)]
        .iter()
        .map(|(name, token)| entry_json(name, &tokens::hash_token(token).unwrap(), &[], &[]))
        .collect();
    c.add_tokens(tokens::parse(&Value::Array(entries)).unwrap())
        .unwrap();
    for (g, access) in [("token:ed=a", Access::Editor), ("token:rd=a", Access::Reader)] {
        c.acl.grants.push(Grant::parse_text_as(g, access).unwrap());
    }
    HttpConfig {
        auth: Some(Arc::new(Authenticator::new(c).unwrap())),
        keepalive: Duration::from_secs(1),
        ..HttpConfig::default()
    }
}

/// One MCP `run_command` as the editor.
fn run(addr: SocketAddr, token: &str, command: &str, params: Value) -> Value {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {
        "name": "run_command", "arguments": {"command": command, "params": params}, "_meta": {
            "io.modelcontextprotocol/protocolVersion": HTTP_PROTOCOL_VERSION,
            "io.modelcontextprotocol/clientCapabilities": {}}}})
    .to_string();
    let req = format!(
        "POST /w/a/mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nAccept: application/json\r\n\
         MCP-Protocol-Version: {HTTP_PROTOCOL_VERSION}\r\nMcp-Method: tools/call\r\nMcp-Name: run_command\r\n\
         Authorization: Bearer {token}\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
    s.write_all(req.as_bytes()).unwrap();
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    let text = String::from_utf8_lossy(&out).into_owned();
    let body = text.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("");
    let v: Value = serde_json::from_str(body).unwrap_or_else(|e| panic!("{e}: {text}"));
    assert_eq!(v["result"]["isError"], false, "{command}: {v}");
    v["result"]["structuredContent"].clone()
}

fn start(root: &Path, t: &Tokens, addr: &str) -> HttpServer {
    HttpServer::start(addr.parse().unwrap(), root, config(t)).unwrap()
}

/// The fixture saved in workspace `a`, opened by the editor (what readers read).
fn workspace(name: &str) -> (Temp, super::Shop) {
    let dir = Temp::new(name);
    let ws = dir.path().join("a");
    std::fs::create_dir_all(&ws).unwrap();
    let (_, shop) = super::shop(&ws);
    (dir, shop)
}

fn app_for(addr: SocketAddr, token: &str) -> App {
    let mut app = crate::app("en-US", None).with_sync_render();
    app.open_server(Server {
        url: format!("http://{addr}"),
        workspace: "a".into(),
        token: token.into(),
    });
    app
}

#[test]
fn readers_and_editors_read_and_click_remotely() {
    let t = Tokens {
        editor: tokens::generate().unwrap(),
        reader: tokens::generate().unwrap(),
    };
    let (dir, n) = workspace("remote");
    let server = start(dir.path(), &t, "127.0.0.1:0");
    let addr = server.addr();
    run(addr, &t.editor, "file.open", json!({"path": "shop.pbip"}));

    // a reader: the saved report, their own view
    let mut r = harness(app_for(addr, &t.reader), (1280.0, 800.0));
    wait(&mut r, "the reader's snapshot", ready);
    assert_eq!(r.state().access.as_deref(), Some("reader"));
    assert!(
        r.state().title().starts_with("shop.pbip · rev "),
        "{}",
        r.state().title()
    );
    assert_eq!(value(r.state(), &n.card), 175.0);
    let sent = click_hit(&mut r, &n.slicer, false, |a| {
        a["action"] == "slicer_toggle" && a["value"] == "Toys"
    });
    assert_eq!(sent["visual"], n.slicer.as_str());
    assert_eq!(value(r.state(), &n.card), 5.0);

    // an editor: another view, untouched by the reader's click
    let mut e = harness(app_for(addr, &t.editor), (1280.0, 800.0));
    wait(&mut e, "the editor's snapshot", ready);
    assert_eq!(e.state().access.as_deref(), Some("editor"));
    assert_eq!(value(e.state(), &n.card), 175.0);

    // the editor changes the model over MCP: the editor's window follows (change feed)
    run(
        addr,
        &t.editor,
        "measure.set",
        json!({"name": "Total", "expression": "SUM(Sales[Amount]) * 2"}),
    );
    wait(&mut e, "the editor's live update", |a| {
        value(a, &n.card) == 350.0 && ready(a)
    });
    // readers keep the saved state until it is saved
    r.step();
    assert_eq!(value(r.state(), &n.card), 5.0);

    // a server restart: the views are gone; the window reconnects and opens a new one
    drop(server);
    let server = start(dir.path(), &t, &addr.to_string());
    run(addr, &t.editor, "file.open", json!({"path": "shop.pbip"}));
    // the first click after the restart meets the old view: the client replaces it
    let before = r.state().answers;
    let pos = super::hit_pos(r.state(), &n.slicer, |a| a["value"] == "Games");
    super::click(&mut r, pos, false);
    wait(&mut r, "an answer", |a| a.answers > before);
    let sent = r.state().last_action.clone().unwrap();
    assert_eq!(sent["value"], "Games");
    wait(&mut r, "the reader after the restart", |a| {
        value(a, &n.card) == 20.0 && ready(a)
    });
    drop(server);
}

#[test]
fn authentication_failures_are_shown() {
    let t = Tokens {
        editor: tokens::generate().unwrap(),
        reader: tokens::generate().unwrap(),
    };
    let (dir, _) = workspace("remote-auth");
    let server = start(dir.path(), &t, "127.0.0.1:0");
    let addr = server.addr();
    let mut h = harness(app_for(addr, &tokens::generate().unwrap()), (900.0, 600.0));
    wait(&mut h, "the 401", |a| a.error.is_some());
    let err = h.state().error.clone().unwrap();
    assert!(err.contains("401"), "{err}");
    // the token never appears in what the window shows
    assert!(!err.contains("bct_"));
    // an unknown workspace looks missing
    let mut app = crate::app("en-US", None).with_sync_render();
    app.open_server(Server {
        url: format!("http://{addr}"),
        workspace: "nope".into(),
        token: t.reader.clone(),
    });
    let mut h = harness(app, (900.0, 600.0));
    wait(&mut h, "the 404", |a| a.error.is_some());
    assert!(h.state().error.clone().unwrap().contains("404"));
    // a server that is not there
    drop(server);
    let mut h = harness(app_for(addr, &t.reader), (900.0, 600.0));
    wait(&mut h, "the network error", |a| a.error.is_some());
    assert!(h.state().error.clone().unwrap().contains("cannot reach"));
}
