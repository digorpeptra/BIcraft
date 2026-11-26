//! The UI control channel's relay (`bicraft view --control`, docs/webui.md "Control channel"):
//! an agent's request reaches a connected app page over its event stream, the page's reply comes
//! back as the answer, and everything else is refused. The page is played here by raw HTTP: the
//! same requests the browser shell (`apps/bicraft-web`) sends.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bicraft_view::{ViewHandle, ViewOptions};
use serde_json::{Value, json};

fn start(control: bool) -> Arc<ViewHandle> {
    let options = ViewOptions {
        opener: None,
        interactions: true,
        control,
    };
    Arc::new(bicraft_view::start_with("127.0.0.1:0".parse().unwrap(), options).unwrap())
}

/// One HTTP exchange: (status, JSON body or null).
fn post(addr: SocketAddr, path: &str, headers: &[(&str, &str)], body: &str) -> (u16, Value) {
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(70))).unwrap();
    let mut req = format!("POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n", addr.port());
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str(&format!("Content-Length: {}\r\n\r\n{body}", body.len()));
    s.write_all(req.as_bytes()).unwrap();
    let mut raw = Vec::new();
    let _ = s.read_to_end(&mut raw);
    let text = String::from_utf8_lossy(&raw).into_owned();
    let status = text.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    let body = text.split_once("\r\n\r\n").map_or("", |(_, b)| b);
    (status, serde_json::from_str(body).unwrap_or(Value::Null))
}

fn agent(addr: SocketAddr, token: &str, body: &Value) -> (u16, Value) {
    let auth = format!("Bearer {token}");
    post(
        addr,
        "/api/control",
        &[("Authorization", &auth), ("Content-Type", "application/json")],
        &body.to_string(),
    )
}

/// The app page's request stream.
struct Page {
    s: TcpStream,
    seen: Vec<u8>,
}

impl Page {
    fn connect(view: &ViewHandle) -> Page {
        let addr = view.addr();
        let mut s = TcpStream::connect(addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
        s.write_all(
            format!(
                "GET /api/control/events HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAccept: text/event-stream\r\n\r\n",
                addr.port()
            )
            .as_bytes(),
        )
        .unwrap();
        let mut p = Page { s, seen: Vec::new() };
        p.until("retry: 1000");
        let deadline = Instant::now() + Duration::from_secs(10);
        while view.control_pages() == 0 {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
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
            assert!(n > 0, "stream closed before {needle:?}");
            self.seen.extend_from_slice(&buf[..n]);
        }
    }

    /// The next relayed request: (seq, request).
    fn next(&mut self) -> (u64, Value) {
        let ev = self.until("event: control");
        let data = ev.lines().find_map(|l| l.strip_prefix("data: ")).unwrap();
        let v: Value = serde_json::from_str(data).unwrap();
        (v["seq"].as_u64().unwrap(), v["request"].clone())
    }
}

fn page_reply(view: &ViewHandle, headers: &[(&str, &str)], body: &Value) -> (u16, Value) {
    let origin = format!("http://127.0.0.1:{}", view.addr().port());
    let token = view.interact_token().unwrap().to_string();
    let mut h: Vec<(&str, &str)> = vec![
        ("Content-Type", "application/json"),
        ("Origin", &origin),
        ("Sec-Fetch-Site", "same-origin"),
        ("X-Bicraft-Token", &token),
    ];
    for (k, v) in headers {
        h.retain(|(n, _)| n != k);
        if !v.is_empty() {
            h.push((k, v));
        }
    }
    post(view.addr(), "/api/control/reply", &h, &body.to_string())
}

#[test]
fn a_request_reaches_the_page_and_its_reply_comes_back() {
    let view = start(true);
    let token = view.control_token().unwrap().to_string();
    assert_eq!(token.len(), 32);
    assert_ne!(
        Some(token.as_str()),
        view.interact_token(),
        "the agent's token is its own"
    );
    let mut page = Page::connect(&view);
    let addr = view.addr();
    let t = token.clone();
    let asker = std::thread::spawn(move || {
        agent(
            addr,
            &t,
            &json!({"id": 4, "method": "ui.inspect", "params": {}, "token": "x"}),
        )
    });
    let (seq, request) = page.next();
    // the page sees the request, never the agent's token
    assert_eq!(request, json!({"id": 4, "method": "ui.inspect", "params": {}}));
    let reply = json!({"id": 4, "ok": true, "result": {"view": "report"}});
    let (status, body) = page_reply(&view, &[], &json!({"seq": seq, "reply": reply}));
    assert_eq!((status, body["ok"].clone()), (200, json!(true)));
    let (status, answer) = asker.join().unwrap();
    assert_eq!(status, 200);
    assert_eq!(answer, reply);
    // a second reply to the same request finds nobody waiting
    let (status, _) = page_reply(&view, &[], &json!({"seq": seq, "reply": reply}));
    assert_eq!(status, 404);
}

#[test]
fn the_agent_needs_its_token_and_cannot_be_a_web_page() {
    let view = start(true);
    let token = view.control_token().unwrap().to_string();
    let addr = view.addr();
    let body = json!({"id": 1, "method": "ui.inspect"}).to_string();
    let json_type = ("Content-Type", "application/json");
    for (headers, want) in [
        (vec![json_type], 401),
        (
            vec![
                json_type,
                ("Authorization", "Bearer 00000000000000000000000000000000"),
            ],
            401,
        ),
        (vec![json_type, ("Authorization", token.as_str())], 401),
    ] {
        assert_eq!(post(addr, "/api/control", &headers, &body).0, want, "{headers:?}");
    }
    let auth = format!("Bearer {token}");
    // the page's own token is not the agent's
    let page_auth = format!("Bearer {}", view.interact_token().unwrap());
    assert_eq!(
        post(
            addr,
            "/api/control",
            &[json_type, ("Authorization", &page_auth)],
            &body
        )
        .0,
        401
    );
    // a web page holding the token is still refused
    let origin = format!("http://127.0.0.1:{}", addr.port());
    for extra in [("Origin", origin.as_str()), ("Sec-Fetch-Site", "same-origin")] {
        let (status, v) = post(
            addr,
            "/api/control",
            &[json_type, ("Authorization", &auth), extra],
            &body,
        );
        assert_eq!(status, 403, "{extra:?}");
        assert!(v["error"].as_str().unwrap().contains("not for web pages"));
    }
    // bodies
    let a = ("Authorization", auth.as_str());
    assert_eq!(post(addr, "/api/control", &[a], &body).0, 415);
    assert_eq!(post(addr, "/api/control", &[a, json_type], "[1]").0, 400);
    assert_eq!(post(addr, "/api/control", &[a, json_type], "{\"id\":1}").0, 400);
    assert_eq!(post(addr, "/api/control", &[a, json_type], "not json").0, 400);
    let huge = format!(
        "{{\"method\":\"m\",\"x\":\"{}\"}}",
        "a".repeat(bicraft_view::control::MAX_REQUEST_BYTES)
    );
    assert_eq!(post(addr, "/api/control", &[a, json_type], &huge).0, 413);
    // no page connected
    let (status, v) = agent(addr, &token, &json!({"id": 2, "method": "ui.inspect"}));
    assert_eq!(status, 503);
    assert_eq!(v["id"], 2);
    assert!(v["error"].as_str().unwrap().contains("no app page"));
    // GET on the agent's route, POST on the stream's
    let mut s = TcpStream::connect(addr).unwrap();
    s.write_all(
        format!(
            "GET /api/control HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n\r\n",
            addr.port()
        )
        .as_bytes(),
    )
    .unwrap();
    let mut raw = String::new();
    let _ = s.read_to_string(&mut raw);
    assert!(raw.starts_with("HTTP/1.1 405"), "{raw}");
    assert_eq!(post(addr, "/api/control/events", &[], "").0, 405);
}

#[test]
fn replies_need_the_page_token_and_this_origin() {
    let view = start(true);
    let ok = json!({"seq": 1, "reply": {"ok": true}});
    for (headers, want) in [
        (vec![("X-Bicraft-Token", "")], 403),
        (vec![("X-Bicraft-Token", "00000000000000000000000000000000")], 403),
        (vec![("Origin", "http://evil.example")], 403),
        (vec![("Sec-Fetch-Site", "cross-site")], 403),
        (vec![("Content-Type", "text/plain")], 415),
        (vec![], 404),
    ] {
        assert_eq!(page_reply(&view, &headers, &ok).0, want, "{headers:?}");
    }
    assert_eq!(page_reply(&view, &[], &json!({"seq": "x"})).0, 400);
    assert_eq!(page_reply(&view, &[], &json!([1])).0, 400);
}

#[test]
fn without_control_the_routes_are_not_there() {
    let view = start(false);
    assert!(view.control_token().is_none());
    let addr = view.addr();
    let (status, v) = post(
        addr,
        "/api/control",
        &[
            ("Authorization", "Bearer x"),
            ("Content-Type", "application/json"),
        ],
        "{\"method\":\"m\"}",
    );
    assert_eq!(status, 404);
    assert!(v["error"].as_str().unwrap().contains("off"));
    assert_eq!(post(addr, "/api/control/reply", &[], "{}").0, 404);
}

#[test]
fn a_page_that_goes_leaves_the_next_request_to_another() {
    let view = start(true);
    let token = view.control_token().unwrap().to_string();
    let addr = view.addr();
    // a page that takes a request and goes away without answering (its agent waits out its
    // timeout; not waited for here)
    let mut first = Page::connect(&view);
    let t = token.clone();
    let _lost = std::thread::spawn(move || agent(addr, &t, &json!({"id": "lost", "method": "m"})));
    let (seq, _) = first.next();
    drop(first);
    // a new page takes the next request, even when the gone page's stream wakes first
    let mut second = Page::connect(&view);
    let t = token.clone();
    let asker = std::thread::spawn(move || agent(addr, &t, &json!({"id": 2, "method": "m"})));
    let (seq2, req) = second.next();
    assert_ne!(seq, seq2);
    assert_eq!(req["id"], 2);
    page_reply(
        &view,
        &[],
        &json!({"seq": seq2, "reply": {"id": 2, "ok": true, "result": 1}}),
    );
    assert_eq!(asker.join().unwrap().1["result"], 1);
}
