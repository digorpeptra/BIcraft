//! L5: the live viewer page and its loopback server (no engine dependency: `bicraft serve`
//! serves the same page from the MCP HTTP endpoint, [`server_page`]). Serves an embedded web page
//! on localhost and pushes session
//! snapshots (docs/view-snapshot.md) to it as they are published.
//!
//! The server is plain `std`: a loopback-only [`TcpListener`], one short-lived thread per
//! connection (capped), and a tiny HTTP/1.1 `GET`-only handler. Routes:
//!
//! - `GET /`: the embedded single-page viewer (`page.html`, compiled in).
//! - `GET /app/`: the browser build of the egui app (docs/webui.md, [`webui`]), with its files
//!   `/app/boot.js`, `/app/bicraft_web.js` and `/app/bicraft_web_bg.wasm` (gzip when accepted,
//!   revalidated by entity tag); 404 in a build without it.
//! - `GET /api/state`: the latest snapshot JSON (`null` before the first publish).
//! - `GET /api/events`: Server-Sent Events. Each publish sends `event: snapshot` whose data is
//!   `{"revision","seq","snapshot"}`; `snapshot` is omitted when the JSON exceeds 1 MiB (the page
//!   then fetches `/api/state`). Comment keepalives are sent every 15 s.
//! - `POST /api/open-in-desktop`: opens the session's saved file with the OS default application
//!   (Power BI Desktop), when the server was started with an opener ([`start_with`]). Only the
//!   path the front end set with [`ViewHandle::set_open_target`] is opened, never one from the
//!   request (which has no body). The request must carry the per-run random token embedded in the
//!   page (`X-Bicraft-Token`), and a browser's `Origin` / `Sec-Fetch-Site` must be this server's.
//! - `POST /api/interact`: a reader action (a slicer click, a data-point selection, a page change,
//!   a button), when the server was started with interactions ([`ViewOptions::interactions`]).
//!   The JSON body (at most 64 KiB, `Content-Type: application/json`) is handed to the front end,
//!   which owns the session ([`ViewHandle::next_interaction`]); its answer is the response, and the
//!   new state reaches the page as the next snapshot. Same protections as the open-in-Desktop
//!   route: the per-run token, `Origin` and `Sec-Fetch-Site`.
//! - `GET /api/asset/<id>`: an image the snapshot references (report wallpaper, backgrounds, image
//!   visuals), published with [`ViewHandle::publish_with_assets`]. `id` is a content hash
//!   (16-64 lowercase hex digits), so responses are cached as immutable. Only PNG, JPEG, GIF, WebP
//!   and SVG are served, each with its canonical `Content-Type`, `nosniff`, a sandboxing CSP (an
//!   SVG opened directly runs no script) and `Cross-Origin-Resource-Policy: same-origin` (other
//!   sites cannot embed them). Assets are capped per file, in number and in total.
//!
//! Every request must carry a localhost `Host` header (DNS-rebinding protection). Hostile clients
//! are bounded by request size, header count, deadlines, write timeouts and connection caps.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

use std::collections::HashMap;
use std::io::{self, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

// The HTTP plumbing is shared with the MCP HTTP endpoint (`bicraft-http`).
#[cfg(test)]
use bicraft_http::head_end;
use bicraft_http::{
    COMMON_HEADERS, Connections, Handler, Limits, ReadError, Slot, Status, finish, host_allowed,
    random_token, respond_status, same_token, write_response,
};
use serde_json::{Value, json};

pub mod control;
pub mod webui;

/// The embedded viewer page.
const PAGE: &str = include_str!("page.html");
/// The page's slot for the open-in-Desktop token (empty: no button).
const TOKEN_SLOT: &str = r#"<meta name="bicraft-open-token" content="">"#;
/// The header carrying the token.
pub const TOKEN_HEADER: &str = "X-Bicraft-Token";
/// The open-in-Desktop route.
pub const OPEN_ROUTE: &str = "/api/open-in-desktop";
/// The interaction route.
pub const INTERACT_ROUTE: &str = "/api/interact";
/// The page's slot for the interaction token (empty: the page is read-only).
const INTERACT_SLOT: &str = r#"<meta name="bicraft-interact-token" content="">"#;
/// Largest interaction body.
pub const MAX_BODY_BYTES: usize = 64 * 1024;

/// The page's slot for the private view id of server mode (empty: `bicraft view`).
const VIEW_ID_SLOT: &str = r#"<meta name="bicraft-view-id" content="">"#;
/// The page's slot for server mode's download link (empty: no link).
const DOWNLOAD_SLOT: &str = r#"<meta name="bicraft-download" content="">"#;

/// The page as `bicraft serve` serves it under `/w/<id>/view/` (docs/kubernetes.md, "Viewer"):
/// the interaction token (`None`: read-only page), the page's private view id, and a download
/// link of the saved file instead of the "Open in Power BI Desktop" button. Both values must be
/// 32 lowercase hex digits ([`random_token`]); anything else leaves its slot empty.
pub fn server_page(interact_token: Option<&str>, view_id: &str) -> String {
    let hex = |t: &str| {
        t.len() == 32
            && t.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    };
    let mut page = PAGE.to_string();
    if let Some(t) = interact_token.filter(|t| hex(t)) {
        page = page.replacen(
            INTERACT_SLOT,
            &format!(r#"<meta name="bicraft-interact-token" content="{t}">"#),
            1,
        );
    }
    if hex(view_id) {
        page = page.replacen(
            VIEW_ID_SLOT,
            &format!(r#"<meta name="bicraft-view-id" content="{view_id}">"#),
            1,
        );
    }
    page.replacen(DOWNLOAD_SLOT, r#"<meta name="bicraft-download" content="1">"#, 1)
}

/// The page's `Content-Security-Policy` (also in server mode).
pub const PAGE_CONTENT_SECURITY_POLICY: &str = PAGE_CSP;
/// The headers of an asset response: inert as a document, same-origin only.
pub const ASSET_CONTENT_SECURITY_POLICY: &str = ASSET_CSP;
/// The `Cache-Control` of an asset (content-addressed, immutable).
pub const ASSET_CACHE_CONTROL: &str = ASSET_CACHE;

/// The served `Content-Type` of an accepted image type; `None` for anything else.
pub fn asset_mime(mime: &str) -> Option<&'static str> {
    canonical_mime(mime)
}

/// The SSE `event: snapshot` message (`snapshot` inline when at most 1 MiB; `None`: never
/// inline, the page then fetches the state itself).
pub fn snapshot_event(snapshot: Option<&str>, revision: u64, seq: u64) -> Vec<u8> {
    match snapshot {
        Some(json) => event_message(json, revision, seq),
        None => format!("event: snapshot\nid: {seq}\ndata: {{\"revision\":{revision},\"seq\":{seq}}}\n\n")
            .into_bytes(),
    }
}
/// How long a request waits for the front end's answer.
const INTERACT_TIMEOUT: Duration = Duration::from_secs(30);
/// Interactions queued for the front end at most (others get 503).
const MAX_QUEUED: usize = 16;
/// Minimum time between two launches (a double click opens one Desktop).
const OPEN_COOLDOWN: Duration = Duration::from_secs(3);

/// Opens a file with the OS default application (`bicraft_engine::open_with_default_app` in
/// the CLI; a recorder in tests).
pub type Opener = Arc<dyn Fn(&Path) -> Result<(), String> + Send + Sync>;

/// Options of [`start_with`].
#[derive(Default, Clone)]
pub struct ViewOptions {
    /// Enables `POST /api/open-in-desktop` and the page's "Open in Power BI Desktop" button.
    pub opener: Option<Opener>,
    /// Enables `POST /api/interact` (the page becomes a report reader: slicers, cross-filtering,
    /// navigation); the front end answers with [`ViewHandle::next_interaction`].
    pub interactions: bool,
    /// Enables the UI control channel's relay to the browser app (`/api/control`, [`control`]):
    /// an agent with [`ViewHandle::control_token`] drives the app page.
    pub control: bool,
}

/// A reader action posted by the page, waiting for the front end's answer.
pub struct Interaction {
    /// The request body (a JSON object: `{action, …}`).
    pub action: Value,
    reply: std::sync::mpsc::SyncSender<Result<Value, String>>,
}

impl Interaction {
    /// Answers the page: `Ok` is sent as `{ok: true, result}` (200), `Err` as `{ok: false,
    /// error}` (400). Publish the new snapshot before answering, so the page's next state is
    /// already on its way.
    pub fn respond(self, result: Result<Value, String>) {
        let _ = self.reply.try_send(result);
    }
}

/// The interaction queue between connection threads and the front end.
struct InteractQueue {
    tx: std::sync::mpsc::SyncSender<Interaction>,
    rx: Mutex<std::sync::mpsc::Receiver<Interaction>>,
}

/// The open-in-Desktop action: its token, opener and current target.
struct OpenAction {
    token: String,
    opener: Opener,
    /// The session's saved file, and whether the session has edits that are not in it.
    target: Mutex<(Option<PathBuf>, bool)>,
    /// Last successful launch (cooldown).
    last: Mutex<Option<Instant>>,
}

/// Concurrent connections (all kinds, SSE included).
const MAX_CONNECTIONS: usize = 64;
/// Concurrent SSE subscribers (kept below `MAX_CONNECTIONS` so plain requests still get through).
const MAX_SUBSCRIBERS: usize = 32;
/// Whole-request deadline (slow-loris protection): the head's ([`Limits::default`]: 8 KiB, 64
/// headers, 2 KiB targets, 5 s) and the body's.
const REQUEST_DEADLINE: Duration = Duration::from_secs(5);
/// The request headers the viewer reads: each may appear once.
const UNIQUE_HEADERS: &[&str] = &[
    "host",
    "if-none-match",
    "origin",
    TOKEN_HEADER,
    "sec-fetch-site",
    "content-length",
    "content-type",
    "accept-encoding",
    "authorization",
];
/// A client that does not drain its socket within this time is dropped.
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);
const KEEPALIVE: Duration = Duration::from_secs(15);
/// Snapshots up to this size are sent inline in the SSE event.
const INLINE_LIMIT: usize = 1024 * 1024;
/// Largest asset served, and the total kept for one snapshot; assets past either cap are dropped.
pub const MAX_ASSET_BYTES: usize = 16 << 20;
pub const MAX_ASSETS_TOTAL_BYTES: usize = 128 << 20;
/// Most assets kept for one snapshot.
pub const MAX_ASSETS: usize = 1024;

/// The latest published snapshot, serialized once.
struct Latest {
    json: Arc<str>,
    revision: u64,
    /// Publish counter (0 = nothing published yet).
    seq: u64,
}

/// An image served at `/api/asset/<id>` (see [`ViewHandle::publish_with_assets`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    /// Content hash: 16 to 64 lowercase hex digits.
    pub id: String,
    /// `image/png`, `image/jpeg`, `image/gif`, `image/webp` or `image/svg+xml`.
    pub mime: String,
    pub bytes: Arc<[u8]>,
}

/// Assets by id, swapped as a whole on publish.
type Assets = Arc<HashMap<String, (&'static str, Arc<[u8]>)>>;

struct Shared {
    /// The page as served (with the token when the open action or interactions are enabled).
    page: Arc<str>,
    /// The browser app's page (`/app/`, docs/webui.md), with the same tokens.
    app_page: Arc<str>,
    open: Option<OpenAction>,
    /// The per-run token of the POST routes (`None`: neither is enabled).
    token: Option<String>,
    interact: Option<InteractQueue>,
    /// The control channel's relay (`ViewOptions::control`).
    control: Option<control::Relay>,
    latest: Mutex<Latest>,
    assets: Mutex<Assets>,
    changed: Condvar,
    stop: AtomicBool,
    connections: Arc<Connections>,
    subscribers: AtomicUsize,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Latest> {
        self.latest.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn assets(&self) -> Assets {
        Arc::clone(&self.assets.lock().unwrap_or_else(|e| e.into_inner()))
    }

    fn stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }
}

/// A running viewer. Dropping it stops the server.
pub struct ViewHandle {
    addr: SocketAddr,
    shared: Arc<Shared>,
    accept: Option<JoinHandle<()>>,
}

impl ViewHandle {
    /// Address the server listens on (port 0 at start = OS-chosen).
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// URL to open in a browser.
    pub fn url(&self) -> String {
        format!("http://{}/", self.addr)
    }

    /// Publishes a new snapshot; connected pages update immediately.
    ///
    /// Never blocks on clients: subscribers pick up the latest snapshot on their own threads, and
    /// intermediate snapshots are skipped for clients that fall behind.
    pub fn publish(&self, snapshot: Value) {
        let Ok(json) = serde_json::to_string(&snapshot) else {
            return;
        };
        let json: Arc<str> = Arc::from(json);
        let old = {
            let mut latest = self.shared.lock();
            latest.seq = latest.seq.saturating_add(1);
            latest.revision = snapshot
                .get("revision")
                .and_then(Value::as_u64)
                .unwrap_or(latest.seq);
            std::mem::replace(&mut latest.json, json)
        };
        self.shared.changed.notify_all();
        drop(old);
    }

    /// Replaces the served assets, then publishes `snapshot`, so a page never sees a snapshot
    /// whose images are not served yet. Returns how many assets were accepted: invalid ids,
    /// unsupported types, duplicates and assets past [`MAX_ASSET_BYTES`], [`MAX_ASSETS`] or
    /// [`MAX_ASSETS_TOTAL_BYTES`] are dropped.
    pub fn publish_with_assets(&self, snapshot: Value, assets: impl IntoIterator<Item = Asset>) -> usize {
        let accepted = self.publish_assets(assets);
        self.publish(snapshot);
        accepted
    }

    /// Sets the file `POST /api/open-in-desktop` opens: the session's saved file (`None`: nothing
    /// to open), and whether the session has edits that are not in it. Call it on every publish.
    /// No effect when the server has no opener.
    pub fn set_open_target(&self, path: Option<PathBuf>, unsaved: bool) {
        if let Some(open) = &self.shared.open {
            *open.target.lock().unwrap_or_else(|e| e.into_inner()) = (path, unsaved);
        }
    }

    /// The per-run token `POST /api/open-in-desktop` requires (also embedded in the page);
    /// `None` when the server has no opener.
    pub fn open_token(&self) -> Option<&str> {
        self.shared.open.as_ref().map(|o| o.token.as_str())
    }

    /// The next reader action posted by the page, waiting at most `timeout`; `None` when there
    /// is none (or interactions are disabled). Answer it with [`Interaction::respond`].
    pub fn next_interaction(&self, timeout: Duration) -> Option<Interaction> {
        let q = self.shared.interact.as_ref()?;
        let rx = q.rx.lock().unwrap_or_else(|e| e.into_inner());
        if timeout.is_zero() {
            rx.try_recv().ok()
        } else {
            rx.recv_timeout(timeout).ok()
        }
    }

    /// The per-run token `POST /api/interact` requires (also embedded in the page); `None` when
    /// interactions are disabled.
    pub fn interact_token(&self) -> Option<&str> {
        self.shared.interact.as_ref().and(self.shared.token.as_deref())
    }

    /// The token an agent sends to `POST /api/control` (`Authorization: Bearer`); `None` when
    /// the control channel is off. Never put it in a page or a log.
    pub fn control_token(&self) -> Option<&str> {
        self.shared.control.as_ref().map(|c| c.token.as_str())
    }

    /// App pages connected to the control channel now.
    pub fn control_pages(&self) -> usize {
        self.shared.control.as_ref().map_or(0, control::Relay::pages)
    }

    /// Whether `POST /api/interact` is enabled.
    pub fn interactions_enabled(&self) -> bool {
        self.shared.interact.is_some()
    }

    /// Replaces the served assets (the previous set is dropped); see
    /// [`ViewHandle::publish_with_assets`].
    pub fn publish_assets(&self, assets: impl IntoIterator<Item = Asset>) -> usize {
        let mut map = HashMap::new();
        let mut total = 0usize;
        for a in assets {
            if map.len() >= MAX_ASSETS {
                break;
            }
            let Some(mime) = canonical_mime(&a.mime) else {
                continue;
            };
            if !valid_asset_id(&a.id) || a.bytes.len() > MAX_ASSET_BYTES || map.contains_key(&a.id) {
                continue;
            }
            let Some(next) = total
                .checked_add(a.bytes.len())
                .filter(|t| *t <= MAX_ASSETS_TOTAL_BYTES)
            else {
                continue;
            };
            total = next;
            map.insert(a.id, (mime, a.bytes));
        }
        let accepted = map.len();
        let old = {
            let mut guard = self.shared.assets.lock().unwrap_or_else(|e| e.into_inner());
            std::mem::replace(&mut *guard, Arc::new(map))
        };
        drop(old);
        accepted
    }
}

/// Asset ids are content hashes: 16 to 64 lowercase hex digits, nothing else.
pub fn valid_asset_id(id: &str) -> bool {
    (16..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The served `Content-Type` of an accepted image type; `None` for anything else.
fn canonical_mime(mime: &str) -> Option<&'static str> {
    const TYPES: [&str; 5] = [
        "image/png",
        "image/jpeg",
        "image/gif",
        "image/webp",
        "image/svg+xml",
    ];
    let mime = mime.trim();
    TYPES.into_iter().find(|t| t.eq_ignore_ascii_case(mime))
}

impl Drop for ViewHandle {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        {
            // Taking the lock orders the stop flag with subscribers that are about to wait.
            let _guard = self.shared.lock();
            self.shared.changed.notify_all();
        }
        if let Some(c) = &self.shared.control {
            c.wake();
        }
        if let Some(accept) = self.accept.take() {
            let _ = accept.join();
        }
    }
}

/// Starts the viewer on `addr` (use 127.0.0.1 only; port 0 picks a free port).
///
/// Non-loopback addresses are refused with [`io::ErrorKind::InvalidInput`].
pub fn start(addr: SocketAddr) -> io::Result<ViewHandle> {
    start_with(addr, ViewOptions::default())
}

/// [`start`] with options: `opener` enables the "Open in Power BI Desktop" button.
pub fn start_with(addr: SocketAddr, options: ViewOptions) -> io::Result<ViewHandle> {
    let token = (options.opener.is_some() || options.interactions || options.control).then(random_token);
    let open = options
        .opener
        .zip(token.clone())
        .map(|(opener, token)| OpenAction {
            token,
            opener,
            target: Mutex::new((None, false)),
            last: Mutex::new(None),
        });
    let mut page = PAGE.to_string();
    if let Some(o) = &open {
        page = page.replacen(
            TOKEN_SLOT,
            &format!(r#"<meta name="bicraft-open-token" content="{}">"#, o.token),
            1,
        );
    }
    let interact = match (&token, options.interactions) {
        (Some(t), true) => {
            page = page.replacen(
                INTERACT_SLOT,
                &format!(r#"<meta name="bicraft-interact-token" content="{t}">"#),
                1,
            );
            let (tx, rx) = std::sync::mpsc::sync_channel(MAX_QUEUED);
            Some(InteractQueue {
                tx,
                rx: Mutex::new(rx),
            })
        }
        _ => None,
    };
    let mut app_page = webui::app_page(
        interact.as_ref().and(token.as_deref()),
        open.as_ref().map(|o| o.token.as_str()),
        None,
    );
    let relay = match (&token, options.control) {
        (Some(t), true) => {
            app_page = webui::with_control(&app_page, t);
            Some(control::Relay::new(random_token()))
        }
        _ => None,
    };
    let app_page: Arc<str> = Arc::from(app_page);
    let page: Arc<str> = Arc::from(page);
    if !addr.ip().is_loopback() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("the viewer only listens on loopback addresses, not {}", addr.ip()),
        ));
    }
    let listener = TcpListener::bind(addr)?;
    listener.set_nonblocking(true)?;
    let addr = listener.local_addr()?;
    let shared = Arc::new(Shared {
        page,
        app_page,
        open,
        token,
        interact,
        control: relay,
        latest: Mutex::new(Latest {
            json: Arc::from("null"),
            revision: 0,
            seq: 0,
        }),
        assets: Mutex::new(Arc::new(HashMap::new())),
        changed: Condvar::new(),
        stop: AtomicBool::new(false),
        connections: Connections::new(MAX_CONNECTIONS, None),
        subscribers: AtomicUsize::new(0),
    });
    let accept_shared = Arc::clone(&shared);
    let accept = thread::Builder::new()
        .name("bicraft-view".into())
        .spawn(move || {
            let conn_shared = Arc::clone(&accept_shared);
            let handler: Handler = Arc::new(move |stream, _peer| handle_connection(stream, &conn_shared));
            bicraft_http::accept_loop(
                listener,
                &accept_shared.stop,
                &accept_shared.connections,
                "bicraft-view-conn",
                &handler,
            );
        })?;
    Ok(ViewHandle {
        addr,
        shared,
        accept: Some(accept),
    })
}

// ---------------------------------------------------------------------------------------------
// Connection handling (accept loop, caps and HTTP plumbing: `bicraft-http`)

fn handle_connection(mut stream: TcpStream, shared: &Shared) {
    if stream.set_write_timeout(Some(WRITE_TIMEOUT)).is_err() {
        return;
    }
    let (request, rest) = match read_request(&mut stream) {
        Ok(request) => request,
        Err(ReadError::Close) => return,
        Err(ReadError::Respond(status)) => {
            respond_status(&mut stream, status, &[]);
            finish(stream);
            return;
        }
    };
    if !request.host.as_deref().is_some_and(host_allowed) {
        let status = if request.host.is_none() {
            Status::BadRequest
        } else {
            Status::Forbidden
        };
        respond_status(&mut stream, status, &[]);
        finish(stream);
        return;
    }
    if request.path == OPEN_ROUTE {
        if request.method == "POST" {
            open_in_desktop(&mut stream, shared, &request);
        } else {
            respond_status(&mut stream, Status::MethodNotAllowed, &[("Allow", "POST")]);
        }
        finish(stream);
        return;
    }
    if [control::ROUTE, control::EVENTS_ROUTE, control::REPLY_ROUTE].contains(&request.path.as_str()) {
        control_route(&mut stream, shared, &request, rest);
        if request.path == control::EVENTS_ROUTE {
            let _ = stream.shutdown(Shutdown::Both);
            return;
        }
        finish(stream);
        return;
    }
    if request.path == INTERACT_ROUTE {
        if request.method == "POST" {
            interact(&mut stream, shared, &request, rest);
        } else {
            respond_status(&mut stream, Status::MethodNotAllowed, &[("Allow", "POST")]);
        }
        finish(stream);
        return;
    }
    if request.method != "GET" {
        respond_status(&mut stream, Status::MethodNotAllowed, &[("Allow", "GET")]);
        finish(stream);
        return;
    }
    match request.path.as_str() {
        "/" | "/index.html" => {
            let _ = write_response(
                &mut stream,
                Status::Ok,
                "text/html; charset=utf-8",
                &[
                    ("Cache-Control", "no-store"),
                    ("Content-Security-Policy", PAGE_CSP),
                ],
                shared.page.as_bytes(),
            );
        }
        "/app" => {
            respond_status(&mut stream, Status::PermanentRedirect, &[("Location", "/app/")]);
        }
        "/app/" | "/app/index.html" => {
            let _ = if webui::available() {
                webui::write_page(&mut stream, &shared.app_page)
            } else {
                webui::write_not_bundled(&mut stream)
            };
        }
        path if path.starts_with("/app/") => {
            let name = path.trim_start_matches("/app/");
            match webui::file(
                name,
                request.accept_encoding.as_deref(),
                request.if_none_match.as_deref(),
            ) {
                Some(f) => {
                    let _ = f.write(&mut stream);
                }
                None if !webui::available() => {
                    let _ = webui::write_not_bundled(&mut stream);
                }
                None => respond_status(&mut stream, Status::NotFound, &[]),
            }
        }
        "/api/state" => {
            let json = Arc::clone(&shared.lock().json);
            let _ = write_response(
                &mut stream,
                Status::Ok,
                "application/json; charset=utf-8",
                &[("Cache-Control", "no-store")],
                json.as_bytes(),
            );
        }
        path if path.starts_with("/api/asset/") => {
            serve_asset(&mut stream, shared, path, request.if_none_match.as_deref());
        }
        "/api/events" => {
            serve_events(&mut stream, shared);
            // The stream is open-ended; no lingering drain.
            let _ = stream.shutdown(Shutdown::Both);
            return;
        }
        _ => respond_status(&mut stream, Status::NotFound, &[]),
    }
    finish(stream);
}

const PAGE_CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; \
                        img-src data: 'self'; connect-src 'self'; base-uri 'none'; form-action 'none'; \
                        frame-ancestors 'none'";

/// Asset responses: inert when opened as a document (an SVG runs no script and loads nothing).
const ASSET_CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; img-src data:; sandbox";
const ASSET_CACHE: &str = "public, max-age=31536000, immutable";

fn serve_asset(stream: &mut TcpStream, shared: &Shared, path: &str, if_none_match: Option<&str>) {
    let id = path.strip_prefix("/api/asset/").unwrap_or_default();
    if !valid_asset_id(id) {
        respond_status(stream, Status::BadRequest, &[]);
        return;
    }
    let assets = shared.assets();
    let Some((mime, bytes)) = assets.get(id) else {
        respond_status(stream, Status::NotFound, &[("Cache-Control", "no-store")]);
        return;
    };
    let etag = format!("\"{id}\"");
    let headers = [
        ("Cache-Control", ASSET_CACHE),
        ("ETag", etag.as_str()),
        ("Content-Security-Policy", ASSET_CSP),
        ("Cross-Origin-Resource-Policy", "same-origin"),
        ("Content-Disposition", "inline"),
    ];
    let matched = if_none_match.is_some_and(|v| {
        v.split(',')
            .map(|t| t.trim().trim_start_matches("W/"))
            .any(|t| t == etag || t == "*")
    });
    let (status, body): (Status, &[u8]) = if matched {
        (Status::NotModified, &[])
    } else {
        (Status::Ok, bytes)
    };
    let _ = write_response(stream, status, mime, &headers, body);
}

/// `POST /api/open-in-desktop`: opens the session's saved file. Checks, in order: the action is
/// enabled (404), the request comes from this server's page (403: token, `Origin`,
/// `Sec-Fetch-Site`), there is a file (409), no launch in the last few seconds (429). Answers
/// JSON `{ok, path?, unsaved?, error?}`.
fn open_in_desktop(stream: &mut TcpStream, shared: &Shared, request: &Request) {
    let reply = |stream: &mut TcpStream, status: Status, body: Value| {
        let _ = write_response(
            stream,
            status,
            "application/json; charset=utf-8",
            &[("Cache-Control", "no-store")],
            body.to_string().as_bytes(),
        );
    };
    let Some(open) = &shared.open else {
        reply(
            stream,
            Status::NotFound,
            json!({"ok": false, "error": "the open-in-Desktop button is disabled"}),
        );
        return;
    };
    if let Err(why) = same_origin(request, &open.token) {
        reply(stream, Status::Forbidden, json!({"ok": false, "error": why}));
        return;
    }
    let (path, unsaved) = open.target.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let Some(path) = path else {
        reply(
            stream,
            Status::Conflict,
            json!({"ok": false, "error": "nothing to open: save the session as a .pbix or .pbip file first"}),
        );
        return;
    };
    {
        let mut last = open.last.lock().unwrap_or_else(|e| e.into_inner());
        if last.is_some_and(|t| t.elapsed() < OPEN_COOLDOWN) {
            drop(last);
            reply(
                stream,
                Status::TooManyRequests,
                json!({"ok": false, "error": "already opening; try again in a few seconds"}),
            );
            return;
        }
        // Claimed before launching, so concurrent clicks start one Desktop.
        *last = Some(Instant::now());
    }
    match (open.opener)(&path) {
        Ok(()) => reply(
            stream,
            Status::Ok,
            json!({"ok": true, "path": path.display().to_string(), "unsaved": unsaved}),
        ),
        Err(e) => {
            *open.last.lock().unwrap_or_else(|e| e.into_inner()) = None;
            reply(stream, Status::InternalError, json!({"ok": false, "error": e}));
        }
    }
}

/// The request comes from this server's own page: the right token, and, when the client sends
/// them (browsers do), an `Origin` naming this host and `Sec-Fetch-Site: same-origin`.
fn same_origin(request: &Request, token: &str) -> Result<(), &'static str> {
    if !request.token.as_deref().is_some_and(|t| same_token(t, token)) {
        return Err("missing or wrong token");
    }
    if let Some(origin) = &request.origin {
        let host = request.host.as_deref().unwrap_or_default();
        let matches = origin
            .strip_prefix("http://")
            .is_some_and(|o| o.eq_ignore_ascii_case(host) && host_allowed(o));
        if !matches {
            return Err("cross-origin request refused");
        }
    }
    if let Some(site) = &request.fetch_site
        && !site.eq_ignore_ascii_case("same-origin")
    {
        return Err("cross-site request refused");
    }
    Ok(())
}

/// `POST /api/interact`: hands a reader action to the front end and answers with its result.
/// Checks, in order: interactions are enabled (404), the request comes from this server's page
/// (403), a JSON body of at most [`MAX_BODY_BYTES`] (415, 411, 413, 400), the front end takes it
/// (503 when its queue is full or it does not answer in time).
fn interact(stream: &mut TcpStream, shared: &Shared, request: &Request, rest: Vec<u8>) {
    let reply = |stream: &mut TcpStream, status: Status, body: Value| {
        let _ = write_response(
            stream,
            status,
            "application/json; charset=utf-8",
            &[("Cache-Control", "no-store")],
            body.to_string().as_bytes(),
        );
    };
    let (Some(queue), Some(token)) = (&shared.interact, &shared.token) else {
        reply(
            stream,
            Status::NotFound,
            json!({"ok": false, "error": "interactions are disabled"}),
        );
        return;
    };
    if let Err(why) = same_origin(request, token) {
        reply(stream, Status::Forbidden, json!({"ok": false, "error": why}));
        return;
    }
    let json_type = request.content_type.as_deref().is_some_and(|t| {
        t.split(';')
            .next()
            .is_some_and(|m| m.trim().eq_ignore_ascii_case("application/json"))
    });
    if !json_type {
        reply(
            stream,
            Status::UnsupportedMedia,
            json!({"ok": false, "error": "the body must be application/json"}),
        );
        return;
    }
    let Some(length) = request.content_length else {
        reply(
            stream,
            Status::LengthRequired,
            json!({"ok": false, "error": "Content-Length is required"}),
        );
        return;
    };
    if length > MAX_BODY_BYTES {
        reply(
            stream,
            Status::PayloadTooLarge,
            json!({"ok": false, "error": "the body is too large"}),
        );
        return;
    }
    let body = match bicraft_http::read_body(stream, rest, length, REQUEST_DEADLINE) {
        Ok(b) => b,
        Err(status) => {
            respond_status(stream, status, &[]);
            return;
        }
    };
    let action: Value = match serde_json::from_slice(&body) {
        Ok(v @ Value::Object(_)) => v,
        _ => {
            reply(
                stream,
                Status::BadRequest,
                json!({"ok": false, "error": "the body must be a JSON object"}),
            );
            return;
        }
    };
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    if queue.tx.try_send(Interaction { action, reply: tx }).is_err() {
        reply(
            stream,
            Status::Unavailable,
            json!({"ok": false, "error": "busy; try again"}),
        );
        return;
    }
    match rx.recv_timeout(INTERACT_TIMEOUT) {
        Ok(Ok(v)) => reply(stream, Status::Ok, json!({"ok": true, "result": v})),
        Ok(Err(e)) => reply(stream, Status::BadRequest, json!({"ok": false, "error": e})),
        Err(_) => reply(
            stream,
            Status::Unavailable,
            json!({"ok": false, "error": "the viewer did not answer in time"}),
        ),
    }
}

/// The control channel's routes ([`control`]): 404 when it is off; the agent's POST, the page's
/// stream (GET) and the page's reply (POST, with the page token and this origin).
fn control_route(stream: &mut TcpStream, shared: &Shared, request: &Request, rest: Vec<u8>) {
    let (Some(relay), Some(token)) = (&shared.control, &shared.token) else {
        let body = json!({"ok": false, "error": "the control channel is off (bicraft view --control)"});
        let _ = write_response(
            stream,
            Status::NotFound,
            "application/json; charset=utf-8",
            &[("Cache-Control", "no-store")],
            body.to_string().as_bytes(),
        );
        return;
    };
    let ask = control::Ask {
        token: request
            .authorization
            .as_deref()
            .and_then(|a| a.strip_prefix("Bearer ")),
        origin: request.origin.as_deref(),
        fetch_site: request.fetch_site.as_deref(),
        content_type: request.content_type.as_deref(),
        content_length: request.content_length,
    };
    let method = if request.path == control::EVENTS_ROUTE {
        "GET"
    } else {
        "POST"
    };
    if request.method != method {
        respond_status(stream, Status::MethodNotAllowed, &[("Allow", method)]);
        return;
    }
    match request.path.as_str() {
        control::ROUTE => {
            relay.agent(stream, &ask, rest, REQUEST_DEADLINE, &control::To::default());
        }
        control::EVENTS_ROUTE => relay.events(stream, None, &|| shared.stopped()),
        _ => match same_origin(request, token) {
            Err(why) => {
                let body = json!({"ok": false, "error": why});
                let _ = write_response(
                    stream,
                    Status::Forbidden,
                    "application/json; charset=utf-8",
                    &[("Cache-Control", "no-store")],
                    body.to_string().as_bytes(),
                );
            }
            Ok(()) => relay.page_reply(stream, &ask, rest, REPLY_DEADLINE, None),
        },
    }
}

/// How long the page has to send a reply body (a screenshot of a large window).
const REPLY_DEADLINE: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------------------------
// Server-Sent Events

enum Next {
    Stop,
    Keepalive,
    Snapshot { json: Arc<str>, revision: u64, seq: u64 },
}

fn serve_events(stream: &mut TcpStream, shared: &Shared) {
    let Some(_subscriber) = Slot::acquire(&shared.subscribers, MAX_SUBSCRIBERS) else {
        respond_status(stream, Status::Unavailable, &[("Retry-After", "5")]);
        return;
    };
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream; charset=utf-8\r\nCache-Control: no-store\r\n\
         X-Accel-Buffering: no\r\n{COMMON_HEADERS}\r\nretry: 1000\n\n"
    );
    if stream.write_all(head.as_bytes()).is_err() {
        return;
    }
    let mut sent: Option<u64> = None;
    let mut last_write = Instant::now();
    loop {
        let next = wait_next(shared, sent, last_write);
        let message = match next {
            Next::Stop => return,
            Next::Keepalive => b": keepalive\n\n".to_vec(),
            Next::Snapshot { json, revision, seq } => {
                sent = Some(seq);
                event_message(&json, revision, seq)
            }
        };
        if stream.write_all(&message).is_err() || stream.flush().is_err() {
            return;
        }
        last_write = Instant::now();
    }
}

fn wait_next(shared: &Shared, sent: Option<u64>, last_write: Instant) -> Next {
    let mut latest = shared.lock();
    loop {
        if shared.stopped() {
            return Next::Stop;
        }
        if sent != Some(latest.seq) {
            return Next::Snapshot {
                json: Arc::clone(&latest.json),
                revision: latest.revision,
                seq: latest.seq,
            };
        }
        let Some(left) = KEEPALIVE
            .checked_sub(last_write.elapsed())
            .filter(|d| !d.is_zero())
        else {
            return Next::Keepalive;
        };
        latest = match shared.changed.wait_timeout(latest, left) {
            Ok((guard, _)) => guard,
            Err(poisoned) => poisoned.into_inner().0,
        };
    }
}

/// `event: snapshot` with the snapshot inline when it is small enough. Serialized JSON never
/// contains raw newlines, so a single `data:` line is enough.
fn event_message(json: &str, revision: u64, seq: u64) -> Vec<u8> {
    let inline = json.len() <= INLINE_LIMIT;
    let mut out = Vec::with_capacity(if inline { json.len().saturating_add(96) } else { 96 });
    out.extend_from_slice(
        format!("event: snapshot\nid: {seq}\ndata: {{\"revision\":{revision},\"seq\":{seq}").as_bytes(),
    );
    if inline {
        out.extend_from_slice(b",\"snapshot\":");
        out.extend_from_slice(json.as_bytes());
    }
    out.extend_from_slice(b"}\n\n");
    out
}

// ---------------------------------------------------------------------------------------------
// HTTP plumbing (the generic parts are `bicraft-http`'s)

#[derive(Debug, PartialEq, Eq)]
struct Request {
    method: String,
    /// Path without the query string.
    path: String,
    host: Option<String>,
    if_none_match: Option<String>,
    accept_encoding: Option<String>,
    origin: Option<String>,
    /// `X-Bicraft-Token`.
    token: Option<String>,
    /// `Sec-Fetch-Site`.
    fetch_site: Option<String>,
    /// `Content-Length` (POST bodies).
    content_length: Option<usize>,
    content_type: Option<String>,
    /// `Authorization` (the control channel's agent).
    authorization: Option<String>,
}

impl Request {
    fn from_head(head: bicraft_http::Head) -> Self {
        let get = |name: &str| head.header(name).map(str::to_string);
        Request {
            host: get("host"),
            if_none_match: get("if-none-match"),
            accept_encoding: get("accept-encoding"),
            origin: get("origin"),
            token: get(TOKEN_HEADER),
            fetch_site: get("sec-fetch-site"),
            content_type: get("content-type"),
            authorization: get("authorization"),
            content_length: head.content_length,
            method: head.method,
            path: head.path,
        }
    }
}

/// The request head, and the bytes read past it (the start of a body).
fn read_request(stream: &mut TcpStream) -> Result<(Request, Vec<u8>), ReadError> {
    bicraft_http::read_request(stream, &Limits::default(), UNIQUE_HEADERS)
        .map(|(head, rest)| (Request::from_head(head), rest))
}

#[cfg(test)]
fn parse_head(head: &[u8]) -> Result<Request, ReadError> {
    bicraft_http::parse_head(head, &Limits::default(), UNIQUE_HEADERS).map(Request::from_head)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts() {
        for ok in [
            "localhost",
            "localhost:8080",
            "LOCALHOST.",
            "app.localhost:1",
            "127.0.0.1",
            "127.1.2.3:80",
            "[::1]",
            "[::1]:9",
        ] {
            assert!(host_allowed(ok), "{ok}");
        }
        for bad in [
            "",
            "evil.com",
            "evil.com:80",
            "localhost.evil.com",
            "localhost:99999",
            "localhost:",
            "localhost:x",
            "10.0.0.1",
            "0.0.0.0",
            "[::2]",
            "[::1",
            "[::1]x",
            ".localhost",
            "127.0.0.1.evil.com",
        ] {
            assert!(!host_allowed(bad), "{bad}");
        }
    }

    #[test]
    fn head_parsing() {
        let req =
            parse_head(b"GET /api/state?x=1 HTTP/1.1\r\nHost: localhost:1\r\nAccept: */*\r\n\r\n").unwrap();
        assert_eq!(req.method, "GET");
        assert_eq!(req.path, "/api/state");
        assert_eq!(req.host.as_deref(), Some("localhost:1"));
        assert_eq!(
            parse_head(b"GET / HTTP/2\r\n\r\n"),
            Err(ReadError::Respond(Status::VersionNotSupported))
        );
        assert_eq!(
            parse_head(b"GET  / HTTP/1.1\r\n\r\n"),
            Err(ReadError::Respond(Status::BadRequest))
        );
        assert_eq!(
            parse_head(b"GET http://x/ HTTP/1.1\r\n\r\n"),
            Err(ReadError::Respond(Status::BadRequest))
        );
        assert_eq!(
            parse_head(b"G(T / HTTP/1.1\r\n\r\n"),
            Err(ReadError::Respond(Status::BadRequest))
        );
        assert_eq!(
            parse_head(b"GET / HTTP/1.1\r\nno colon\r\n\r\n"),
            Err(ReadError::Respond(Status::BadRequest))
        );
        assert_eq!(
            parse_head(b"GET / HTTP/1.1\r\nHost: a\r\nHost: b\r\n\r\n"),
            Err(ReadError::Respond(Status::BadRequest))
        );
        assert_eq!(
            parse_head(b"\xff\xfe / HTTP/1.1\r\n\r\n"),
            Err(ReadError::Respond(Status::BadRequest))
        );
        assert_eq!(head_end(b"GET / HTTP/1.1\r\n\r\nrest"), Some(18));
        assert_eq!(head_end(b"GET / HTTP/1.1\n\n"), Some(16));
        assert_eq!(head_end(b"GET / HTTP/1.1\r\n"), None);
    }

    #[test]
    fn inline_and_reference_events() {
        let small = String::from_utf8(event_message("{\"a\":1}", 3, 4)).unwrap();
        assert_eq!(
            small,
            "event: snapshot\nid: 4\ndata: {\"revision\":3,\"seq\":4,\"snapshot\":{\"a\":1}}\n\n"
        );
        let big = "x".repeat(INLINE_LIMIT + 1);
        let big = String::from_utf8(event_message(&big, 3, 4)).unwrap();
        assert_eq!(
            big,
            "event: snapshot\nid: 4\ndata: {\"revision\":3,\"seq\":4}\n\n"
        );
    }

    #[test]
    fn the_page_has_the_token_slot_once() {
        assert_eq!(PAGE.matches(TOKEN_SLOT).count(), 1);
        assert_eq!(PAGE.matches(INTERACT_SLOT).count(), 1);
        assert_eq!(PAGE.matches(VIEW_ID_SLOT).count(), 1);
        assert_eq!(PAGE.matches(DOWNLOAD_SLOT).count(), 1);
    }

    #[test]
    fn server_pages_fill_their_slots_with_hex_only() {
        let (t, v) = (random_token(), random_token());
        let page = server_page(Some(&t), &v);
        assert!(page.contains(&format!(r#"<meta name="bicraft-interact-token" content="{t}">"#)));
        assert!(page.contains(&format!(r#"<meta name="bicraft-view-id" content="{v}">"#)));
        assert!(page.contains(r#"<meta name="bicraft-download" content="1">"#));
        // never the open-in-Desktop token in server mode
        assert_eq!(page.matches(TOKEN_SLOT).count(), 1);
        // anything but 32 lowercase hex digits leaves the slot empty
        let page = server_page(Some("\"><script>"), "ABCDEF0123456789ABCDEF0123456789");
        assert_eq!(page.matches(INTERACT_SLOT).count(), 1);
        assert_eq!(page.matches(VIEW_ID_SLOT).count(), 1);
        assert_eq!(server_page(None, &v).matches(INTERACT_SLOT).count(), 1);
        assert_eq!(
            String::from_utf8(snapshot_event(None, 3, 4)).unwrap(),
            "event: snapshot\nid: 4\ndata: {\"revision\":3,\"seq\":4}\n\n"
        );
    }

    #[test]
    fn tokens_are_random_hex() {
        let (a, b) = (random_token(), random_token());
        assert_eq!(a.len(), 32);
        assert!(
            a.bytes()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
        assert_ne!(a, b);
        assert!(same_token(&a, &a.clone()));
        assert!(!same_token(&a, &b));
        assert!(!same_token(&a, &a[..31]));
        assert!(!same_token("", &a));
    }

    #[test]
    fn same_origin_checks() {
        let req = |origin: Option<&str>, token: Option<&str>, site: Option<&str>| Request {
            method: "POST".into(),
            path: OPEN_ROUTE.into(),
            host: Some("127.0.0.1:8080".into()),
            if_none_match: None,
            accept_encoding: None,
            origin: origin.map(Into::into),
            token: token.map(Into::into),
            fetch_site: site.map(Into::into),
            content_length: None,
            content_type: None,
            authorization: None,
        };
        let t = "0123456789abcdef0123456789abcdef";
        assert!(same_origin(&req(None, Some(t), None), t).is_ok());
        assert!(
            same_origin(
                &req(Some("http://127.0.0.1:8080"), Some(t), Some("same-origin")),
                t
            )
            .is_ok()
        );
        for bad in [
            req(None, None, None),
            req(None, Some(""), None),
            req(None, Some("0123456789abcdef0123456789abcdee"), None),
            req(Some("http://evil.com"), Some(t), None),
            req(Some("null"), Some(t), None),
            req(Some("https://127.0.0.1:8080"), Some(t), None),
            req(Some("http://127.0.0.1:8081"), Some(t), None),
            req(Some("http://localhost:8080"), Some(t), None),
            req(None, Some(t), Some("cross-site")),
            req(None, Some(t), Some("same-site")),
        ] {
            assert!(same_origin(&bad, t).is_err(), "{bad:?}");
        }
        let parsed = parse_head(
            b"POST /api/open-in-desktop HTTP/1.1\r\nHost: localhost\r\nOrigin: http://localhost\r\n\
              x-bicraft-token: abc\r\nSec-Fetch-Site: same-origin\r\n\r\n",
        )
        .unwrap();
        assert_eq!(parsed.origin.as_deref(), Some("http://localhost"));
        assert_eq!(parsed.token.as_deref(), Some("abc"));
        assert_eq!(parsed.fetch_site.as_deref(), Some("same-origin"));
        // a repeated token or origin is refused
        assert_eq!(
            parse_head(b"POST / HTTP/1.1\r\nX-Bicraft-Token: a\r\nX-Bicraft-Token: b\r\n\r\n"),
            Err(ReadError::Respond(Status::BadRequest))
        );
        assert_eq!(
            parse_head(b"POST / HTTP/1.1\r\nOrigin: a\r\norigin: b\r\n\r\n"),
            Err(ReadError::Respond(Status::BadRequest))
        );
    }

    #[test]
    fn refuses_non_loopback() {
        let err = start("0.0.0.0:0".parse().unwrap()).err().unwrap();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }
}
