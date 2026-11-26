//! The pure half of a BIcraft server's client, shared by the native client
//! (`apps/bicraft-desktop/src/remote.rs`, blocking HTTPS) and the browser's source
//! (`apps/bicraft-web`, `fetch`): the viewer API's URLs, the reading of its answers and the
//! wording of its failures (`docs/desktop.md`, "Remote mode").

use serde_json::Value;

/// Largest snapshot read (the server caps its own; this bounds a hostile one).
pub const MAX_SNAPSHOT_BYTES: u64 = 256 * 1024 * 1024;
/// Largest image read.
pub const MAX_ASSET_BYTES: u64 = 32 * 1024 * 1024;
/// Most images fetched for one snapshot.
const MAX_ASSETS: usize = 256;

/// An HTTP failure, worded for the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// 401: missing or rejected credentials.
    Unauthorized,
    /// 403: the server refused (a CSRF check, a role).
    Forbidden(String),
    /// 404 on the view: it expired (a restart): open a new one.
    ViewGone,
    /// 404 on the session: no such workspace, or no access to it (the server does not say which).
    NoWorkspace,
    /// Another HTTP status, with the server's message.
    Status(u16, String),
    /// The server could not be reached.
    Network(String),
    /// The answer was not what the viewer API sends.
    Protocol(String),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::Unauthorized => write!(f, "authentication failed (401): check the token"),
            Failure::Forbidden(m) => write!(f, "refused by the server (403): {m}"),
            Failure::ViewGone => write!(f, "the server forgot this view"),
            Failure::NoWorkspace => write!(f, "no such workspace, or no access to it (404)"),
            Failure::Status(s, m) => write!(f, "server error {s}: {m}"),
            Failure::Network(m) => write!(f, "cannot reach the server: {m}"),
            Failure::Protocol(m) => write!(f, "unexpected answer from the server: {m}"),
        }
    }
}

/// `https://host/` + `w/sales` → `https://host/w/sales/view`.
pub fn view_base(url: &str, workspace: &str) -> String {
    format!("{}/w/{}/view", url.trim().trim_end_matches('/'), workspace.trim())
}

/// A workspace id the server can route (letters, digits, `-`, `_`, `.`).
pub fn valid_workspace(ws: &str) -> bool {
    !ws.is_empty()
        && ws.len() <= 128
        && ws
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// The image ids a snapshot references (`{"id": <hex>, "mime": "image/…"}` objects).
pub fn asset_ids(snap: &Value) -> Vec<String> {
    fn walk(v: &Value, depth: usize, out: &mut Vec<String>) {
        if depth > 64 || out.len() >= MAX_ASSETS {
            return;
        }
        match v {
            Value::Object(o) => {
                if let (Some(Value::String(id)), Some(Value::String(mime))) = (o.get("id"), o.get("mime"))
                    && mime.starts_with("image/")
                    && (16..=64).contains(&id.len())
                    && id.bytes().all(|b| b.is_ascii_hexdigit())
                    && !out.contains(id)
                {
                    out.push(id.clone());
                }
                for x in o.values() {
                    walk(x, depth + 1, out);
                }
            }
            Value::Array(a) => {
                for x in a {
                    walk(x, depth + 1, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(snap, 0, &mut out);
    out
}

/// The failure an HTTP status means (`Ok` for 2xx); `on_view`: a 404 is a view the server forgot,
/// else a workspace there is no access to. Shared with the browser's source (`apps/bicraft-web`).
pub fn classify(status: u16, body: &[u8], on_view: bool) -> Result<(), Failure> {
    match status {
        200..=299 => Ok(()),
        401 => Err(Failure::Unauthorized),
        403 => Err(Failure::Forbidden(message(body))),
        404 if on_view => Err(Failure::ViewGone),
        404 => Err(Failure::NoWorkspace),
        s => Err(Failure::Status(s, message(body))),
    }
}

/// The server's message in an error answer (`{"error": …}`, or text), shortened.
pub fn message(body: &[u8]) -> String {
    let text = match serde_json::from_slice::<Value>(body) {
        Ok(v) => v
            .get("error")
            .map(|e| match e {
                Value::String(s) => s.clone(),
                Value::Object(o) => o
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                other => other.to_string(),
            })
            .unwrap_or_default(),
        Err(_) => String::from_utf8_lossy(body).into_owned(),
    };
    text.chars().take(300).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn urls_ids_and_messages() {
        assert_eq!(view_base("https://h/ ", "ws"), "https://h/w/ws/view");
        assert_eq!(view_base("http://h:8080", "a"), "http://h:8080/w/a/view");
        assert!(valid_workspace("sales-2025_v1.0"));
        assert!(!valid_workspace("../x") && !valid_workspace("") && !valid_workspace("a b"));
        let id = "0123456789abcdef0123456789abcdef";
        let snap = json!({"report": {"pages": [{"wallpaper": {"image": {"id": id, "mime": "image/png"}},
            "visuals": [{"image": {"id": id, "mime": "image/png"}}, {"name": {"id": "x", "mime": "image/png"}}]}]}});
        assert_eq!(asset_ids(&snap), vec![id.to_string()]);
        assert_eq!(message(br#"{"ok": false, "error": "bad"}"#), "bad");
        assert_eq!(
            message(br#"{"error": {"code": 1, "message": "denied"}}"#),
            "denied"
        );
        assert_eq!(message(b"plain"), "plain");
        assert!(Failure::Unauthorized.to_string().contains("401"));
    }
}

// --- commands and files (the workspace's MCP endpoint and file routes) ------------------------

/// The MCP revision the endpoint speaks (`crates/mcp/src/http.rs`, `HTTP_PROTOCOL_VERSION`).
pub const MCP_VERSION: &str = "2026-07-28";
/// Largest command answer read.
pub const MAX_ANSWER_BYTES: u64 = 64 * 1024 * 1024;
/// Most workspace files listed.
pub const MAX_FILES: usize = 10_000;

/// `https://host/` + `sales` → `https://host/w/sales` (the workspace's routes: `/mcp`, `/files/`).
pub fn workspace_base(url: &str, workspace: &str) -> String {
    format!("{}/w/{}", url.trim().trim_end_matches('/'), workspace.trim())
}

/// The body of a `tools/call` of `run_command {command: id, params}` on the workspace's MCP
/// endpoint (docs/mcp.md), with the revision's `_meta`.
pub fn command_body(id: &str, params: &Value) -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "run_command",
            "arguments": {"command": id, "params": params},
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": MCP_VERSION,
                "io.modelcontextprotocol/clientInfo": {"name": "bicraft-ui", "version": "1"},
                "io.modelcontextprotocol/clientCapabilities": {},
            },
        },
    })
    .to_string()
}

/// The headers that go with [`command_body`] (the endpoint checks they mirror the body).
pub const COMMAND_HEADERS: [(&str, &str); 5] = [
    ("Content-Type", "application/json"),
    ("Accept", "application/json"),
    ("MCP-Protocol-Version", MCP_VERSION),
    ("Mcp-Method", "tools/call"),
    ("Mcp-Name", "run_command"),
];

/// The command's result in the endpoint's answer: `structuredContent`, or its `error` when the
/// tool call failed (`isError`); a JSON-RPC error or an HTTP failure is an error too.
pub fn command_answer(status: u16, body: &[u8]) -> Result<Value, String> {
    classify(status, body, false).map_err(|f| f.to_string())?;
    let v: Value = serde_json::from_slice(body).map_err(|e| Failure::Protocol(e.to_string()).to_string())?;
    if let Some(e) = v.get("error") {
        let m = e.get("message").and_then(Value::as_str).unwrap_or("error");
        return Err(m.chars().take(500).collect());
    }
    let result = v
        .get("result")
        .ok_or("unexpected answer from the server: no result")?;
    let content = result.get("structuredContent").cloned().unwrap_or(Value::Null);
    if result.get("isError").and_then(Value::as_bool) == Some(true) {
        let m = match content.get("error") {
            Some(Value::String(s)) => s.clone(),
            Some(other) => other.to_string(),
            None => "the command failed".to_string(),
        };
        return Err(m.chars().take(2000).collect());
    }
    Ok(content)
}

/// The files of a `GET /w/<id>/files/` answer (`{entries: [{path, type, size?, modified}]}`):
/// files only, at most [`MAX_FILES`], with relative `/`-separated paths.
pub fn read_files(body: &[u8]) -> Result<Vec<crate::source::WsFile>, String> {
    let v: Value = serde_json::from_slice(body).map_err(|e| Failure::Protocol(e.to_string()).to_string())?;
    let entries = v
        .get("entries")
        .and_then(Value::as_array)
        .ok_or("unexpected answer from the server: no file list")?;
    Ok(entries
        .iter()
        .filter(|e| e.get("type").and_then(Value::as_str) == Some("file"))
        .filter_map(|e| {
            let path = e.get("path")?.as_str()?;
            safe_relative(path).then(|| crate::source::WsFile {
                path: path.to_string(),
                size: e.get("size").and_then(Value::as_u64),
                modified: e
                    .get("modified")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .chars()
                    .take(40)
                    .collect(),
            })
        })
        .take(MAX_FILES)
        .collect())
}

/// A workspace-relative path the app may keep or send: `/`-separated, not absolute, no `..`, no
/// drive letter or backslash, at most 400 bytes.
pub fn safe_relative(p: &str) -> bool {
    !p.is_empty()
        && p.len() <= 400
        && !p.starts_with('/')
        && !p.contains('\\')
        && !p.contains(':')
        && !p.chars().any(char::is_control)
        && p.split('/').all(|s| !s.is_empty() && s != "." && s != "..")
}

#[cfg(test)]
mod command_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn command_calls_and_answers() {
        let b: Value =
            serde_json::from_str(&command_body("page.add", &json!({"display_name": "P"}))).unwrap();
        assert_eq!(b["params"]["arguments"]["command"], "page.add");
        assert_eq!(
            b["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"],
            MCP_VERSION
        );
        assert_eq!(workspace_base("https://h/", "ws"), "https://h/w/ws");
        let ok = br#"{"jsonrpc":"2.0","id":1,"result":{"isError":false,"structuredContent":{"name":"x"}}}"#;
        assert_eq!(command_answer(200, ok), Ok(json!({"name": "x"})));
        let bad =
            br#"{"jsonrpc":"2.0","id":1,"result":{"isError":true,"structuredContent":{"error":"no page"}}}"#;
        assert_eq!(command_answer(200, bad), Err("no page".into()));
        let rpc = br#"{"jsonrpc":"2.0","id":1,"error":{"code":-32020,"message":"mismatch"}}"#;
        assert!(command_answer(400, rpc).unwrap_err().contains("400"));
        assert_eq!(command_answer(200, rpc), Err("mismatch".into()));
        assert!(command_answer(401, b"").unwrap_err().contains("401"));
        assert!(command_answer(200, b"{not json").is_err());
        assert!(command_answer(200, b"{}").is_err());
    }

    #[test]
    fn file_lists_keep_safe_files_only() {
        let body = br#"{"path":"","entries":[{"path":"a/Sales.pbix","type":"file","size":10,"modified":"2026-11-04T10:00:00Z"},
            {"path":"a","type":"dir"},{"path":"../x.pbix","type":"file"},{"path":"C:/x.pbix","type":"file"}]}"#;
        let f = read_files(body).unwrap();
        assert_eq!(f.len(), 1);
        assert_eq!((f[0].path.as_str(), f[0].size), ("a/Sales.pbix", Some(10)));
        assert!(read_files(b"[]").is_err() && read_files(b"nope").is_err());
        for bad in ["", "/a", "a/../b", "a\\b", "C:/a", "a//b", "./a", "a\nb"] {
            assert!(!safe_relative(bad), "{bad:?}");
        }
        assert!(safe_relative("Sales.pbip") && safe_relative("x/y z.pbix"));
    }
}
