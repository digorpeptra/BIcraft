//! The control channel's protocol: requests refused before they reach the window, replies,
//! tickets given up, PNG and base64.

use super::*;

#[test]
fn a_request_is_one_json_object_with_a_method() {
    let (r, token) = parse(br#"{"id": 7, "method": "ui.inspect", "token": "abc"}"#).unwrap();
    assert_eq!(r.id, json!(7));
    assert_eq!(r.method, "ui.inspect");
    assert_eq!(r.params, json!({}));
    assert_eq!(token.as_deref(), Some("abc"));
    let (r, token) = parse(br#"{"id":"a","method":"command","params":{"command":"x"}}"#).unwrap();
    assert_eq!(r.id, json!("a"));
    assert_eq!(r.params["command"], "x");
    assert!(token.is_none());
    // CRLF and spaces around it
    assert!(parse(b"  {\"method\": \"methods\"}\r\n").is_ok());
}

#[test]
fn anything_else_is_refused() {
    for bad in [
        &b"GET / HTTP/1.1"[..],
        b"POST /control HTTP/1.1\r\nHost: x",
        b"not json",
        b"[1, 2]",
        b"42",
        b"\"method\"",
        b"{}",
        b"{\"method\": 3}",
        b"{\"method\": \"\"}",
        b"{\"method\": \"ui.inspect\", \"id\": {}}",
        b"{\"method\": \"ui.inspect\", \"id\": [1]}",
        b"{\"method\": \"ui.inspect\", \"params\": [1]}",
        b"{\"method\": \"ui.inspect\", \"params\": \"x\"}",
        b"{\"method\": \"ui.inspect\", \"token\": 5}",
        b"{\"method\": \"ui.inspect\", \"extra\": 1}",
        b"{\"method\": \"ui.inspect\"} {\"method\": \"ui.inspect\"}",
        b"\xff\xfe{}",
    ] {
        assert!(parse(bad).is_err(), "{:?}", String::from_utf8_lossy(bad));
    }
    let long = format!("{{\"method\": \"{}\"}}", "m".repeat(65));
    assert!(parse(long.as_bytes()).is_err());
    let huge = format!(
        "{{\"method\": \"ui.inspect\", \"params\": {{\"q\": \"{}\"}}}}",
        "x".repeat(MAX_REQUEST_BYTES)
    );
    assert!(parse(huge.as_bytes()).unwrap_err().contains("larger"));
}

#[test]
fn replies_have_the_family_shape() {
    assert_eq!(
        ok(&json!(1), json!({"a": 1})),
        json!({"id": 1, "ok": true, "result": {"a": 1}})
    );
    assert_eq!(
        err(&Value::Null, "no"),
        json!({"id": null, "ok": false, "error": "no"})
    );
}

#[test]
fn a_request_given_up_before_it_started_never_runs() {
    let req = || Request {
        id: json!(3),
        method: "ui.inspect".into(),
        params: json!({}),
    };
    // given up while queued: the window skips it
    let (job, ticket) = Job::new(req());
    let r = ticket.give_up(60);
    assert!(r["error"].as_str().unwrap().contains("will not run"), "{r}");
    assert_eq!(r["id"], 3);
    assert!(
        job.state
            .compare_exchange(QUEUED, STARTED, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
    );
    // started, then given up: the outcome is unknown
    let (job, ticket) = Job::new(req());
    job.state.store(STARTED, Ordering::SeqCst);
    assert!(ticket.give_up(60)["error"].as_str().unwrap().contains("unknown"));
    // a reply that came wins
    let (job, ticket) = Job::new(req());
    job.reply.try_send(ok(&json!(3), json!(1))).unwrap();
    assert_eq!(ticket.give_up(60)["ok"], true);
}

#[test]
fn base64_round_trips() {
    for (raw, enc) in [
        (&b""[..], ""),
        (b"f", "Zg=="),
        (b"fo", "Zm8="),
        (b"foo", "Zm9v"),
        (b"foobar", "Zm9vYmFy"),
    ] {
        assert_eq!(base64(raw), enc);
        assert_eq!(unbase64(enc).unwrap(), raw);
    }
    let all: Vec<u8> = (0..=255).collect();
    assert_eq!(unbase64(&base64(&all)).unwrap(), all);
    assert!(unbase64("a b").is_none());
    assert!(unbase64("é").is_none());
}

#[test]
fn a_frame_becomes_a_png() {
    let mut img = egui::ColorImage::filled([3, 2], egui::Color32::from_rgb(10, 20, 30));
    if let Some(p) = img.pixels.get_mut(1) {
        *p = egui::Color32::WHITE;
    }
    let png = png(&img).unwrap();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    let back = tiny_skia::Pixmap::decode_png(&png).unwrap();
    assert_eq!((back.width(), back.height()), (3, 2));
    assert_eq!(back.pixel(1, 0).unwrap().red(), 255);
    assert_eq!(back.pixel(0, 1).unwrap().blue(), 30);
    let r = shot_result(&img).unwrap();
    assert_eq!(r["width"], 3);
    assert_eq!(unbase64(r["png"].as_str().unwrap()).unwrap(), png);
    // a damaged frame is an error, never a panic
    let mut bad = img.clone();
    bad.pixels.pop();
    assert!(png_or_err(&bad));
    let empty = egui::ColorImage::filled([0, 0], egui::Color32::BLACK);
    assert!(png_or_err(&empty));
}

fn png_or_err(img: &egui::ColorImage) -> bool {
    png(img).is_err()
}

#[test]
fn every_method_is_listed_once() {
    let mut names: Vec<&str> = METHODS.iter().map(|m| m.0).collect();
    let n = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), n);
    for (m, _, what) in METHODS {
        assert!(!what.is_empty(), "{m}");
    }
}
