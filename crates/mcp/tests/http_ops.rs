//! Operations of `bicraft-cli serve` over real sockets (docs/web-deployment.md, "Operating
//! `bicraft-cli serve`"): file upload, download, listing and deletion per workspace (streamed, capped,
//! sandboxed, no overwrite by default, plain files only), zipped projects (zip-slip, links, bombs),
//! authentication and ACL on the file routes, uploads against an open session, the probes, the
//! graceful drain, and the idle close with lazy reopen.

#[path = "../../../testsupport/tempdir.rs"]
mod tempdir;

use std::io::{Cursor, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bicraft_auth::tokens::{self, entry_json};
use bicraft_auth::{AuthConfig, Authenticator, Grant};
use bicraft_mcp::http::{HTTP_PROTOCOL_VERSION, SHUTTING_DOWN, UNAUTHORIZED};
use bicraft_mcp::{HttpConfig, HttpServer, ZipLimits};
use serde_json::{Value, json};
use tempdir::TempDir;
use zip::write::SimpleFileOptions;

const MODEL: &str = include_str!("../../engine/tests/table_functions/model.bim");
/// Minutes of evaluation without a budget: only a cancel or a timeout ends it.
const LONG: &str =
    "EVALUATE ROW(\"x\", SUMX(GENERATESERIES(1, 100000), SUMX(GENERATESERIES(1, 100000), [Value])))";

struct Fixture {
    dir: TempDir,
    server: Option<HttpServer>,
    addr: SocketAddr,
}

fn fixture(name: &str, config: HttpConfig) -> Fixture {
    let dir = TempDir::new(name);
    for ws in ["a", "b"] {
        std::fs::create_dir_all(dir.path().join(ws)).unwrap();
    }
    let server = HttpServer::start("127.0.0.1:0".parse().unwrap(), dir.path(), config).unwrap();
    let addr = server.addr();
    Fixture {
        dir,
        server: Some(server),
        addr,
    }
}

impl Fixture {
    fn ws(&self, ws: &str) -> std::path::PathBuf {
        self.dir.path().join(ws)
    }
    fn server(&self) -> &HttpServer {
        self.server.as_ref().unwrap()
    }
    fn take(&mut self) -> HttpServer {
        self.server.take().unwrap()
    }
}

#[derive(Debug)]
struct Response {
    status: u16,
    head: String,
    body: Vec<u8>,
}

impl Response {
    fn header(&self, name: &str) -> Option<&str> {
        self.head.lines().skip(1).find_map(|l| {
            let (n, v) = l.split_once(':')?;
            n.trim().eq_ignore_ascii_case(name).then(|| v.trim())
        })
    }
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|e| panic!("{e}: {}", self.text()))
    }
    /// The JSON-RPC message of a reply (plain JSON or the one SSE event).
    fn message(&self) -> Value {
        let t = self.text();
        match t.lines().find_map(|l| l.strip_prefix("data: ")) {
            Some(d) => serde_json::from_str(d).unwrap(),
            None => self.json(),
        }
    }
}

fn read_response(mut s: TcpStream) -> Response {
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    let split = out.windows(4).position(|w| w == b"\r\n\r\n").unwrap_or(out.len());
    let head = String::from_utf8_lossy(&out[..split]).into_owned();
    let body = out.get(split + 4..).unwrap_or_default().to_vec();
    Response {
        status: head.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0),
        head,
        body,
    }
}

fn connect(addr: SocketAddr) -> TcpStream {
    let s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
    s
}

/// A request with a body; `headers` are extra lines (`Name: value`).
fn request(addr: SocketAddr, method: &str, path: &str, headers: &[&str], body: &[u8]) -> Response {
    let mut s = connect(addr);
    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\n",
        body.len()
    );
    for h in headers {
        req.push_str(h);
        req.push_str("\r\n");
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes()).unwrap();
    let _ = s.write_all(body);
    read_response(s)
}

fn get(addr: SocketAddr, path: &str, headers: &[&str]) -> Response {
    let mut s = connect(addr);
    let mut req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n");
    for h in headers {
        req.push_str(h);
        req.push_str("\r\n");
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes()).unwrap();
    read_response(s)
}

fn put(addr: SocketAddr, path: &str, headers: &[&str], body: &[u8]) -> Response {
    request(addr, "PUT", path, headers, body)
}

fn call_bytes(ws: &str, tool: &str, args: Value) -> Vec<u8> {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {
        "name": tool, "arguments": args, "_meta": {
            "io.modelcontextprotocol/protocolVersion": HTTP_PROTOCOL_VERSION,
            "io.modelcontextprotocol/clientCapabilities": {}}}})
    .to_string();
    format!(
        "POST /w/{ws}/mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
         Accept: application/json\r\nMCP-Protocol-Version: {HTTP_PROTOCOL_VERSION}\r\n\
         Mcp-Method: tools/call\r\nMcp-Name: {tool}\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

/// Calls a tool; the tool's result object.
fn call(addr: SocketAddr, ws: &str, tool: &str, args: Value) -> Value {
    let mut s = connect(addr);
    s.write_all(&call_bytes(ws, tool, args)).unwrap();
    let r = read_response(s);
    assert_eq!(r.status, 200, "{}", r.text());
    r.message()["result"].clone()
}

fn tool_text(result: &Value) -> String {
    result["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in entries {
        w.start_file(*name, SimpleFileOptions::default()).unwrap();
        w.write_all(data).unwrap();
    }
    w.finish().unwrap().into_inner()
}

/// Makes `link` point to the folder `target`; false when this OS or account cannot.
fn link_dir(target: &Path, link: &Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).is_ok()
    }
    #[cfg(windows)]
    {
        if std::os::windows::fs::symlink_dir(target, link).is_ok() {
            return true;
        }
        std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .is_ok_and(|o| o.status.success())
            && link.exists()
    }
}

/// Every file under `dir` (relative, `/`-separated), sorted.
fn files_under(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                let rel = p.strip_prefix(dir).unwrap().to_string_lossy().replace('\\', "/");
                out.push(rel);
            }
        }
    }
    out.sort();
    out
}

// ---------------------------------------------------------------------------------------------
// Files

#[test]
fn upload_download_list_and_delete() {
    let f = fixture("ops-files", HttpConfig::default());
    let a = f.addr;
    // a 5 MiB upload, streamed in pieces
    let big: Vec<u8> = (0..5 * 1024 * 1024u32).map(|i| (i % 251) as u8).collect();
    let mut s = connect(a);
    write!(
        s,
        "PUT /w/a/files/data/big.pbix HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\n\r\n",
        big.len()
    )
    .unwrap();
    for chunk in big.chunks(700_000) {
        s.write_all(chunk).unwrap();
        std::thread::sleep(Duration::from_millis(5));
    }
    let r = read_response(s);
    assert_eq!(r.status, 201, "{}", r.text());
    assert_eq!(r.json()["size"], big.len());
    assert_eq!(r.json()["replaced"], false);
    assert_eq!(std::fs::read(f.ws("a").join("data/big.pbix")).unwrap(), big);
    // no temp file left behind, nothing in the other workspace
    assert_eq!(files_under(&f.ws("a")), ["data/big.pbix"]);
    assert!(files_under(&f.ws("b")).is_empty());

    // download: bytes, type, length, disposition
    let r = get(a, "/w/a/files/data/big.pbix", &[]);
    assert_eq!(r.status, 200);
    assert_eq!(r.body, big);
    assert_eq!(r.header("Content-Type"), Some("application/octet-stream"));
    assert_eq!(r.header("Content-Length"), Some(big.len().to_string().as_str()));
    assert!(
        r.header("Content-Disposition")
            .unwrap()
            .contains("filename=\"big.pbix\"")
    );
    // HEAD: the same head, no body
    let mut s = connect(a);
    s.write_all(b"HEAD /w/a/files/data/big.pbix HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
        .unwrap();
    let r = read_response(s);
    assert_eq!(r.status, 200);
    assert!(r.body.is_empty());

    // overwrite rules: refused by default (412, before the body), replaced with Overwrite: T
    let r = put(a, "/w/a/files/model.bim", &[], MODEL.as_bytes());
    assert_eq!(r.status, 201, "{}", r.text());
    let r = put(a, "/w/a/files/model.bim", &[], b"{}");
    assert_eq!(r.status, 412, "{}", r.text());
    assert_eq!(
        std::fs::read_to_string(f.ws("a").join("model.bim")).unwrap(),
        MODEL
    );
    let r = put(a, "/w/a/files/model.bim", &["Overwrite: F"], b"{}");
    assert_eq!(r.status, 412);
    let r = put(a, "/w/a/files/model.bim", &["Overwrite: maybe"], b"{}");
    assert_eq!(r.status, 400);
    let r = put(a, "/w/a/files/model.bim", &["Overwrite: T"], b"{\"x\":1}");
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.json()["replaced"], true);
    assert_eq!(
        std::fs::read_to_string(f.ws("a").join("model.bim")).unwrap(),
        "{\"x\":1}"
    );
    // a folder in the way, a trailing slash, Content-Length missing
    let r = put(a, "/w/a/files/data", &["Overwrite: T"], b"x");
    assert_eq!(r.status, 409, "{}", r.text());
    let r = put(a, "/w/a/files/data/", &[], b"x");
    assert_eq!(r.status, 400);
    let mut s = connect(a);
    s.write_all(b"PUT /w/a/files/x.txt HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
        .unwrap();
    assert_eq!(read_response(s).status, 411);
    // an encoded name
    let r = put(a, "/w/a/files/Ventes%20%C3%A9t%C3%A9.txt", &[], b"hi");
    assert_eq!(r.status, 201, "{}", r.text());
    assert!(f.ws("a").join("Ventes été.txt").is_file());
    let r = get(a, "/w/a/files/Ventes%20%C3%A9t%C3%A9.txt", &[]);
    assert_eq!(r.body, b"hi");
    assert!(
        r.header("Content-Disposition")
            .unwrap()
            .contains("filename*=UTF-8''Ventes%20%C3%A9t%C3%A9.txt")
    );

    // listing: recursive, sizes, mtimes, sorted; a sub-folder alone
    let r = get(a, "/w/a/files/", &[]);
    assert_eq!(r.status, 200, "{}", r.text());
    let list = r.json();
    let paths: Vec<&str> = list["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, ["Ventes été.txt", "data", "data/big.pbix", "model.bim"]);
    let big_entry = &list["entries"][2];
    assert_eq!(big_entry["type"], "file");
    assert_eq!(big_entry["size"], big.len());
    assert!(big_entry["modified"].as_str().unwrap().ends_with('Z'));
    assert_eq!(list["truncated"], false);
    let r = get(a, "/w/a/files/data", &[]);
    assert_eq!(r.json()["entries"].as_array().unwrap().len(), 1);
    assert_eq!(get(a, "/w/a/files", &[]).status, 200);

    // delete: a file, a non-empty folder (409, then Depth: infinity), then gone
    let r = request(a, "DELETE", "/w/a/files/model.bim", &[], b"");
    assert_eq!(r.status, 204, "{}", r.text());
    assert!(r.header("Content-Length").is_none());
    assert_eq!(get(a, "/w/a/files/model.bim", &[]).status, 404);
    let r = request(a, "DELETE", "/w/a/files/data", &[], b"");
    assert_eq!(r.status, 409, "{}", r.text());
    let r = request(a, "DELETE", "/w/a/files/data", &["Depth: infinity"], b"");
    assert_eq!(r.status, 204, "{}", r.text());
    assert!(!f.ws("a").join("data").exists());
    assert_eq!(request(a, "DELETE", "/w/a/files/", &[], b"").status, 405);
    assert_eq!(request(a, "DELETE", "/w/a/files/nothing", &[], b"").status, 404);
    assert_eq!(request(a, "PATCH", "/w/a/files/x", &[], b"").status, 405);
    // unknown workspace
    assert_eq!(get(a, "/w/zz/files/", &[]).status, 404);
}

#[test]
fn hostile_paths_are_refused() {
    let f = fixture("ops-paths", HttpConfig::default());
    let a = f.addr;
    let outside = TempDir::new("ops-paths-outside");
    std::fs::write(outside.path().join("secret.txt"), "secret").unwrap();
    std::fs::write(f.ws("b").join("other.txt"), "b's").unwrap();
    for path in [
        "/w/a/files/../b/other.txt",
        "/w/a/files/%2e%2e/b/other.txt",
        "/w/a/files/x/%2E%2E/%2E%2E/b/other.txt",
        "/w/a/files//etc/passwd",
        "/w/a/files/%2Fetc%2Fpasswd",
        "/w/a/files/C:/Windows/win.ini",
        "/w/a/files/%5C%5Cserver%5Cshare%5Cx",
        "/w/a/files/a%00b",
        "/w/a/files/%zz",
    ] {
        let r = get(a, path, &[]);
        assert!(
            matches!(r.status, 400 | 403 | 404),
            "{path}: {} {}",
            r.status,
            r.text()
        );
        assert!(!r.text().contains("b's"), "{path}");
        let r = put(a, path, &["Overwrite: T"], b"pwned");
        assert!(
            matches!(r.status, 400 | 403 | 404),
            "{path}: {} {}",
            r.status,
            r.text()
        );
    }
    assert_eq!(
        std::fs::read_to_string(f.ws("b").join("other.txt")).unwrap(),
        "b's"
    );
    // the reserved temp folder is invisible
    std::fs::create_dir_all(f.ws("a").join(".bicraft-tmp")).unwrap();
    std::fs::write(f.ws("a").join(".bicraft-tmp").join("x.part"), "t").unwrap();
    assert_eq!(get(a, "/w/a/files/.bicraft-tmp/x.part", &[]).status, 404);
    assert_eq!(put(a, "/w/a/files/.bicraft-tmp/y", &[], b"y").status, 404);
    assert!(
        get(a, "/w/a/files/", &[]).json()["entries"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    // a link inside the workspace pointing outside: never read, written or listed through
    if link_dir(outside.path(), &f.ws("a").join("out")) {
        let r = get(a, "/w/a/files/out/secret.txt", &[]);
        assert_eq!(r.status, 403, "{}", r.text());
        let r = put(a, "/w/a/files/out/new.txt", &[], b"x");
        assert_eq!(r.status, 403, "{}", r.text());
        assert!(!outside.path().join("new.txt").exists());
        let list = get(a, "/w/a/files/", &[]).json();
        assert!(!list.to_string().contains("secret"), "{list}");
        let r = request(a, "DELETE", "/w/a/files/out", &["Depth: infinity"], b"");
        assert_ne!(r.status, 204);
        assert!(outside.path().join("secret.txt").exists());
    } else {
        eprintln!("note: links cannot be created here; the link part is skipped");
    }
}

#[test]
fn upload_size_cap() {
    let config = HttpConfig {
        max_upload_bytes: 1000,
        ..HttpConfig::default()
    };
    let f = fixture("ops-cap", config);
    // refused from the head alone, before any body byte
    let mut s = connect(f.addr);
    s.write_all(b"PUT /w/a/files/big.bin HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 1001\r\n\r\n")
        .unwrap();
    let r = read_response(s);
    assert_eq!(r.status, 413, "{}", r.text());
    assert!(files_under(&f.ws("a")).is_empty());
    assert_eq!(put(f.addr, "/w/a/files/ok.bin", &[], &[7u8; 1000]).status, 201);
    // a body cut short leaves nothing behind
    let mut s = connect(f.addr);
    s.write_all(b"PUT /w/a/files/cut.bin HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 900\r\n\r\nshort")
        .unwrap();
    s.shutdown(std::net::Shutdown::Write).unwrap();
    let r = read_response(s);
    assert_eq!(r.status, 400, "{}", r.text());
    assert_eq!(files_under(&f.ws("a")), ["ok.bin"]);
}

#[test]
fn zipped_projects_are_extracted_safely() {
    let config = HttpConfig {
        zip_limits: ZipLimits {
            max_ratio: 100,
            ..ZipLimits::default()
        },
        ..HttpConfig::default()
    };
    let f = fixture("ops-zip", config);
    let a = f.addr;
    let zip_type = "Content-Type: application/zip";
    let project = zip_of(&[
        ("Sales/model.bim", MODEL.as_bytes()),
        ("Sales/notes/readme.txt", b"hello"),
    ]);
    let r = request(a, "POST", "/w/a/files/proj", &[zip_type], &project);
    assert_eq!(r.status, 201, "{}", r.text());
    assert_eq!(r.json()["files"], 2);
    assert_eq!(
        files_under(&f.ws("a")),
        ["proj/Sales/model.bim", "proj/Sales/notes/readme.txt"]
    );
    // the extracted model opens over MCP
    let opened = call(a, "a", "open_file", json!({"path": "proj/Sales/model.bim"}));
    assert_eq!(opened["isError"], false, "{opened}");
    // again: the files exist (412) unless Overwrite: T
    let r = request(a, "POST", "/w/a/files/proj", &[zip_type], &project);
    assert_eq!(r.status, 412, "{}", r.text());
    // into the workspace root
    let r = request(a, "POST", "/w/b/files/", &[zip_type], &zip_of(&[("x.txt", b"x")]));
    assert_eq!(r.status, 201, "{}", r.text());
    assert_eq!(files_under(&f.ws("b")), ["x.txt"]);

    let before = files_under(&f.ws("a"));
    // zip-slip, absolute names, the reserved folder: refused, nothing written anywhere
    for evil in [
        "../evil.txt",
        "a/../../evil.txt",
        "/abs.txt",
        "C:/x.txt",
        "ok/..\\..\\evil.txt",
    ] {
        let z = zip_of(&[("fine.txt", b"fine"), (evil, b"evil")]);
        let r = request(a, "POST", "/w/a/files/slip", &[zip_type], &z);
        assert_eq!(r.status, 400, "{evil}: {}", r.text());
    }
    let r = request(
        a,
        "POST",
        "/w/a/files/",
        &[zip_type],
        &zip_of(&[(".bicraft-tmp/x", b"x")]),
    );
    assert_eq!(r.status, 400, "{}", r.text());
    assert!(!f.dir.path().join("evil.txt").exists());
    assert!(!f.ws("a").join("slip").exists());
    // symbolic link entries
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    w.add_symlink("link", "/etc/passwd", SimpleFileOptions::default())
        .unwrap();
    let z = w.finish().unwrap().into_inner();
    let r = request(a, "POST", "/w/a/files/links", &[zip_type], &z);
    assert_eq!(r.status, 400, "{}", r.text());
    assert!(r.text().contains("symbolic link"), "{}", r.text());
    // a ZIP bomb: stops at the ratio cap
    let zeros = vec![0u8; 4 * 1024 * 1024];
    let r = request(
        a,
        "POST",
        "/w/a/files/bomb",
        &[zip_type],
        &zip_of(&[("zeros.bin", &zeros)]),
    );
    assert_eq!(r.status, 413, "{}", r.text());
    // not an archive, or not declared as one
    assert_eq!(
        request(a, "POST", "/w/a/files/x", &[zip_type], b"not a zip").status,
        400
    );
    assert_eq!(request(a, "POST", "/w/a/files/x", &[], &project).status, 415);
    assert_eq!(files_under(&f.ws("a")), before);
}

fn token_config(token: &str, acl: &[&str]) -> AuthConfig {
    let mut c = AuthConfig {
        acl: bicraft_auth::Acl {
            grants: acl.iter().map(|g| Grant::parse_text(g).unwrap()).collect(),
        },
        ..AuthConfig::default()
    };
    let doc = json!([entry_json("ci", &tokens::hash_token(token).unwrap(), &[], &[])]);
    c.add_tokens(tokens::parse(&doc).unwrap()).unwrap();
    c
}

#[test]
fn file_routes_are_authenticated_and_acl_checked() {
    let token = tokens::generate().unwrap();
    let config = HttpConfig {
        auth: Some(Arc::new(
            Authenticator::new(token_config(&token, &["token:ci=a"])).unwrap(),
        )),
        ..HttpConfig::default()
    };
    let f = fixture("ops-auth", config);
    let a = f.addr;
    std::fs::write(f.ws("b").join("b.txt"), "b").unwrap();
    let bearer = format!("Authorization: Bearer {token}");
    // no credentials: 401 with a challenge, on every method, before anything is written
    for (method, path) in [
        ("GET", "/w/a/files/"),
        ("PUT", "/w/a/files/x.txt"),
        ("DELETE", "/w/a/files/x.txt"),
        ("POST", "/w/a/files/"),
    ] {
        let r = request(a, method, path, &[], b"x");
        assert_eq!(r.status, 401, "{method}: {}", r.text());
        assert!(r.header("WWW-Authenticate").is_some());
        assert_eq!(r.json()["error"]["code"], UNAUTHORIZED);
    }
    let r = request(a, "GET", "/w/a/files/", &["Authorization: Bearer wrong"], b"");
    assert_eq!(r.status, 401);
    assert!(files_under(&f.ws("a")).is_empty());
    // granted workspace
    assert_eq!(put(a, "/w/a/files/x.txt", &[&bearer], b"x").status, 201);
    assert_eq!(get(a, "/w/a/files/x.txt", &[&bearer]).body, b"x");
    // a workspace outside the grant looks missing, like an unknown one
    let closed = get(a, "/w/b/files/b.txt", &[&bearer]);
    let missing = get(a, "/w/nope/files/b.txt", &[&bearer]);
    assert_eq!(closed.status, 404);
    assert_eq!(closed.body, missing.body);
    assert_eq!(put(a, "/w/b/files/y.txt", &[&bearer], b"y").status, 404);
    assert!(!f.ws("b").join("y.txt").exists());
    // a disallowed browser origin
    assert_eq!(
        get(a, "/w/a/files/", &[&bearer, "Origin: https://evil.example"]).status,
        403
    );
}

#[test]
fn uploads_respect_the_open_session() {
    let f = fixture("ops-session", HttpConfig::default());
    let a = f.addr;
    std::fs::write(f.ws("a").join("model.bim"), MODEL).unwrap();
    let opened = call(a, "a", "open_file", json!({"path": "model.bim"}));
    assert_eq!(opened["isError"], false, "{opened}");
    let added = call(
        a,
        "a",
        "add_measure",
        json!({"table": "Product", "name": "M1", "expression": "1"}),
    );
    assert_eq!(added["isError"], false, "{added}");
    // unsaved changes to the open file: replacing or deleting it is refused
    let uploaded = MODEL.replace("\"Avg Price\"", "\"Uploaded Measure\"");
    let r = put(a, "/w/a/files/model.bim", &["Overwrite: T"], uploaded.as_bytes());
    assert_eq!(r.status, 409, "{}", r.text());
    assert_eq!(request(a, "DELETE", "/w/a/files/model.bim", &[], b"").status, 409);
    assert_eq!(
        std::fs::read_to_string(f.ws("a").join("model.bim")).unwrap(),
        MODEL
    );
    // other files are fine meanwhile
    assert_eq!(put(a, "/w/a/files/other.txt", &[], b"o").status, 201);
    // reopening discards the changes; then the upload closes the session and the next call
    // reopens the new file
    call(a, "a", "open_file", json!({"path": "model.bim"}));
    let r = put(a, "/w/a/files/model.bim", &["Overwrite: T"], uploaded.as_bytes());
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.json()["session_closed"], true);
    let summary = tool_text(&call(a, "a", "model_summary", json!({})));
    assert!(summary.contains("Uploaded Measure"), "{summary}");
    assert!(!summary.contains("M1"), "{summary}");
}

// ---------------------------------------------------------------------------------------------
// Probes, drain, idle close

#[test]
fn probes_answer_without_auth_and_without_data() {
    let token = tokens::generate().unwrap();
    let config = HttpConfig {
        auth: Some(Arc::new(
            Authenticator::new(token_config(&token, &["token:ci=*"])).unwrap(),
        )),
        hosts: bicraft_mcp::HostPolicy::Names(vec!["bi.example.com".into()]),
        ..HttpConfig::default()
    };
    let mut f = fixture("ops-probes", config);
    for path in ["/healthz", "/readyz"] {
        // any Host (the kubelet sends the pod address), no credentials
        let mut s = connect(f.addr);
        write!(s, "GET {path} HTTP/1.1\r\nHost: 10.1.2.3:7878\r\n\r\n").unwrap();
        let r = read_response(s);
        assert_eq!(r.status, 200, "{path}");
        assert_eq!(r.text(), "ok\n");
        let r = request(f.addr, "POST", path, &[], b"");
        assert_eq!(r.status, 405);
        let mut s = connect(f.addr);
        write!(s, "HEAD {path} HTTP/1.1\r\n\r\n").unwrap();
        let r = read_response(s);
        assert_eq!(r.status, 200);
        assert!(r.body.is_empty());
    }
    // everything else still checks the Host
    assert_eq!(get(f.addr, "/w/a/files/", &[]).status, 403);
    let server = f.take();
    let report = server.drain(Duration::from_secs(1));
    assert!(report.clean(), "{report:?}");
}

#[test]
fn drain_lets_running_requests_finish_and_refuses_new_ones() {
    let mut f = fixture("ops-drain", HttpConfig::default());
    let a = f.addr;
    std::fs::write(f.ws("a").join("model.bim"), MODEL).unwrap();
    std::fs::create_dir_all(f.ws("b").join(".bicraft-tmp")).unwrap();
    call(a, "a", "open_file", json!({"path": "model.bim"}));
    // an upload in flight: its head and half its body are sent before the drain starts
    let body = vec![1u8; 200_000];
    let mut up = connect(a);
    write!(
        up,
        "PUT /w/b/files/late.bin HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )
    .unwrap();
    up.write_all(&body[..100_000]).unwrap();
    wait_until("the upload is active", || {
        f.dir
            .path()
            .join("b")
            .join(".bicraft-tmp")
            .read_dir()
            .is_ok_and(|mut d| d.next().is_some())
    });
    let server = f.take();
    assert!(!server.is_draining());
    let drain = std::thread::spawn(move || server.drain(Duration::from_secs(20)));
    wait_until("readyz flips", || get(a, "/readyz", &[]).status == 503);
    assert_eq!(get(a, "/readyz", &[]).text(), "draining\n");
    assert_eq!(get(a, "/healthz", &[]).status, 200);
    // new work: 503 with a retry hint, JSON-RPC for the endpoint
    let mut s = connect(a);
    s.write_all(&call_bytes("a", "model_summary", json!({}))).unwrap();
    let r = read_response(s);
    assert_eq!(r.status, 503);
    assert_eq!(r.header("Retry-After"), Some("5"));
    assert_eq!(r.json()["error"]["code"], SHUTTING_DOWN);
    let r = get(a, "/w/a/files/", &[]);
    assert_eq!(r.status, 503);
    assert_eq!(r.json()["error"]["code"], SHUTTING_DOWN);
    // the running upload finishes and is answered
    up.write_all(&body[100_000..]).unwrap();
    let r = read_response(up);
    assert_eq!(r.status, 201, "{}", r.text());
    let report = drain.join().unwrap();
    assert!(report.finished_in_grace, "{report:?}");
    assert!(report.clean(), "{report:?}");
    assert_eq!(report.closed, 1);
    assert_eq!(std::fs::read(f.ws("b").join("late.bin")).unwrap(), body);
    assert!(!f.ws("b").join(".bicraft-tmp").exists());
    // the listener is closed
    assert!(
        TcpStream::connect_timeout(&a, Duration::from_secs(2)).is_err()
            || get(a, "/healthz", &[]).status != 200
    );
}

#[test]
fn drain_cancels_long_queries_and_never_saves() {
    let mut f = fixture("ops-drain-cancel", HttpConfig::default());
    let a = f.addr;
    std::fs::write(f.ws("a").join("model.bim"), MODEL).unwrap();
    call(a, "a", "open_file", json!({"path": "model.bim"}));
    let added = call(
        a,
        "a",
        "add_measure",
        json!({"table": "Product", "name": "Unsaved", "expression": "2"}),
    );
    assert_eq!(added["isError"], false, "{added}");
    std::fs::write(f.ws("b").join("model.bim"), MODEL).unwrap();
    call(a, "b", "open_file", json!({"path": "model.bim"}));
    let mut long = connect(a);
    long.write_all(&call_bytes(
        "b",
        "evaluate_dax_query",
        json!({"query": LONG, "timeout_ms": 600_000}),
    ))
    .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let server = f.take();
    let start = Instant::now();
    let report = server.drain(Duration::from_millis(500));
    assert!(start.elapsed() < Duration::from_secs(20), "{:?}", start.elapsed());
    assert!(!report.finished_in_grace, "{report:?}");
    assert_eq!(report.cancelled, 1, "{report:?}");
    assert_eq!(report.abandoned, 0, "{report:?}");
    assert_eq!(report.unsaved, ["a"]);
    assert!(!report.clean());
    // nothing more was sent for the cancelled query, and the edit was never written
    let r = read_response(long);
    assert!(
        r.body.is_empty() || r.status != 200 || !r.text().contains("\"result\""),
        "{}",
        r.text()
    );
    assert_eq!(
        std::fs::read_to_string(f.ws("a").join("model.bim")).unwrap(),
        MODEL
    );
}

#[test]
fn idle_workspaces_close_and_reopen_lazily() {
    let config = HttpConfig {
        idle_timeout: Some(Duration::from_millis(300)),
        ..HttpConfig::default()
    };
    let mut f = fixture("ops-idle", config);
    let a = f.addr;
    std::fs::write(f.ws("a").join("model.bim"), MODEL).unwrap();
    std::fs::write(f.ws("b").join("model.bim"), MODEL).unwrap();
    call(a, "a", "open_file", json!({"path": "model.bim"}));
    call(a, "b", "open_file", json!({"path": "model.bim"}));
    // b gets unsaved changes: it stays open
    call(
        a,
        "b",
        "add_measure",
        json!({"table": "Product", "name": "Kept", "expression": "3"}),
    );
    assert_eq!(f.server().open_workspaces(), 2);
    wait_until("a closes", || f.server().open_workspaces() == 1);
    std::thread::sleep(Duration::from_millis(800));
    assert_eq!(f.server().open_workspaces(), 1, "b has unsaved changes");
    // the next call reopens a's file
    let summary = tool_text(&call(a, "a", "model_summary", json!({})));
    assert!(summary.contains("Avg Price"), "{summary}");
    assert_eq!(f.server().open_workspaces(), 2);
    let summary_b = tool_text(&call(a, "b", "model_summary", json!({})));
    assert!(summary_b.contains("Kept"), "{summary_b}");
    let report = f.take().drain(Duration::from_secs(1));
    assert_eq!(report.unsaved, ["b"]);
}
