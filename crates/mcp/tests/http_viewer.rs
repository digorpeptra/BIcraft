//! The live viewer of `bicraft-cli serve` over real sockets (docs/kubernetes.md, "Viewer"; docs/auth.md,
//! "The viewer in a browser"): the page and its private views, readers seeing saved files only
//! (also over MCP), a reader's private `file.open` keeping the editor's undo history, copies
//! reloaded on a save, an upload and an external change, the browser login, CSRF, the trusted
//! header, metrics and logs without model data, and the viewer turned off.

#[path = "../../../testsupport/tempdir.rs"]
mod tempdir;

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bicraft_auth::tokens::{self, entry_json};
use bicraft_auth::{Access, AuthConfig, Authenticator, Grant};
use bicraft_mcp::http::HTTP_PROTOCOL_VERSION;
use bicraft_mcp::{HttpConfig, HttpServer};
use bicraft_mcp::{LogFormat, LogSink};
use serde_json::{Value, json};
use tempdir::TempDir;

const MODEL: &str = include_str!("../../engine/tests/table_functions/model.bim");

struct Tokens {
    editor: String,
    reader: String,
    reader2: String,
}

fn tokens() -> Tokens {
    Tokens {
        editor: tokens::generate().unwrap(),
        reader: tokens::generate().unwrap(),
        reader2: tokens::generate().unwrap(),
    }
}

fn auth(t: &Tokens, proxy: bool) -> AuthConfig {
    let mut c = AuthConfig::default();
    let entries: Vec<Value> = [("ed", &t.editor), ("rd", &t.reader), ("rd2", &t.reader2)]
        .iter()
        .map(|(name, token)| entry_json(name, &tokens::hash_token(token).unwrap(), &[], &[]))
        .collect();
    c.add_tokens(tokens::parse(&Value::Array(entries)).unwrap())
        .unwrap();
    for (g, access) in [
        ("token:ed=a", Access::Editor),
        ("token:rd=a", Access::Reader),
        ("token:rd2=a", Access::Reader),
        ("user:ana=a", Access::Reader),
    ] {
        c.acl.grants.push(Grant::parse_text_as(g, access).unwrap());
    }
    if proxy {
        c.merge_json(
            &json!({"proxy": {"trusted": ["127.0.0.1/32", "::1/128"]}}),
            std::path::Path::new("."),
        )
        .unwrap();
    }
    c
}

fn config(t: &Tokens) -> HttpConfig {
    HttpConfig {
        auth: Some(Arc::new(Authenticator::new(auth(t, false)).unwrap())),
        ..HttpConfig::default()
    }
}

fn start(dir: &TempDir, config: HttpConfig) -> HttpServer {
    HttpServer::start("127.0.0.1:0".parse().unwrap(), dir.path(), config).unwrap()
}

#[derive(Debug)]
struct Response {
    status: u16,
    head: String,
    body: Vec<u8>,
}

impl Response {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|e| panic!("{e}: {}", self.text()))
    }
    fn header(&self, name: &str) -> Option<String> {
        self.head.lines().find_map(|l| {
            let (n, v) = l.split_once(':')?;
            n.trim().eq_ignore_ascii_case(name).then(|| v.trim().to_string())
        })
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
        head,
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

fn call(addr: SocketAddr, token: &str, tool: &str, args: Value) -> Value {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {
        "name": tool, "arguments": args, "_meta": {
            "io.modelcontextprotocol/protocolVersion": HTTP_PROTOCOL_VERSION,
            "io.modelcontextprotocol/clientCapabilities": {}}}})
    .to_string();
    let headers = format!(
        "Content-Type: application/json\r\nAccept: application/json\r\n\
         MCP-Protocol-Version: {HTTP_PROTOCOL_VERSION}\r\nMcp-Method: tools/call\r\nMcp-Name: {tool}\r\n{}",
        bearer(token)
    );
    let r = request(addr, "POST", "/w/a/mcp", &headers, body.as_bytes());
    assert_eq!(r.status, 200, "{}", r.text());
    r.json()["result"].clone()
}

fn ok(addr: SocketAddr, token: &str, tool: &str, args: Value) -> Value {
    let r = call(addr, token, tool, args.clone());
    assert_eq!(r["isError"], false, "{tool} {args}: {r}");
    r["structuredContent"].clone()
}

fn run(addr: SocketAddr, token: &str, command: &str, params: Value) -> Value {
    ok(
        addr,
        token,
        "run_command",
        json!({"command": command, "params": params}),
    )
}

/// The value of a one-measure DAX query over MCP.
fn measure(addr: SocketAddr, token: &str, name: &str) -> f64 {
    let r = ok(
        addr,
        token,
        "evaluate_dax_query",
        json!({"query": format!("EVALUATE ROW(\"v\", [{name}])")}),
    );
    r["results"][0]["rows"][0]["[v]"].as_f64().unwrap_or(f64::NAN)
}

/// A page: its view id and interaction token (from the page's meta slots).
struct Page {
    vid: String,
    token: String,
}

fn meta(html: &str, name: &str) -> String {
    let needle = format!(r#"<meta name="{name}" content=""#);
    let at = html.find(&needle).unwrap_or_else(|| panic!("no {name}")) + needle.len();
    html[at..].split('"').next().unwrap().to_string()
}

fn page(addr: SocketAddr, auth: &str) -> Page {
    let r = request(addr, "GET", "/w/a/view/", auth, b"");
    assert_eq!(r.status, 200, "{}", r.text());
    assert!(r.header("content-security-policy").is_some());
    let html = r.text();
    assert!(html.contains(r#"<meta name="bicraft-download" content="1">"#));
    // never the open-in-Desktop token on a server
    assert!(html.contains(r#"<meta name="bicraft-open-token" content="">"#));
    Page {
        vid: meta(&html, "bicraft-view-id"),
        token: meta(&html, "bicraft-interact-token"),
    }
}

fn state(addr: SocketAddr, auth: &str, p: &Page) -> Value {
    let r = request(
        addr,
        "GET",
        &format!("/w/a/view/api/state?v={}", p.vid),
        auth,
        b"",
    );
    assert_eq!(r.status, 200, "{}", r.text());
    r.json()
}

fn interact(addr: SocketAddr, auth: &str, p: &Page, action: &Value) -> Response {
    let body = action.to_string();
    let headers = format!(
        "{auth}Origin: http://127.0.0.1\r\nSec-Fetch-Site: same-origin\r\nX-Bicraft-Token: {}\r\n\
         Content-Type: application/json\r\n",
        p.token
    );
    request(
        addr,
        "POST",
        &format!("/w/a/view/api/interact?v={}", p.vid),
        &headers,
        body.as_bytes(),
    )
}

/// The first value of visual `name` in a snapshot, waiting out visuals left without data.
fn card(addr: SocketAddr, auth: &str, p: &Page, name: &str) -> Value {
    for _ in 0..100 {
        let snap = state(addr, auth, p);
        let pages = snap
            .pointer("/report/pages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let v = pages
            .iter()
            .flat_map(|p| p["visuals"].as_array().cloned().unwrap_or_default())
            .find(|v| v["name"] == name)
            .unwrap_or_else(|| panic!("no visual {name}: {snap}"));
        if !v["data"].is_null() {
            return v["data"]["rows"][0][0].clone();
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("visual {name} never got its data")
}

/// Builds and saves `shop.pbip` (Sales by Category, a slicer and a card) as the editor; returns
/// the slicer's and the card's names.
fn shop(addr: SocketAddr, ed: &str) -> (String, String) {
    let sales = r#"#table({"Category", "Amount"}, {{"Wheels", 100}, {"Wheels", 50}, {"Toys", 5}})"#;
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Category", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        ("report.new", json!({})),
        ("page.add", json!({"display_name": "Main"})),
    ] {
        run(addr, ed, id, p);
    }
    let add = |ty: &str, field: &str| -> String {
        run(
            addr,
            ed,
            "visual.add",
            json!({"page": "Main", "visual_type": ty, "x": 0, "y": 0, "width": 100, "height": 100,
                   "fields": [{"role": "Values", "field": field}]}),
        )["visual"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let slicer = add("slicer", "Sales[Category]");
    let card = add("card", "[Total]");
    run(addr, ed, "file.save_as", json!({"path": "shop.pbip"}));
    (slicer, card)
}

fn workspace(name: &str) -> TempDir {
    let dir = TempDir::new(name);
    std::fs::create_dir_all(dir.path().join("a")).unwrap();
    std::fs::write(dir.path().join("a").join("model.bim"), MODEL).unwrap();
    dir
}

#[test]
fn private_views_and_readers_see_saved_files() {
    let t = tokens();
    let dir = workspace("viewer-private");
    let server = start(&dir, config(&t));
    let a = server.addr();
    let (ed, rd, rd2) = (bearer(&t.editor), bearer(&t.reader), bearer(&t.reader2));
    let (slicer, card_name) = shop(a, &t.editor);

    // the page: /view redirects to /view/; the open-in-Desktop route does not exist on a server
    let r = request(a, "GET", "/w/a/view", &rd, b"");
    assert_eq!(r.status, 308);
    assert_eq!(r.header("location").as_deref(), Some("/w/a/view/"));
    let r1 = page(a, &rd);
    let r2 = page(a, &rd2);
    let e1 = page(a, &ed);
    assert_ne!(r1.vid, r2.vid);
    let r = request(a, "POST", "/w/a/view/api/open-in-desktop", &rd, b"");
    assert_eq!(r.status, 404);

    // everybody starts on the saved file; a reader is served by the clean editor session
    assert_eq!(card(a, &rd, &r1, &card_name), json!(155));
    let snap = state(a, &rd, &r1);
    assert_eq!(snap["view"]["role"], "reader");
    assert_eq!(snap["view"]["session"], "editor");
    assert_eq!(
        snap["source"]["path"], "shop.pbip",
        "workspace-relative paths only"
    );
    assert!(
        snap["source"]["download"]["path"].is_null(),
        "a project downloads file by file"
    );

    // reader 1's slicer changes reader 1's view only (nor the editor's, nor the session's)
    let r = interact(
        a,
        &rd,
        &r1,
        &json!({"action": "slicer_toggle", "page": "Main", "visual": slicer, "value": "Toys"}),
    );
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.json()["result"]["changed"], true);
    assert_eq!(card(a, &rd, &r1, &card_name), json!(5));
    assert_eq!(card(a, &rd2, &r2, &card_name), json!(155));
    assert_eq!(card(a, &ed, &e1, &card_name), json!(155));
    assert_eq!(run(a, &t.editor, "view.interactions", json!({}))["seq"], 0);
    // another identity cannot use reader 1's view
    let r = request(a, "GET", &format!("/w/a/view/api/state?v={}", r1.vid), &rd2, b"");
    assert_eq!(r.status, 404);

    // the editor edits without saving: the editor sees it, readers keep the saved values (from a
    // read-only copy), over MCP too
    run(
        a,
        &t.editor,
        "measure.set",
        json!({"name": "Total", "expression": "SUM(Sales[Amount]) * 2"}),
    );
    assert_eq!(card(a, &ed, &e1, &card_name), json!(310));
    assert_eq!(card(a, &rd2, &r2, &card_name), json!(155));
    assert_eq!(
        card(a, &rd, &r1, &card_name),
        json!(5),
        "reader 1 keeps its slicer"
    );
    assert_eq!(state(a, &rd2, &r2)["view"]["session"], "copy");
    assert_eq!(measure(a, &t.reader, "Total"), 155.0);
    assert_eq!(measure(a, &t.editor, "Total"), 310.0);
    let metrics = server.metrics_text();
    assert!(metrics.contains("bicraft_viewer_copies_open 1"), "{metrics}");
    assert!(metrics.contains("bicraft_viewer_copy_loads_total 1"), "{metrics}");

    // the editor saves: readers see the new values on their next call
    ok(a, &t.editor, "save", json!({}));
    assert_eq!(card(a, &rd2, &r2, &card_name), json!(310));
    assert_eq!(measure(a, &t.reader, "Total"), 310.0);
    assert_eq!(state(a, &rd2, &r2)["view"]["session"], "editor");

    // nothing a viewer did wrote a file, and the editor's history is intact
    let history = run(a, &t.editor, "edit.history", json!({}));
    assert!(history.to_string().contains("measure.set"), "{history}");
}

#[test]
fn a_readers_file_open_is_private_and_keeps_the_editors_history() {
    let t = tokens();
    let dir = workspace("viewer-open");
    std::fs::write(dir.path().join("a").join("other.bim"), MODEL).unwrap();
    let server = start(&dir, config(&t));
    let a = server.addr();
    ok(a, &t.editor, "open_file", json!({"path": "model.bim"}));
    ok(
        a,
        &t.editor,
        "add_measure",
        json!({"table": "Product", "name": "Draft", "expression": "1"}),
    );
    let before = run(a, &t.editor, "edit.history", json!({}));

    // the reader opens another file, then the editor's file: allowed (private), unsaved edits
    // stay invisible to the reader
    let r = ok(a, &t.reader, "open_file", json!({"path": "other.bim"}));
    assert_eq!(r["private"], true, "{r}");
    assert_eq!(r["path"], "other.bim");
    ok(a, &t.reader, "open_file", json!({"path": "model.bim"}));
    let summary = ok(a, &t.reader, "model_summary", json!({})).to_string();
    assert!(
        summary.contains("Avg Price") && !summary.contains("Draft"),
        "{summary}"
    );
    // a reader's open inside a batch is refused (it runs as a call of its own)
    let r = call(
        a,
        &t.reader,
        "batch",
        json!({"commands": [{"command": "file.open", "params": {"path": "model.bim"}}]}),
    );
    assert_eq!(r["isError"], true);
    // a path leaving the workspace is refused like any tool path
    let r = call(a, &t.reader, "open_file", json!({"path": "../x.bim"}));
    assert_eq!(r["isError"], true, "{r}");

    // the editor's session, draft and undo history are untouched
    assert_eq!(run(a, &t.editor, "edit.history", json!({})), before);
    assert!(
        ok(a, &t.editor, "model_summary", json!({}))
            .to_string()
            .contains("Draft")
    );
    run(a, &t.editor, "edit.undo", json!({}));
    assert!(
        !ok(a, &t.editor, "model_summary", json!({}))
            .to_string()
            .contains("Draft")
    );
}

#[test]
fn copies_reload_when_the_saved_file_changes() {
    let t = tokens();
    let dir = workspace("viewer-reload");
    let server = start(&dir, config(&t));
    let a = server.addr();
    ok(a, &t.reader, "open_file", json!({"path": "model.bim"}));
    let probe = r#"EVALUATE ROW("v", 1)"#;
    let _ = probe;
    let has = |name: &str| {
        ok(a, &t.reader, "model_summary", json!({}))
            .to_string()
            .contains(name)
    };
    assert!(!has("Uploaded"));

    // an upload over the file (through the server): the reader's copy is reloaded
    let changed = MODEL.replacen(
        "\"measures\": [",
        "\"measures\": [{\"name\": \"Uploaded\", \"expression\": \"1\"},",
        1,
    );
    assert_ne!(changed, MODEL, "the fixture has a measures list");
    let r = request(
        a,
        "PUT",
        "/w/a/files/model.bim",
        &format!("{}Overwrite: T\r\n", bearer(&t.editor)),
        changed.as_bytes(),
    );
    assert_eq!(r.status, 200, "{}", r.text());
    assert!(has("Uploaded"));

    // an external change (another process wrote the file): the fingerprint differs
    let external = MODEL.replacen(
        "\"measures\": [",
        "\"measures\": [{\"name\": \"External\", \"expression\": \"2\"},",
        1,
    );
    std::thread::sleep(Duration::from_millis(20));
    std::fs::write(dir.path().join("a").join("model.bim"), external).unwrap();
    assert!(has("External") && !has("Uploaded"));
    let metrics = server.metrics_text();
    assert!(metrics.contains("bicraft_viewer_copy_loads_total 3"), "{metrics}");
}

#[test]
fn copies_are_capped_and_evicted() {
    let t = tokens();
    let dir = workspace("viewer-cap");
    std::fs::write(dir.path().join("a").join("other.bim"), MODEL).unwrap();
    let server = start(&dir, config(&t));
    let a = server.addr();
    // two readers on two files with one copy slot: the unused copy makes room
    ok(a, &t.reader, "open_file", json!({"path": "model.bim"}));
    ok(a, &t.reader2, "open_file", json!({"path": "other.bim"}));
    ok(a, &t.reader, "model_summary", json!({}));
    ok(a, &t.reader2, "model_summary", json!({}));
    let metrics = server.metrics_text();
    assert!(metrics.contains("bicraft_viewer_copies_open 1"), "{metrics}");
    assert!(metrics.contains("bicraft_viewer_copy_loads_total 4"), "{metrics}");
}

#[test]
fn browser_login_with_a_one_time_url() {
    let t = tokens();
    let dir = workspace("viewer-login");
    std::fs::create_dir_all(dir.path().join("b")).unwrap();
    let server = start(&dir, config(&t));
    let a = server.addr();
    // without credentials: 401; a login needs a bearer credential
    assert_eq!(request(a, "GET", "/w/a/view/", "", b"").status, 401);
    assert_eq!(request(a, "POST", "/w/a/view/login", "", b"").status, 401);
    let r = request(a, "POST", "/w/a/view/login", &bearer(&t.reader), b"");
    assert_eq!(r.status, 200, "{}", r.text());
    let url = r.json()["url"].as_str().unwrap().to_string();
    let path = url.split_once("127.0.0.1").unwrap().1;
    let path = &path[path.find('/').unwrap()..];
    assert!(path.starts_with("/w/a/view/login/"), "{path}");
    // the code buys a cookie once
    let r = request(a, "GET", path, "", b"");
    assert_eq!(r.status, 303, "{}", r.text());
    assert_eq!(r.header("location").as_deref(), Some("/w/a/view/"));
    let set = r.header("set-cookie").unwrap();
    assert!(
        set.contains("HttpOnly") && set.contains("SameSite=Lax") && set.contains("Path=/w/a/"),
        "{set}"
    );
    assert!(!set.contains("Secure"), "plain http test server");
    let sid = set.split(';').next().unwrap().to_string();
    assert_eq!(request(a, "GET", path, "", b"").status, 404, "single use");
    let cookie = format!("Cookie: {sid}\r\n");
    let p = page(a, &cookie);
    assert_eq!(state(a, &cookie, &p)["view"]["role"], "reader");
    // the cookie reads files (the download link), never writes them
    assert_eq!(
        request(a, "GET", "/w/a/files/model.bim", &cookie, b"").status,
        200
    );
    assert_eq!(request(a, "PUT", "/w/a/files/x.txt", &cookie, b"x").status, 401);
    assert_eq!(
        request(a, "DELETE", "/w/a/files/model.bim", &cookie, b"").status,
        401
    );
    // nor the MCP endpoint, nor another workspace
    let body = b"{}";
    assert_eq!(
        request(
            a,
            "POST",
            "/w/a/mcp",
            &format!("{cookie}Content-Type: application/json\r\n"),
            body
        )
        .status,
        401
    );
    assert_eq!(request(a, "GET", "/w/b/view/", &cookie, b"").status, 401);
    // a forged or malformed cookie
    let forged = format!("Cookie: bicraft_view={}\r\n", "a".repeat(64));
    assert_eq!(request(a, "GET", "/w/a/view/", &forged, b"").status, 401);
    assert_eq!(
        request(a, "GET", "/w/a/view/", "Cookie: bicraft_view=x\r\n", b"").status,
        401
    );
    // a stale or foreign code
    assert_eq!(
        request(a, "GET", &format!("/w/a/view/login/{}", "0".repeat(64)), "", b"").status,
        404
    );
    let r = request(a, "POST", "/w/b/view/login", &bearer(&t.reader), b"");
    assert_eq!(r.status, 404, "no grant on b");
}

#[test]
fn csrf_and_origin_checks_on_interactions() {
    let t = tokens();
    let dir = workspace("viewer-csrf");
    let server = start(&dir, config(&t));
    let a = server.addr();
    let rd = bearer(&t.reader);
    shop(a, &t.editor);
    let p = page(a, &rd);
    let path = format!("/w/a/view/api/interact?v={}", p.vid);
    let body = br#"{"action":"reset"}"#;
    for (why, headers) in [
        ("no token", format!("{rd}Content-Type: application/json\r\n")),
        (
            "wrong token",
            format!(
                "{rd}X-Bicraft-Token: {}\r\nContent-Type: application/json\r\n",
                "0".repeat(32)
            ),
        ),
        (
            "foreign origin",
            format!(
                "{rd}Origin: http://evil.example\r\nX-Bicraft-Token: {}\r\nContent-Type: application/json\r\n",
                p.token
            ),
        ),
        (
            "cross-site",
            format!(
                "{rd}Sec-Fetch-Site: cross-site\r\nX-Bicraft-Token: {}\r\nContent-Type: application/json\r\n",
                p.token
            ),
        ),
    ] {
        let r = request(a, "POST", &path, &headers, body);
        assert_eq!(r.status, 403, "{why}: {}", r.text());
    }
    let ok_headers = format!(
        "{rd}Origin: http://127.0.0.1\r\nSec-Fetch-Site: same-origin\r\nX-Bicraft-Token: {}\r\n\
         Content-Type: application/json\r\n",
        p.token
    );
    assert_eq!(request(a, "POST", &path, &ok_headers, body).status, 200);
    // body checks
    let r = request(
        a,
        "POST",
        &path,
        &ok_headers.replace("application/json", "text/plain"),
        body,
    );
    assert_eq!(r.status, 415);
    assert_eq!(request(a, "POST", &path, &ok_headers, b"[1]").status, 400);
    let big = vec![b' '; 70 * 1024];
    assert_eq!(request(a, "POST", &path, &ok_headers, &big).status, 413);
    // a bad action is the engine's error, the view unchanged
    let r = interact(a, &rd, &p, &json!({"action": "nope"}));
    assert_eq!(r.status, 400);
    // GET only on the read routes, POST only on interact
    assert_eq!(request(a, "POST", "/w/a/view/api/state", &rd, b"").status, 405);
    assert_eq!(request(a, "GET", &path, &rd, b"").status, 405);
    // a Host outside the allow-list
    let r = send(
        a,
        format!("GET /w/a/view/ HTTP/1.1\r\nHost: evil.example\r\n{rd}\r\n").as_bytes(),
    );
    assert_eq!(r.status, 403);
}

#[test]
fn trusted_header_and_no_authentication() {
    let t = tokens();
    let dir = workspace("viewer-proxy");
    let server = start(
        &dir,
        HttpConfig {
            auth: Some(Arc::new(Authenticator::new(auth(&t, true)).unwrap())),
            // a gateway in front rewrites Host: the public origin is an allowed origin
            allowed_origins: vec!["https://gw.example".into()],
            ..HttpConfig::default()
        },
    );
    let a = server.addr();
    let ana = "X-Forwarded-User: ana\r\n";
    let p = page(a, ana);
    assert_eq!(state(a, ana, &p)["view"]["role"], "reader");
    let body = br#"{"action":"reset"}"#;
    for (origin, expected) in [("https://gw.example", 200), ("https://evil.example", 403)] {
        let headers = format!(
            "{ana}Origin: {origin}\r\nX-Bicraft-Token: {}\r\nContent-Type: application/json\r\n",
            p.token
        );
        let path = format!("/w/a/view/api/interact?v={}", p.vid);
        assert_eq!(
            request(a, "POST", &path, &headers, body).status,
            expected,
            "{origin}"
        );
    }
    // someone without a grant: 404 like a missing workspace
    assert_eq!(
        request(a, "GET", "/w/a/view/", "X-Forwarded-User: bob\r\n", b"").status,
        404
    );
    drop(server);

    // a loopback server without authentication: every caller is an editor
    let server = start(&dir, HttpConfig::default());
    let a = server.addr();
    let p = page(a, "");
    let snap = state(a, "", &p);
    assert_eq!(snap["view"]["role"], "editor");
    // no login on a server without authentication
    assert_eq!(request(a, "POST", "/w/a/view/login", "", b"").status, 404);
    // unknown routes and workspaces
    assert_eq!(request(a, "GET", "/w/a/view/nope", "", b"").status, 404);
    assert_eq!(request(a, "GET", "/w/zz/view/", "", b"").status, 404);
}

#[test]
fn events_logs_and_metrics_carry_no_model_data() {
    let t = tokens();
    let dir = workspace("viewer-obs");
    let lines: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink_lines = Arc::clone(&lines);
    let sink = LogSink(Arc::new(move |l: &str| {
        sink_lines.lock().unwrap().push(l.to_string())
    }));
    let mut c = config(&t);
    c.log_format = LogFormat::Json;
    c.log_sink = Some(sink);
    let server = start(&dir, c);
    let a = server.addr();
    let rd = bearer(&t.reader);
    let (slicer, card_name) = shop(a, &t.editor);
    let p = page(a, &rd);
    assert_eq!(card(a, &rd, &p, &card_name), json!(155));
    // the event stream sends a first event at once
    let mut s = TcpStream::connect(a).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    s.write_all(
        format!(
            "GET /w/a/view/api/events?v={} HTTP/1.1\r\nHost: 127.0.0.1\r\n{rd}\r\n",
            p.vid
        )
        .as_bytes(),
    )
    .unwrap();
    let mut got = Vec::new();
    let mut buf = [0u8; 4096];
    while !String::from_utf8_lossy(&got).contains("event: snapshot") {
        let n = s.read(&mut buf).unwrap();
        assert!(n > 0, "stream closed");
        got.extend_from_slice(&buf[..n]);
    }
    assert!(String::from_utf8_lossy(&got).contains("text/event-stream"));
    drop(s);
    let r = interact(
        a,
        &rd,
        &p,
        &json!({"action": "slicer_toggle", "page": "Main", "visual": slicer, "value": "Toys"}),
    );
    assert_eq!(r.status, 200);
    let metrics = server.metrics_text();
    assert!(metrics.contains(r#"route="view""#), "{metrics}");
    assert!(
        metrics.contains("bicraft_viewer_interactions_total 1"),
        "{metrics}"
    );
    assert!(metrics.contains("bicraft_viewer_pages_total 1"), "{metrics}");
    std::thread::sleep(Duration::from_millis(200));
    let lines = lines.lock().unwrap().clone();
    let views: Vec<&String> = lines.iter().filter(|l| l.contains(r#""route":"view""#)).collect();
    assert!(
        views.iter().any(|l| l.contains(r#""view":"interact""#)),
        "{lines:?}"
    );
    assert!(views.iter().any(|l| l.contains(r#""view":"state""#)), "{lines:?}");
    for l in &lines {
        for secret in ["Toys", "Wheels", "Total", "shop.pbip", "Amount", &t.reader] {
            assert!(!l.contains(secret), "{secret} in {l}");
        }
    }
}

#[test]
fn the_viewer_can_be_turned_off() {
    let dir = workspace("viewer-off");
    let server = start(
        &dir,
        HttpConfig {
            viewer: false,
            ..HttpConfig::default()
        },
    );
    assert_eq!(request(server.addr(), "GET", "/w/a/view/", "", b"").status, 404);
}

/// `POST …/view/api/session`: a native client's view (bicraft-desktop), with the same checks as
/// the page, bearer credentials only.
fn session(addr: SocketAddr, auth: &str) -> Response {
    request(addr, "POST", "/w/a/view/api/session", auth, b"")
}

#[test]
fn native_clients_get_a_view_without_the_page() {
    let t = tokens();
    let dir = workspace("viewer-session");
    let server = start(&dir, config(&t));
    let a = server.addr();
    let (ed, rd) = (bearer(&t.editor), bearer(&t.reader));
    let (slicer, card_name) = shop(a, &t.editor);

    // no credentials, a wrong token, GET: refused; another workspace looks missing
    assert_eq!(session(a, "").status, 401);
    assert_eq!(session(a, &bearer(&"x".repeat(43))).status, 401);
    assert_eq!(request(a, "GET", "/w/a/view/api/session", &rd, b"").status, 405);
    assert_eq!(request(a, "POST", "/w/b/view/api/session", &rd, b"").status, 404);

    // a reader and an editor each get a view, their role and an interaction token
    let r = session(a, &rd);
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.header("cache-control").as_deref(), Some("no-store"));
    let j = r.json();
    assert_eq!(j["version"], 1);
    assert_eq!(j["access"], "reader");
    let rv = Page {
        vid: j["view"].as_str().unwrap().to_string(),
        token: j["token"].as_str().unwrap().to_string(),
    };
    assert_eq!(rv.vid.len(), 32);
    assert_eq!(session(a, &ed).json()["access"], "editor");

    // the view works like a page's: state, a slicer click on this view only
    assert_eq!(card(a, &rd, &rv, &card_name), json!(155));
    let r = interact(
        a,
        &rd,
        &rv,
        &json!({"action": "slicer_toggle", "page": "Main", "visual": slicer, "value": "Toys"}),
    );
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(card(a, &rd, &rv, &card_name), json!(5));
    // a native client sends no Origin: the token alone is checked
    let body = json!({"action": "reset"}).to_string();
    let headers = format!(
        "{rd}X-Bicraft-Token: {}\r\nContent-Type: application/json\r\n",
        rv.token
    );
    let r = request(
        a,
        "POST",
        &format!("/w/a/view/api/interact?v={}", rv.vid),
        &headers,
        body.as_bytes(),
    );
    assert_eq!(r.status, 200, "{}", r.text());
    let headers = format!(
        "{rd}X-Bicraft-Token: {}\r\nContent-Type: application/json\r\n",
        "0".repeat(32)
    );
    let r = request(
        a,
        "POST",
        &format!("/w/a/view/api/interact?v={}", rv.vid),
        &headers,
        body.as_bytes(),
    );
    assert_eq!(r.status, 403);

    // the browser cookie cannot mint a view (a cross-site form would)
    let r = request(a, "POST", "/w/a/view/login", &rd, b"");
    let url = r.json()["url"].as_str().unwrap().to_string();
    let path = url.split_once("127.0.0.1").unwrap().1;
    let path = &path[path.find('/').unwrap()..];
    let set = request(a, "GET", path, "", b"").header("set-cookie").unwrap();
    let cookie = format!("Cookie: {}\r\n", set.split(';').next().unwrap());
    assert_eq!(session(a, &cookie).status, 401);
}

/// `…/view/app/`: the browser build of the egui app (docs/webui.md), with the page's checks and
/// its own private view; its files under `app/`. In a build without the app (no
/// `BICRAFT_WEBUI_DIR`), the routes answer 404 and say so.
#[test]
fn the_browser_app_gets_its_own_view() {
    let t = tokens();
    let dir = workspace("viewer-app");
    let server = start(&dir, config(&t));
    let a = server.addr();
    let rd = bearer(&t.reader);
    let (_slicer, card_name) = shop(a, &t.editor);

    // the page's checks: credentials, the workspace ACL, GET only
    assert_eq!(request(a, "GET", "/w/a/view/app/", "", b"").status, 401);
    assert_eq!(request(a, "GET", "/w/b/view/app/", &rd, b"").status, 404);
    let r = request(a, "GET", "/w/a/view/app", &rd, b"");
    assert_eq!(r.status, 308);
    assert_eq!(r.header("location").as_deref(), Some("/w/a/view/app/"));

    let r = request(a, "GET", "/w/a/view/app/", &rd, b"");
    if !bicraft_view::webui::available() {
        assert_eq!(r.status, 404);
        assert!(r.text().contains("without the browser app"), "{}", r.text());
        assert_eq!(
            request(a, "GET", "/w/a/view/app/bicraft_web_bg.wasm", &rd, b"").status,
            404
        );
        return;
    }
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(
        r.header("content-security-policy").as_deref(),
        Some(bicraft_view::webui::APP_CONTENT_SECURITY_POLICY)
    );
    assert_eq!(r.header("cache-control").as_deref(), Some("no-store"));
    let html = r.text();
    let p = Page {
        vid: meta(&html, "bicraft-view-id"),
        token: meta(&html, "bicraft-interact-token"),
    };
    assert_eq!((p.vid.len(), p.token.len()), (32, 32));
    // never the open-in-Desktop token on a server
    assert!(html.contains(r#"<meta name="bicraft-open-token" content="">"#));
    // the view works as the HTML page's does (the app sends the same requests)
    assert_eq!(card(a, &rd, &p, &card_name), json!(155));
    let r = interact(a, &rd, &p, &json!({"action": "reset"}));
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(request(a, "POST", "/w/a/view/app/", &rd, b"").status, 405);

    // the files: gzip when accepted, revalidated by entity tag, inflated otherwise
    let boot = request(a, "GET", "/w/a/view/app/boot.js", &rd, b"");
    assert_eq!(boot.status, 200);
    assert!(
        boot.header("content-type")
            .unwrap()
            .starts_with("text/javascript")
    );
    let gz = request(
        a,
        "GET",
        "/w/a/view/app/bicraft_web_bg.wasm",
        &format!("{rd}Accept-Encoding: gzip, br\r\n"),
        b"",
    );
    assert_eq!(gz.status, 200);
    assert_eq!(gz.header("content-type").as_deref(), Some("application/wasm"));
    assert_eq!(gz.header("content-encoding").as_deref(), Some("gzip"));
    assert_eq!(gz.header("cache-control").as_deref(), Some("no-cache"));
    let etag = gz.header("etag").unwrap();
    let again = request(
        a,
        "GET",
        "/w/a/view/app/bicraft_web_bg.wasm",
        &format!("{rd}Accept-Encoding: gzip\r\nIf-None-Match: {etag}\r\n"),
        b"",
    );
    assert_eq!(again.status, 304);
    let raw = request(a, "GET", "/w/a/view/app/bicraft_web_bg.wasm", &rd, b"");
    assert!(raw.header("content-encoding").is_none());
    assert!(raw.body.starts_with(b"\0asm"));
    let js = request(a, "GET", "/w/a/view/app/bicraft_web.js", &rd, b"");
    assert_eq!(js.status, 200);
    assert_ne!(request(a, "GET", "/w/a/view/app/../mcp", &rd, b"").status, 200);
    assert_eq!(request(a, "GET", "/w/a/view/app/other.js", &rd, b"").status, 404);
}
