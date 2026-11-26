//! `bicraft setup doctor`: start `<command> mcp`, run the MCP handshake, list the tools.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const REPLY_TIMEOUT: Duration = Duration::from_secs(15);
const EXIT_TIMEOUT: Duration = Duration::from_secs(5);
const PROTOCOL: &str = "2025-06-18";

/// The outcome of the handshake, as JSON:
/// `{ok, server_name, server_version, protocol_version, tools, error}`.
pub fn handshake(command: &Path) -> Value {
    let mut child = match Command::new(command)
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return failed(format!("cannot start `{} mcp`: {e}", command.display())),
    };
    let result = talk(&mut child);
    stop(&mut child);
    match result {
        Ok(v) => v,
        Err(e) => failed(e),
    }
}

fn failed(error: String) -> Value {
    json!({"ok": false, "server_name": null, "server_version": null, "protocol_version": null,
           "tools": null, "error": error})
}

fn talk(child: &mut Child) -> Result<Value, String> {
    let mut stdin = child.stdin.take().ok_or("no stdin pipe")?;
    let stdout = child.stdout.take().ok_or("no stdout pipe")?;
    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if tx.send(line).is_err() {
                        break;
                    }
                }
            }
        }
    });
    let mut send = |v: Value| -> Result<(), String> {
        writeln!(stdin, "{v}")
            .and_then(|()| stdin.flush())
            .map_err(|e| format!("cannot write to the server: {e}"))
    };
    let recv = |id: i64| -> Result<Value, String> {
        let deadline = Instant::now() + REPLY_TIMEOUT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let line = rx.recv_timeout(left).map_err(|e| match e {
                mpsc::RecvTimeoutError::Timeout => format!("no reply within {}s", REPLY_TIMEOUT.as_secs()),
                mpsc::RecvTimeoutError::Disconnected => "the server exited without replying".to_string(),
            })?;
            let v: Value = serde_json::from_str(line.trim())
                .map_err(|e| format!("the server wrote something that is not JSON-RPC: {e}"))?;
            if v.get("id").and_then(Value::as_i64) == Some(id) {
                if let Some(err) = v.get("error") {
                    return Err(format!("the server answered with an error: {err}"));
                }
                return v
                    .get("result")
                    .cloned()
                    .ok_or_else(|| "reply without a result".to_string());
            }
        }
    };

    send(
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": PROTOCOL, "capabilities": {},
        "clientInfo": {"name": "bicraft-setup-doctor", "version": env!("CARGO_PKG_VERSION")}}}),
    )?;
    let init = recv(1)?;
    let info = init.get("serverInfo");
    let name = info.and_then(|i| i.get("name")).cloned().unwrap_or(Value::Null);
    let version = info
        .and_then(|i| i.get("version"))
        .cloned()
        .unwrap_or(Value::Null);
    let protocol = init.get("protocolVersion").cloned().unwrap_or(Value::Null);
    if name.is_null() {
        return Err("the initialize reply has no serverInfo.name".into());
    }
    send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))?;
    send(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}))?;
    let tools = recv(2)?
        .get("tools")
        .and_then(Value::as_array)
        .map(Vec::len)
        .ok_or("the tools/list reply has no tools")?;
    Ok(json!({"ok": true, "server_name": name, "server_version": version,
              "protocol_version": protocol, "tools": tools, "error": null}))
}

/// Closes stdin (the server then exits), killing the process if it does not.
fn stop(child: &mut Child) {
    drop(child.stdin.take());
    let deadline = Instant::now() + EXIT_TIMEOUT;
    while Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => break,
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}
