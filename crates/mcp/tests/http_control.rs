//! The UI control channel relayed by `bicraft-cli serve --viewer-control` (docs/web-deployment.md,
//! "UI control channel"; docs/webui.md): an editor's agent sends a request with its credential
//! and the control token, the server hands it to that identity's app page over the page's event
//! stream, and the page's reply is the agent's answer. The page is played by raw HTTP, as the
//! browser shell (`apps/bicraft-web`) talks to the server.

#[path = "../../../testsupport/tempdir.rs"]
mod tempdir;

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bicraft_auth::tokens::{self, entry_json};
use bicraft_auth::{Access, AuthConfig, Authenticator, Grant};
use bicraft_mcp::{ControlToken, HttpConfig, HttpServer};
use serde_json::{Value, json};
use tempdir::TempDir;

const MODEL: &str = include_str!("../../engine/tests/table_functions/model.bim");
const CONTROL: &str = "c0ntrol-t0ken-of-thirty-two-chars-at-least";

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
    s.set_read_timeout(Some(Duration::from_secs(70))).unwrap();
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n{headers}Content-Length: {}\r\n\r\n",
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

fn bearer(token: &str) -> String {
    format!("Authorization: Bearer {token}\r\n")
}

struct Tokens {
    ed: String,
    ed2: String,
    rd: String,
}

fn server(name: &str, control: bool) -> (TempDir, HttpServer, Tokens) {
    let dir = TempDir::new(name);
    let a = dir.path().join("a");
    std::fs::create_dir_all(&a).unwrap();
    std::fs::write(a.join("one.bim"), MODEL).unwrap();
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
    let config = HttpConfig {
        auth: Some(Arc::new(Authenticator::new(c).unwrap())),
        viewer_control: control.then(|| ControlToken::new(CONTROL).unwrap()),
        ..HttpConfig::default()
    };
    let server = HttpServer::start("127.0.0.1:0".parse().unwrap(), dir.path(), config).unwrap();
    (dir, server, t)
}

fn meta(html: &str, name: &str) -> String {
    let needle = format!(r#"<meta name="{name}" content=""#);
    let at = html.find(&needle).unwrap_or_else(|| panic!("no {name}")) + needle.len();
    html[at..].split('"').next().unwrap().to_string()
}

/// A page's view: its id and token (the HTML page's slots; the app page has the same).
struct View {
    vid: String,
    token: String,
}

fn page_view(addr: SocketAddr, who: &str) -> View {
    let r = request(addr, "GET", "/w/a/view/", &bearer(who), b"");
    assert_eq!(r.status, 200, "{}", r.text());
    let html = r.text();
    View {
        vid: meta(&html, "bicraft-view-id"),
        token: meta(&html, "bicraft-interact-token"),
    }
}

/// The page's request stream.
struct Stream {
    s: TcpStream,
    seen: Vec<u8>,
}

impl Stream {
    fn open(addr: SocketAddr, who: &str, vid: &str) -> Stream {
        let mut s = TcpStream::connect(addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
        s.write_all(
            format!(
                "GET /w/a/view/api/control/events?v={vid} HTTP/1.1\r\nHost: 127.0.0.1\r\n{}Accept: text/event-stream\r\n\r\n",
                bearer(who)
            )
            .as_bytes(),
        )
        .unwrap();
        let mut p = Stream { s, seen: Vec::new() };
        p.until("retry: 1000");
        p
    }

    fn until(&mut self, needle: &str) -> String {
        let mut buf = [0u8; 65536];
        loop {
            let text = String::from_utf8_lossy(&self.seen).into_owned();
            if let Some(i) = text.find(needle) {
                let end = text[i..].find("\n\n").map_or(text.len(), |e| i + e + 2);
                self.seen.drain(..end);
                return text[i..end].to_string();
            }
            let n = self.s.read(&mut buf).expect("the stream");
            assert!(n > 0, "stream closed before {needle:?}: {text}");
            self.seen.extend_from_slice(&buf[..n]);
        }
    }

    /// The next relayed request: `{seq, request, doc?}`.
    fn next(&mut self) -> Value {
        let ev = self.until("event: control");
        let data = ev.lines().find_map(|l| l.strip_prefix("data: ")).unwrap();
        serde_json::from_str(data).unwrap()
    }
}

/// The agent's request (a thread: it waits for the page's reply).
fn agent(
    addr: SocketAddr,
    who: Option<&str>,
    control: Option<&str>,
    query: &str,
    extra: &str,
    body: Value,
) -> Response {
    let mut h = String::from("Content-Type: application/json\r\n");
    if let Some(w) = who {
        h.push_str(&bearer(w));
    }
    if let Some(c) = control {
        h.push_str(&format!("X-Bicraft-Control: {c}\r\n"));
    }
    h.push_str(extra);
    request(
        addr,
        "POST",
        &format!("/w/a/view/api/control{query}"),
        &h,
        body.to_string().as_bytes(),
    )
}

fn agent_later(addr: SocketAddr, who: &str, query: &str, body: Value) -> std::thread::JoinHandle<Response> {
    let (who, query) = (who.to_string(), query.to_string());
    std::thread::spawn(move || agent(addr, Some(&who), Some(CONTROL), &query, "", body))
}

fn reply(addr: SocketAddr, who: &str, v: &View, extra: &str, body: Value) -> Response {
    let h = format!(
        "{}Content-Type: application/json\r\nX-Bicraft-Token: {}\r\n{extra}",
        bearer(who),
        v.token
    );
    request(
        addr,
        "POST",
        &format!("/w/a/view/api/control/reply?v={}", v.vid),
        &h,
        body.to_string().as_bytes(),
    )
}

#[test]
fn the_relay_is_off_without_the_flag() {
    let (_dir, server, t) = server("control-off", false);
    let a = server.addr();
    let r = agent(
        a,
        Some(&t.ed),
        Some(CONTROL),
        "",
        "",
        json!({"method": "ui.inspect"}),
    );
    assert_eq!(r.status, 404, "{}", r.text());
    assert!(r.text().contains("--viewer-control"));
    let v = page_view(a, &t.ed);
    let r = request(
        a,
        "GET",
        &format!("/w/a/view/api/control/events?v={}", v.vid),
        &bearer(&t.ed),
        b"",
    );
    assert_eq!(r.status, 404);
}

/// An editor's request reaches its own page (by default, by the page's view, by a document view
/// the page opened: that file's tab), never with the control token, and the page's reply is the
/// answer.
#[test]
fn an_editor_drives_its_own_app_page() {
    let (_dir, server, t) = server("control-drive", true);
    let a = server.addr();
    let v = page_view(a, &t.ed);
    let mut page = Stream::open(a, &t.ed, &v.vid);
    // the page's default: the caller's page connected last
    let h = agent_later(
        a,
        &t.ed,
        "",
        json!({"id": 7, "method": "ui.inspect", "token": "agent's", "params": {}}),
    );
    let got = page.next();
    assert_eq!(got["request"]["method"], "ui.inspect");
    assert!(
        got["request"].get("token").is_none(),
        "the page never sees a token"
    );
    assert!(got.get("doc").is_none());
    let seq = got["seq"].as_u64().unwrap();
    // only with the view's token, from this origin
    let r = reply(
        a,
        &t.ed,
        &View {
            vid: v.vid.clone(),
            token: "0".repeat(32),
        },
        "",
        json!({"seq": seq, "reply": {}}),
    );
    assert_eq!(r.status, 403);
    let r = reply(
        a,
        &t.ed,
        &v,
        "Sec-Fetch-Site: cross-site\r\n",
        json!({"seq": seq, "reply": {}}),
    );
    assert_eq!(r.status, 403);
    let r = reply(
        a,
        &t.ed,
        &v,
        "",
        json!({"seq": seq, "reply": {"id": 7, "ok": true, "result": {"view": "report"}}}),
    );
    assert_eq!(r.status, 200, "{}", r.text());
    let answer = h.join().unwrap();
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(
        answer.json(),
        json!({"id": 7, "ok": true, "result": {"view": "report"}})
    );
    // addressed by the page's view
    let h = agent_later(a, &t.ed, &format!("?v={}", v.vid), json!({"method": "methods"}));
    let got = page.next();
    let seq = got["seq"].as_u64().unwrap();
    assert_eq!(
        reply(a, &t.ed, &v, "", json!({"seq": seq, "reply": {"ok": true}})).status,
        200
    );
    assert_eq!(h.join().unwrap().json(), json!({"ok": true}));
    // a document view the page opened (`api/session?v=<page>`): its file's tab, on that page
    let r = request(
        a,
        "POST",
        &format!("/w/a/view/api/session?v={}", v.vid),
        &format!("{}Content-Type: application/json\r\n", bearer(&t.ed)),
        json!({"file": "one.bim"}).to_string().as_bytes(),
    );
    assert_eq!(r.status, 200, "{}", r.text());
    let doc = r.json()["view"].as_str().unwrap().to_string();
    // the agent lists its views to address one
    let r = request(
        a,
        "GET",
        "/w/a/view/api/control",
        &format!(
            "{}X-Bicraft-Control: {CONTROL}
",
            bearer(&t.ed)
        ),
        b"",
    );
    assert_eq!(r.status, 200, "{}", r.text());
    let pages = r.json()["result"]["pages"].clone();
    let mine = pages
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["view"] == v.vid.as_str())
        .unwrap()
        .clone();
    assert_eq!(mine["connected"], true);
    assert_eq!(mine["docs"], json!([{"view": doc, "file": "one.bim"}]));
    let r = request(a, "GET", "/w/a/view/api/control", &bearer(&t.ed), b"");
    assert_eq!(r.status, 401, "the listing needs the control token too");
    let h = agent_later(a, &t.ed, &format!("?v={doc}"), json!({"method": "ui.screenshot"}));
    let got = page.next();
    assert_eq!(got["doc"], "one.bim");
    let seq = got["seq"].as_u64().unwrap();
    // a document view has no stream of its own, and only the page given the request answers it
    let r = request(
        a,
        "GET",
        &format!("/w/a/view/api/control/events?v={doc}"),
        &bearer(&t.ed),
        b"",
    );
    assert_eq!(r.status, 404);
    assert_eq!(
        reply(
            a,
            &t.ed,
            &v,
            "",
            json!({"seq": seq, "reply": {"ok": true, "result": {"png": "iVBORw=="}}})
        )
        .status,
        200
    );
    assert_eq!(h.join().unwrap().json()["result"]["png"], "iVBORw==");
    // a document view no page opened (a native client's): refused
    let r = request(
        a,
        "POST",
        "/w/a/view/api/session",
        &format!("{}Content-Type: application/json\r\n", bearer(&t.ed)),
        json!({"file": "one.bim"}).to_string().as_bytes(),
    );
    let native = r.json()["view"].as_str().unwrap().to_string();
    let r = agent(
        a,
        Some(&t.ed),
        Some(CONTROL),
        &format!("?v={native}"),
        "",
        json!({"method": "methods"}),
    );
    assert_eq!(r.status, 409, "{}", r.text());
}

/// Everything else is refused: without the control token or the credential, a reader, a web
/// page, another identity's view or page, no page connected, a reply from another page.
#[test]
fn the_agent_route_needs_an_editor_and_the_control_token() {
    let (_dir, server, t) = server("control-refused", true);
    let a = server.addr();
    let m = || json!({"method": "methods"});
    let r = agent(a, Some(&t.ed), None, "", "", m());
    assert_eq!(r.status, 401, "{}", r.text());
    let r = agent(
        a,
        Some(&t.ed),
        Some("wrong-token-wrong-token-wrong-token"),
        "",
        "",
        m(),
    );
    assert_eq!(r.status, 401);
    let r = agent(a, None, Some(CONTROL), "", "", m());
    assert_eq!(r.status, 401, "the server's credential first");
    let r = agent(a, Some(&t.rd), Some(CONTROL), "", "", m());
    assert_eq!(r.status, 403, "{}", r.text());
    assert!(r.text().contains("editor"));
    for extra in ["Origin: http://127.0.0.1\r\n", "Sec-Fetch-Site: same-origin\r\n"] {
        let r = agent(a, Some(&t.ed), Some(CONTROL), "", extra, m());
        assert_eq!(r.status, 403, "{extra}");
    }
    // the control token in the bearer slot is not a credential
    let r = agent(a, Some(CONTROL), Some(CONTROL), "", "", m());
    assert_eq!(r.status, 401);
    // no page of the caller connected
    let r = agent(a, Some(&t.ed), Some(CONTROL), "", "", m());
    assert_eq!(r.status, 503, "{}", r.text());
    let v = page_view(a, &t.ed);
    let mut page = Stream::open(a, &t.ed, &v.vid);
    // another editor reaches neither ed's page nor its view
    let r = agent(a, Some(&t.ed2), Some(CONTROL), "", "", m());
    assert_eq!(r.status, 503, "{}", r.text());
    let r = agent(a, Some(&t.ed2), Some(CONTROL), &format!("?v={}", v.vid), "", m());
    assert_eq!(r.status, 404);
    let r = agent(a, Some(&t.ed), Some(CONTROL), "?v=nothex", "", m());
    assert_eq!(r.status, 404);
    // a page's stream is its owner's
    let r = request(
        a,
        "GET",
        &format!("/w/a/view/api/control/events?v={}", v.vid),
        &bearer(&t.ed2),
        b"",
    );
    assert_eq!(r.status, 404);
    // bodies
    let r = agent(a, Some(&t.ed), Some(CONTROL), "", "", json!({"id": 1}));
    assert_eq!(r.status, 400);
    // a reply from another page of the same identity does not answer this page's request
    let other = page_view(a, &t.ed);
    let h = agent_later(a, &t.ed, &format!("?v={}", v.vid), m());
    let seq = page.next()["seq"].as_u64().unwrap();
    let r = reply(a, &t.ed, &other, "", json!({"seq": seq, "reply": {"ok": false}}));
    assert_eq!(r.status, 404, "{}", r.text());
    assert_eq!(
        reply(a, &t.ed, &v, "", json!({"seq": seq, "reply": {"ok": true}})).status,
        200
    );
    assert_eq!(h.join().unwrap().json(), json!({"ok": true}));
    // /metrics counts the agent's requests that reached the relay, and every one that failed
    // (refused, no page, bad body); the last one succeeded
    let text = server.metrics_text();
    let series = |name: &str| -> u64 {
        text.lines()
            .find_map(|l| l.strip_prefix(name)?.strip_prefix(' ')?.parse().ok())
            .unwrap_or_else(|| panic!("{name} missing: {text}"))
    };
    let (requests, errors) = (
        series("bicraft_control_requests_total"),
        series("bicraft_control_request_errors_total"),
    );
    assert!(requests >= 10, "{requests}");
    assert_eq!(requests - errors, 1, "{text}");
}

/// A drain ends the page streams at once (they would otherwise hold it for the grace period).
#[test]
fn a_drain_ends_the_page_streams() {
    let (_dir, server, t) = server("control-drain", true);
    let a = server.addr();
    let v = page_view(a, &t.ed);
    let mut page = Stream::open(a, &t.ed, &v.vid);
    let started = Instant::now();
    let report = server.drain(Duration::from_secs(20));
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    assert!(report.finished_in_grace);
    let mut rest = Vec::new();
    let _ = page.s.read_to_end(&mut rest);
    assert!(started.elapsed() < Duration::from_secs(10), "the stream closed");
}
