//! The browser source's protocol, without the browser: what the page gives the app, the URLs of
//! the viewer API relative to the page, and how answers are read. Pure functions, so they are
//! tested natively (`cargo test -p bicraft-web`); `web.rs` only moves bytes with `fetch` and
//! `EventSource`.
//!
//! The routes are the HTML page's (`docs/view-snapshot.md`, `docs/kubernetes.md`, "Viewer"):
//!
//! ```text
//! page                          API root          view query
//! http://127.0.0.1:P/app/       /api/…            (none: bicraft-cli view has one view)
//! https://h/w/sales/view/app/   /w/sales/view/api/…   ?v=<view id from the page>
//! ```

use bicraft_ui::remote::{Failure, MAX_ASSET_BYTES, MAX_SNAPSHOT_BYTES, classify, message};
use serde_json::Value;

/// What the server put in the page's `<meta>` slots (`bicraft_view::webui::app_page`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    /// `X-Bicraft-Token` of `api/interact` (`None`: a read-only viewer).
    pub interact_token: Option<String>,
    /// The open-in-Desktop token (`bicraft-cli view` only; not used by the app yet).
    pub open_token: Option<String>,
    /// The private view (`bicraft-cli serve` only).
    pub view: Option<String>,
    /// The page token of the UI control channel's relay (`bicraft-cli view --control`): the app takes
    /// the relayed requests and posts its replies with it. `None`: no control channel.
    pub control_token: Option<String>,
}

fn hex32(t: &str) -> Option<String> {
    let t = t.trim();
    (t.len() == 32 && t.bytes().all(|b| b.is_ascii_hexdigit())).then(|| t.to_string())
}

impl Config {
    /// Reads the slots with `meta(name)` (the `content` of `<meta name=…>`); a value that is not
    /// 32 hex digits counts as missing.
    pub fn from_meta(meta: impl Fn(&str) -> Option<String>) -> Config {
        let get = |name: &str| meta(name).as_deref().and_then(hex32);
        Config {
            interact_token: get("bicraft-interact-token"),
            open_token: get("bicraft-open-token"),
            view: get("bicraft-view-id"),
            control_token: get("bicraft-control-token"),
        }
    }
}

/// The API root of the page at `pathname`: the path up to the last `app/` segment
/// (`/w/sales/view/app/` → `/w/sales/view/`, `/app/index.html` → `/`); `/` when there is none.
pub fn api_root(pathname: &str) -> String {
    let mut root = match pathname.rfind("/app/") {
        Some(i) => pathname.get(..=i).unwrap_or("/").to_string(),
        None => match pathname.strip_suffix("/app") {
            Some(p) => format!("{p}/"),
            None => "/".to_string(),
        },
    };
    if !root.starts_with('/') {
        root.insert(0, '/');
    }
    root
}

/// Whether the page's query asks for the WebGL2 backend (`?webgl`, `?a=1&webgl`).
pub fn wants_webgl(search: &str) -> bool {
    search
        .trim_start_matches('?')
        .split('&')
        .any(|p| p == "webgl" || p.starts_with("webgl="))
}

/// The workspace of a server page (`/w/<id>/view/app/`): the title, Open > Browse (its file
/// routes) and the commands (its MCP endpoint); `None` on `bicraft-cli view` (`/app/`), which has
/// none of them.
pub fn workspace_of(pathname: &str) -> Option<String> {
    let rest = pathname.strip_prefix("/w/")?;
    let (id, tail) = rest.split_once('/')?;
    (tail.starts_with("view") && bicraft_ui::remote::valid_workspace(id) && !id.bytes().all(|b| b == b'.'))
        .then(|| id.to_string())
}

/// The viewer API's routes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route<'a> {
    State,
    Events,
    Interact,
    Asset(&'a str),
    /// The control channel's request stream and replies (`bicraft-cli view --control`).
    ControlEvents,
    ControlReply,
}

/// The URL of `route`, with `?v=<view>` on a server.
pub fn url(root: &str, route: &Route<'_>, cfg: &Config) -> String {
    let path = match route {
        Route::State => "api/state".to_string(),
        Route::Events => "api/events".to_string(),
        Route::Interact => "api/interact".to_string(),
        Route::Asset(id) => format!("api/asset/{id}"),
        Route::ControlEvents => "api/control/events".to_string(),
        Route::ControlReply => "api/control/reply".to_string(),
    };
    match &cfg.view {
        Some(v) => format!("{root}{path}?v={v}"),
        None => format!("{root}{path}"),
    }
}

/// The largest body accepted from `route` (as the desktop's client).
pub fn limit(route: &Route<'_>) -> u64 {
    match route {
        Route::State => MAX_SNAPSHOT_BYTES,
        Route::Asset(_) => MAX_ASSET_BYTES,
        Route::Events | Route::Interact | Route::ControlEvents | Route::ControlReply => 1 << 20,
    }
}

/// A snapshot answer: the JSON, or the failure (a 404 is the view gone).
pub fn read_state(status: u16, body: &[u8]) -> Result<Value, Failure> {
    classify(status, body, true)?;
    serde_json::from_slice(body).map_err(|e| Failure::Protocol(e.to_string()))
}

/// An interaction's answer as the window takes it: the action's result, or the reason it was
/// refused (worded as the desktop's remote client words it).
pub fn read_interaction(status: u16, body: &[u8]) -> Result<Value, String> {
    if status == 400 {
        return Err(Failure::Status(400, message(body)).to_string());
    }
    classify(status, body, true).map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_slice(body).map_err(|e| Failure::Protocol(e.to_string()).to_string())?;
    if v.get("ok").and_then(Value::as_bool) == Some(false) {
        return Err(Failure::Status(status, message(body)).to_string());
    }
    Ok(v.get("result").cloned().unwrap_or(Value::Null))
}

/// A relayed control request (the data of an SSE `control` event, `{"seq", "request", "doc"?}`):
/// its sequence number, the request, checked as the desktop's socket checks a line
/// (`bicraft_ui::control::parse`), and the workspace file whose document tab it runs in (a
/// request `bicraft-cli serve` relays to a document view). `Err` holds the seq and the reply to send
/// when the request is not one.
pub type ControlEvent = (u64, bicraft_ui::control::Request, Option<String>);

/// Reads a relayed control request ([`ControlEvent`]).
pub fn control_event(data: &str) -> Result<ControlEvent, Option<(u64, Value)>> {
    let v: Value = serde_json::from_str(data).map_err(|_| None)?;
    let seq = v.get("seq").and_then(Value::as_u64).ok_or(None)?;
    let request = v.get("request").ok_or(None)?.to_string();
    let doc = match v.get("doc") {
        None | Some(Value::Null) => None,
        Some(Value::String(f)) if !f.is_empty() && f.len() <= 1024 => Some(f.clone()),
        Some(_) => {
            return Err(Some((
                seq,
                bicraft_ui::control::err(&Value::Null, "the relayed `doc` is not a file name"),
            )));
        }
    };
    match bicraft_ui::control::parse(request.as_bytes()) {
        Ok((r, _)) => Ok((seq, r, doc)),
        Err(e) => Err(Some((seq, bicraft_ui::control::err(&Value::Null, e)))),
    }
}

/// The body of `api/control/reply`.
pub fn control_reply(seq: u64, reply: &Value) -> String {
    serde_json::json!({"seq": seq, "reply": reply}).to_string()
}

/// The snapshot carried by an SSE `snapshot` event, when it is inline (`bicraft-cli view` sends
/// snapshots up to 1 MiB inline; otherwise, and on a server, the app fetches `api/state`).
pub fn inline_snapshot(data: &str) -> Option<Value> {
    match serde_json::from_str::<Value>(data) {
        Ok(Value::Object(mut o)) => o.remove("snapshot"),
        _ => None,
    }
}

/// The caller's role on a server (`view.role` of the snapshot: `reader` or `editor`).
pub fn role(snap: &Value) -> Option<String> {
    snap.get("view")
        .and_then(|v| v.get("role"))
        .and_then(Value::as_str)
        .map(|r| r.chars().take(20).collect())
}

/// What a 404 on the view means in a browser: the page's view is gone and only a new page load
/// gets another one.
pub fn wording(f: &Failure) -> String {
    match f {
        Failure::ViewGone => "the server forgot this view (a restart?): reload the page".into(),
        other => other.to_string(),
    }
}

/// The URL of `POST …/api/session`: with the page's own view (`?v=`), so the server knows which
/// page holds the document view (the control channel addresses it there).
pub fn session_url(root: &str, page_view: Option<&str>) -> String {
    match page_view.and_then(hex32) {
        Some(v) => format!("{root}api/session?v={v}"),
        None => format!("{root}api/session"),
    }
}

/// The body of `POST …/api/session` that opens a document view on workspace file `file` (a
/// browser document tab, docs/webui.md, "Document tabs").
pub fn session_body(file: &str) -> String {
    serde_json::json!({ "file": file }).to_string()
}

/// A document view the server opened: its own view id and interaction token, and the caller's
/// access (`editor`, `reader`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocView {
    pub cfg: Config,
    pub access: String,
}

/// Why a document view could not open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The server's limit or a 503 (its caps on open views, its memory, a busy workspace): the
    /// tab closes and the tab strip shows the message.
    Limit(String),
    /// Anything else (the file cannot be opened, credentials): an error in the tab.
    Error(String),
}

/// `POST …/api/session {file}`'s answer.
pub fn read_session(status: u16, body: &[u8]) -> Result<DocView, Refusal> {
    if status == 429 || status == 503 {
        return Err(Refusal::Limit(message(body)));
    }
    classify(status, body, false).map_err(|e| Refusal::Error(wording(&e)))?;
    let v: Value = serde_json::from_slice(body)
        .map_err(|e| Refusal::Error(Failure::Protocol(e.to_string()).to_string()))?;
    let get = |k: &str| v.get(k).and_then(Value::as_str).and_then(hex32);
    let (Some(view), Some(token)) = (get("view"), get("token")) else {
        return Err(Refusal::Error(
            Failure::Protocol("the view has no id or token".into()).to_string(),
        ));
    };
    let access = v
        .get("access")
        .and_then(Value::as_str)
        .unwrap_or("reader")
        .chars()
        .take(20)
        .collect();
    Ok(DocView {
        cfg: Config {
            interact_token: Some(token),
            open_token: None,
            view: Some(view),
            control_token: None,
        },
        access,
    })
}

/// The URL of `POST …/api/close?v=<view>` (a closed tab's view goes on the server).
pub fn close_url(root: &str, cfg: &Config) -> Option<String> {
    cfg.view.as_ref().map(|v| format!("{root}api/close?v={v}"))
}

/// The document views this page holds open on the server, so that the page going away
/// (`pagehide`) releases them now instead of leaving them to `--view-idle` (docs/webui.md,
/// "Document tabs in the browser"). Each entry is a view's close request: its URL and token.
#[derive(Debug, Default)]
pub struct Held(std::collections::BTreeMap<String, String>);

impl Held {
    /// A tab's view opened (`cfg` from [`read_session`]).
    pub fn hold(&mut self, root: &str, cfg: &Config) {
        if let Some(url) = close_url(root, cfg) {
            self.0.insert(url, cfg.interact_token.clone().unwrap_or_default());
        }
    }

    /// A tab closed: the close request of its view, once.
    pub fn release(&mut self, root: &str, cfg: &Config) -> Option<(String, String)> {
        let url = close_url(root, cfg)?;
        self.0.remove_entry(&url)
    }

    /// The page is hidden for good (`pagehide`; `persisted`: kept in the back-forward cache, it
    /// may come back, so its views stay): the close requests of every view still held.
    pub fn page_hidden(&mut self, persisted: bool) -> Vec<(String, String)> {
        if persisted {
            return Vec::new();
        }
        std::mem::take(&mut self.0).into_iter().collect()
    }
}

/// The MCP endpoint of workspace `ws` for the page's commands: the workspace's session
/// (`/w/<ws>/mcp`), or a document view's own (`/w/<ws>/mcp/view/<view>`).
pub fn mcp_url(ws: &str, doc_view: Option<&str>) -> String {
    match doc_view {
        Some(v) => format!("/w/{ws}/mcp/view/{v}"),
        None => format!("/w/{ws}/mcp"),
    }
}

/// The download URL of workspace file `rel` (`/w/<ws>/files/<rel>`, each segment
/// percent-encoded): the exports the page hands to the user. `None` for a path that leaves the
/// workspace (`..`, an absolute path, a drive) or is empty.
pub fn file_url(ws: &str, rel: &str) -> Option<String> {
    let rel = rel.replace('\\', "/");
    let segs: Vec<&str> = rel.split('/').filter(|s| !s.is_empty() && *s != ".").collect();
    if segs.is_empty() || rel.starts_with('/') || segs.iter().any(|s| *s == ".." || s.contains(':')) {
        return None;
    }
    let mut out = format!("/w/{ws}/files");
    for s in segs {
        out.push('/');
        for b in s.bytes() {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                out.push(char::from(b));
            } else {
                out.push_str(&format!("%{b:02X}"));
            }
        }
    }
    Some(out)
}

/// The server's cap on document views per user (`view.limits.views_per_user` of a snapshot).
pub fn views_per_user(snap: &Value) -> Option<usize> {
    let n = snap.pointer("/view/limits/views_per_user")?.as_u64()?;
    usize::try_from(n.min(1024)).ok()
}

/// Coalesces refetches: one request at a time, and one more after it when something changed
/// meanwhile (a burst of events is one fetch, never a queue).
#[derive(Debug, Default)]
pub struct Refetch {
    busy: bool,
    again: bool,
}

impl Refetch {
    /// A change was seen: `true` when the caller should fetch now.
    pub fn want(&mut self) -> bool {
        if self.busy {
            self.again = true;
            false
        } else {
            self.busy = true;
            true
        }
    }

    /// A fetch ended: `true` when the caller should fetch again.
    pub fn done(&mut self) -> bool {
        if self.again {
            self.again = false;
            true
        } else {
            self.busy = false;
            false
        }
    }
}

/// The reconnection delay of the event stream after `failures` failures in a row (0.5 s
/// doubling to 15 s, as the HTML page).
pub fn backoff_ms(failures: u32) -> u32 {
    500u32.saturating_mul(1u32 << failures.min(5)).min(15_000)
}

/// Whether the page stops the browser's own action for a key press (`code` is the physical key,
/// `KeyN`): F5 alone (the browser's reload; the app's Refresh or Reload, docs/webui.md "Keyboard
/// in the browser") and Alt+N (the app's New in a browser, which never hands Ctrl+N to a page).
/// Ctrl+F5, Shift+F5 and Ctrl+R still reload the page.
pub fn stops_browser_key(key: &str, code: &str, ctrl: bool, shift: bool, alt: bool, meta: bool) -> bool {
    let none = !ctrl && !shift && !alt && !meta;
    (key == "F5" && none)
        || (code == "KeyN" && alt && !ctrl && !shift && !meta)
        // Ctrl+D is Duplicate (the browser's bookmark dialog otherwise)
        || (code == "KeyD" && (ctrl || meta) && !shift && !alt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const V: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn download_urls_stay_in_the_workspace() {
        assert_eq!(
            file_url("sales", "out/Report 1.pdf").as_deref(),
            Some("/w/sales/files/out/Report%201.pdf")
        );
        assert_eq!(file_url("s", "é.csv").as_deref(), Some("/w/s/files/%C3%A9.csv"));
        for bad in ["", "/", "../x.pdf", "a/../../x", "/etc/passwd", "C:/x.pdf", "c:x"] {
            assert_eq!(file_url("s", bad), None, "{bad}");
        }
    }

    #[test]
    fn the_workspace_of_the_page() {
        assert_eq!(workspace_of("/w/sales/view/app/").as_deref(), Some("sales"));
        assert_eq!(workspace_of("/w/sales/view/app"), Some("sales".into()));
        for none in [
            "/app/",
            "/",
            "/w/../view/app/",
            "/w/sales/files/",
            "/w/a b/view/app/",
            "",
        ] {
            assert_eq!(workspace_of(none), None, "{none}");
        }
    }

    #[test]
    fn config_from_the_page() {
        let c = Config::from_meta(|n| match n {
            "bicraft-interact-token" => Some(V.into()),
            "bicraft-view-id" => Some(format!(" {V} ")),
            "bicraft-open-token" => Some("\"><script>".into()),
            _ => None,
        });
        assert_eq!(c.interact_token.as_deref(), Some(V));
        assert_eq!(c.view.as_deref(), Some(V));
        assert_eq!(c.open_token, None);
        assert_eq!(c.control_token, None);
        let c = Config::from_meta(|n| (n == "bicraft-control-token").then(|| V.into()));
        assert_eq!(c.control_token.as_deref(), Some(V));
        assert_eq!(Config::from_meta(|_| Some(String::new())), Config::default());
    }

    #[test]
    fn urls_relative_to_the_page() {
        assert_eq!(api_root("/app/"), "/");
        assert_eq!(api_root("/app/index.html"), "/");
        assert_eq!(api_root("/app"), "/");
        assert_eq!(api_root("/w/sales/view/app/"), "/w/sales/view/");
        assert_eq!(api_root("/prefix/w/a/view/app/index.html"), "/prefix/w/a/view/");
        assert_eq!(api_root("/elsewhere"), "/");
        assert_eq!(api_root(""), "/");
        let local = Config::default();
        assert_eq!(url("/", &Route::State, &local), "/api/state");
        assert_eq!(url("/", &Route::Asset("ab12"), &local), "/api/asset/ab12");
        let server = Config {
            view: Some(V.into()),
            ..Config::default()
        };
        assert_eq!(
            url("/w/s/view/", &Route::Interact, &server),
            format!("/w/s/view/api/interact?v={V}")
        );
        assert_eq!(
            url("/w/s/view/", &Route::Events, &server),
            format!("/w/s/view/api/events?v={V}")
        );
        assert_eq!(workspace_of("/w/sales/view/app/").as_deref(), Some("sales"));
        assert_eq!(workspace_of("/app/"), None);
        assert_eq!(workspace_of("/w/../view/app/"), None);
        assert!(limit(&Route::State) > limit(&Route::Asset("x")));
        assert_eq!(url("/", &Route::ControlEvents, &local), "/api/control/events");
        assert_eq!(url("/", &Route::ControlReply, &local), "/api/control/reply");
        assert!(wants_webgl("?webgl") && wants_webgl("?a=1&webgl=1"));
        assert!(!wants_webgl("") && !wants_webgl("?nowebgl") && !wants_webgl("?x=webgl"));
    }

    #[test]
    fn answers() {
        assert_eq!(read_state(200, b"{\"a\":1}").unwrap(), json!({"a": 1}));
        assert_eq!(read_state(200, b"null").unwrap(), Value::Null);
        assert!(matches!(read_state(404, b""), Err(Failure::ViewGone)));
        assert!(matches!(read_state(401, b""), Err(Failure::Unauthorized)));
        assert!(matches!(read_state(200, b"{"), Err(Failure::Protocol(_))));
        assert!(wording(&Failure::ViewGone).contains("reload the page"));
        assert_eq!(
            read_interaction(200, br#"{"ok": true, "result": {"page": "p2"}}"#).unwrap(),
            json!({"page": "p2"})
        );
        let refused = read_interaction(400, br#"{"ok": false, "error": "no such visual"}"#).unwrap_err();
        assert!(refused.contains("no such visual"), "{refused}");
        let forbidden =
            read_interaction(403, br#"{"ok": false, "error": "missing or wrong token"}"#).unwrap_err();
        assert!(
            forbidden.contains("403") && forbidden.contains("token"),
            "{forbidden}"
        );
        assert!(read_interaction(200, b"not json").is_err());
        assert_eq!(
            inline_snapshot(r#"{"revision":1,"seq":2,"snapshot":{"x":1}}"#),
            Some(json!({"x": 1}))
        );
        assert_eq!(inline_snapshot(r#"{"revision":1,"seq":2}"#), None);
        assert_eq!(inline_snapshot("garbage"), None);
        assert_eq!(
            role(&json!({"view": {"role": "editor"}})).as_deref(),
            Some("editor")
        );
        assert_eq!(role(&json!({})), None);
    }

    #[test]
    fn document_views() {
        assert_eq!(
            session_body("reports/a \"b\".pbix"),
            r#"{"file":"reports/a \"b\".pbix"}"#
        );
        let ok = format!(r#"{{"version":1,"view":"{V}","token":"{V}","access":"editor","file":"a.pbix"}}"#);
        let d = read_session(200, ok.as_bytes()).unwrap();
        assert_eq!(d.cfg.view.as_deref(), Some(V));
        assert_eq!(d.cfg.interact_token.as_deref(), Some(V));
        assert_eq!(d.access, "editor");
        // the server's limit and a busy server: a message for the tab strip
        let limit = read_session(
            429,
            br#"{"error":{"code":-31001,"message":"too many open files: at most 8 per user"}}"#,
        );
        assert!(
            matches!(&limit, Err(Refusal::Limit(m)) if m.contains("at most 8")),
            "{limit:?}"
        );
        assert!(matches!(read_session(503, b"{}"), Err(Refusal::Limit(_))));
        // anything else is an error of the tab
        assert!(
            matches!(read_session(409, br#"{"error":{"message":"the file cannot be opened"}}"#), Err(Refusal::Error(m)) if m.contains("cannot be opened"))
        );
        assert!(matches!(read_session(401, b""), Err(Refusal::Error(_))));
        assert!(matches!(
            read_session(200, br#"{"view":"x","token":"y"}"#),
            Err(Refusal::Error(_))
        ));
        assert!(matches!(read_session(200, b"garbage"), Err(Refusal::Error(_))));
        let cfg = d.cfg;
        assert_eq!(
            close_url("/w/s/view/", &cfg),
            Some(format!("/w/s/view/api/close?v={V}"))
        );
        assert_eq!(close_url("/w/s/view/", &Config::default()), None);
        assert_eq!(mcp_url("s", None), "/w/s/mcp");
        assert_eq!(mcp_url("s", Some(V)), format!("/w/s/mcp/view/{V}"));
        assert_eq!(
            views_per_user(&json!({"view": {"limits": {"views_per_user": 3}}})),
            Some(3)
        );
        assert_eq!(views_per_user(&json!({"view": {}})), None);
    }

    #[test]
    fn a_hidden_page_releases_the_views_it_holds() {
        let view = |c: &str| Config {
            interact_token: Some(c.repeat(32)),
            view: Some(c.repeat(32)),
            ..Config::default()
        };
        let close = |c: &str| (format!("/w/s/view/api/close?v={}", c.repeat(32)), c.repeat(32));
        let (a, b) = (view("a"), view("b"));
        let mut held = Held::default();
        held.hold("/w/s/view/", &a);
        held.hold("/w/s/view/", &b);
        held.hold("/w/s/view/", &Config::default());
        // a closed tab's view is released once, by the tab
        assert_eq!(held.release("/w/s/view/", &a), Some(close("a")));
        assert_eq!(held.release("/w/s/view/", &a), None);
        // a page kept for the back button may come back: its views stay
        assert!(held.page_hidden(true).is_empty());
        // a page that goes releases what is left, once
        assert_eq!(held.page_hidden(false), vec![close("b")]);
        assert!(held.page_hidden(false).is_empty());
    }

    #[test]
    fn refetches_coalesce() {
        let mut r = Refetch::default();
        assert!(r.want());
        assert!(!r.want() && !r.want());
        // one more fetch for the whole burst, then idle
        assert!(r.done());
        assert!(!r.done());
        assert!(r.want());
        assert!(!r.done());
        assert_eq!(backoff_ms(0), 500);
        assert_eq!(backoff_ms(1), 1000);
        assert_eq!(backoff_ms(40), 15_000);
    }

    #[test]
    fn the_page_keeps_f5_and_alt_n() {
        assert!(stops_browser_key("F5", "F5", false, false, false, false));
        assert!(
            stops_browser_key("d", "KeyD", true, false, false, false),
            "Ctrl+D duplicates"
        );
        assert!(
            !stops_browser_key("D", "KeyD", true, true, false, false),
            "Ctrl+Shift+D is the browser's"
        );
        assert!(
            !stops_browser_key("F5", "F5", true, false, false, false),
            "Ctrl+F5 reloads"
        );
        assert!(
            !stops_browser_key("F5", "F5", false, true, false, false),
            "Shift+F5 reloads"
        );
        assert!(
            !stops_browser_key("r", "KeyR", true, false, false, false),
            "Ctrl+R reloads"
        );
        assert!(stops_browser_key("n", "KeyN", false, false, true, false));
        assert!(
            stops_browser_key("Dead", "KeyN", false, false, true, false),
            "Option+N on a Mac"
        );
        assert!(
            !stops_browser_key("n", "KeyN", true, false, false, false),
            "Ctrl+N is the browser's"
        );
        assert!(
            !stops_browser_key("n", "KeyN", false, false, false, false),
            "typing n"
        );
    }

    #[test]
    fn the_session_url_names_the_page() {
        let v = "0123456789abcdef0123456789abcdef";
        assert_eq!(
            session_url("/w/a/view/", Some(v)),
            format!("/w/a/view/api/session?v={v}")
        );
        assert_eq!(session_url("/w/a/view/", Some("x&y")), "/w/a/view/api/session");
        assert_eq!(session_url("/", None), "/api/session");
    }

    #[test]
    fn relayed_control_requests() {
        let (seq, r, doc) = control_event(r#"{"seq":7,"request":{"id":1,"method":"ui.inspect"}}"#).unwrap();
        assert_eq!(
            (seq, r.method.as_str(), r.id.clone(), doc),
            (7, "ui.inspect", serde_json::json!(1), None)
        );
        // addressed to a document view: its file's tab
        let (_, _, doc) =
            control_event(r#"{"seq":9,"request":{"method":"ui.inspect"},"doc":"reports/a.pbix"}"#).unwrap();
        assert_eq!(doc.as_deref(), Some("reports/a.pbix"));
        let Err(Some((10, _))) = control_event(r#"{"seq":10,"request":{"method":"m"},"doc":5}"#) else {
            panic!("a bad doc is refused")
        };
        // a request that is not one: the reply to send, with its seq
        let Err(Some((seq, reply))) = control_event(r#"{"seq":8,"request":{"id":1}}"#) else {
            panic!("refused")
        };
        assert_eq!(seq, 8);
        assert_eq!(reply["ok"], false);
        for junk in [
            "",
            "[]",
            "{\"request\":{}}",
            "{\"seq\":\"x\",\"request\":{}}",
            "{\"seq\":1}",
        ] {
            assert!(matches!(control_event(junk), Err(None)), "{junk}");
        }
        let body: Value = serde_json::from_str(&control_reply(3, &serde_json::json!({"ok": true}))).unwrap();
        assert_eq!(body, serde_json::json!({"seq": 3, "reply": {"ok": true}}));
    }
}
