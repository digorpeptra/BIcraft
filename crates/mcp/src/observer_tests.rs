//! Tests of the session observer.

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::{Server, serve_with_observer};

/// (revision, last command id, last command ok, has model) seen at each call.
type Seen = Arc<Mutex<Vec<(u64, Option<String>, Option<bool>, bool)>>>;

fn observed() -> (Server, Seen) {
    let seen: Seen = Arc::default();
    let log = Arc::clone(&seen);
    let server = Server::new().with_observer(move |s| {
        let last = s.last_command.as_ref();
        log.lock().unwrap().push((
            s.revision,
            last.map(|l| l.id.clone()),
            last.map(|l| l.ok),
            s.model.is_some(),
        ));
    });
    (server, seen)
}

fn msg(id: u64, method: &str, params: Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string()
}

fn call(name: &str, args: Value) -> Value {
    json!({"name": name, "arguments": args})
}

#[test]
fn observer_sees_the_session_after_every_tool_call() {
    let (mut server, seen) = observed();
    server.handle_line(&msg(1, "initialize", json!({"protocolVersion": "2025-06-18"})));
    server.handle_line(&msg(2, "tools/list", json!({})));
    server.handle_line(&msg(3, "ping", json!({})));
    assert!(seen.lock().unwrap().is_empty(), "only tools/call notifies");

    server.handle_line(&msg(4, "tools/call", call("new_model", json!({"name": "M"}))));
    server.handle_line(&msg(5, "tools/call", call("add_table", json!({"name": "T"}))));
    server.handle_line(&msg(
        6,
        "tools/call",
        call(
            "add_measure",
            json!({"table": "T", "name": "X", "expression": "1"}),
        ),
    ));
    // A failed call is observed too, with the error in last_command.
    server.handle_line(&msg(
        7,
        "tools/call",
        call(
            "add_measure",
            json!({"table": "Nope", "name": "Y", "expression": "1"}),
        ),
    ));
    // Read-only: observed, revision unchanged.
    server.handle_line(&msg(8, "tools/call", call("model_summary", json!({}))));
    // Protocol errors (unknown tool) never reach the session.
    server.handle_line(&msg(9, "tools/call", call("no_such_tool", json!({}))));

    let seen = seen.lock().unwrap().clone();
    let s = |id: &str| Some(id.to_string());
    assert_eq!(
        seen,
        vec![
            (1, s("model.new"), Some(true), true),
            (2, s("table.add"), Some(true), true),
            (3, s("measure.add"), Some(true), true),
            (3, s("measure.add"), Some(false), true),
            (3, s("model.summary"), Some(true), true),
        ]
    );
}

#[test]
fn a_panicking_observer_does_not_stop_the_server() {
    let mut server = Server::new().with_observer(|_| panic!("observer bug"));
    let reply = server
        .handle_line(&msg(1, "tools/call", call("new_model", json!({"name": "M"}))))
        .unwrap();
    let reply: Value = serde_json::from_str(&reply).unwrap();
    assert_eq!(reply["result"]["isError"], false);
    assert_eq!(server.session().revision, 1);
}

#[test]
fn serve_with_observer_notifies_per_call() {
    let seen: Arc<Mutex<Vec<u64>>> = Arc::default();
    let log = Arc::clone(&seen);
    let input = [
        msg(1, "initialize", json!({})),
        msg(2, "tools/call", call("new_model", json!({"name": "M"}))),
        msg(3, "tools/call", call("add_table", json!({"name": "T"}))),
    ]
    .join("\n");
    let mut out = Vec::new();
    serve_with_observer(input.as_bytes(), &mut out, move |s| {
        log.lock().unwrap().push(s.revision);
    })
    .unwrap();
    assert_eq!(*seen.lock().unwrap(), vec![1, 2]);
    assert_eq!(String::from_utf8(out).unwrap().lines().count(), 3);
}
