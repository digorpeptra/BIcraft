//! Document views of `bicraft-cli serve` over real sockets (docs/mcp.md, "Document views";
//! docs/webui.md, "Document tabs"): each browser tab's private session on its own workspace
//! file, edited side by side; the per-file save check; the caps per user and per workspace, the
//! memory check, idle expiry, closing; readers' read-only views; the workspace's own session
//! unchanged for existing MCP clients.

#[path = "../../../testsupport/tempdir.rs"]
mod tempdir;

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::Duration;

use bicraft_auth::tokens::{self, entry_json};
use bicraft_auth::{Access, AuthConfig, Authenticator, Grant};
use bicraft_mcp::http::HTTP_PROTOCOL_VERSION;
use bicraft_mcp::{HttpConfig, HttpServer};
use serde_json::{Value, json};
use tempdir::TempDir;

const MODEL: &str = include_str!("../../engine/tests/table_functions/model.bim");

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

/// A request with extra header lines (each ending in `\r\n`).
fn request(addr: SocketAddr, method: &str, path: &str, headers: &str, body: &[u8]) -> Response {
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n{headers}Content-Length: {}\r\n\r\n",
        body.len()
    );
    let mut bytes = req.into_bytes();
    bytes.extend_from_slice(body);
    send(addr, &bytes)
}

fn bearer(token: &str) -> String {
    format!("Authorization: Bearer {token}\r\n")
}

/// A document view: its id and token.
#[derive(Clone)]
struct View {
    vid: String,
    token: String,
}

/// `POST /w/a/view/api/session {"file": file}`.
fn open_view(addr: SocketAddr, auth: &str, file: &str) -> Response {
    let body = json!({"file": file}).to_string();
    let headers = format!("{auth}Content-Type: application/json\r\n");
    request(addr, "POST", "/w/a/view/api/session", &headers, body.as_bytes())
}

fn view(addr: SocketAddr, auth: &str, file: &str) -> View {
    let r = open_view(addr, auth, file);
    assert_eq!(r.status, 200, "{}", r.text());
    let j = r.json();
    assert_eq!(j["file"], file);
    View {
        vid: j["view"].as_str().unwrap().to_string(),
        token: j["token"].as_str().unwrap().to_string(),
    }
}

/// A tools/call on `path` (`/w/a/mcp` or a view's endpoint), with extra header lines.
fn call_at(addr: SocketAddr, path: &str, extra: &str, tool: &str, args: Value) -> Response {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {
        "name": tool, "arguments": args, "_meta": {
            "io.modelcontextprotocol/protocolVersion": HTTP_PROTOCOL_VERSION,
            "io.modelcontextprotocol/clientCapabilities": {}}}})
    .to_string();
    let headers = format!(
        "Content-Type: application/json\r\nAccept: application/json\r\n\
         MCP-Protocol-Version: {HTTP_PROTOCOL_VERSION}\r\nMcp-Method: tools/call\r\nMcp-Name: {tool}\r\n{extra}"
    );
    request(addr, "POST", path, &headers, body.as_bytes())
}

/// `run_command` on view `v` (its endpoint and token): the tool result.
fn on_view(addr: SocketAddr, auth: &str, v: &View, command: &str, params: Value) -> Value {
    let r = call_at(
        addr,
        &format!("/w/a/mcp/view/{}", v.vid),
        &format!("{auth}X-Bicraft-Token: {}\r\n", v.token),
        "run_command",
        json!({"command": command, "params": params}),
    );
    assert_eq!(r.status, 200, "{}", r.text());
    r.json()["result"].clone()
}

/// `run_command` on the workspace's own session (an existing MCP client).
fn on_workspace(addr: SocketAddr, auth: &str, command: &str, params: Value) -> Value {
    let r = call_at(
        addr,
        "/w/a/mcp",
        auth,
        "run_command",
        json!({"command": command, "params": params}),
    );
    assert_eq!(r.status, 200, "{}", r.text());
    r.json()["result"].clone()
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

fn workspace(name: &str) -> TempDir {
    let dir = TempDir::new(name);
    let a = dir.path().join("a");
    std::fs::create_dir_all(&a).unwrap();
    for f in ["one.bim", "two.bim"] {
        std::fs::write(a.join(f), MODEL).unwrap();
    }
    dir
}

fn on_disk(dir: &TempDir, file: &str) -> String {
    std::fs::read_to_string(dir.path().join("a").join(file)).unwrap()
}

fn start(dir: &TempDir, config: HttpConfig) -> HttpServer {
    HttpServer::start("127.0.0.1:0".parse().unwrap(), dir.path(), config).unwrap()
}

struct Tokens {
    ed: String,
    ed2: String,
    rd: String,
}

fn authed(config: HttpConfig) -> (HttpConfig, Tokens) {
    let t = Tokens {
        ed: tokens::generate().unwrap(),
        ed2: tokens::generate().unwrap(),
        rd: tokens::generate().unwrap(),
    };
    let mut c = AuthConfig::default();
    let entries: Vec<Value> = [("ed", &t.ed), ("ed2", &t.ed2), ("rd", &t.rd)]
        .iter()
        .map(|(name, token)| entry_json(name, &tokens::hash_token(token).unwrap(), &[], &[]))
        .collect();
    c.add_tokens(tokens::parse(&Value::Array(entries)).unwrap())
        .unwrap();
    for (g, access) in [
        ("token:ed=a", Access::Editor),
        ("token:ed2=a", Access::Editor),
        ("token:rd=a", Access::Reader),
    ] {
        c.acl.grants.push(Grant::parse_text_as(g, access).unwrap());
    }
    (
        HttpConfig {
            auth: Some(Arc::new(Authenticator::new(c).unwrap())),
            ..config
        },
        t,
    )
}

/// Two views on two files, edited at the same time from two threads: each keeps its own edits,
/// its own snapshot and revision, saves its own file; the workspace's session (an agent's) sees
/// none of it.
#[test]
fn two_views_edit_two_files_side_by_side() {
    let dir = workspace("docviews-two");
    let server = start(&dir, HttpConfig::default());
    let a = server.addr();
    // an existing client works on the workspace's own session, as before
    ok(on_workspace(a, "", "file.open", json!({"path": "one.bim"})));
    let v1 = view(a, "", "one.bim");
    let v2 = view(a, "", "two.bim");
    assert_ne!(v1.vid, v2.vid);
    let handles: Vec<_> = [("A", v1.clone()), ("B", v2.clone())]
        .into_iter()
        .map(|(tag, v)| {
            std::thread::spawn(move || {
                for i in 0..6 {
                    add_measure(a, "", &v, &format!("{tag}{i}"));
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    // each view shows its own file with its own edits
    let s1 = state(a, "", &v1);
    let s2 = state(a, "", &v2);
    assert_eq!((s1.status, s2.status), (200, 200));
    let (j1, j2) = (s1.json(), s2.json());
    assert_eq!(j1["source"]["path"], "one.bim");
    assert_eq!(j2["source"]["path"], "two.bim");
    assert_eq!(j1["view"]["doc"], true);
    assert_eq!(j1["view"]["session"], "view");
    assert_eq!(j1["view"]["limits"]["views_per_user"], 8);
    assert_eq!(j1["dirty"]["model"], true);
    let (t1, t2) = (s1.text(), s2.text());
    assert!(
        t1.contains("\"A5\"") && !t1.contains("\"B0\""),
        "view 1 shows view 2's edits"
    );
    assert!(
        t2.contains("\"B5\"") && !t2.contains("\"A0\""),
        "view 2 shows view 1's edits"
    );
    // the workspace's own session has one.bim open, untouched by view 1
    let ws = on_workspace(a, "", "session.revision", json!({}));
    assert_eq!(
        ok(ws)["dirty"],
        false,
        "the workspace's session was edited by a view"
    );
    // each save writes its own file
    ok(on_view(a, "", &v1, "file.save", json!({})));
    ok(on_view(a, "", &v2, "file.save", json!({})));
    let (d1, d2) = (on_disk(&dir, "one.bim"), on_disk(&dir, "two.bim"));
    assert!(d1.contains("A5") && !d1.contains("B0"));
    assert!(d2.contains("B5") && !d2.contains("A0"));
    // expected_revision works per view: a revision of the other view never matches
    let rev1 = ok(on_view(a, "", &v1, "session.revision", json!({})))["revision"].clone();
    let r = call_at(
        a,
        &format!("/w/a/mcp/view/{}", v2.vid),
        &format!("X-Bicraft-Token: {}\r\n", v2.token),
        "run_command",
        json!({"command": "measure.add", "params": {"table": "Product", "name": "X", "expression": "1"},
               "expected_revision": rev1}),
    );
    assert_eq!(
        r.json()["result"]["structuredContent"]["code"],
        "revision_conflict"
    );
    // a view's endpoint needs its token, and only document views have one
    let r = call_at(
        a,
        &format!("/w/a/mcp/view/{}", v1.vid),
        "",
        "run_command",
        json!({"command": "model.summary"}),
    );
    assert_eq!(r.status, 403, "{}", r.text());
    let r = call_at(
        a,
        &format!("/w/a/mcp/view/{}", "0".repeat(32)),
        &format!("X-Bicraft-Token: {}\r\n", v1.token),
        "run_command",
        json!({"command": "model.summary"}),
    );
    assert_eq!(r.status, 404, "{}", r.text());
    // closing a view drops it (its token, then nothing)
    let headers = format!("X-Bicraft-Token: {}\r\n", v2.token);
    let r = request(
        a,
        "POST",
        &format!("/w/a/view/api/close?v={}", v2.vid),
        &headers,
        b"",
    );
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(state(a, "", &v2).status, 404);
    assert_eq!(state(a, "", &v1).status, 200);
    // bad bodies
    let r = request(
        a,
        "POST",
        "/w/a/view/api/session",
        "Content-Type: text/plain\r\n",
        b"x",
    );
    assert_eq!(r.status, 415);
    let json_type = "Content-Type: application/json\r\n";
    for bad in [
        &b"[1]"[..],
        br#"{"file": 1}"#,
        br#"{"file": "one.bim", "x": 1}"#,
        b"{",
    ] {
        assert_eq!(
            request(a, "POST", "/w/a/view/api/session", json_type, bad).status,
            400
        );
    }
    // `{}` (or no body): a page's view, as before
    let r = request(a, "POST", "/w/a/view/api/session", json_type, b"{}");
    assert_eq!(r.status, 200, "{}", r.text());
    assert!(r.json().get("file").is_none());
    assert_eq!(open_view(a, "", "../one.bim").status, 400);
    assert_eq!(open_view(a, "", "missing.bim").status, 409);
    let cross = format!("{json_type}Origin: http://evil.example\r\n");
    assert_eq!(
        request(
            a,
            "POST",
            "/w/a/view/api/session",
            &cross,
            br#"{"file": "one.bim"}"#
        )
        .status,
        403
    );
}

/// The per-file save check: a save over a file another session saved since this one read it
/// is refused and writes nothing; reopening (or Save As) resolves it. The workspace's own
/// session is held to the same rule, and an upload over a file a view edits is refused.
#[test]
fn a_save_over_a_newer_file_is_a_conflict() {
    let dir = workspace("docviews-conflict");
    let server = start(&dir, HttpConfig::default());
    let a = server.addr();
    let v1 = view(a, "", "one.bim");
    let v2 = view(a, "", "one.bim");
    ok(on_workspace(a, "", "file.open", json!({"path": "one.bim"})));
    add_measure(a, "", &v1, "First");
    add_measure(a, "", &v2, "Second");
    ok(on_view(a, "", &v1, "file.save", json!({})));
    assert!(on_disk(&dir, "one.bim").contains("First"));
    // view 2 read the file before view 1's save: refused, nothing written
    let r = on_view(a, "", &v2, "file.save", json!({}));
    assert_eq!(r["isError"], true, "{r}");
    let c = &r["structuredContent"];
    assert_eq!(c["code"], "file_conflict", "{r}");
    assert_eq!(c["file"], "one.bim");
    assert_eq!(
        (c["read_revision"].as_u64(), c["file_revision"].as_u64()),
        (Some(0), Some(1))
    );
    let disk = on_disk(&dir, "one.bim");
    assert!(disk.contains("First") && !disk.contains("Second"));
    // in a batch too
    let r = call_at(
        a,
        &format!("/w/a/mcp/view/{}", v2.vid),
        &format!("X-Bicraft-Token: {}\r\n", v2.token),
        "batch",
        json!({"commands": [{"command": "file.save"}]}),
    );
    assert_eq!(r.json()["result"]["structuredContent"]["code"], "file_conflict");
    // the workspace's own session read it before too
    ok(on_workspace(
        a,
        "",
        "measure.add",
        json!({"table": "Product", "name": "Agent", "expression": "1"}),
    ));
    let r = on_workspace(a, "", "file.save", json!({}));
    assert_eq!(r["structuredContent"]["code"], "file_conflict", "{r}");
    // Save As elsewhere works, and so does a save after reopening
    ok(on_view(a, "", &v2, "file.save_as", json!({"path": "copy.bim"})));
    assert!(on_disk(&dir, "copy.bim").contains("Second"));
    ok(on_workspace(a, "", "file.open", json!({"path": "one.bim"})));
    ok(on_workspace(
        a,
        "",
        "measure.add",
        json!({"table": "Product", "name": "Agent", "expression": "1"}),
    ));
    ok(on_workspace(a, "", "file.save", json!({})));
    let disk = on_disk(&dir, "one.bim");
    assert!(disk.contains("First") && disk.contains("Agent"));
    // now view 1 is behind
    add_measure(a, "", &v1, "Third");
    let r = on_view(a, "", &v1, "file.save", json!({}));
    assert_eq!(r["structuredContent"]["code"], "file_conflict", "{r}");
    // an upload over a file a view holds unsaved edits to: refused (409); once it is clean,
    // the view reopens the uploaded file
    let up = |body: &[u8]| request(a, "PUT", "/w/a/files/two.bim", "Overwrite: T\r\n", body);
    let v3 = view(a, "", "two.bim");
    add_measure(a, "", &v3, "Unsaved");
    assert_eq!(up(MODEL.as_bytes()).status, 409);
    ok(on_view(a, "", &v3, "file.open", json!({"path": "two.bim"})));
    let r = up(MODEL.replace("Avg Price", "Mean Price").as_bytes());
    assert_eq!(r.status, 200, "{}", r.text());
    let s = state(a, "", &v3).text();
    assert!(
        s.contains("Mean Price") && !s.contains("Unsaved"),
        "the view did not reopen the upload"
    );
}

/// The caps: per user over the server, per workspace; at a cap, the caller's least recently
/// used clean view makes room, else 429 with the limit in words. And the memory check.
#[test]
fn document_views_are_capped() {
    let (config, t) = authed(HttpConfig {
        max_views_per_user: 2,
        max_views_per_workspace: 3,
        ..HttpConfig::default()
    });
    let dir = workspace("docviews-caps");
    let server = start(&dir, config);
    let a = server.addr();
    let (ed, ed2) = (bearer(&t.ed), bearer(&t.ed2));
    let v1 = view(a, &ed, "one.bim");
    let r = open_view(a, &ed, "two.bim");
    assert_eq!(r.json()["limits"]["views_per_user"], 2);
    assert_eq!(r.json()["access"], "editor");
    let v2 = View {
        vid: r.json()["view"].as_str().unwrap().to_string(),
        token: r.json()["token"].as_str().unwrap().to_string(),
    };
    // a third: the least recently used clean view goes (v1 used before v2)
    std::thread::sleep(Duration::from_millis(5));
    assert_eq!(state(a, &ed, &v2).status, 200);
    let v3 = view(a, &ed, "one.bim");
    assert_eq!(state(a, &ed, &v1).status, 404, "the oldest clean view stays");
    // unsaved edits are never dropped to make room: 429
    add_measure(a, &ed, &v2, "Keep2");
    add_measure(a, &ed, &v3, "Keep3");
    let r = open_view(a, &ed, "two.bim");
    assert_eq!(r.status, 429, "{}", r.text());
    assert!(r.text().contains("at most 2 per user"), "{}", r.text());
    // the workspace's cap counts everyone's: ed2 takes the third place, then its own clean
    // view makes room for its next one; once dirty, the workspace is full
    let w1 = view(a, &ed2, "two.bim");
    let w2 = view(a, &ed2, "one.bim");
    assert_eq!(state(a, &ed2, &w1).status, 404);
    add_measure(a, &ed2, &w2, "Keep");
    let r = open_view(a, &ed2, "two.bim");
    assert_eq!(r.status, 429, "{}", r.text());
    assert!(r.text().contains("on this workspace (at most 3)"), "{}", r.text());
    // another identity's views are not its own: 404
    assert_eq!(state(a, &ed2, &v2).status, 404);
    // closing one frees a place
    let headers = format!("{ed}X-Bicraft-Token: {}\r\n", v3.token);
    assert_eq!(
        request(
            a,
            "POST",
            &format!("/w/a/view/api/close?v={}", v3.vid),
            &headers,
            b""
        )
        .status,
        200
    );
    assert_eq!(open_view(a, &ed2, "two.bim").status, 200);
    // the memory budget: a server above its memory threshold opens no new view (503)
    bicraft_mem::set_limit(Some(1 << 40));
    let dir2 = workspace("docviews-memory");
    let tight = start(
        &dir2,
        HttpConfig {
            ready_memory_percent: Some(0),
            ..HttpConfig::default()
        },
    );
    let r = open_view(tight.addr(), "", "one.bim");
    bicraft_mem::set_limit(None);
    assert_eq!(r.status, 503, "{}", r.text());
    assert!(r.text().contains("memory"), "{}", r.text());
}

/// `/metrics` counts the open document views (editors' and readers', per workspace with the
/// opt-in label) and the recovery journals kept in `--recovery-dir`.
#[test]
fn document_views_and_journals_are_in_the_metrics() {
    let rec = TempDir::new("docviews-metrics-recovery");
    let (config, t) = authed(HttpConfig {
        metrics_workspace_label: true,
        recovery_dir: Some(rec.path().to_path_buf()),
        ..HttpConfig::default()
    });
    let dir = workspace("docviews-metrics");
    let server = start(&dir, config);
    let a = server.addr();
    let series = |name: &str| -> f64 {
        let text = server.metrics_text();
        text.lines()
            .find_map(|l| l.strip_prefix(name)?.strip_prefix(' ')?.parse().ok())
            .unwrap_or_else(|| panic!("{name} missing: {text}"))
    };
    assert_eq!(series("bicraft_document_views_open{mode=\"edit\"}"), 0.0);
    assert_eq!(series("bicraft_recovery_journals"), 0.0);
    let v1 = view(a, &bearer(&t.ed), "one.bim");
    let _v2 = view(a, &bearer(&t.ed2), "two.bim");
    let _r = view(a, &bearer(&t.rd), "one.bim");
    assert_eq!(series("bicraft_document_views_open{mode=\"edit\"}"), 2.0);
    assert_eq!(series("bicraft_document_views_open{mode=\"read\"}"), 1.0);
    assert_eq!(
        series("bicraft_workspace_document_views_open{workspace=\"a\"}"),
        3.0
    );
    assert_eq!(series("bicraft_document_views_max"), 32.0);
    // an edit is journaled in the workspace's subfolder
    add_measure(a, &bearer(&t.ed), &v1, "Journaled");
    let kept = series("bicraft_recovery_journals");
    assert!(kept >= 1.0, "{kept}");
    assert_eq!(
        series("bicraft_workspace_recovery_journals{workspace=\"a\"}"),
        kept
    );
    assert!(series("bicraft_recovery_journal_bytes") > 0.0);
}

/// Idle views expire: a clean one after the idle time, one with unsaved edits after four
/// times as long; a view with an open event stream is in use.
#[test]
fn idle_document_views_expire() {
    let dir = workspace("docviews-idle");
    let server = start(
        &dir,
        HttpConfig {
            view_idle: Duration::from_millis(400),
            ..HttpConfig::default()
        },
    );
    let a = server.addr();
    let clean = view(a, "", "one.bim");
    let dirty = view(a, "", "two.bim");
    add_measure(a, "", &dirty, "Unsaved");
    let streamed = view(a, "", "one.bim");
    // an open event stream on `streamed`
    let mut es = TcpStream::connect(a).unwrap();
    es.write_all(
        format!(
            "GET /w/a/view/api/events?v={} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
            streamed.vid
        )
        .as_bytes(),
    )
    .unwrap();
    es.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut buf = [0u8; 256];
    assert!(es.read(&mut buf).unwrap() > 0);
    std::thread::sleep(Duration::from_millis(900));
    // any viewer request of the workspace purges
    assert_eq!(state(a, "", &clean).status, 404, "the idle clean view stayed");
    assert_eq!(state(a, "", &streamed).status, 200, "a streamed view expired");
    let r = state(a, "", &dirty);
    assert_eq!(r.status, 200, "a view with unsaved edits expired early");
    std::thread::sleep(Duration::from_millis(1800));
    assert_eq!(state(a, "", &clean).status, 404);
    assert_eq!(
        state(a, "", &dirty).status,
        404,
        "the idle dirty view never expired"
    );
    let r = call_at(
        a,
        &format!("/w/a/mcp/view/{}", dirty.vid),
        &format!("X-Bicraft-Token: {}\r\n", dirty.token),
        "run_command",
        json!({"command": "model.summary"}),
    );
    assert_eq!(r.status, 404);
    drop(es);
}

/// A page that goes away releases its tabs' views (the browser app's `pagehide`, docs/webui.md):
/// the release, sent as the browser sends it while the tab's event stream is still open, drops
/// the view at once, long before the idle time, and ends its stream; only the view's own
/// identity, with its token, can release it.
#[test]
fn a_released_view_is_gone_before_the_idle_time() {
    let dir = workspace("docviews-release");
    let (config, t) = authed(HttpConfig {
        view_idle: Duration::from_secs(3600),
        ..HttpConfig::default()
    });
    let server = start(&dir, config);
    let a = server.addr();
    let (ed, ed2) = (bearer(&t.ed), bearer(&t.ed2));
    let tab = view(a, &ed, "one.bim");
    add_measure(a, &ed, &tab, "Unsaved");
    let mut es = TcpStream::connect(a).unwrap();
    es.write_all(
        format!(
            "GET /w/a/view/api/events?v={} HTTP/1.1\r\nHost: 127.0.0.1\r\n{ed}\r\n",
            tab.vid
        )
        .as_bytes(),
    )
    .unwrap();
    es.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    let mut buf = [0u8; 4096];
    assert!(es.read(&mut buf).unwrap() > 0);
    let close = format!("/w/a/view/api/close?v={}", tab.vid);
    let browser = "Origin: http://127.0.0.1\r\nSec-Fetch-Site: same-origin\r\n";
    // another identity, even with the token, and the owner without it, release nothing
    let other = format!("{ed2}{browser}X-Bicraft-Token: {}\r\n", tab.token);
    assert_eq!(request(a, "POST", &close, &other, b"").status, 404);
    let tokenless = format!("{ed}{browser}");
    assert_eq!(request(a, "POST", &close, &tokenless, b"").status, 403);
    assert_eq!(state(a, &ed, &tab).status, 200);
    // the owner's release: gone now, its stream ended
    let own = format!("{ed}{browser}X-Bicraft-Token: {}\r\n", tab.token);
    let r = request(a, "POST", &close, &own, b"");
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(state(a, &ed, &tab).status, 404);
    loop {
        match es.read(&mut buf) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) => panic!("the released view's stream stayed open: {e}"),
        }
    }
}

/// Readers get read-only views: the saved state of their file (never an editor view's unsaved
/// edits), commands that read, and editing commands refused.
#[test]
fn readers_get_read_only_views() {
    let (config, t) = authed(HttpConfig::default());
    let dir = workspace("docviews-readers");
    let server = start(&dir, config);
    let a = server.addr();
    let (ed, rd) = (bearer(&t.ed), bearer(&t.rd));
    let e = view(a, &ed, "one.bim");
    add_measure(a, &ed, &e, "EditorUnsaved");
    let r = open_view(a, &rd, "one.bim");
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.json()["access"], "reader");
    let rv = View {
        vid: r.json()["view"].as_str().unwrap().to_string(),
        token: r.json()["token"].as_str().unwrap().to_string(),
    };
    let s = state(a, &rd, &rv);
    assert_eq!(s.status, 200);
    let j = s.json();
    assert_eq!(j["view"]["role"], "reader");
    assert_eq!(j["view"]["doc"], true);
    assert!(
        !s.text().contains("EditorUnsaved"),
        "a reader sees an editor's unsaved edits"
    );
    // reading works, editing is refused, saving too
    let summary = ok(on_view(a, &rd, &rv, "model.summary", json!({})));
    assert!(summary.to_string().contains("Product"));
    for (command, params) in [
        (
            "measure.add",
            json!({"table": "Product", "name": "R", "expression": "1"}),
        ),
        ("file.save", json!({})),
    ] {
        let r = on_view(a, &rd, &rv, command, params);
        assert_eq!(r["isError"], true, "a reader ran {command}: {r}");
    }
    assert!(!on_disk(&dir, "one.bim").contains("\"R\""));
    // another identity cannot use the reader's view
    assert_eq!(state(a, &ed, &rv).status, 404);
    // the editor's save reaches the reader's view (its saved state)
    ok(on_view(a, &ed, &e, "file.save", json!({})));
    assert!(state(a, &rd, &rv).text().contains("EditorUnsaved"));
}

/// `model.refresh` on an open PBIX writes the new data into the file in place: the per-file
/// check applies as for `file.save` (a refresh over a file another session saved or refreshed
/// since is refused), and a refresh is a new revision of the file for the other sessions.
#[test]
fn a_refresh_over_a_newer_pbix_is_a_conflict() {
    let dir = workspace("docviews-refresh");
    // a PBIX with one import table (an M literal: refreshed without any file)
    let mut s = bicraft_engine::Session::new();
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.import",
            json!({"name": "Product", "m": "let Source = #table(type table [Key = Int64.Type], {{1}, {2}}) in Source"}),
        ),
        (
            "file.save_as",
            json!({"path": dir.path().join("a").join("shop.pbix")}),
        ),
    ] {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    drop(s);
    let server = start(&dir, HttpConfig::default());
    let a = server.addr();
    let v1 = view(a, "", "shop.pbix");
    let v2 = view(a, "", "shop.pbix");
    let out = ok(on_view(a, "", &v1, "model.refresh", json!({})));
    assert_eq!(out["format"], "pbix", "{out}");
    // view 2 read the file before view 1's refresh: refused, nothing written
    let before = std::fs::read(dir.path().join("a").join("shop.pbix")).unwrap();
    let r = on_view(a, "", &v2, "model.refresh", json!({}));
    assert_eq!(r["structuredContent"]["code"], "file_conflict", "{r}");
    assert_eq!(
        std::fs::read(dir.path().join("a").join("shop.pbix")).unwrap(),
        before
    );
    // reopened, it refreshes; then view 1 is behind for a save
    ok(on_view(a, "", &v2, "file.open", json!({"path": "shop.pbix"})));
    ok(on_view(a, "", &v2, "model.refresh", json!({})));
    add_measure(a, "", &v1, "Late");
    let r = on_view(a, "", &v1, "file.save", json!({}));
    assert_eq!(r["structuredContent"]["code"], "file_conflict", "{r}");
}
