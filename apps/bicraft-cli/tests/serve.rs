//! End-to-end test of `bicraft serve`: the MCP Streamable HTTP endpoint over a workspace root,
//! authentication (API tokens), non-loopback refusal, and `serve token create|hash`.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{Value, json};

#[path = "../../../testsupport/tempdir.rs"]
mod tempdir;
use tempdir::TempDir;

/// Variables `serve` reads: removed so the developer's environment cannot change the tests.
const SERVE_VARS: &[&str] = &[
    "BICRAFT_SERVER_NETWORK",
    "BICRAFT_AUTH_CONFIG",
    "BICRAFT_PUBLIC_URL",
    "BICRAFT_ALLOWED_HOSTS",
    "BICRAFT_OIDC_ISSUERS",
    "BICRAFT_OIDC_AUDIENCES",
    "BICRAFT_OIDC_SCOPES",
    "BICRAFT_API_TOKENS_FILE",
    "BICRAFT_TRUSTED_PROXIES",
    "BICRAFT_AUTH_GRANTS",
];

fn bicraft() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_bicraft"));
    for v in SERVE_VARS {
        c.env_remove(v);
    }
    c
}

/// Kills the child when the test ends, even on failure.
struct Kill(Child);

impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Starts `bicraft serve ARGS`; returns the child, the address it printed and its startup line.
fn spawn_serve(args: &[&str], root: &std::path::Path) -> (Kill, String, String) {
    spawn_serve_env(args, root, &[])
}

/// `spawn_serve` with extra environment variables. A thread reads standard error until the child
/// exits, so the server never writes to a pipe the test closed.
fn spawn_serve_env(args: &[&str], root: &std::path::Path, env: &[(&str, &str)]) -> (Kill, String, String) {
    let mut command = bicraft();
    for (k, v) in env {
        command.env(k, v);
    }
    let mut child = command
        .arg("serve")
        .args(args)
        .arg("--root")
        .arg(root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stderr = child.stderr.take().unwrap();
    let child = Kill(child);
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            let Ok(line) = line else { break };
            eprintln!("[bicraft] {line}");
            if let Some(i) = line.find("http://") {
                let rest = &line[i + "http://".len()..];
                let _ = tx.send((
                    rest.split('/').next().unwrap_or_default().to_string(),
                    line.clone(),
                ));
            }
        }
    });
    let (addr, line) = rx
        .recv_timeout(Duration::from_secs(30))
        .expect("the endpoint is printed on stderr");
    (child, addr, line)
}

/// `server/discover` on workspace ws1 with extra headers; the raw response.
fn discover(addr: &str, extra: &str) -> String {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "server/discover", "params": {"_meta": {
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {}}}})
    .to_string();
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    write!(
        s,
        "POST /w/ws1/mcp HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\n\
         Accept: application/json, text/event-stream\r\nMCP-Protocol-Version: 2026-07-28\r\n\
         Mcp-Method: server/discover\r\n{extra}Content-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut raw = String::new();
    let _ = s.read_to_string(&mut raw);
    raw
}

#[test]
fn serve_refuses_non_loopback_addresses_and_bad_network_settings() {
    let dir = TempDir::new("cli-serve-refuse");
    let out = bicraft()
        .args(["serve", "--listen", "0.0.0.0:0", "--root"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("loopback") && err.contains("authentication"),
        "{err}"
    );
    let out = bicraft()
        .args(["serve", "--listen", "127.0.0.1:0", "--root"])
        .arg(dir.path())
        .env("BICRAFT_SERVER_NETWORK", "maybe")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("BICRAFT_SERVER_NETWORK"));
}

#[test]
fn serve_answers_mcp_over_http() {
    let dir = TempDir::new("cli-serve");
    std::fs::create_dir_all(dir.path().join("ws1")).unwrap();
    let (child, addr, line) = spawn_serve(&["--listen", "127.0.0.1:0"], dir.path());
    assert!(line.contains("no authentication"), "{line}");
    let raw = discover(&addr, "");
    assert!(raw.starts_with("HTTP/1.1 200"), "{raw}");
    let json: Value = serde_json::from_str(raw.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(json["result"]["supportedVersions"], json!(["2026-07-28"]));
    drop(child);
}

/// `serve token create`: the token (stdout, once) and its entry.
fn create_token(name: &str) -> (String, Value) {
    let out = bicraft()
        .args(["serve", "token", "create", "--name", name, "--role", "agent"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    let token = stdout
        .lines()
        .find_map(|l| l.strip_prefix("token: "))
        .unwrap()
        .to_string();
    let entry: Value =
        serde_json::from_str(stdout.lines().find_map(|l| l.strip_prefix("entry: ")).unwrap()).unwrap();
    (token, entry)
}

#[test]
fn token_helpers() {
    let (token, entry) = create_token("ci");
    assert!(token.starts_with("bct_"), "{token}");
    assert_eq!(entry["name"], "ci");
    assert_eq!(entry["roles"], json!(["agent"]));
    assert_eq!(entry["sha256"].as_str().unwrap().len(), 64);
    // `hash` gives the same entry for the same token, and never prints the token
    let mut child = bicraft()
        .args(["serve", "token", "hash", "--name", "ci", "--role", "agent"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(format!("{token}\n").as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(!stdout.contains(&token));
    let again: Value =
        serde_json::from_str(stdout.lines().find_map(|l| l.strip_prefix("entry: ")).unwrap()).unwrap();
    assert_eq!(again, entry);
    // short tokens are refused
    let mut child = bicraft()
        .args(["serve", "token", "hash", "--name", "x"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"password\n").unwrap();
    assert_eq!(child.wait_with_output().unwrap().status.code(), Some(2));
    let out = bicraft().args(["serve", "token", "create"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn serve_with_api_tokens() {
    let dir = TempDir::new("cli-serve-auth");
    let root = dir.path().join("root");
    std::fs::create_dir_all(root.join("ws1")).unwrap();
    let (token, entry) = create_token("ci");
    let tokens_file = dir.path().join("tokens.json");
    std::fs::write(&tokens_file, json!({"tokens": [entry]}).to_string()).unwrap();
    let tf = tokens_file.to_str().unwrap();
    // authentication without a grant is a configuration error
    let out = bicraft()
        .args(["serve", "--api-tokens", tf, "--root"])
        .arg(&root)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("grant"));
    // a non-loopback address with authentication still needs the public host names
    let out = bicraft()
        .args([
            "serve",
            "--listen",
            "0.0.0.0:0",
            "--api-tokens",
            tf,
            "--grant",
            "token:ci=*",
            "--root",
        ])
        .arg(&root)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--public-url"));
    // the variables work like the flags
    let (child, addr, line) = spawn_serve_env(
        &["--listen", "127.0.0.1:0"],
        &root,
        &[
            ("BICRAFT_API_TOKENS_FILE", tf),
            ("BICRAFT_AUTH_GRANTS", "token:ci=ws1"),
        ],
    );
    assert!(
        line.contains("authentication: 1 API token(s); 1 ACL grant(s)"),
        "{line}"
    );
    assert!(!line.contains(&token));
    assert!(discover(&addr, "").starts_with("HTTP/1.1 401"));
    let ok = discover(&addr, &format!("Authorization: Bearer {token}\r\n"));
    assert!(ok.starts_with("HTTP/1.1 200"), "{ok}");
    drop(child);
    // on Linux, a non-loopback address with authentication and a host name starts
    if cfg!(unix) {
        let (child, _addr, line) = spawn_serve(
            &[
                "--listen",
                "0.0.0.0:0",
                "--api-tokens",
                tf,
                "--grant",
                "token:ci=*",
                "--allow-host",
                "bi.example.com",
            ],
            &root,
        );
        assert!(line.contains("authentication"), "{line}");
        drop(child);
    }
}

/// A raw GET; the whole response.
fn get_raw(addr: &str, path: &str) -> String {
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    write!(s, "GET {path} HTTP/1.1\r\nHost: {addr}\r\n\r\n").unwrap();
    let mut raw = String::new();
    let _ = s.read_to_string(&mut raw);
    raw
}

#[test]
fn serve_answers_probes_and_files() {
    let dir = TempDir::new("cli-serve-ops");
    std::fs::create_dir_all(dir.path().join("ws1")).unwrap();
    std::fs::write(dir.path().join("ws1").join("a.txt"), "hello").unwrap();
    let (child, addr, _line) = spawn_serve(&["--listen", "127.0.0.1:0", "--shutdown-grace", "5"], dir.path());
    assert!(get_raw(&addr, "/healthz").starts_with("HTTP/1.1 200"));
    assert!(get_raw(&addr, "/readyz").ends_with("ok\n"));
    let r = get_raw(&addr, "/w/ws1/files/a.txt");
    assert!(r.starts_with("HTTP/1.1 200") && r.ends_with("hello"), "{r}");
    assert!(get_raw(&addr, "/w/ws1/files/").contains("\"a.txt\""));
    drop(child);
}

/// SIGTERM drains and exits 0 when nothing was lost (Unix: the signal is sent with `kill`).
#[cfg(unix)]
#[test]
fn serve_drains_on_sigterm() {
    let dir = TempDir::new("cli-serve-sigterm");
    std::fs::create_dir_all(dir.path().join("ws1")).unwrap();
    let (mut child, addr, _line) =
        spawn_serve(&["--listen", "127.0.0.1:0", "--shutdown-grace", "5"], dir.path());
    assert!(get_raw(&addr, "/healthz").starts_with("HTTP/1.1 200"));
    let pid = child.0.id().to_string();
    let killed = Command::new("kill").args(["-TERM", &pid]).status().unwrap();
    assert!(killed.success());
    let start = std::time::Instant::now();
    let status = loop {
        if let Some(s) = child.0.try_wait().unwrap() {
            break s;
        }
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "serve did not exit after SIGTERM"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(status.code(), Some(0), "{status:?}");
}

/// `--drain-on-stdin-close`: closing standard input drains like SIGTERM (also on Windows, which
/// has no SIGTERM) and exits 0 when nothing was lost.
#[test]
fn serve_drains_when_stdin_closes() {
    let dir = TempDir::new("cli-serve-stdin");
    std::fs::create_dir_all(dir.path().join("ws1")).unwrap();
    let mut child = bicraft()
        .args([
            "serve",
            "--listen",
            "127.0.0.1:0",
            "--shutdown-grace",
            "5",
            "--drain-on-stdin-close",
            "--root",
        ])
        .arg(dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdin = child.stdin.take().unwrap();
    let mut lines = BufReader::new(child.stderr.take().unwrap()).lines();
    let mut child = Kill(child);
    let line = lines
        .by_ref()
        .find_map(|l| l.ok().filter(|l| l.contains("http://")))
        .expect("startup line");
    let i = line.find("http://").unwrap() + "http://".len();
    let addr = line[i..].split('/').next().unwrap().to_string();
    let notice = lines
        .by_ref()
        .find_map(|l| l.ok().filter(|l| l.contains("probes")))
        .expect("second startup line");
    assert!(notice.contains("end of standard input"), "{notice}");
    assert!(get_raw(&addr, "/healthz").starts_with("HTTP/1.1 200"));
    drop(stdin);
    let rest: Vec<String> = lines.map_while(Result::ok).collect();
    let start = std::time::Instant::now();
    let status = loop {
        if let Some(s) = child.0.try_wait().unwrap() {
            break s;
        }
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "serve did not exit after stdin closed"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(status.code(), Some(0), "{status:?} {rest:?}");
    assert!(rest.iter().any(|l| l.contains("exit 0")), "{rest:?}");
}

/// Regression: a server whose standard error was closed (a supervisor or a test that stopped
/// reading) kept serving only until its next event line, where `eprintln!` panicked (exit 101).
/// The drain's lines are written after the pipe is closed, so the old code always failed here.
#[test]
fn serve_survives_a_closed_stderr() {
    let dir = TempDir::new("cli-serve-stderr");
    std::fs::create_dir_all(dir.path().join("ws1")).unwrap();
    let mut child = bicraft()
        .args([
            "serve",
            "--listen",
            "127.0.0.1:0",
            "--shutdown-grace",
            "5",
            "--drain-on-stdin-close",
            "--root",
        ])
        .arg(dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdin = child.stdin.take().unwrap();
    let mut lines = BufReader::new(child.stderr.take().unwrap()).lines();
    let mut child = Kill(child);
    let line = lines
        .by_ref()
        .find_map(|l| l.ok().filter(|l| l.contains("http://")))
        .expect("startup line");
    let i = line.find("http://").unwrap() + "http://".len();
    let addr = line[i..].split('/').next().unwrap().to_string();
    drop(lines);
    assert!(get_raw(&addr, "/healthz").starts_with("HTTP/1.1 200"));
    assert!(discover(&addr, "").starts_with("HTTP/1.1 200"));
    drop(stdin);
    let start = std::time::Instant::now();
    let status = loop {
        if let Some(s) = child.0.try_wait().unwrap() {
            break s;
        }
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "serve did not exit after stdin closed"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(status.code(), Some(0), "{status:?}");
}
