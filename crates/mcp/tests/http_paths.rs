//! No server path in any answer over HTTP (docs/server-mode.md, "Paths in answers"): every engine
//! command is run in server mode, with no parameters and with path parameters (existing files,
//! missing files, outputs), and nothing it answers (results and errors) may contain the server's
//! folders; nor may the viewer's snapshot or a reader's answers.

#[path = "../../../testsupport/tempdir.rs"]
mod tempdir;

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use bicraft_mcp::http::HTTP_PROTOCOL_VERSION;
use bicraft_mcp::{HttpConfig, HttpServer};
use serde_json::{Value, json};
use tempdir::TempDir;

const MODEL: &str = include_str!("../../engine/tests/table_functions/model.bim");

fn send(addr: SocketAddr, bytes: &[u8]) -> (u16, String) {
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(120))).unwrap();
    s.write_all(bytes).unwrap();
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    let text = String::from_utf8_lossy(&out).into_owned();
    let status = text.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    let body = text
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.to_string())
        .unwrap_or_default();
    (status, body)
}

/// A tool call; the whole HTTP body (result or error).
fn call(addr: SocketAddr, tool: &str, args: Value) -> String {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {
        "name": tool, "arguments": args, "_meta": {
            "io.modelcontextprotocol/protocolVersion": HTTP_PROTOCOL_VERSION,
            "io.modelcontextprotocol/clientCapabilities": {}}}})
    .to_string();
    let req = format!(
        "POST /w/a/mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
         Accept: application/json\r\nMCP-Protocol-Version: {HTTP_PROTOCOL_VERSION}\r\n\
         Mcp-Method: tools/call\r\nMcp-Name: {tool}\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    send(addr, req.as_bytes()).1
}

fn get(addr: SocketAddr, path: &str) -> (u16, String) {
    send(
        addr,
        format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").as_bytes(),
    )
}

#[test]
fn no_command_answer_carries_a_server_path() {
    let dir = TempDir::new("paths-scan");
    let ws = dir.path().join("a");
    std::fs::create_dir_all(ws.join("sub")).unwrap();
    std::fs::write(ws.join("model.bim"), MODEL).unwrap();
    std::fs::write(ws.join("sub").join("broken.bim"), "{ not json").unwrap();
    std::fs::write(ws.join("broken.pbix"), "not a zip").unwrap();
    let server =
        HttpServer::start("127.0.0.1:0".parse().unwrap(), dir.path(), HttpConfig::default()).unwrap();
    let a = server.addr();
    // the temp folder's own name: any spelling of a server path contains it
    let marker = dir.path().file_name().unwrap().to_string_lossy().into_owned();
    let mut leaks = Vec::new();
    let mut check = |what: &str, text: &str| {
        if text.contains(&marker) || text.contains(r"\\?\") {
            leaks.push(format!("{what}: {}", text.chars().take(400).collect::<String>()));
        }
    };

    let opened = call(a, "open_file", json!({"path": "model.bim"}));
    assert!(opened.contains("\"isError\":false"), "{opened}");
    check("open_file", &opened);

    // (id, whether its parameters name a file or folder)
    let ids: Vec<(String, bool)> = {
        let list = call(a, "list_commands", json!({}));
        let v: Value = serde_json::from_str(&list).unwrap();
        v["result"]["structuredContent"]["commands"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| {
                let doc = c["params"].as_str().unwrap_or_default();
                let pathy = ["path", "out", "from", "data_root", "cache_abf", "dir", "file"]
                    .iter()
                    .any(|k| doc.contains(k));
                (c["id"].as_str().unwrap().to_string(), pathy)
            })
            .collect()
    };
    assert!(ids.len() > 100, "{}", ids.len());
    let path_params = [
        json!({}),
        json!({"path": "model.bim"}),
        json!({"path": "missing/nothing.pbix"}),
        json!({"path": "sub/broken.bim"}),
        json!({"path": "out/Proj.pbip"}),
        json!({"out": "out/page.png"}),
        json!({"from": "model.bim", "to": "out/conv.pbip"}),
        json!({"cache_abf": "missing.abf", "data_root": "missing"}),
    ];
    for (id, pathy) in &ids {
        // every command without parameters; the path variants where its parameters name paths
        let runs = if *pathy {
            &path_params[..]
        } else {
            &path_params[..1]
        };
        for params in runs {
            let r = call(a, "run_command", json!({"command": id, "params": params}));
            check(&format!("{id} {params}"), &r);
            // the next run starts from the model open and saved again
            if bicraft_engine::is_mutating(id) {
                let _ = call(a, "open_file", json!({"path": "model.bim"}));
            }
        }
    }
    // the convenience tools that name paths
    for (tool, args) in [
        ("inspect_file", json!({"path": "broken.pbix"})),
        ("open_file", json!({"path": "missing.pbix"})),
        ("save_as", json!({"path": "out/again.bim"})),
        ("save", json!({})),
        ("render_page", json!({"out": "out/p.png"})),
    ] {
        let r = call(a, tool, args.clone());
        check(&format!("{tool} {args}"), &r);
    }
    // the viewer's snapshot, a file listing and a refused file route
    let _ = call(a, "open_file", json!({"path": "model.bim"}));
    let (status, page) = get(a, "/w/a/view/");
    assert_eq!(status, 200);
    let needle = r#"name="bicraft-view-id" content=""#;
    let at = page.find(needle).unwrap() + needle.len();
    let vid = &page[at..at + 32];
    let (status, snap) = get(a, &format!("/w/a/view/api/state?v={vid}"));
    assert_eq!(status, 200, "{snap}");
    check("viewer state", &snap);
    for path in ["/w/a/files/", "/w/a/files/missing.txt", "/w/a/files/%2e%2e/x"] {
        let (_, body) = get(a, path);
        check(path, &body);
    }
    assert!(
        leaks.is_empty(),
        "{} answers carry a server path:\n{}",
        leaks.len(),
        leaks.join("\n")
    );
}
