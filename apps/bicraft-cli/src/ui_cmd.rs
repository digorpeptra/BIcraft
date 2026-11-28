//! `bicraft-cli ui`: the client of the UI control channel (ROADMAP M11.6, docs/desktop.md "Control
//! channel"), so agents, scripts and tests drive a running app and look at it:
//!
//! ```text
//! bicraft-cli ui [--control FILE | --view | --serve] [--url URL] [--workspace ID] [--view-id V]
//!            METHOD [PARAMS-JSON] [--out FILE.png]
//! bicraft-cli ui command ID [PARAMS-JSON]
//! ```
//!
//! The token file says where the app listens: `{"addr", "token"}` is the desktop's loopback
//! socket (one JSON line each way), `{"url", "token"}` the relay of `bicraft-cli view --control` to
//! the browser app (an HTTP POST with the token as bearer), `{"kind": "serve", "url", "token"}`
//! the relay of `bicraft-cli serve --viewer-control` (`--workspace` names the workspace). For a
//! server, `--url` gives the viewer's URL instead (`https://h/w/<id>/view/`, the token from
//! `BICRAFT_CONTROL_TOKEN` or the file), the server's own credential comes from
//! `BICRAFT_SERVER_TOKEN` (`Authorization: Bearer`) and the control token goes in
//! `X-Bicraft-Control`; `--view-id` addresses one view (a page's, or a document view's tab).
//! The reply is printed as JSON; a screenshot's PNG goes to `--out` instead of the terminal.
//! Exit 0 when the reply is `ok`, 1 when the app refused, 2 when it could not be reached.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

use super::{Failure, INVALID, OK, USAGE_ERROR, usage};

/// Largest reply read (a screenshot of a large window, base64; `bicraft_ui::control`).
const MAX_REPLY: u64 = 48 << 20;
/// How long a reply is waited for (the app's own wait is 60 s).
const WAIT: Duration = Duration::from_secs(75);

/// Which app's default token file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// `bicraft --control`: `control.json`.
    Desktop,
    /// `bicraft-cli view --control`: `control-view.json`.
    View,
    /// `bicraft-cli serve --viewer-control`: `control-serve.json`.
    Serve,
}

/// Where the app writes its token file by default (as `bicraft`'s
/// `control::default_file`: next to its settings file).
pub(crate) fn default_file(kind: Kind) -> Option<PathBuf> {
    let dir = if cfg!(windows) {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("BIcraft"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .map(|c| c.join("bicraft"))
    };
    let name = match kind {
        Kind::Desktop => "control.json",
        Kind::View => "control-view.json",
        Kind::Serve => "control-serve.json",
    };
    dir.map(|d| d.join(name))
}

/// What the command line says about the target, besides the token file.
#[derive(Debug, Default, PartialEq, Eq)]
struct Where {
    url: Option<String>,
    workspace: Option<String>,
    view: Option<String>,
}

pub(crate) fn ui(args: &[String]) -> Result<u8, Failure> {
    let mut file = None;
    let mut kind = Kind::Desktop;
    let mut at = Where::default();
    let mut out = None;
    let mut rest = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut next = |what: &str| -> Result<String, Failure> {
            it.next()
                .cloned()
                .ok_or_else(|| usage(format!("{a} needs {what}")))
        };
        match a.as_str() {
            "--control" => file = Some(PathBuf::from(next("a FILE")?)),
            "--view" => kind = Kind::View,
            "--serve" => kind = Kind::Serve,
            "--url" => at.url = Some(next("a URL")?),
            "--workspace" => at.workspace = Some(next("an ID")?),
            "--view-id" => at.view = Some(next("a view id")?),
            "--out" => out = Some(PathBuf::from(next("a FILE.png")?)),
            s if s.starts_with("--") => return Err(usage(format!("ui: unknown option `{s}`"))),
            s => rest.push(s.to_string()),
        }
    }
    let request = request(&rest)?;
    let target = match (&at.url, bicraft_env::var("BICRAFT_CONTROL_TOKEN").ok()) {
        // a server's URL and the token from the environment: no file
        (Some(_), Some(token)) if file.is_none() => Target::serve(&at, None, token.trim())?,
        _ => {
            let file = file
                .or_else(|| bicraft_env::var_os("BICRAFT_CONTROL_FILE").map(PathBuf::from))
                .or_else(|| default_file(if at.url.is_some() { Kind::Serve } else { kind }))
                .ok_or_else(|| usage("ui: no token file: give --control FILE"))?;
            Target::read(&file, &at)?
        }
    };
    let mut reply = target.call(&request)?;
    if let Some(path) = &out {
        save_png(&mut reply, path)?;
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&reply).unwrap_or_else(|_| reply.to_string())
    );
    Ok(if reply.get("ok") == Some(&Value::Bool(true)) {
        OK
    } else {
        INVALID
    })
}

/// `METHOD [PARAMS]`, or `command ID [PARAMS]`.
fn request(rest: &[String]) -> Result<Value, Failure> {
    let parse = |s: &str| -> Result<Value, Failure> {
        match serde_json::from_str(s) {
            Ok(v @ Value::Object(_)) => Ok(v),
            _ => Err(usage(format!("ui: PARAMS must be a JSON object, not `{s}`"))),
        }
    };
    let (method, params) = match rest {
        [] => return Err(usage("ui needs a METHOD (`bicraft-cli ui methods` lists them)")),
        [m] => (m.clone(), json!({})),
        [m, id] if m == "command" && !id.trim_start().starts_with('{') => (m.clone(), json!({"command": id})),
        [m, id, p] if m == "command" && !id.trim_start().starts_with('{') => {
            (m.clone(), json!({"command": id, "params": parse(p)?}))
        }
        [m, p] => (m.clone(), parse(p)?),
        _ => return Err(usage("ui takes METHOD [PARAMS-JSON]")),
    };
    Ok(json!({"id": 1, "method": method, "params": params}))
}

/// Where the app listens, from its token file.
#[derive(Debug, PartialEq)]
enum Target {
    /// The desktop's socket.
    Socket { addr: SocketAddr, token: String },
    /// `bicraft-cli view --control`: `http://127.0.0.1:PORT/api/control`.
    Http {
        addr: SocketAddr,
        path: String,
        token: String,
    },
    /// `bicraft-cli serve --viewer-control`: `http(s)://host/w/<id>/view/api/control[?v=<view>]`,
    /// the control token in `X-Bicraft-Control`, the server's credential (if any) as bearer.
    Serve {
        url: String,
        token: String,
        bearer: Option<String>,
    },
}

/// The agent's route of a server's relay: from a viewer URL (`…/w/<id>/view/`, its app page
/// `…/view/app/`, or the route itself) or a server's base URL and `workspace`; `?v=` for `view`.
fn serve_url(url: &str, workspace: Option<&str>, view: Option<&str>) -> Result<String, String> {
    let url = url.trim();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(format!("`{url}`: a server URL is http:// or https://"));
    }
    if url.contains('?') || url.contains('#') {
        return Err(format!("`{url}`: no query here (use --view-id)"));
    }
    let base = url.trim_end_matches('/');
    let base = base.strip_suffix("/app").unwrap_or(base);
    let route = if base.ends_with("/view/api/control") {
        base.to_string()
    } else if base.ends_with("/view") && base.contains("/w/") {
        format!("{base}/api/control")
    } else {
        let ws = workspace
            .filter(|w| {
                !w.is_empty()
                    && w.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            })
            .ok_or_else(|| format!("`{url}` names no workspace: give --workspace ID or the viewer's URL"))?;
        format!("{base}/w/{ws}/view/api/control")
    };
    match view {
        None => Ok(route),
        Some(v)
            if v.len() == 32
                && v.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) =>
        {
            Ok(format!("{route}?v={v}"))
        }
        Some(v) => Err(format!("--view-id `{v}`: a view id is 32 lowercase hex digits")),
    }
}

impl Target {
    fn read(file: &Path, at: &Where) -> Result<Target, Failure> {
        let unreachable = |e: String| (USAGE_ERROR, format!("{}: {e}", file.display()));
        let small = std::fs::metadata(file).is_ok_and(|m| m.len() <= 64 * 1024);
        let text = if small {
            std::fs::read_to_string(file).map_err(|e| unreachable(e.to_string()))?
        } else {
            return Err(unreachable(
                "no token file here (is the app running with --control?)".into(),
            ));
        };
        let v: Value = serde_json::from_str(&text).map_err(|_| unreachable("not a token file".into()))?;
        let serve = at.url.is_some() || v.get("kind").and_then(Value::as_str) == Some("serve");
        if serve {
            let token = v.get("token").and_then(Value::as_str).unwrap_or_default();
            let mut at = Where {
                url: at.url.clone(),
                workspace: at.workspace.clone(),
                view: at.view.clone(),
            };
            if at.url.is_none() {
                at.url = v.get("url").and_then(Value::as_str).map(str::to_string);
            }
            return Target::serve(&at, Some(file), token);
        }
        if at.workspace.is_some() || at.view.is_some() {
            return Err(usage(
                "--workspace and --view-id go with a server (--serve or --url)",
            ));
        }
        Target::from_json(&v).map_err(unreachable)
    }

    /// A server's relay: the URL from `at`, the token given, the credential from
    /// `BICRAFT_SERVER_TOKEN`.
    fn serve(at: &Where, file: Option<&Path>, token: &str) -> Result<Target, Failure> {
        let fail = |e: String| match file {
            Some(f) => (USAGE_ERROR, format!("{}: {e}", f.display())),
            None => usage(format!("ui: {e}")),
        };
        if token.is_empty() {
            return Err(fail("no control token".into()));
        }
        let url = at
            .url
            .as_deref()
            .ok_or_else(|| fail("no server URL: give --url".into()))?;
        let url = serve_url(url, at.workspace.as_deref(), at.view.as_deref()).map_err(fail)?;
        let bearer = bicraft_env::var("BICRAFT_SERVER_TOKEN")
            .ok()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty());
        Ok(Target::Serve {
            url,
            token: token.to_string(),
            bearer,
        })
    }

    fn from_json(v: &Value) -> Result<Target, String> {
        let token = v
            .get("token")
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
            .ok_or("no token")?
            .to_string();
        let loopback = |a: SocketAddr| {
            if a.ip().is_loopback() {
                Ok(a)
            } else {
                Err(format!("{a} is not a loopback address"))
            }
        };
        if let Some(a) = v.get("addr").and_then(Value::as_str) {
            let addr = loopback(a.parse().map_err(|_| format!("bad address `{a}`"))?)?;
            return Ok(Target::Socket { addr, token });
        }
        let url = v.get("url").and_then(Value::as_str).ok_or("no addr or url")?;
        let rest = url
            .strip_prefix("http://")
            .ok_or_else(|| format!("`{url}`: only http:// on loopback"))?;
        let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
        let host = host.replace("localhost", "127.0.0.1");
        let addr = loopback(host.parse().map_err(|_| format!("bad address in `{url}`"))?)?;
        Ok(Target::Http {
            addr,
            path: format!("/{path}"),
            token,
        })
    }

    fn call(&self, request: &Value) -> Result<Value, Failure> {
        let down = |e: std::io::Error| (USAGE_ERROR, format!("ui: the app does not answer: {e}"));
        match self {
            Target::Socket { addr, token } => {
                let mut req = request.clone();
                if let Some(o) = req.as_object_mut() {
                    o.insert("token".into(), json!(token));
                }
                let mut s = TcpStream::connect_timeout(addr, Duration::from_secs(5)).map_err(down)?;
                s.set_read_timeout(Some(WAIT)).map_err(down)?;
                s.write_all(format!("{req}\n").as_bytes()).map_err(down)?;
                let mut line = Vec::new();
                BufReader::new(s)
                    .take(MAX_REPLY)
                    .read_until(b'\n', &mut line)
                    .map_err(down)?;
                if line.last() != Some(&b'\n') {
                    return Err((USAGE_ERROR, "ui: the app closed the connection".into()));
                }
                serde_json::from_slice(&line).map_err(|_| (USAGE_ERROR, "ui: the reply is not JSON".into()))
            }
            Target::Http { addr, path, token } => {
                let body = request.to_string();
                let head = format!(
                    "POST {path} HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {token}\r\n\
                     Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let mut s = TcpStream::connect_timeout(addr, Duration::from_secs(5)).map_err(down)?;
                s.set_read_timeout(Some(WAIT)).map_err(down)?;
                s.write_all(head.as_bytes()).map_err(down)?;
                s.write_all(body.as_bytes()).map_err(down)?;
                let mut raw = Vec::new();
                s.take(MAX_REPLY).read_to_end(&mut raw).map_err(down)?;
                http_reply(&raw)
            }
            Target::Serve { url, token, bearer } => {
                let agent = ureq::Agent::config_builder()
                    .http_status_as_error(false)
                    .timeout_connect(Some(Duration::from_secs(10)))
                    .timeout_global(Some(WAIT))
                    .user_agent(concat!("bicraft-ui/", env!("CARGO_PKG_VERSION")))
                    .build()
                    .new_agent();
                let mut r = agent
                    .post(url)
                    .header("Content-Type", "application/json")
                    .header("X-Bicraft-Control", token);
                if let Some(b) = bearer {
                    r = r.header("Authorization", &format!("Bearer {b}"));
                }
                let net = |e: ureq::Error| (USAGE_ERROR, format!("ui: the server does not answer: {e}"));
                // `server.views`: the server lists the caller's views (to address one with --view-id)
                let listing = request.get("method").and_then(Value::as_str) == Some("server.views");
                let mut resp = if listing {
                    let mut g = agent.get(url).header("X-Bicraft-Control", token);
                    if let Some(b) = bearer {
                        g = g.header("Authorization", &format!("Bearer {b}"));
                    }
                    g.call().map_err(net)?
                } else {
                    r.send(request.to_string()).map_err(net)?
                };
                let status = resp.status();
                let body = resp
                    .body_mut()
                    .with_config()
                    .limit(MAX_REPLY)
                    .read_to_vec()
                    .map_err(net)?;
                serde_json::from_slice(&body).map_err(|_| (USAGE_ERROR, format!("ui: HTTP {status}")))
            }
        }
    }
}

/// The JSON body of an HTTP answer (any status: the relay answers errors as replies too).
fn http_reply(raw: &[u8]) -> Result<Value, Failure> {
    let at = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or((USAGE_ERROR, "ui: not an HTTP answer".to_string()))?;
    let head = String::from_utf8_lossy(raw.get(..at).unwrap_or_default());
    let body = raw.get(at.saturating_add(4)..).unwrap_or_default();
    serde_json::from_slice(body).map_err(|_| {
        let status = head.lines().next().unwrap_or("");
        (USAGE_ERROR, format!("ui: {status}"))
    })
}

/// Writes a screenshot reply's PNG to `path` and leaves its size in the reply instead.
fn save_png(reply: &mut Value, path: &Path) -> Result<(), Failure> {
    let Some(b64) = reply.pointer("/result/png").and_then(Value::as_str) else {
        return Ok(());
    };
    let png = unbase64(b64).ok_or((INVALID, "ui: the screenshot is not base64".to_string()))?;
    std::fs::write(path, &png).map_err(|e| (USAGE_ERROR, format!("{}: {e}", path.display())))?;
    if let Some(r) = reply.get_mut("result").and_then(Value::as_object_mut) {
        r.insert(
            "png".into(),
            json!(format!("{} bytes written to {}", png.len(), path.display())),
        );
    }
    Ok(())
}

/// Standard base64 (RFC 4648 §4), padding optional.
fn unbase64(s: &str) -> Option<Vec<u8>> {
    const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let s = s.trim_end_matches('=');
    let mut out = Vec::with_capacity(s.len().saturating_mul(3) / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = u32::try_from(B64.iter().position(|x| *x == c)?).ok()?;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((acc >> bits) & 0xff).ok()?);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn requests_from_the_command_line() {
        assert_eq!(request(&args(&["ui.inspect"])).unwrap()["method"], "ui.inspect");
        let r = request(&args(&["ui.set", r#"{"view":"model"}"#])).unwrap();
        assert_eq!(r["params"]["view"], "model");
        let r = request(&args(&["command", "measure.add", r#"{"table":"T"}"#])).unwrap();
        assert_eq!(
            r["params"],
            json!({"command": "measure.add", "params": {"table": "T"}})
        );
        let r = request(&args(&["command", "file.save"])).unwrap();
        assert_eq!(r["params"], json!({"command": "file.save"}));
        assert!(request(&[]).is_err());
        assert!(request(&args(&["ui.set", "[1]"])).is_err());
        assert!(request(&args(&["a", "{}", "{}"])).is_err());
    }

    #[test]
    fn token_files_name_a_loopback_target() {
        let t = Target::from_json(&json!({"addr": "127.0.0.1:7990", "token": "t"})).unwrap();
        assert!(matches!(t, Target::Socket { .. }));
        let t = Target::from_json(&json!({"url": "http://127.0.0.1:81/api/control", "token": "t"})).unwrap();
        assert_eq!(
            t,
            Target::Http {
                addr: "127.0.0.1:81".parse().unwrap(),
                path: "/api/control".into(),
                token: "t".into()
            }
        );
        for bad in [
            json!({"addr": "10.0.0.1:7990", "token": "t"}),
            json!({"addr": "127.0.0.1:7990"}),
            json!({"addr": "127.0.0.1:7990", "token": ""}),
            json!({"url": "https://127.0.0.1:81/api/control", "token": "t"}),
            json!({"url": "http://example.com/api/control", "token": "t"}),
            json!({"token": "t"}),
        ] {
            assert!(Target::from_json(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_server_relay_is_addressed_by_its_viewer_url() {
        let v = "0123456789abcdef0123456789abcdef";
        for (url, ws) in [
            ("https://h/w/sales/view/", None),
            ("https://h/w/sales/view/app/", None),
            ("https://h/w/sales/view/api/control", None),
            ("https://h", Some("sales")),
            ("https://h/", Some("sales")),
        ] {
            assert_eq!(
                serve_url(url, ws, None).unwrap(),
                "https://h/w/sales/view/api/control",
                "{url}"
            );
        }
        assert_eq!(
            serve_url("http://127.0.0.1:7878", Some("ws1"), Some(v)).unwrap(),
            format!("http://127.0.0.1:7878/w/ws1/view/api/control?v={v}")
        );
        for (url, ws, view) in [
            ("ftp://h/w/a/view/", None, None),
            ("https://h", None, None),
            ("https://h", Some("../x"), None),
            ("https://h/w/a/view/?v=1", None, None),
            ("https://h/w/a/view/", None, Some("ABC")),
        ] {
            assert!(serve_url(url, ws, view).is_err(), "{url} {ws:?} {view:?}");
        }
        // a serve token file and a workspace: the route, the token
        let dir = std::env::temp_dir().join(format!("bicraft-ui-serve-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("control-serve.json");
        std::fs::write(
            &file,
            r#"{"kind": "serve", "url": "http://127.0.0.1:7878", "token": "t0", "pid": 1}"#,
        )
        .unwrap();
        let at = Where {
            workspace: Some("ws1".into()),
            ..Where::default()
        };
        let Target::Serve { url, token, .. } = Target::read(&file, &at).unwrap() else {
            panic!("a server target")
        };
        assert_eq!(
            (url.as_str(), token.as_str()),
            ("http://127.0.0.1:7878/w/ws1/view/api/control", "t0")
        );
        assert!(Target::read(&file, &Where::default()).is_err(), "no workspace");
        // a desktop file with server options is a usage error
        std::fs::write(&file, r#"{"addr": "127.0.0.1:1", "token": "t"}"#).unwrap();
        assert!(Target::read(&file, &at).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn http_answers_and_screenshots() {
        let r =
            http_reply(b"HTTP/1.1 401 Unauthorized\r\nX: y\r\n\r\n{\"ok\":false,\"error\":\"no\"}").unwrap();
        assert_eq!(r["ok"], false);
        assert!(http_reply(b"HTTP/1.1 502 Bad\r\n\r\n<html>").is_err());
        assert!(http_reply(b"garbage").is_err());
        assert_eq!(unbase64("Zm9vYmFy").unwrap(), b"foobar");
        assert!(unbase64("**").is_none());
        let dir = std::env::temp_dir().join(format!("bicraft-ui-cmd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("shot.png");
        let mut reply = json!({"ok": true, "result": {"png": "iVBORw==", "width": 1}});
        save_png(&mut reply, &out).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), [0x89, b'P', b'N', b'G']);
        assert!(
            reply["result"]["png"]
                .as_str()
                .unwrap()
                .contains("4 bytes written")
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
