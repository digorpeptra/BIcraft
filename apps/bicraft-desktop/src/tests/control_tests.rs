//! The control channel's transport over a real loopback socket: requests without the token,
//! what is not a request, the bounds; then the desktop app driven through it end to end (the
//! kittest harness steps the real window, a client thread talks to the socket). With
//! `--features gpu-tests` the screenshot is the window drawn by wgpu.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use bicraft_ui::control::Job;
use serde_json::{Value, json};

use super::*;

fn quiet() -> Notify {
    Arc::new(|| {})
}

fn server(limits: Limits) -> (Server, Receiver<Job>) {
    let (tx, rx) = std::sync::mpsc::sync_channel(bicraft_ui::control::MAX_QUEUED);
    (start(0, tx, quiet(), limits).unwrap(), rx)
}

/// A window that answers every request with its method (the window's half replaced).
fn echo(rx: Receiver<Job>) {
    std::thread::spawn(move || {
        while let Ok(job) = rx.recv() {
            let r = bicraft_ui::control::ok(&job.request.id, json!({"method": job.request.method}));
            job.answer(r);
        }
    });
}

struct Client {
    w: TcpStream,
    r: BufReader<TcpStream>,
}

impl Client {
    fn new(addr: SocketAddr) -> Client {
        let w = TcpStream::connect(addr).unwrap();
        w.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
        let r = BufReader::new(w.try_clone().unwrap());
        Client { w, r }
    }

    fn raw(&mut self, bytes: &[u8]) {
        self.w.write_all(bytes).unwrap();
    }

    /// The next reply line (`None`: the server closed the connection).
    fn line(&mut self) -> Option<Value> {
        let mut s = String::new();
        match self.r.read_line(&mut s) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(serde_json::from_str(&s).unwrap()),
        }
    }

    fn call(&mut self, v: Value) -> Value {
        self.raw(format!("{v}\n").as_bytes());
        self.line().expect("a reply")
    }

    fn closed(&mut self) -> bool {
        let mut b = [0u8; 1];
        matches!(self.r.read(&mut b), Ok(0) | Err(_))
    }
}

#[test]
fn requests_without_the_token_are_refused_and_the_connection_closed() {
    let (srv, rx) = server(Limits::default());
    echo(rx);
    let token = srv.token().to_string();
    assert_eq!(token.len(), 32);
    // the right token works, on one connection, in order
    let mut c = Client::new(srv.addr());
    let r = c.call(json!({"id": 1, "method": "ui.inspect", "token": token}));
    assert_eq!(
        r,
        json!({"id": 1, "ok": true, "result": {"method": "ui.inspect"}})
    );
    let r = c.call(json!({"id": "b", "method": "methods", "token": token}));
    assert_eq!(r["id"], "b");
    // blank lines are skipped
    c.raw(b"\r\n\n");
    assert_eq!(c.call(json!({"id": 2, "method": "m", "token": token}))["id"], 2);
    for bad in [
        json!({"id": 3, "method": "ui.inspect"}),
        json!({"id": 4, "method": "ui.inspect", "token": "0".repeat(32)}),
        json!({"id": 5, "method": "ui.inspect", "token": format!("{token}x")}),
    ] {
        let mut c = Client::new(srv.addr());
        let r = c.call(bad.clone());
        assert_eq!(r["ok"], false, "{bad}");
        assert!(r["error"].as_str().unwrap().contains("unauthorized"), "{r}");
        assert_eq!(r["id"], bad["id"]);
        // nothing after it runs
        c.raw(format!("{}\n", json!({"id": 9, "method": "x", "token": token})).as_bytes());
        assert!(c.closed());
    }
}

#[test]
fn what_is_not_a_request_closes_the_connection() {
    let (srv, rx) = server(Limits::default());
    echo(rx);
    let token = srv.token().to_string();
    let after = format!("{}\n", json!({"id": 1, "method": "x", "token": token}));
    for bad in [
        // a browser's fetch to the port: its request line is refused before its body is read
        format!("POST / HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: text/plain\r\n\r\n{after}"),
        "not json\n".to_string(),
        "[1]\n".to_string(),
        format!("{}\n", json!({"id": 1, "token": token})),
        format!(
            "{}\n",
            json!({"id": 1, "method": "x", "token": token, "params": 5})
        ),
    ] {
        let mut c = Client::new(srv.addr());
        c.raw(bad.as_bytes());
        let r = c.line().expect("one error reply");
        assert_eq!(r["ok"], false, "{bad}");
        assert!(
            r["error"].as_str().unwrap().contains("closing the connection"),
            "{r}"
        );
        c.raw(after.as_bytes());
        assert!(c.closed(), "{bad}");
    }
    // invalid UTF-8
    let mut c = Client::new(srv.addr());
    c.raw(b"\xff\xfe\n");
    assert_eq!(c.line().unwrap()["ok"], false);
    assert!(c.closed());
}

#[test]
fn the_bounds_hold() {
    let limits = Limits {
        connections: 2,
        per_second: 3,
        line_bytes: 256,
        idle: Duration::from_millis(400),
        write: Duration::from_secs(5),
        reply: Duration::from_millis(300),
    };
    let (srv, rx) = server(limits);
    let token = srv.token().to_string();
    // a line too long: one error, closed
    let mut c = Client::new(srv.addr());
    c.raw(&[b'x'; 300]);
    let r = c.line().unwrap();
    assert!(r["error"].as_str().unwrap().contains("longer than 256"), "{r}");
    assert!(c.closed());
    // the window does not answer: a timeout, and the request will never run
    let mut c = Client::new(srv.addr());
    let r = c.call(json!({"id": 1, "method": "ui.inspect", "token": token}));
    assert!(r["error"].as_str().unwrap().contains("will not run"), "{r}");
    let job = rx.try_recv().unwrap();
    assert!(!job.answer(json!(null)), "an abandoned job ran");
    // idle: a notice, then closed
    let r = c.line().unwrap();
    assert_eq!(r["closed"], "idle");
    assert!(c.closed());
    // requests per second
    echo(rx);
    let mut c = Client::new(srv.addr());
    let replies: Vec<Value> = (0..5)
        .map(|i| c.call(json!({"id": i, "method": "m", "token": token})))
        .collect();
    assert!(replies.iter().take(3).all(|r| r["ok"] == true));
    assert!(
        replies
            .iter()
            .skip(3)
            .all(|r| r["error"].as_str().unwrap().contains("busy"))
    );
    // connections at once: the third is refused
    drop(c);
    std::thread::sleep(Duration::from_millis(100));
    let mut a = Client::new(srv.addr());
    let mut b = Client::new(srv.addr());
    assert_eq!(
        a.call(json!({"id": 1, "method": "m", "token": token}))["ok"],
        true
    );
    assert_eq!(
        b.call(json!({"id": 1, "method": "m", "token": token}))["ok"],
        true
    );
    let mut d = Client::new(srv.addr());
    let r = d.line().unwrap();
    assert!(
        r["error"].as_str().unwrap().contains("busy: 2 connections"),
        "{r}"
    );
    assert!(d.closed());
}

#[test]
fn the_token_file_is_written_and_removed() {
    let dir = crate::tests::Temp::new("control-file");
    let path = dir.path().join("control.json");
    let (srv, _rx) = server(Limits::default());
    srv.write_file(&path).unwrap();
    let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(v["kind"], "desktop");
    assert_eq!(v["token"], srv.token());
    assert_eq!(v["addr"], srv.addr().to_string());
    drop(srv);
    assert!(!path.exists());
}

/// Drives the real window (the native shell, an engine session on its worker thread) through
/// the socket: open a file, switch pages and tabs, select a visual, the palette, a form, a
/// command, a screenshot.
#[test]
fn an_agent_drives_the_desktop_app_through_the_socket() {
    drive(false);
}

#[cfg(feature = "gpu-tests")]
#[test]
fn an_agent_takes_a_screenshot_of_the_window() {
    drive(true);
}

fn drive(gpu: bool) {
    let dir = crate::tests::Temp::new(if gpu { "control-gpu" } else { "control" });
    let (path, names) = crate::tests::shop(dir.path());
    let (tx, rx) = std::sync::mpsc::sync_channel(bicraft_ui::control::MAX_QUEUED);
    let srv = start(0, tx, quiet(), Limits::default()).unwrap();
    let mut app = crate::app("en-US", None).with_sync_render();
    app.control.attach(rx);
    let builder = egui_kittest::Harness::builder().with_size(egui::vec2(1280.0, 800.0));
    #[cfg(feature = "gpu-tests")]
    let builder = if gpu { builder.wgpu() } else { builder };
    let mut h = builder.build_ui_state(
        |ui, app: &mut crate::App| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    let addr = srv.addr();
    let token = srv.token().to_string();
    let file = path.display().to_string();
    let card = names.card.clone();
    let client = std::thread::spawn(move || {
        let mut c = Client::new(addr);
        let mut n = 0;
        let mut call = |method: &str, params: Value| -> Value {
            n += 1;
            let r = c.call(json!({"id": n, "method": method, "params": params, "token": token}));
            assert_eq!(r["id"], n);
            r
        };
        let mut log = Vec::new();
        let r = call("app.open", json!({"path": file}));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["result"]["idle"], true);
        log.push(r["result"]["title"].clone());
        let r = call("ui.set", json!({"page": "Second"}));
        assert_eq!(r["ok"], true, "{r}");
        let r = call(
            "ui.set",
            json!({"page": "Main", "visual": card, "ribbonTab": "visual"}),
        );
        assert_eq!(r["result"]["ribbonTab"], "visual", "{r}");
        let r = call("ui.palette", json!({"query": "measure"}));
        assert!(r["result"]["palette"].is_object(), "{r}");
        let r = call("ui.close", json!({}));
        assert_eq!(r["result"]["palette"], Value::Null);
        let r = call(
            "command",
            json!({"command": "measure.add", "params": {"table": "Sales", "name": "Agent", "expression": "1"}}),
        );
        assert_eq!(r["ok"], true, "{r}");
        let r = call("ui.inspect", json!({}));
        assert_eq!(r["result"]["dirty"]["model"], true, "{r}");
        log.push(r["result"]["title"].clone());
        let r = call("ui.screenshot", json!({}));
        log.push(r.clone());
        log
    });
    let deadline = Instant::now() + Duration::from_secs(120);
    while !client.is_finished() {
        h.step();
        assert!(Instant::now() < deadline, "the agent's session timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
    let log = client.join().unwrap();
    assert_eq!(log[0], "shop.pbip · rev 0");
    assert!(log[1].as_str().unwrap().starts_with("● shop.pbip"), "{}", log[1]);
    let shot = &log[2];
    // the same command reached the engine as a click would
    assert!(h.state().sent.iter().any(|c| c.id == "measure.add"));
    if !gpu {
        // kittest without wgpu draws no frame: the screenshot says so
        assert!(shot["error"].as_str().unwrap().contains("no frame"), "{shot}");
        return;
    }
    assert_eq!(shot["ok"], true, "{shot}");
    let png = bicraft_ui::control::unbase64(shot["result"]["png"].as_str().unwrap()).unwrap();
    let pm = tiny_skia::Pixmap::decode_png(&png).unwrap();
    assert!(
        pm.width() >= 1280 && pm.height() >= 800,
        "{}x{}",
        pm.width(),
        pm.height()
    );
    assert_eq!(shot["result"]["width"], pm.width());
    if let Some(d) = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS") {
        let d = std::path::PathBuf::from(d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("control-screenshot.png"), &png).unwrap();
    }
}
