//! End-to-end tests of the live viewer commands: `bicraft-cli view PATH` and `bicraft-cli mcp --view`.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

#[path = "../../../testsupport/tempdir.rs"]
mod tempdir;
use tempdir::TempDir;

fn bicraft() -> Command {
    Command::new(env!("CARGO_BIN_EXE_bicraft-cli"))
}

fn temp_dir(name: &str) -> TempDir {
    TempDir::new(&format!("cli-view-{name}"))
}

/// Kills the child when the test ends, even on failure.
struct Kill(Child);

impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Reads the child's stderr on a thread and returns the viewer's `host:port` once announced.
fn viewer_addr(child: &mut Child) -> String {
    let stderr = child.stderr.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            let Ok(line) = line else { break };
            eprintln!("[bicraft] {line}");
            if let Some(i) = line.find("http://") {
                let url = line[i + "http://".len()..].trim();
                let _ = tx.send(url.trim_end_matches('/').to_string());
            }
        }
    });
    rx.recv_timeout(Duration::from_secs(30))
        .expect("the viewer URL is printed on stderr")
}

/// `GET path` with a minimal HTTP/1.1 client; returns the body.
fn get(addr: &str, path: &str) -> String {
    let mut stream = TcpStream::connect(addr).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut raw = Vec::new();
    let _ = stream.read_to_end(&mut raw);
    let text = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").expect("an HTTP response");
    assert!(
        head.starts_with("HTTP/1.1 200") || head.starts_with("HTTP/1.0 200"),
        "{head}"
    );
    if head.to_ascii_lowercase().contains("transfer-encoding: chunked") {
        dechunk(body)
    } else {
        body.to_string()
    }
}

fn dechunk(mut body: &str) -> String {
    let mut out = String::new();
    while let Some((size, rest)) = body.split_once("\r\n") {
        let n = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
        if n == 0 {
            break;
        }
        out.push_str(&rest[..n]);
        body = rest[n..].trim_start_matches("\r\n");
    }
    out
}

fn state(addr: &str) -> Value {
    serde_json::from_str(&get(addr, "/api/state")).expect("/api/state is JSON")
}

/// Polls `/api/state` until `pred` holds (or panics after `secs`).
fn wait_for(addr: &str, secs: u64, pred: impl Fn(&Value) -> bool) -> Value {
    let start = Instant::now();
    loop {
        let s = state(addr);
        if pred(&s) {
            return s;
        }
        assert!(
            start.elapsed() < Duration::from_secs(secs),
            "timed out; last state: {s}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn measures(s: &Value) -> Vec<String> {
    s["model"]["tables"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|t| t["measures"].as_array().cloned().unwrap_or_default())
        .filter_map(|m| m["name"].as_str().map(str::to_string))
        .collect()
}

#[test]
fn view_rereads_a_project_when_its_files_change() {
    let dir = temp_dir("watch");
    let out = bicraft()
        .args([
            "run",
            "--cmd",
            r#"model.new={"name":"Shop"}"#,
            "--cmd",
            r#"table.add={"name":"Sales"}"#,
            "--cmd",
            r#"column.add={"table":"Sales","name":"Amount","data_type":"double"}"#,
            "--cmd",
        ])
        .arg(format!(
            r#"project.new={{"dir":{},"name":"Shop"}}"#,
            json!(dir.to_string_lossy())
        ))
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let pbip = dir.join("Shop.pbip");

    let mut child = Kill(
        bicraft()
            .arg("view")
            .arg(&pbip)
            .args(["--port", "0"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let addr = viewer_addr(&mut child.0);
    let s = wait_for(&addr, 10, |s| s["version"] == 1);
    assert_eq!(s["revision"], 0);
    assert_eq!(s["source"]["kind"], "project");
    // the "Open in Power BI Desktop" button opens the project's .pbip file
    assert!(
        s["source"]["desktop"]["path"]
            .as_str()
            .unwrap()
            .ends_with("Shop.pbip"),
        "{}",
        s["source"]
    );
    assert_eq!(s["source"]["desktop"]["unsaved"], false);
    open_button_is_guarded(&addr, true);
    assert_eq!(s["model"]["name"], "Shop");
    assert!(measures(&s).is_empty());

    // Another process edits the project on disk.
    let out = bicraft()
        .arg("run")
        .arg("--open")
        .arg(&pbip)
        .args([
            "--cmd",
            r#"measure.add={"table":"Sales","name":"Total","expression":"SUM(Sales[Amount])"}"#,
            "--save",
        ])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));

    let s = wait_for(&addr, 10, |s| s["revision"].as_u64().unwrap_or(0) > 0);
    assert_eq!(measures(&s), ["Total"]);
    assert_eq!(s["last_command"]["id"], "file.open");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mcp_view_publishes_every_tool_call() {
    let mut child = Kill(
        bicraft()
            .args(["mcp", "--view", "--view-port", "0"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let addr = viewer_addr(&mut child.0);
    let initial = wait_for(&addr, 10, |s| s["version"] == 1);
    assert_eq!(initial["revision"], 0);
    assert_eq!(initial["model"], Value::Null);

    let mut stdin = child.0.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.0.stdout.take().unwrap());
    let mut id = 0;
    let mut request = |method: &str, params: Value| -> Value {
        id += 1;
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(stdin, "{msg}").unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        let reply: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(reply["id"], id);
        reply
    };
    request(
        "initialize",
        json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}),
    );
    let tool = |name: &str, args: Value| json!({"name": name, "arguments": args});
    request("tools/call", tool("new_model", json!({"name": "Shop"})));
    request("tools/call", tool("add_table", json!({"name": "Sales"})));
    let r = request(
        "tools/call",
        tool(
            "add_measure",
            json!({"table": "Sales", "name": "Total", "expression": "1"}),
        ),
    );
    assert_eq!(r["result"]["isError"], false, "{r}");

    let s = wait_for(&addr, 10, |s| s["revision"] == 3);
    assert_eq!(measures(&s), ["Total"]);
    assert_eq!(s["last_command"]["id"], "measure.add");
    assert_eq!(s["dirty"]["model"], true);

    // A failed call shows up as the last command.
    let r = request(
        "tools/call",
        tool(
            "add_measure",
            json!({"table": "Nope", "name": "X", "expression": "1"}),
        ),
    );
    assert_eq!(r["result"]["isError"], true);
    let s = wait_for(&addr, 10, |s| s["last_command"]["ok"] == false);
    assert_eq!(s["revision"], 3);
}

/// `POST /api/open-in-desktop` status, with an optional token. Tests never send the real token,
/// so nothing is ever launched.
fn post_open(addr: &str, token: Option<&str>) -> u16 {
    let mut stream = TcpStream::connect(addr).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let token = token
        .map(|t| format!("X-Bicraft-Token: {t}\r\n"))
        .unwrap_or_default();
    write!(
        stream,
        "POST /api/open-in-desktop HTTP/1.1\r\nHost: {addr}\r\n{token}Content-Length: 0\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut raw = Vec::new();
    let _ = stream.read_to_end(&mut raw);
    let text = String::from_utf8_lossy(&raw).into_owned();
    text.split(' ').nth(1).and_then(|s| s.parse().ok()).unwrap_or(0)
}

/// The page carries a token only when the button is on, and the route refuses requests without it.
fn open_button_is_guarded(addr: &str, enabled: bool) {
    let page = get(addr, "/");
    let empty = page.contains(r#"<meta name="bicraft-open-token" content="">"#);
    assert_eq!(empty, !enabled);
    if enabled {
        assert_eq!(post_open(addr, None), 403);
        assert_eq!(post_open(addr, Some("0123456789abcdef0123456789abcdef")), 403);
    } else {
        assert_eq!(post_open(addr, Some("0123456789abcdef0123456789abcdef")), 404);
    }
}

#[test]
fn no_open_button_flag_removes_the_button() {
    let mut child = Kill(
        bicraft()
            .args(["mcp", "--view-port", "0", "--no-open-button"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let addr = viewer_addr(&mut child.0);
    let s = wait_for(&addr, 10, |s| s["version"] == 1);
    // a new session: nothing saved to open
    assert_eq!(s["source"]["desktop"]["path"], Value::Null);
    open_button_is_guarded(&addr, false);
}
