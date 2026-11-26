//! End-to-end tests of the viewer's HTTP server over raw TCP.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use bicraft_view::ViewHandle;
use serde_json::{Value, json};

fn start() -> ViewHandle {
    bicraft_view::start("127.0.0.1:0".parse().unwrap()).unwrap()
}

fn connect(addr: SocketAddr) -> TcpStream {
    let s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
    s
}

fn read_all(s: &mut TcpStream) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = [0u8; 16384];
    loop {
        match s.read(&mut buf) {
            Ok(0) | Err(_) => return out,
            Ok(n) => out.extend_from_slice(&buf[..n]),
        }
    }
}

struct Response {
    status: u16,
    head: String,
    body: Vec<u8>,
}

impl Response {
    fn header(&self, name: &str) -> Option<String> {
        self.head.lines().skip(1).find_map(|l| {
            let (n, v) = l.split_once(':')?;
            n.eq_ignore_ascii_case(name).then(|| v.trim().to_string())
        })
    }
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

fn parse(raw: &[u8]) -> Response {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("response head");
    let head = String::from_utf8(raw[..split].to_vec()).unwrap();
    let status = head.split(' ').nth(1).unwrap().parse().unwrap();
    Response {
        status,
        head,
        body: raw[split + 4..].to_vec(),
    }
}

fn send(addr: SocketAddr, request: &[u8]) -> Response {
    let mut s = connect(addr);
    s.write_all(request).unwrap();
    parse(&read_all(&mut s))
}

fn get(addr: SocketAddr, path: &str) -> Response {
    send(
        addr,
        format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n\r\n", addr.port()).as_bytes(),
    )
}

/// Reads from an open stream until `needle` appears; returns everything read so far.
fn read_until(s: &mut TcpStream, seen: &mut Vec<u8>, needle: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut buf = [0u8; 65536];
    loop {
        let text = String::from_utf8_lossy(seen).into_owned();
        if text.contains(needle) {
            return text;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {needle:?}; got {text:?}"
        );
        match s.read(&mut buf) {
            Ok(0) => panic!("stream closed before {needle:?}; got {text:?}"),
            Ok(n) => seen.extend_from_slice(&buf[..n]),
            Err(e) => panic!("read error {e} before {needle:?}; got {text:?}"),
        }
    }
}

fn subscribe(addr: SocketAddr) -> (TcpStream, Vec<u8>) {
    let mut s = connect(addr);
    s.write_all(
        format!(
            "GET /api/events HTTP/1.1\r\nHost: localhost:{}\r\nAccept: text/event-stream\r\n\r\n",
            addr.port()
        )
        .as_bytes(),
    )
    .unwrap();
    let mut seen = Vec::new();
    read_until(&mut s, &mut seen, "event: snapshot");
    (s, seen)
}

#[test]
fn handle_is_send_and_sync() {
    fn check<T: Send + Sync>() {}
    check::<ViewHandle>();
}

#[test]
fn serves_the_page() {
    let view = start();
    assert!(view.url().starts_with("http://127.0.0.1:"));
    let r = get(view.addr(), "/");
    assert_eq!(r.status, 200);
    assert!(r.header("content-type").unwrap().starts_with("text/html"));
    assert_eq!(r.header("content-length").unwrap(), r.body.len().to_string());
    assert!(r.header("content-security-policy").is_some());
    assert!(r.header("access-control-allow-origin").is_none());
    let text = r.text();
    assert!(text.starts_with("<!doctype html>"));
    assert!(text.contains("'api/events'"));
    assert_eq!(get(view.addr(), "/?x=1").status, 200);
}

#[test]
fn state_returns_the_latest_snapshot() {
    let view = start();
    let r = get(view.addr(), "/api/state");
    assert_eq!(r.status, 200);
    assert_eq!(r.text(), "null");
    let snap =
        json!({ "version": 1, "revision": 3, "model": null, "report": { "pages": [] }, "extra": "ünïcode" });
    view.publish(snap.clone());
    let r = get(view.addr(), "/api/state");
    assert_eq!(r.status, 200);
    assert_eq!(r.header("cache-control").as_deref(), Some("no-store"));
    assert!(r.header("content-type").unwrap().starts_with("application/json"));
    assert_eq!(serde_json::from_slice::<Value>(&r.body).unwrap(), snap);
    view.publish(json!({ "revision": 4 }));
    assert_eq!(
        serde_json::from_slice::<Value>(&get(view.addr(), "/api/state").body).unwrap(),
        json!({ "revision": 4 })
    );
}

#[test]
fn sse_delivers_published_snapshots() {
    let view = start();
    let (mut s, mut seen) = subscribe(view.addr());
    let head = read_until(&mut s, &mut seen, "\"seq\":0");
    assert!(head.starts_with("HTTP/1.1 200"));
    assert!(
        head.to_ascii_lowercase()
            .contains("content-type: text/event-stream")
    );
    assert!(head.contains("\"snapshot\":null"));

    view.publish(json!({ "version": 1, "revision": 7, "report": null }));
    let text = read_until(&mut s, &mut seen, "\"revision\":7,\"seq\":1");
    assert!(text.contains(
        "data: {\"revision\":7,\"seq\":1,\"snapshot\":{\"version\":1,\"revision\":7,\"report\":null}}\n\n"
    ));

    view.publish(json!({ "revision": 8 }));
    read_until(&mut s, &mut seen, "\"revision\":8,\"seq\":2");
}

#[test]
fn sse_references_large_snapshots() {
    let view = start();
    let (mut s, mut seen) = subscribe(view.addr());
    let big = "x".repeat(1_200_000);
    view.publish(json!({ "revision": 9, "blob": big }));
    let text = read_until(&mut s, &mut seen, "\"revision\":9,\"seq\":1}");
    assert!(!text.contains("xxxx"));
    let state: Value = serde_json::from_slice(&get(view.addr(), "/api/state").body).unwrap();
    assert_eq!(state["blob"].as_str().map(str::len), Some(1_200_000));
}

#[test]
fn unknown_paths_and_methods() {
    let view = start();
    let addr = view.addr();
    assert_eq!(get(addr, "/nope").status, 404);
    assert_eq!(get(addr, "/api/state/x").status, 404);
    for method in ["POST", "PUT", "DELETE", "OPTIONS", "HEAD"] {
        let r = send(
            addr,
            format!("{method} /api/state HTTP/1.1\r\nHost: localhost\r\nContent-Length: 5\r\n\r\nhello")
                .as_bytes(),
        );
        assert_eq!(r.status, 405, "{method}");
        assert_eq!(r.header("allow").as_deref(), Some("GET"));
    }
}

#[test]
fn host_header_is_checked() {
    let view = start();
    let addr = view.addr();
    let with_host = |host: &str| {
        send(
            addr,
            format!("GET /api/state HTTP/1.1\r\nHost: {host}\r\n\r\n").as_bytes(),
        )
        .status
    };
    assert_eq!(with_host(&format!("localhost:{}", addr.port())), 200);
    assert_eq!(with_host("127.0.0.1"), 200);
    assert_eq!(with_host("[::1]:80"), 200);
    assert_eq!(with_host("evil.example"), 403);
    assert_eq!(with_host(&format!("evil.example:{}", addr.port())), 403);
    assert_eq!(with_host("localhost.evil.example"), 403);
    assert_eq!(with_host("192.168.1.10"), 403);
    assert_eq!(send(addr, b"GET / HTTP/1.1\r\n\r\n").status, 400);
    assert_eq!(
        send(addr, b"GET /api/events HTTP/1.1\r\nHost: evil.example\r\n\r\n").status,
        403
    );
}

#[test]
fn malformed_and_oversized_requests() {
    let view = start();
    let addr = view.addr();
    assert_eq!(send(addr, b"\x00\x01\x02\xff garbage\r\n\r\n").status, 400);
    assert_eq!(send(addr, b"GET\r\n\r\n").status, 400);
    assert_eq!(
        send(addr, b"GET / HTTP/2.0\r\nHost: localhost\r\n\r\n").status,
        505
    );
    assert_eq!(
        send(addr, b"GET / HTTP/1.1\r\nHost localhost\r\n\r\n").status,
        400
    );
    assert_eq!(
        send(addr, b"GET / HTTP/1.1\r\nHost: a\r\nHost: localhost\r\n\r\n").status,
        400
    );
    let long_target = format!("GET /{} HTTP/1.1\r\nHost: localhost\r\n\r\n", "a".repeat(3000));
    assert_eq!(send(addr, long_target.as_bytes()).status, 414);
    let huge = format!(
        "GET / HTTP/1.1\r\nHost: localhost\r\nX-Big: {}\r\n\r\n",
        "b".repeat(20_000)
    );
    assert_eq!(send(addr, huge.as_bytes()).status, 431);
    let mut many = String::from("GET / HTTP/1.1\r\nHost: localhost\r\n");
    for i in 0..100 {
        many.push_str(&format!("X-{i}: 1\r\n"));
    }
    many.push_str("\r\n");
    assert_eq!(send(addr, many.as_bytes()).status, 431);
    // Peer that hangs up mid-request.
    {
        let mut s = connect(addr);
        s.write_all(b"GET / HT").unwrap();
    }
    // Lenient bare-LF request still parses.
    assert_eq!(send(addr, b"GET / HTTP/1.1\nHost: localhost\n\n").status, 200);
    assert_eq!(get(addr, "/").status, 200);
}

#[test]
fn slow_clients_time_out() {
    let view = start();
    let addr = view.addr();
    let started = Instant::now();
    let mut s = connect(addr);
    s.write_all(b"GET / HTTP/1.1\r\nHost: local").unwrap();
    let r = parse(&read_all(&mut s));
    assert_eq!(r.status, 408);
    assert!(started.elapsed() < Duration::from_secs(12));
    assert_eq!(get(addr, "/").status, 200);
}

#[test]
fn many_connections_do_not_block_publish() {
    let view = start();
    let addr = view.addr();
    // Idle connections (more than the cap) and SSE subscribers that never read.
    let idle: Vec<TcpStream> = (0..100).map(|_| connect(addr)).collect();
    let mut stalled = Vec::new();
    for _ in 0..40 {
        let mut s = connect(addr);
        s.write_all(b"GET /api/events HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .unwrap();
        stalled.push(s);
    }
    let payload = "p".repeat(300_000);
    let started = Instant::now();
    for revision in 0..30 {
        view.publish(json!({ "revision": revision, "payload": payload }));
    }
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "publish blocked for {:?}",
        started.elapsed()
    );
    drop(idle);
    drop(stalled);
    // Slots free up once the server notices the hang-ups.
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let mut s = connect(addr);
        let ok = s
            .write_all(
                format!(
                    "GET /api/state HTTP/1.1\r\nHost: localhost:{}\r\n\r\n",
                    addr.port()
                )
                .as_bytes(),
            )
            .is_ok();
        let raw = read_all(&mut s);
        if ok && raw.starts_with(b"HTTP/1.1 200") {
            let body: Value = serde_json::from_slice(&parse(&raw).body).unwrap();
            assert_eq!(body["revision"], 29);
            break;
        }
        assert!(Instant::now() < deadline, "server did not recover");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn publish_from_many_threads() {
    let view = std::sync::Arc::new(start());
    let (mut s, mut seen) = subscribe(view.addr());
    let threads: Vec<_> = (0..8)
        .map(|t| {
            let view = std::sync::Arc::clone(&view);
            std::thread::spawn(move || {
                for i in 0..50 {
                    view.publish(json!({ "thread": t, "i": i }));
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    // 400 publishes; the subscriber eventually sees the last sequence number.
    read_until(&mut s, &mut seen, "\"seq\":400");
}

#[test]
fn drop_stops_the_server() {
    let view = start();
    let addr = view.addr();
    let (mut s, mut seen) = subscribe(addr);
    read_until(&mut s, &mut seen, "\"seq\":0");
    let started = Instant::now();
    drop(view);
    assert!(started.elapsed() < Duration::from_secs(2));
    // The SSE stream ends.
    let rest = read_all(&mut s);
    assert!(!String::from_utf8_lossy(&rest).contains("event: snapshot"));
    assert!(started.elapsed() < Duration::from_secs(5));
    // Nothing listens any more.
    assert!(TcpStream::connect_timeout(&addr, Duration::from_secs(3)).is_err());
}

#[test]
fn refuses_non_loopback_addresses() {
    let err = bicraft_view::start("0.0.0.0:0".parse().unwrap()).err().unwrap();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    // IPv6 loopback is allowed (it may still fail to bind where IPv6 is disabled).
    if let Err(e) = bicraft_view::start("[::1]:0".parse().unwrap()) {
        assert_ne!(e.kind(), std::io::ErrorKind::InvalidInput);
    }
}

// ---------------------------------------------------------------------------------------------
// /api/asset/<id>

const PNG_ID: &str = "0123456789abcdef0123456789abcdef";
const SVG_ID: &str = "fedcba9876543210fedcba9876543210";

fn asset(id: &str, mime: &str, bytes: &[u8]) -> bicraft_view::Asset {
    bicraft_view::Asset {
        id: id.into(),
        mime: mime.into(),
        bytes: bytes.into(),
    }
}

fn with_assets() -> ViewHandle {
    let view = start();
    let accepted = view.publish_with_assets(
        json!({"revision": 1}),
        [
            asset(PNG_ID, "image/png", b"\x89PNG\r\n\x1a\nfake"),
            asset(
                SVG_ID,
                "IMAGE/SVG+XML",
                b"<svg xmlns='http://www.w3.org/2000/svg'><script>alert(1)</script></svg>",
            ),
        ],
    );
    assert_eq!(accepted, 2);
    view
}

#[test]
fn assets_are_served_with_safe_headers() {
    let view = with_assets();
    let addr = view.addr();
    let r = get(addr, &format!("/api/asset/{PNG_ID}"));
    assert_eq!(r.status, 200);
    assert_eq!(r.body, b"\x89PNG\r\n\x1a\nfake");
    assert_eq!(r.header("content-type").as_deref(), Some("image/png"));
    assert_eq!(r.header("x-content-type-options").as_deref(), Some("nosniff"));
    assert_eq!(
        r.header("cross-origin-resource-policy").as_deref(),
        Some("same-origin")
    );
    assert!(r.header("cache-control").unwrap().contains("immutable"));
    assert_eq!(r.header("etag"), Some(format!("\"{PNG_ID}\"")));
    assert!(r.header("access-control-allow-origin").is_none());
    // SVG keeps its canonical type and is sandboxed when opened as a document.
    let r = get(addr, &format!("/api/asset/{SVG_ID}"));
    assert_eq!(r.status, 200);
    assert_eq!(r.header("content-type").as_deref(), Some("image/svg+xml"));
    let csp = r.header("content-security-policy").unwrap();
    assert!(
        csp.contains("sandbox") && csp.contains("default-src 'none'"),
        "{csp}"
    );
    assert!(!csp.contains("script-src"));
    // Query strings are ignored; revalidation answers 304 without a body.
    assert_eq!(get(addr, &format!("/api/asset/{PNG_ID}?v=2")).status, 200);
    let r = send(
        addr,
        format!("GET /api/asset/{PNG_ID} HTTP/1.1\r\nHost: localhost\r\nIf-None-Match: \"{PNG_ID}\"\r\n\r\n")
            .as_bytes(),
    );
    assert_eq!(r.status, 304);
    assert!(r.body.is_empty());
    // The page may load same-origin images.
    let page = get(addr, "/");
    assert!(
        page.header("content-security-policy")
            .unwrap()
            .contains("img-src data: 'self'")
    );
}

#[test]
fn asset_ids_methods_and_hosts_are_checked() {
    let view = with_assets();
    let addr = view.addr();
    for bad in [
        "/api/asset/",
        "/api/asset/xyz",
        "/api/asset/0123456789ABCDEF0123456789ABCDEF",
        "/api/asset/0123456789abcde",
        "/api/asset/../state",
        "/api/asset/%2e%2e%2fstate",
        "/api/asset/0123456789abcdef0123456789abcdef/x",
        "/api/asset/0123456789abcdef0123456789abcdef.png",
    ] {
        assert_eq!(get(addr, bad).status, 400, "{bad}");
    }
    let long = "a".repeat(65);
    assert_eq!(get(addr, &format!("/api/asset/{long}")).status, 400);
    // Well-formed but unknown.
    assert_eq!(get(addr, &format!("/api/asset/{}", "0".repeat(32))).status, 404);
    for method in ["POST", "PUT", "DELETE", "HEAD"] {
        let r = send(
            addr,
            format!("{method} /api/asset/{PNG_ID} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes(),
        );
        assert_eq!(r.status, 405, "{method}");
    }
    let r = send(
        addr,
        format!("GET /api/asset/{PNG_ID} HTTP/1.1\r\nHost: evil.example\r\n\r\n").as_bytes(),
    );
    assert_eq!(r.status, 403);
    assert!(r.body.starts_with(b"403"));
    assert_eq!(
        send(
            addr,
            format!("GET /api/asset/{PNG_ID} HTTP/1.1\r\n\r\n").as_bytes()
        )
        .status,
        400
    );
}

#[test]
fn assets_are_filtered_and_capped() {
    let view = start();
    let big = vec![0u8; bicraft_view::MAX_ASSET_BYTES + 1];
    let accepted = view.publish_assets([
        asset(PNG_ID, "image/png", b"x"),
        // duplicate id: the first one wins
        asset(PNG_ID, "image/png", b"y"),
        asset("1111111111111111", "text/html", b"<script></script>"),
        asset("2222222222222222", "application/octet-stream", b"x"),
        asset("3333333333333333", "image/svg+xml; charset=utf-8", b"<svg/>"),
        asset("BAD-ID-0000000000", "image/png", b"x"),
        asset("4444444444444444", "image/jpeg", &big),
        asset("5555555555555555", "image/webp", b"RIFF"),
        asset("6666666666666666", "image/gif", b"GIF89a"),
    ]);
    assert_eq!(accepted, 3);
    let addr = view.addr();
    assert_eq!(get(addr, &format!("/api/asset/{PNG_ID}")).body, b"x");
    for (id, status) in [
        ("1111111111111111", 404),
        ("2222222222222222", 404),
        ("3333333333333333", 404),
        ("4444444444444444", 404),
        ("5555555555555555", 200),
        ("6666666666666666", 200),
    ] {
        assert_eq!(get(addr, &format!("/api/asset/{id}")).status, status, "{id}");
    }
    // The total is capped too: 9 assets of 16 MiB exceed 128 MiB.
    let chunk: std::sync::Arc<[u8]> = vec![1u8; bicraft_view::MAX_ASSET_BYTES].into();
    let many = (0..9).map(|i| bicraft_view::Asset {
        id: format!("{i:032x}"),
        mime: "image/png".into(),
        bytes: chunk.clone(),
    });
    assert_eq!(view.publish_assets(many), 8);
    // A new publish replaces the previous set.
    assert_eq!(get(addr, &format!("/api/asset/{PNG_ID}")).status, 404);
    assert_eq!(view.publish_assets([]), 0);
    assert_eq!(get(addr, &format!("/api/asset/{:032x}", 0)).status, 404);
}

#[test]
fn snapshot_and_assets_publish_together() {
    let view = start();
    view.publish_with_assets(json!({"revision": 7}), [asset(PNG_ID, "image/png", b"p")]);
    let addr = view.addr();
    assert_eq!(get(addr, "/api/state").text(), "{\"revision\":7}");
    assert_eq!(get(addr, &format!("/api/asset/{PNG_ID}")).body, b"p");
}

// ---------------------------------------------------------------------------------------------
// POST /api/open-in-desktop (the "Open in Power BI Desktop" button)

type Opened = std::sync::Arc<std::sync::Mutex<Vec<std::path::PathBuf>>>;

/// A viewer whose opener records the paths instead of launching anything.
fn with_opener(fail: Option<&'static str>) -> (ViewHandle, Opened) {
    let seen: Opened = Default::default();
    let log = std::sync::Arc::clone(&seen);
    let opener: bicraft_view::Opener = std::sync::Arc::new(move |p: &std::path::Path| {
        log.lock().unwrap().push(p.to_path_buf());
        fail.map_or(Ok(()), |e| Err(e.to_string()))
    });
    let options = bicraft_view::ViewOptions {
        opener: Some(opener),
        ..Default::default()
    };
    let view = bicraft_view::start_with("127.0.0.1:0".parse().unwrap(), options).unwrap();
    (view, seen)
}

/// A POST to the route with optional extra headers and body.
fn post_open(addr: SocketAddr, target: &str, headers: &[(&str, &str)], body: &str) -> Response {
    let mut req = format!(
        "POST {target} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Length: {}\r\n",
        addr.port(),
        body.len()
    );
    for (n, v) in headers {
        req.push_str(&format!("{n}: {v}\r\n"));
    }
    req.push_str("\r\n");
    req.push_str(body);
    send(addr, req.as_bytes())
}

fn json_body(r: &Response) -> Value {
    serde_json::from_slice(&r.body).unwrap()
}

#[test]
fn the_page_carries_the_token_only_when_enabled() {
    let plain = start();
    assert!(plain.open_token().is_none());
    let page = get(plain.addr(), "/").text();
    assert!(page.contains(r#"<meta name="bicraft-open-token" content="">"#));
    let r = post_open(
        plain.addr(),
        "/api/open-in-desktop",
        &[("X-Bicraft-Token", "x")],
        "",
    );
    assert_eq!(r.status, 404);

    let (view, _) = with_opener(None);
    let token = view.open_token().unwrap().to_string();
    assert_eq!(token.len(), 32);
    let r = get(view.addr(), "/");
    assert_eq!(r.header("cache-control").as_deref(), Some("no-store"));
    assert!(
        r.text()
            .contains(&format!(r#"<meta name="bicraft-open-token" content="{token}">"#))
    );
    // another run has another token
    let (other, _) = with_opener(None);
    assert_ne!(other.open_token().unwrap(), token);
}

#[test]
fn open_requires_the_token_and_this_origin() {
    let (view, seen) = with_opener(None);
    let addr = view.addr();
    let file = std::path::PathBuf::from("C:/work/Shop.pbix");
    view.set_open_target(Some(file.clone()), false);
    let token = view.open_token().unwrap().to_string();
    let origin = format!("http://127.0.0.1:{}", addr.port());
    let foreign = [
        vec![],
        vec![("X-Bicraft-Token", "0123456789abcdef0123456789abcdef")],
        vec![("X-Bicraft-Token", "")],
        vec![
            ("X-Bicraft-Token", token.as_str()),
            ("Origin", "http://evil.example"),
        ],
        vec![("X-Bicraft-Token", token.as_str()), ("Origin", "null")],
        vec![
            ("X-Bicraft-Token", token.as_str()),
            ("Origin", "http://127.0.0.1:1"),
        ],
        vec![
            ("X-Bicraft-Token", token.as_str()),
            ("Sec-Fetch-Site", "cross-site"),
        ],
        vec![
            ("X-Bicraft-Token", token.as_str()),
            ("Sec-Fetch-Site", "same-site"),
        ],
        // the token in the query string or body does not count
    ];
    for headers in &foreign {
        let r = post_open(addr, "/api/open-in-desktop", headers, "");
        assert_eq!(r.status, 403, "{headers:?}");
        assert_eq!(json_body(&r)["ok"], false);
    }
    let r = post_open(addr, &format!("/api/open-in-desktop?token={token}"), &[], &token);
    assert_eq!(r.status, 403);
    // a simple cross-site form post (no custom header possible) is refused
    let r = post_open(
        addr,
        "/api/open-in-desktop",
        &[
            ("Origin", "http://evil.example"),
            ("Content-Type", "application/x-www-form-urlencoded"),
        ],
        "path=C%3A%2Fevil.exe",
    );
    assert_eq!(r.status, 403);
    // other methods
    let r = get(addr, "/api/open-in-desktop");
    assert_eq!(r.status, 405);
    assert_eq!(r.header("allow").as_deref(), Some("POST"));
    // a foreign Host is refused before anything else (DNS rebinding)
    let r = send(
        addr,
        format!(
            "POST /api/open-in-desktop HTTP/1.1\r\nHost: evil.example\r\nX-Bicraft-Token: {token}\r\n\r\n"
        )
        .as_bytes(),
    );
    assert_eq!(r.status, 403);
    assert!(seen.lock().unwrap().is_empty(), "nothing opened");

    // the page's own request: token, same Origin, same-origin fetch; a path in the request is ignored
    let ok = post_open(
        addr,
        "/api/open-in-desktop?path=C:/Windows/notepad.exe",
        &[
            ("X-Bicraft-Token", token.as_str()),
            ("Origin", origin.as_str()),
            ("Sec-Fetch-Site", "same-origin"),
        ],
        r#"{"path":"C:/Windows/notepad.exe"}"#,
    );
    assert_eq!(ok.status, 200, "{}", ok.text());
    let body = json_body(&ok);
    assert_eq!(body["ok"], true);
    assert_eq!(body["unsaved"], false);
    assert_eq!(*seen.lock().unwrap(), vec![file]);
}

#[test]
fn open_states_no_file_unsaved_cooldown_and_errors() {
    let (view, seen) = with_opener(None);
    let addr = view.addr();
    let token = view.open_token().unwrap().to_string();
    let h = [("X-Bicraft-Token", token.as_str())];
    // nothing saved yet
    let r = post_open(addr, "/api/open-in-desktop", &h, "");
    assert_eq!(r.status, 409);
    assert!(json_body(&r)["error"].as_str().unwrap().contains("save"));
    view.set_open_target(None, true);
    assert_eq!(post_open(addr, "/api/open-in-desktop", &h, "").status, 409);
    assert!(seen.lock().unwrap().is_empty());
    // unsaved edits are reported back
    view.set_open_target(Some("x/Shop.pbip".into()), true);
    let r = post_open(addr, "/api/open-in-desktop", &h, "");
    assert_eq!(r.status, 200);
    assert_eq!(json_body(&r)["unsaved"], true);
    // a second click right away does not start a second Desktop
    let r = post_open(addr, "/api/open-in-desktop", &h, "");
    assert_eq!(r.status, 429);
    assert_eq!(seen.lock().unwrap().len(), 1);

    // the opener's error reaches the page, and does not hold the cooldown
    let (view, seen) = with_opener(Some("no application is registered for .pbix files"));
    let token = view.open_token().unwrap().to_string();
    let h = [("X-Bicraft-Token", token.as_str())];
    view.set_open_target(Some("Shop.pbix".into()), false);
    for _ in 0..2 {
        let r = post_open(view.addr(), "/api/open-in-desktop", &h, "");
        assert_eq!(r.status, 500);
        assert!(
            json_body(&r)["error"]
                .as_str()
                .unwrap()
                .contains("no application")
        );
    }
    assert_eq!(seen.lock().unwrap().len(), 2);
}

/// A viewer with interactions, and a front-end thread that answers each action with
/// `{echo: action}` (or an error for `{"action": "fail"}`) until the handle is dropped.
fn with_interactions() -> std::sync::Arc<ViewHandle> {
    let options = bicraft_view::ViewOptions {
        interactions: true,
        ..Default::default()
    };
    let view =
        std::sync::Arc::new(bicraft_view::start_with("127.0.0.1:0".parse().unwrap(), options).unwrap());
    let weak = std::sync::Arc::downgrade(&view);
    std::thread::spawn(move || {
        while let Some(v) = weak.upgrade() {
            if let Some(i) = v.next_interaction(Duration::from_millis(50)) {
                let result = if i.action["action"] == "fail" {
                    Err("no such action".to_string())
                } else {
                    Ok(json!({"echo": i.action.clone()}))
                };
                i.respond(result);
            }
        }
    });
    view
}

#[test]
fn interactions_need_the_token_this_origin_and_a_json_body() {
    // disabled by default: no token in the page, 404
    let plain = start();
    assert!(plain.interact_token().is_none());
    assert!(
        get(plain.addr(), "/")
            .text()
            .contains(r#"<meta name="bicraft-interact-token" content="">"#)
    );
    let r = post_open(
        plain.addr(),
        "/api/interact",
        &[("Content-Type", "application/json")],
        "{}",
    );
    assert_eq!(r.status, 404);

    let view = with_interactions();
    let addr = view.addr();
    let token = view.interact_token().unwrap().to_string();
    assert!(view.open_token().is_none(), "the open button stays off");
    assert!(get(addr, "/").text().contains(&format!(
        r#"<meta name="bicraft-interact-token" content="{token}">"#
    )));
    let json_type = ("Content-Type", "application/json");
    let tok = ("X-Bicraft-Token", token.as_str());
    for headers in [
        vec![json_type],
        vec![json_type, ("X-Bicraft-Token", "0123456789abcdef0123456789abcdef")],
        vec![json_type, tok, ("Origin", "http://evil.example")],
        vec![json_type, tok, ("Sec-Fetch-Site", "cross-site")],
    ] {
        let r = post_open(addr, "/api/interact", &headers, r#"{"action":"page"}"#);
        assert_eq!(r.status, 403, "{headers:?}");
    }
    // a form post or a text body is refused, so is a non-object, an oversized or truncated body
    let r = post_open(
        addr,
        "/api/interact",
        &[tok, ("Content-Type", "text/plain")],
        "{}",
    );
    assert_eq!(r.status, 415);
    let r = post_open(addr, "/api/interact", &[tok, json_type], "[1]");
    assert_eq!(r.status, 400);
    let r = post_open(addr, "/api/interact", &[tok, json_type], "{not json");
    assert_eq!(r.status, 400);
    let big = format!(r#"{{"x":"{}"}}"#, "a".repeat(70 * 1024));
    let r = post_open(addr, "/api/interact", &[tok, json_type], &big);
    assert_eq!(r.status, 413);
    let r = send(
        addr,
        format!(
            "POST /api/interact HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nX-Bicraft-Token: {token}\r\n\
             Content-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{{}}\r\n0\r\n\r\n",
            addr.port()
        )
        .as_bytes(),
    );
    assert_eq!(r.status, 411);
    let r = get(addr, "/api/interact");
    assert_eq!(r.status, 405);

    // the page's own request reaches the front end; its answer is the response
    let origin = format!("http://127.0.0.1:{}", addr.port());
    let headers = [
        tok,
        json_type,
        ("Origin", origin.as_str()),
        ("Sec-Fetch-Site", "same-origin"),
    ];
    let ok = post_open(
        addr,
        "/api/interact",
        &headers,
        r#"{"action":"page","page":"P1"}"#,
    );
    assert_eq!(ok.status, 200, "{}", ok.text());
    assert_eq!(
        json_body(&ok),
        json!({"ok": true, "result": {"echo": {"action": "page", "page": "P1"}}})
    );
    let bad = post_open(addr, "/api/interact", &headers, r#"{"action":"fail"}"#);
    assert_eq!(bad.status, 400);
    assert_eq!(json_body(&bad), json!({"ok": false, "error": "no such action"}));
}

/// `/app/`: the browser build of the egui app (docs/webui.md) with the page's tokens, its files,
/// or 404 in a build without it.
#[test]
fn serves_the_browser_app() {
    let view = bicraft_view::start_with(
        "127.0.0.1:0".parse().unwrap(),
        bicraft_view::ViewOptions {
            opener: None,
            interactions: true,
            control: false,
        },
    )
    .unwrap();
    let a = view.addr();
    let r = get(a, "/app");
    assert_eq!(r.status, 308);
    assert_eq!(r.header("location").as_deref(), Some("/app/"));
    let r = get(a, "/app/");
    if !bicraft_view::webui::available() {
        assert_eq!(r.status, 404);
        assert!(r.text().contains("without the browser app"));
        assert_eq!(get(a, "/app/bicraft_web.js").status, 404);
        // the HTML page is unchanged
        assert_eq!(get(a, "/").status, 200);
        return;
    }
    assert_eq!(r.status, 200);
    assert_eq!(
        r.header("content-security-policy").as_deref(),
        Some(bicraft_view::webui::APP_CONTENT_SECURITY_POLICY)
    );
    let html = r.text();
    let token = view.interact_token().unwrap();
    assert!(html.contains(&format!(
        r#"<meta name="bicraft-interact-token" content="{token}">"#
    )));
    assert!(html.contains(r#"<meta name="bicraft-view-id" content="">"#));
    assert_eq!(get(a, "/app/index.html").text(), html);
    let r = send(
        a,
        format!(
            "GET /app/bicraft_web_bg.wasm HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAccept-Encoding: gzip\r\n\r\n",
            a.port()
        )
        .as_bytes(),
    );
    assert_eq!(r.status, 200);
    assert_eq!(r.header("content-encoding").as_deref(), Some("gzip"));
    let etag = r.header("etag").unwrap();
    let r = send(
        a,
        format!(
            "GET /app/bicraft_web_bg.wasm HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nIf-None-Match: {etag}\r\n\r\n",
            a.port()
        )
        .as_bytes(),
    );
    assert_eq!(r.status, 304);
    let r = get(a, "/app/bicraft_web_bg.wasm");
    assert!(r.body.starts_with(b"\0asm") && r.header("content-encoding").is_none());
    assert_eq!(get(a, "/app/boot.js").status, 200);
    assert_eq!(get(a, "/app/nope.js").status, 404);
}
