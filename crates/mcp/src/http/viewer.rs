//! The live viewer in server mode (docs/kubernetes.md, "Viewer"; docs/auth.md, "The viewer in a
//! browser"): `bicraft-cli serve` serves the viewer page of each workspace at `/w/<id>/view/`, with the
//! same `Host` check, authentication and ACL as the MCP endpoint.
//!
//! | Request | Answer |
//! |---|---|
//! | `GET /w/<id>/view` | 308 to `/w/<id>/view/` |
//! | `GET /w/<id>/view/` | the page, with a new private view (id and interaction token embedded) |
//! | `GET /w/<id>/view/app/` | the browser app (docs/webui.md) with a new private view, as the page; its files under `app/` |
//! | `POST …/api/session` | bearer or proxy credentials only: a new private view for a native client, `{version, view, token, access}` |
//! | `GET …/api/state?v=<view>` | the view's snapshot (JSON) |
//! | `GET …/api/events?v=<view>` | SSE: an `event: snapshot` whenever the view should refetch |
//! | `GET …/api/asset/<hash>?v=<view>` | an image of the view's snapshot |
//! | `POST …/api/interact?v=<view>` | a slicer click, a selection, a page change on this view only |
//! | `POST /w/<id>/view/login` | with a bearer credential: a one-time login URL for a browser |
//! | `GET /w/<id>/view/login/<code>` | sets the viewer's login cookie, then 303 to the page |
//!
//! **Private views.** Each page load gets its own view state (slicers, selections, page), kept
//! here and bound to the identity that loaded the page; snapshots are computed from a shared
//! session with that state ([`Session::snapshot_view`]), so one viewer's interactions never
//! change another's view, and N viewers cost N small states, not N models. Editors see the
//! editor session (their MCP session, unsaved edits included); readers see the saved state
//! ([`super::readview`]). Nothing here writes the project: interactions are view state only, and
//! there is no open-in-Desktop route (the page offers a download of the saved file through the
//! file route instead).
//!
//! **Browser credentials.** Behind an auth proxy (oauth2-proxy, the primary browser path) every
//! request carries the proxy's identity headers. Without a proxy, a holder of a bearer credential
//! (an API token, an OIDC access token) asks `POST …/view/login` for a one-time URL (60 s, single
//! use) and opens it in the browser: it sets an `HttpOnly`, `SameSite=Lax` cookie (`Secure` when
//! the public URL is `https`) naming a server-side login (12 h at most, 2 h idle; lost at a
//! restart), valid for that workspace's viewer and the `GET`/`HEAD` file routes only.
//!
//! **CSRF.** `POST` routes need the page's interaction token in `X-Bicraft-Token`, and, when the
//! browser sends them, `Origin` equal to the public origin and `Sec-Fetch-Site: same-origin`.

use std::collections::HashMap;
use std::io::Write;
use std::net::{IpAddr, TcpStream};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use bicraft_auth::{Access, Denial, Identity};
use bicraft_engine::{Interactions, Session, Source};
use bicraft_http::{Head, Status};
use serde_json::{Value, json};

use super::ops::Reopen;
use super::readview::{Served, Target, ViewError, Views, identity_key};
use super::{Done, Reject, State, Workspace, public_base, unauthorized};

/// The login cookie's name.
pub const COOKIE: &str = "bicraft_view";
/// A one-time login URL is valid this long.
const CODE_TTL: Duration = Duration::from_secs(60);
/// A login lasts at most this long, and ends after this long unused.
const LOGIN_TTL: Duration = Duration::from_secs(12 * 3600);
const LOGIN_IDLE: Duration = Duration::from_secs(2 * 3600);
/// Pending login codes and logins at most.
const MAX_CODES: usize = 1024;
const MAX_LOGINS: usize = 4096;
/// How often an event stream asks for a refetch while visuals still wait for their data.
const PENDING_POLL: Duration = Duration::from_millis(250);
/// How often an event stream checks for a stop.
const STREAM_TICK: Duration = Duration::from_millis(250);
/// A view unused for this long may be dropped when the workspace has too many.
const VIEW_IDLE_MS: u64 = 3_600_000;

/// Images of a session's snapshot, by content hash.
pub(super) type AssetMap = Arc<HashMap<String, (&'static str, Arc<[u8]>)>>;

/// One browser page's private view.
pub(super) struct Viewer {
    /// The identity that loaded the page ([`identity_key`], `local` without authentication).
    owner: String,
    /// The page's interaction token (CSRF).
    token: String,
    /// The view state.
    ix: Interactions,
    /// Bumped by this view's own interactions.
    seq: u64,
    /// The last state left visuals without data (the stream asks for a refetch).
    pending: bool,
    /// Which session's assets the last state referenced.
    source: String,
    last_used: u64,
    /// A document view: the view's own session on one workspace file (an editor's), or the file a
    /// reader's view reads (its saved state). `None`: a page's view, which follows the
    /// workspace's session.
    doc: Option<DocView>,
    /// Event streams open on this view now (a document view with one is never idle).
    streams: usize,
    /// A document view: the app page's view that opened it (`POST …/api/session?v=<page>`), where
    /// the control channel addresses it, and the workspace file it opened (its tab's).
    parent: Option<String>,
    file: Option<String>,
}

/// What a document view reads (docs/mcp.md, "Document views").
#[derive(Clone)]
pub(super) enum DocView {
    /// An editor's view: a session of its own, opened on the file, edited and saved on its own.
    Edit(Arc<Workspace>),
    /// A reader's view: the saved state of this file (the clean editor session or the shared
    /// read-only copy, [`super::readview`]).
    Read(Reopen),
}

/// The viewer's browser logins.
#[derive(Default)]
pub(super) struct Logins {
    codes: HashMap<[u8; 32], Pending>,
    sessions: HashMap<[u8; 32], Login>,
}

struct Pending {
    identity: Identity,
    workspace: String,
    expires: Instant,
}

struct Login {
    identity: Identity,
    workspace: String,
    expires: Instant,
    idle_until: Instant,
}

impl Logins {
    fn purge(&mut self, now: Instant) {
        self.codes.retain(|_, p| p.expires > now);
        self.sessions.retain(|_, l| l.expires > now && l.idle_until > now);
    }
}

/// The caller of a viewer request.
pub(super) struct Caller {
    pub(super) key: String,
    pub(super) access: Access,
}

/// `/w/<id>/view` (rest `""`) or `/w/<id>/view/<rest>` (rest starting with `/`).
pub(super) fn route(path: &str) -> Option<(&str, &str)> {
    let after = path.strip_prefix("/w/")?;
    let (id, tail) = after.split_once('/')?;
    if !super::valid_workspace_id(id) {
        return None;
    }
    let rest = tail.strip_prefix("view")?;
    (rest.is_empty() || rest.starts_with('/')).then_some((id, rest))
}

fn err(status: Status, message: &str) -> Reject {
    Reject::plain(status, super::files::FILE_ERROR, message)
}

fn busy(state: &State, why: &str) -> Reject {
    state.metrics.busy.fetch_add(1, Ordering::Relaxed);
    let mut r = Reject::plain(Status::Unavailable, super::WORKSPACE_BUSY, why);
    r.extra.push(("Retry-After", "2".into()));
    r
}

fn view_error(state: &State, e: ViewError) -> Reject {
    match e {
        ViewError::Busy(why) => busy(state, why),
        ViewError::Failed(why) => err(Status::Conflict, &why),
    }
}

/// The value of the login cookie, when the request carries one.
fn cookie(head: &Head) -> Option<&str> {
    head.headers_named("cookie")
        .flat_map(|v| v.split(';'))
        .find_map(|kv| kv.trim().strip_prefix(COOKIE)?.strip_prefix('='))
        .map(str::trim)
}

fn hex64(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn hex32(s: &str) -> bool {
    s.len() == 32
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Authenticates a request to workspace `id` like the MCP endpoint does; with `allow_cookie`,
/// a request without credentials may use the viewer's login cookie instead. A workspace the
/// caller may not open answers 404, like a missing one.
pub(super) fn authenticate(
    state: &State,
    peer: IpAddr,
    head: &Head,
    host: &str,
    id: &str,
    allow_cookie: bool,
) -> Result<Caller, Reject> {
    let Some(auth) = &state.config.auth else {
        super::reqlog::caller("local");
        return Ok(Caller {
            key: "local".into(),
            access: Access::Editor,
        });
    };
    let base = public_base(auth, host);
    let resource = format!("{base}/w/{id}/mcp");
    super::reqlog::caller("unauthenticated");
    let not_found = || Reject::plain(Status::NotFound, super::WORKSPACE_NOT_FOUND, "no such workspace");
    let identity = match auth.authenticate(peer, |n| head.header(n), &[resource, base.clone()]) {
        Ok(identity) => {
            super::reqlog::caller(match identity.label().split(':').next() {
                Some("oidc") => "oidc",
                Some("proxy") => "proxy",
                Some("token") => "token",
                _ => "other",
            });
            identity
        }
        Err(Denial::Missing) if allow_cookie && cookie(head).is_some() => {
            let value = cookie(head).unwrap_or_default();
            let found = hex64(value)
                .then(|| {
                    let now = Instant::now();
                    let mut logins = state.logins.lock().unwrap_or_else(|e| e.into_inner());
                    let key = bicraft_auth::tokens::digest(value);
                    let login = logins.sessions.get_mut(&key)?;
                    if login.expires <= now || login.idle_until <= now || login.workspace != id {
                        return None;
                    }
                    login.idle_until = now.checked_add(LOGIN_IDLE).unwrap_or(now);
                    Some(login.identity.clone())
                })
                .flatten();
            let Some(identity) = found else {
                return Err(unauthorized(
                    auth,
                    &Denial::Missing,
                    &format!("{base}{}/w/{id}/mcp", super::RESOURCE_METADATA_PATH),
                )
                .for_files());
            };
            super::reqlog::caller("cookie");
            identity
        }
        Err(d) => {
            return Err(unauthorized(
                auth,
                &d,
                &format!("{base}{}/w/{id}/mcp", super::RESOURCE_METADATA_PATH),
            )
            .for_files());
        }
    };
    let access = auth.access(&identity, id).ok_or_else(not_found)?;
    Ok(Caller {
        key: identity_key(&identity),
        access,
    })
}

/// The origin the server's pages are served from: the public URL, else `http://<Host>`.
fn expected_origin(state: &State, host: &str) -> String {
    match &state.config.auth {
        Some(a) => public_base(a, host),
        None => format!("http://{}", host.to_ascii_lowercase()),
    }
}

/// Serves `/w/<id>/view…`.
#[allow(clippy::too_many_arguments)]
pub(super) fn serve(
    stream: &mut TcpStream,
    peer: IpAddr,
    state: &State,
    head: &Head,
    body: Vec<u8>,
    host: &str,
    id: &str,
    rest: &str,
) -> Result<Done, Reject> {
    if !state.config.viewer {
        return Err(err(Status::NotFound, "the viewer is disabled on this server"));
    }
    let method = head.method.as_str();
    if rest.is_empty() {
        if method != "GET" && method != "HEAD" {
            bicraft_http::respond_status(stream, Status::MethodNotAllowed, &[("Allow", "GET")]);
            return Ok(Done::Answered);
        }
        let location = format!("/w/{id}/view/");
        bicraft_http::respond_status(
            stream,
            Status::PermanentRedirect,
            &[("Location", location.as_str())],
        );
        return Ok(Done::Answered);
    }
    if rest == "/login" {
        super::reqlog::view("login");
        return login(stream, peer, state, head, host, id);
    }
    if let Some(code) = rest.strip_prefix("/login/") {
        super::reqlog::view("login");
        return redeem(stream, state, head, host, id, code);
    }
    // a native client's view (`apps/bicraft-desktop`): bearer credentials or the proxy's headers
    // only, never the browser cookie, so a cross-site form cannot mint a view and its token
    let caller = authenticate(
        state,
        peer,
        head,
        host,
        id,
        rest != "/api/session" && rest != CONTROL_ROUTE,
    )?;
    let Some(dir) = state.workspace_dir(id) else {
        return Err(Reject::plain(
            Status::NotFound,
            super::WORKSPACE_NOT_FOUND,
            "no such workspace",
        ));
    };
    let views = state.views(id);
    purge_doc_views(state, id, &views);
    let only = |stream: &mut TcpStream, allowed: &str| -> Option<Result<Done, Reject>> {
        (method != allowed).then(|| {
            bicraft_http::respond_status(stream, Status::MethodNotAllowed, &[("Allow", allowed)]);
            Ok(Done::Answered)
        })
    };
    match rest {
        "/" | "/index.html" => {
            super::reqlog::view("page");
            if let Some(r) = only(stream, "GET") {
                return r;
            }
            page(stream, state, &views, &caller)
        }
        "/app" => {
            super::reqlog::view("app");
            let location = format!("/w/{id}/view/app/");
            bicraft_http::respond_status(
                stream,
                Status::PermanentRedirect,
                &[("Location", location.as_str())],
            );
            Ok(Done::Answered)
        }
        "/app/" | "/app/index.html" => {
            // the browser app (docs/webui.md): its own private view, as the HTML page
            super::reqlog::view("app");
            if let Some(r) = only(stream, "GET") {
                return r;
            }
            if !bicraft_view::webui::available() {
                let _ = bicraft_view::webui::write_not_bundled(stream);
                return Ok(Done::Answered);
            }
            let (vid, token) = new_view(state, &views, &caller)?;
            let mut html = bicraft_view::webui::app_page(Some(&token), None, Some(&vid));
            // the control channel's slot: the page takes relayed requests and answers them with
            // its view's token
            if state.config.viewer_control.is_some() {
                html = bicraft_view::webui::with_control(&html, &token);
            }
            let _ = bicraft_view::webui::write_page(stream, &html);
            Ok(Done::Answered)
        }
        r if r.starts_with("/app/") => {
            super::reqlog::view("app-file");
            if let Some(r) = only(stream, "GET") {
                return r;
            }
            let name = r.trim_start_matches("/app/");
            match bicraft_view::webui::file(
                name,
                head.header("accept-encoding"),
                head.header("if-none-match"),
            ) {
                Some(f) => {
                    let _ = f.write(stream);
                }
                None if !bicraft_view::webui::available() => {
                    let _ = bicraft_view::webui::write_not_bundled(stream);
                }
                None => return Err(err(Status::NotFound, "no such file of the browser app")),
            }
            Ok(Done::Answered)
        }
        "/api/session" => {
            super::reqlog::view("session");
            if let Some(r) = only(stream, "POST") {
                return r;
            }
            let access = if caller.access >= Access::Editor {
                "editor"
            } else {
                "reader"
            };
            // with a body naming a file: a document view, a session of its own on that file
            let open = doc_params(state, stream, head, body, host)?;
            let body = match open {
                None => {
                    let (vid, token) = new_view(state, &views, &caller)?;
                    json!({"version": 1, "view": vid, "token": token, "access": access})
                }
                Some(params) => {
                    let parent = page_view(&views, &caller, head.query_param("v"));
                    let (vid, token, file) = new_doc_view(state, &views, &caller, id, &dir, &params, parent)?;
                    json!({"version": 1, "view": vid, "token": token, "access": access,
                           "file": file, "limits": limits(state)})
                }
            };
            let _ = bicraft_http::write_response(
                stream,
                Status::Ok,
                "application/json; charset=utf-8",
                &[("Cache-Control", "no-store")],
                body.to_string().as_bytes(),
            );
            Ok(Done::Answered)
        }
        "/api/close" => {
            super::reqlog::view("close");
            if let Some(r) = only(stream, "POST") {
                return r;
            }
            close_view(stream, state, &views, &caller, head, host)
        }
        "/api/state" => {
            super::reqlog::view("state");
            if let Some(r) = only(stream, "GET") {
                return r;
            }
            state_route(stream, state, &views, &caller, head, id, &dir)
        }
        "/api/events" => {
            super::reqlog::view("events");
            if let Some(r) = only(stream, "GET") {
                return r;
            }
            events(stream, state, &views, &caller, head)
        }
        "/api/interact" => {
            super::reqlog::view("interact");
            if let Some(r) = only(stream, "POST") {
                return r;
            }
            interact(stream, state, &views, &caller, head, body, host, id, &dir)
        }
        CONTROL_ROUTE | CONTROL_EVENTS_ROUTE | CONTROL_REPLY_ROUTE => {
            control(stream, state, &views, &caller, head, body, host, rest)
        }
        r if r.starts_with("/api/asset/") => {
            super::reqlog::view("asset");
            if let Some(r) = only(stream, "GET") {
                return r;
            }
            asset(stream, &views, &caller, head, r.trim_start_matches("/api/asset/"))
        }
        _ => {
            // no open-in-Desktop on a server (the page offers a download instead), nothing else
            super::reqlog::view("other");
            Err(err(Status::NotFound, "no such viewer route"))
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The page and the views

/// Creates a private view for `caller`; returns its id and interaction token.
fn new_view(state: &State, views: &Views, caller: &Caller) -> Result<(String, String), Reject> {
    let (Ok(token), Ok(vid)) = (
        bicraft_auth::tokens::random_hex(16),
        bicraft_auth::tokens::random_hex(16),
    ) else {
        return Err(err(Status::InternalError, "the random generator failed"));
    };
    let now = state.now_ms();
    {
        let mut map = views.viewers.lock().unwrap_or_else(|e| e.into_inner());
        let cap = state.config.max_viewers.max(1);
        // document views have their own caps and expiry (`new_doc_view`, `purge_doc_views`)
        while map.values().filter(|v| v.doc.is_none()).count() >= cap {
            // the least recently used view goes (its page asks to be reloaded)
            let Some(oldest) = map
                .iter()
                .filter(|(_, v)| v.doc.is_none())
                .min_by_key(|(_, v)| v.last_used)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            map.remove(&oldest);
        }
        map.retain(|_, v| {
            v.doc.is_some() || now.saturating_sub(v.last_used) < VIEW_IDLE_MS.saturating_mul(12)
        });
        map.insert(vid.clone(), Viewer::new(caller, &token, now, None));
    }
    state.metrics.viewer_pages.fetch_add(1, Ordering::Relaxed);
    Ok((vid, token))
}

fn page(stream: &mut TcpStream, state: &State, views: &Views, caller: &Caller) -> Result<Done, Reject> {
    let (vid, token) = new_view(state, views, caller)?;
    let html = bicraft_view::server_page(Some(&token), &vid);
    let _ = bicraft_http::write_response(
        stream,
        Status::Ok,
        "text/html; charset=utf-8",
        &[
            ("Cache-Control", "no-store"),
            (
                "Content-Security-Policy",
                bicraft_view::PAGE_CONTENT_SECURITY_POLICY,
            ),
        ],
        html.as_bytes(),
    );
    Ok(Done::Answered)
}

/// The view named by `?v=`, when it exists and belongs to the caller (another identity's view
/// answers 404 like an unknown one). Returns its state and token.
fn view_of(views: &Views, caller: &Caller, head: &Head, now: u64) -> Result<Found, Reject> {
    let gone = || err(Status::NotFound, "this view expired: reload the page");
    let vid = head.query_param("v").filter(|v| hex32(v)).ok_or_else(gone)?;
    let mut map = views.viewers.lock().unwrap_or_else(|e| e.into_inner());
    let v = map
        .get_mut(vid)
        .filter(|v| v.owner == caller.key)
        .ok_or_else(gone)?;
    v.last_used = now;
    Ok(Found {
        vid: vid.to_string(),
        ix: v.ix.clone(),
        token: v.token.clone(),
        doc: v.doc.clone(),
    })
}

/// A view found by [`view_of`].
struct Found {
    vid: String,
    ix: Interactions,
    token: String,
    doc: Option<DocView>,
}

impl Viewer {
    fn new(caller: &Caller, token: &str, now: u64, doc: Option<DocView>) -> Self {
        Viewer {
            owner: caller.key.clone(),
            token: token.to_string(),
            ix: Interactions::default(),
            seq: 0,
            pending: false,
            source: String::new(),
            last_used: now,
            doc,
            streams: 0,
            parent: None,
            file: None,
        }
    }
}

/// What a reader's view reads: the file it opened over MCP, else the workspace's.
fn target_of(views: &Views, caller: &Caller) -> Target {
    match views.reader_file(&caller.key) {
        Some(spec) => Target::File(spec),
        None => Target::Follow,
    }
}

/// The view's snapshot, computed on the session it reads; also the session's assets.
struct Built {
    snap: Value,
    assets: AssetMap,
    pending: bool,
}

fn build(s: &Session, ix: &Interactions, served: &Served, editor: bool, state: &State, doc: bool) -> Built {
    let mut snap = s.snapshot_view(ix);
    s.background_work();
    let assets: HashMap<String, (&'static str, Arc<[u8]>)> = s
        .snapshot_assets()
        .into_iter()
        .filter_map(|a| {
            let mime = bicraft_view::asset_mime(a.mime)?;
            bicraft_view::valid_asset_id(&a.id).then_some((a.id, (mime, a.bytes)))
        })
        .take(bicraft_view::MAX_ASSETS)
        .collect();
    let pending = data_pending(&snap);
    rewrite_source(s, &mut snap);
    let file = snap.pointer("/source/path").cloned().unwrap_or(Value::Null);
    if let Some(o) = snap.as_object_mut() {
        // `session`: what the view reads (`view`: a document view's own session); `doc`: a
        // document view; `limits`: the server's caps on document views (the UI's tab cap)
        let session = if doc && editor { "view" } else { served.name() };
        let mut view = json!({"role": if editor { "editor" } else { "reader" }, "session": session,
                              "doc": doc, "limits": limits(state)});
        if doc {
            view["file"] = file;
        }
        o.insert("view".into(), view);
    }
    Built {
        snap,
        assets: Arc::new(assets),
        pending,
    }
}

/// Server mode shows workspace-relative paths only, and the download of the saved file.
fn rewrite_source(s: &Session, snap: &mut Value) {
    let root = s.server_config().map(|c| c.root().to_path_buf());
    let rel = |p: &Path| root.as_deref().and_then(|r| super::ops::relative(r, p));
    let unsaved = s.dirty || s.report_dirty;
    let (path, download) = match &s.source {
        None => (
            None,
            json!({"path": null, "unsaved": unsaved, "reason": "nothing is saved yet"}),
        ),
        Some(Source::Package { path, .. } | Source::Bim { path }) => {
            let r = rel(path);
            (r.clone(), json!({"path": r, "unsaved": unsaved, "reason": null}))
        }
        Some(Source::Project { path, .. }) => (
            rel(path),
            json!({"path": null, "unsaved": unsaved,
                   "reason": "a project is a folder: its files are listed at ../files/"}),
        ),
    };
    if let Some(src) = snap.get_mut("source").and_then(Value::as_object_mut) {
        src.insert("path".into(), json!(path));
        src.insert("download".into(), download);
    }
}

/// Whether a visual with bound fields has no `data` yet (left by the snapshot's time budget).
fn data_pending(snap: &Value) -> bool {
    let pages = snap.pointer("/report/pages").and_then(Value::as_array);
    pages.into_iter().flatten().any(|p| {
        let visuals = p.get("visuals").and_then(Value::as_array);
        visuals.into_iter().flatten().any(|v| {
            let bound = v
                .get("fields")
                .and_then(Value::as_array)
                .is_some_and(|f| !f.is_empty());
            bound && v.get("data").is_none_or(Value::is_null)
        })
    })
}

fn source_key(served: &Served, snap: &Value) -> String {
    let path = snap
        .pointer("/source/path")
        .and_then(Value::as_str)
        .unwrap_or_default();
    format!("{}:{path}", served.name())
}

/// Runs `f` on the session `caller` views: the editor session for an editor, the read view for
/// a reader.
#[allow(clippy::too_many_arguments)]
fn on_view<R>(
    state: &State,
    views: &Views,
    caller: &Caller,
    id: &str,
    dir: &Path,
    doc: &Option<DocView>,
    f: impl FnOnce(&Session, &Served) -> R,
) -> Result<(R, Served), ViewError> {
    match doc {
        // a document view: its own session, or its file's saved state
        Some(DocView::Edit(w)) => {
            let served = Served::Editor;
            return state
                .with_doc(id, w, |s| f(s, &served))
                .map(|r| (r, Served::Editor));
        }
        Some(DocView::Read(spec)) => {
            let target = Target::File(spec.clone());
            return state.with_read_view(id, dir, &target, false, |s, served| f(s.get(), served));
        }
        None => {}
    }
    if caller.access >= Access::Editor {
        let served = Served::Editor;
        state
            .with_editor(id, dir, |s| f(s, &served))
            .map(|r| (r, Served::Editor))
    } else {
        state.with_read_view(id, dir, &target_of(views, caller), false, |s, served| {
            f(s.get(), served)
        })
    }
}

fn state_route(
    stream: &mut TcpStream,
    state: &State,
    views: &Views,
    caller: &Caller,
    head: &Head,
    id: &str,
    dir: &Path,
) -> Result<Done, Reject> {
    let Found { vid, ix, doc, .. } = view_of(views, caller, head, state.now_ms())?;
    let editor = match &doc {
        Some(DocView::Edit(_)) => true,
        Some(DocView::Read(_)) => false,
        None => caller.access >= Access::Editor,
    };
    let (mut built, served) = on_view(state, views, caller, id, dir, &doc, |s, served| {
        build(s, &ix, served, editor, state, doc.is_some())
    })
    .map_err(|e| view_error(state, e))?;
    // workspace-relative paths only, in messages too (paths.rs)
    super::paths::scrub(
        &mut built.snap,
        &super::paths::workspace_prefixes(dir, &state.root),
    );
    // a document view's own session has assets of its own (its unsaved images)
    let key = match &doc {
        Some(DocView::Edit(_)) => format!("view:{vid}"),
        _ => source_key(&served, &built.snap),
    };
    views
        .assets
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key.clone(), built.assets);
    if let Some(v) = views
        .viewers
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_mut(&vid)
    {
        v.pending = built.pending;
        v.source = key;
    }
    let bytes = serde_json::to_vec(&built.snap).unwrap_or_default();
    let _ = bicraft_http::write_response(
        stream,
        Status::Ok,
        "application/json; charset=utf-8",
        &[("Cache-Control", "no-store")],
        &bytes,
    );
    Ok(Done::Answered)
}

fn events(
    stream: &mut TcpStream,
    state: &State,
    views: &Views,
    caller: &Caller,
    head: &Head,
) -> Result<Done, Reject> {
    let Found { vid, doc, .. } = view_of(views, caller, head, state.now_ms())?;
    let doc = doc.is_some();
    let cap = (state.config.max_connections / 2).max(1);
    let Some(_slot) = bicraft_http::Slot::acquire(&state.viewer_streams, cap) else {
        return Err(busy(state, "too many viewer streams; try again"));
    };
    // a document view with an open stream is in use (never idle-closed)
    let _open = StreamOpen::enter(views, &vid);
    if bicraft_http::write_sse_head(stream, &[("X-Accel-Buffering", "no")]).is_err() {
        return Ok(Done::Answered);
    }
    let _ = stream.write_all(b"retry: 1000\n\n");
    let mut sent: Option<(u64, u64)> = None;
    let mut last_event = Instant::now();
    let mut last_write = Instant::now();
    let mut counter: u64 = 0;
    let mut ws_seq = views.seq();
    loop {
        if state.stop.load(Ordering::SeqCst) || state.draining.load(Ordering::SeqCst) {
            break;
        }
        let own = {
            let mut map = views.viewers.lock().unwrap_or_else(|e| e.into_inner());
            map.get_mut(&vid).map(|v| {
                if doc {
                    v.last_used = state.now_ms();
                }
                (v.seq, v.pending)
            })
        };
        let Some((own_seq, pending)) = own else {
            break;
        };
        // a document view refreshes on its own changes and on the files' (saves, uploads), not
        // on every command of the workspace's session
        let now = if doc {
            (views.disk_generation(), own_seq)
        } else {
            (ws_seq, own_seq)
        };
        let due = sent != Some(now) || (pending && last_event.elapsed() >= PENDING_POLL);
        let message: Option<Vec<u8>> = if due {
            counter = counter.saturating_add(1);
            sent = Some(now);
            last_event = Instant::now();
            Some(bicraft_view::snapshot_event(None, ws_seq, counter))
        } else if last_write.elapsed() >= state.config.keepalive {
            Some(b": keepalive\n\n".to_vec())
        } else {
            None
        };
        if let Some(m) = message {
            bicraft_http::note_bytes(u64::try_from(m.len()).unwrap_or(u64::MAX));
            if stream.write_all(&m).and_then(|()| stream.flush()).is_err() {
                break;
            }
            last_write = Instant::now();
        }
        ws_seq = views.wait_change(ws_seq, STREAM_TICK);
    }
    Ok(Done::Answered)
}

fn asset(
    stream: &mut TcpStream,
    views: &Views,
    caller: &Caller,
    head: &Head,
    id: &str,
) -> Result<Done, Reject> {
    if !bicraft_view::valid_asset_id(id) {
        return Err(err(Status::BadRequest, "malformed asset id"));
    }
    let gone = || err(Status::NotFound, "no such asset");
    let vid = head.query_param("v").filter(|v| hex32(v)).ok_or_else(gone)?;
    let source = {
        let map = views.viewers.lock().unwrap_or_else(|e| e.into_inner());
        map.get(vid)
            .filter(|v| v.owner == caller.key)
            .map(|v| v.source.clone())
            .ok_or_else(gone)?
    };
    let assets = views
        .assets
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&source)
        .map(Arc::clone)
        .ok_or_else(gone)?;
    let (mime, bytes) = assets.get(id).ok_or_else(gone)?;
    let etag = format!("\"{id}\"");
    let matched = head.header("if-none-match").is_some_and(|v| {
        v.split(',')
            .map(|t| t.trim().trim_start_matches("W/"))
            .any(|t| t == etag || t == "*")
    });
    let headers = [
        ("Cache-Control", bicraft_view::ASSET_CACHE_CONTROL),
        ("ETag", etag.as_str()),
        (
            "Content-Security-Policy",
            bicraft_view::ASSET_CONTENT_SECURITY_POLICY,
        ),
        ("Cross-Origin-Resource-Policy", "same-origin"),
        ("Content-Disposition", "inline"),
    ];
    let (status, body): (Status, &[u8]) = if matched {
        (Status::NotModified, &[])
    } else {
        (Status::Ok, bytes)
    };
    let _ = bicraft_http::write_response(stream, status, mime, &headers, body);
    Ok(Done::Answered)
}

/// The request comes from this server's page: the view's token, and, when the browser sends
/// them, `Origin` equal to the public origin and `Sec-Fetch-Site: same-origin`.
fn same_origin(state: &State, head: &Head, host: &str, token: &str) -> Result<(), &'static str> {
    if !head
        .header(bicraft_view::TOKEN_HEADER)
        .is_some_and(|t| bicraft_http::same_token(t, token))
    {
        return Err("missing or wrong token");
    }
    origin_checks(state, head, host)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn interact(
    stream: &mut TcpStream,
    state: &State,
    views: &Views,
    caller: &Caller,
    head: &Head,
    rest: Vec<u8>,
    host: &str,
    id: &str,
    dir: &Path,
) -> Result<Done, Reject> {
    let reply = |stream: &mut TcpStream, status: Status, body: Value| {
        let _ = bicraft_http::write_response(
            stream,
            status,
            "application/json; charset=utf-8",
            &[("Cache-Control", "no-store")],
            body.to_string().as_bytes(),
        );
        Ok(Done::Answered)
    };
    let Found {
        vid,
        mut ix,
        token,
        doc,
    } = view_of(views, caller, head, state.now_ms())?;
    if let Err(why) = same_origin(state, head, host, &token) {
        super::reqlog::error("forbidden");
        return reply(stream, Status::Forbidden, json!({"ok": false, "error": why}));
    }
    let json_type = head.header("content-type").is_some_and(|t| {
        t.split(';')
            .next()
            .is_some_and(|m| m.trim().eq_ignore_ascii_case("application/json"))
    });
    if !json_type {
        return reply(
            stream,
            Status::UnsupportedMedia,
            json!({"ok": false, "error": "the body must be application/json"}),
        );
    }
    let Some(length) = head.content_length else {
        return reply(
            stream,
            Status::LengthRequired,
            json!({"ok": false, "error": "Content-Length is required"}),
        );
    };
    if length > bicraft_view::MAX_BODY_BYTES {
        return reply(
            stream,
            Status::PayloadTooLarge,
            json!({"ok": false, "error": "the body is too large"}),
        );
    }
    let body = bicraft_http::read_body(stream, rest, length, state.config.body_deadline)
        .map_err(|status| err(status, status.line()))?;
    let action: Value = match serde_json::from_slice(&body) {
        Ok(v @ Value::Object(_)) => v,
        _ => {
            return reply(
                stream,
                Status::BadRequest,
                json!({"ok": false, "error": "the body must be a JSON object"}),
            );
        }
    };
    let (result, _) = on_view(state, views, caller, id, dir, &doc, |s, _| {
        let r = s.interact_view(&mut ix, &action);
        (r, ix)
    })
    .map_err(|e| view_error(state, e))?;
    let (result, ix) = result;
    state.metrics.viewer_interactions.fetch_add(1, Ordering::Relaxed);
    match result {
        Ok(v) => {
            if let Some(view) = views
                .viewers
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get_mut(&vid)
            {
                view.ix = ix;
                view.seq = view.seq.wrapping_add(1);
            }
            // this view's stream refetches; the others compare and stay quiet
            views.wake();
            reply(stream, Status::Ok, json!({"ok": true, "result": v}))
        }
        Err(e) => {
            let mut body = json!({"ok": false, "error": e});
            super::paths::scrub(&mut body, &super::paths::workspace_prefixes(dir, &state.root));
            reply(stream, Status::BadRequest, body)
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Document views (docs/mcp.md, "Document views")

/// A document view left unused this long by another identity may make room on a full
/// workspace.
const EVICT_OTHERS_MS: u64 = 60_000;
/// The longest body of `POST …/api/session` (a file name and two optional paths).
const MAX_SESSION_BODY: usize = 16 * 1024;

/// The server's caps on document views, as the snapshot and `POST …/api/session` report them.
fn limits(state: &State) -> Value {
    json!({
        "views_per_user": state.config.max_views_per_user.max(1),
        "views_per_workspace": state.config.max_views_per_workspace.max(1),
        "view_idle_secs": state.config.view_idle.as_secs(),
        "autosave": state.config.allow_autosave,
        "recovery": state.config.recovery_dir.is_some(),
    })
}

/// `Origin` and `Sec-Fetch-Site`, when the browser sends them, say this server's page.
fn origin_checks(state: &State, head: &Head, host: &str) -> Result<(), &'static str> {
    if let Some(origin) = head.header("origin")
        && !origin.trim().eq_ignore_ascii_case(&expected_origin(state, host))
        // behind a gateway that rewrites `Host` (the workspace controller), the public origin
        // is one of the endpoint's allowed origins
        && !state.origin_ok(origin)
    {
        return Err("cross-origin request refused");
    }
    if let Some(site) = head.header("sec-fetch-site")
        && !site.eq_ignore_ascii_case("same-origin")
    {
        return Err("cross-site request refused");
    }
    Ok(())
}

/// The `file.open` parameters of a `POST …/api/session` body (`{"file", "data_root",
/// "cache_abf"}`); `None` without a body (a view that follows the workspace's session).
fn doc_params(
    state: &State,
    stream: &mut TcpStream,
    head: &Head,
    rest: Vec<u8>,
    host: &str,
) -> Result<Option<Value>, Reject> {
    let length = head.content_length.unwrap_or(0);
    if length == 0 {
        return Ok(None);
    }
    let json_type = head.header("content-type").is_some_and(|t| {
        t.split(';')
            .next()
            .is_some_and(|m| m.trim().eq_ignore_ascii_case("application/json"))
    });
    if !json_type {
        return Err(err(Status::UnsupportedMedia, "the body must be application/json"));
    }
    if length > MAX_SESSION_BODY {
        return Err(err(Status::PayloadTooLarge, "the body is too large"));
    }
    origin_checks(state, head, host).map_err(|why| err(Status::Forbidden, why))?;
    let body = bicraft_http::read_body(stream, rest, length, state.config.body_deadline)
        .map_err(|status| err(status, status.line()))?;
    let Ok(Value::Object(o)) = serde_json::from_slice::<Value>(&body) else {
        return Err(err(Status::BadRequest, "the body must be a JSON object"));
    };
    // `{}`: a page's view, as without a body
    if o.is_empty() {
        return Ok(None);
    }
    if o.keys()
        .any(|k| !matches!(k.as_str(), "file" | "data_root" | "cache_abf"))
    {
        return Err(err(
            Status::BadRequest,
            "unknown field: the body takes `file`, `data_root` and `cache_abf`",
        ));
    }
    let Some(file) = o.get("file").and_then(Value::as_str) else {
        return Err(err(Status::BadRequest, "`file` (a workspace file) is required"));
    };
    let mut params = json!({"path": file});
    for k in ["data_root", "cache_abf"] {
        if let Some(v) = o.get(k) {
            params[k] = v.clone();
        }
    }
    Ok(Some(params))
}

/// Whether nothing would be lost by closing document view `v` (no stream, no command running,
/// no unsaved edit).
fn closable(v: &Viewer) -> bool {
    v.streams == 0
        && match &v.doc {
            None => false,
            Some(DocView::Read(_)) => true,
            Some(DocView::Edit(w)) => {
                w.pending.load(Ordering::SeqCst) == 0
                    && w.session.try_write().is_some_and(|s| !s.dirty && !s.report_dirty)
            }
        }
}

/// Makes room for one more document view of `caller` in `map` (this workspace's views), with
/// `elsewhere` views of the caller on other workspaces: under a cap, nothing to do; at a cap,
/// the least recently used view nobody uses goes (the caller's own; on a full workspace, also
/// another identity's unused for a minute); else the limit answer (429).
fn make_room(
    state: &State,
    map: &mut HashMap<String, Viewer>,
    caller: &Caller,
    elsewhere: usize,
) -> Result<Vec<Viewer>, Reject> {
    let per_user = state.config.max_views_per_user.max(1);
    let per_ws = state.config.max_views_per_workspace.max(1);
    let now = state.now_ms();
    let mut gone = Vec::new();
    loop {
        let docs = map.values().filter(|v| v.doc.is_some()).count();
        let mine = map
            .values()
            .filter(|v| v.doc.is_some() && v.owner == caller.key)
            .count()
            .saturating_add(elsewhere);
        let user_full = mine >= per_user;
        let ws_full = docs >= per_ws;
        if !user_full && !ws_full {
            return Ok(gone);
        }
        let victim = map
            .iter()
            .filter(|(_, v)| {
                v.owner == caller.key || (!user_full && now.saturating_sub(v.last_used) >= EVICT_OTHERS_MS)
            })
            .filter(|(_, v)| closable(v))
            .min_by_key(|(_, v)| v.last_used)
            .map(|(k, _)| k.clone());
        match victim.and_then(|k| map.remove(&k)) {
            Some(v) => gone.push(v),
            None => {
                state.metrics.busy.fetch_add(1, Ordering::Relaxed);
                let why = if user_full {
                    format!(
                        "too many open files: at most {per_user} per user on this server; close a tab first"
                    )
                } else {
                    format!(
                        "too many open files on this workspace (at most {per_ws}); close a tab or try again later"
                    )
                };
                return Err(Reject::plain(
                    Status::TooManyRequests,
                    super::WORKSPACE_BUSY,
                    &why,
                ));
            }
        }
    }
}

/// The caller's document views on the other workspaces.
fn views_elsewhere(state: &State, here: &Views, caller: &Caller) -> usize {
    let all: Vec<Arc<Views>> = state
        .views
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .map(Arc::clone)
        .collect();
    all.iter()
        .filter(|v| !std::ptr::eq(Arc::as_ptr(v), here))
        .map(|v| {
            let map = v.viewers.lock().unwrap_or_else(|e| e.into_inner());
            map.values()
                .filter(|x| x.doc.is_some() && x.owner == caller.key)
                .count()
        })
        .fold(0usize, usize::saturating_add)
}

/// Refused while BIcraft's reserved memory is at [`super::HttpConfig::ready_memory_percent`]
/// (90 % by default) of `BICRAFT_MEMORY_LIMIT`: a new session would not fit.
fn memory_left(state: &State) -> Result<(), Reject> {
    let stats = bicraft_mem::stats();
    let Some(limit) = stats.limit else {
        return Ok(());
    };
    let percent = u128::from(state.config.ready_memory_percent.unwrap_or(90).min(100));
    let used = u128::try_from(stats.used).unwrap_or(u128::MAX);
    let limit = u128::try_from(limit).unwrap_or(u128::MAX);
    if used.saturating_mul(100) >= limit.saturating_mul(percent) {
        return Err(busy(
            state,
            "the server's memory is nearly full: close a tab, or try again later",
        ));
    }
    Ok(())
}

/// Creates a document view of `caller` on the file `params` names (`file.open`'s parameters):
/// an editor gets a session of its own with the file open, a reader the file's saved state.
/// Returns the view's id, its interaction token and the file (workspace-relative).
fn new_doc_view(
    state: &State,
    views: &Views,
    caller: &Caller,
    id: &str,
    dir: &Path,
    params: &Value,
    parent: Option<String>,
) -> Result<(String, String, String), Reject> {
    let asked = params.get("path").and_then(Value::as_str).map(str::to_string);
    let cfg = bicraft_engine::ServerConfig::new(dir).map_err(|e| err(Status::InternalError, &e))?;
    let spec = Reopen::from_open_params(&cfg, params).map_err(|e| err(Status::BadRequest, &e))?;
    let file = spec.path().to_string();
    // the caps first (nothing loaded yet), then the memory
    let elsewhere = views_elsewhere(state, views, caller);
    let evicted = {
        let mut map = views.viewers.lock().unwrap_or_else(|e| e.into_inner());
        make_room(state, &mut map, caller, elsewhere)?
    };
    drop(evicted);
    memory_left(state)?;
    let (Ok(token), Ok(vid)) = (
        bicraft_auth::tokens::random_hex(16),
        bicraft_auth::tokens::random_hex(16),
    ) else {
        return Err(err(Status::InternalError, "the random generator failed"));
    };
    let doc = if caller.access >= Access::Editor {
        let mut s = super::new_session(dir, state.config.allow_network)
            .map_err(|e| err(Status::InternalError, &e))?;
        // revisions never repeat: an `expected_revision` of another session never matches
        s.revision = state.revision_floor(&format!("{id}/{vid}"));
        // its unsaved edits are journaled under the caller's own name (docs/recovery.md)
        s.set_recovery(state.recovery_config(id, Some(&caller.key)));
        spec.apply_result(&mut s)
            .map_err(|e| err(Status::Conflict, &format!("the file cannot be opened: {e}")))?;
        let w = Workspace::new(s, state.config.max_concurrent_reads, state.now_ms());
        {
            let g = w.session.read(None);
            views.note(&w.file_seen, &g, true);
        }
        DocView::Edit(Arc::new(w))
    } else {
        // a reader reads the saved file: load it now (the shared copy), so a bad file says so
        let target = Target::File(spec.clone());
        let (summary, _) = state
            .with_read_view(id, dir, &target, false, |s, _| {
                s.get().execute_shared("model.summary", &json!({}), None)
            })
            .map_err(|e| view_error(state, e))?;
        summary.map_err(|e| err(Status::Conflict, &format!("the file cannot be read: {e}")))?;
        DocView::Read(spec)
    };
    // the caps again: other views may have opened while this one loaded
    let elsewhere = views_elsewhere(state, views, caller);
    let evicted = {
        let mut map = views.viewers.lock().unwrap_or_else(|e| e.into_inner());
        let evicted = make_room(state, &mut map, caller, elsewhere)?;
        let mut v = Viewer::new(caller, &token, state.now_ms(), Some(doc));
        v.parent = parent;
        v.file = asked;
        map.insert(vid.clone(), v);
        evicted
    };
    drop(evicted);
    state.metrics.viewer_pages.fetch_add(1, Ordering::Relaxed);
    Ok((vid, token, file))
}

/// `POST …/api/close?v=<view>`: the view goes now (a document view's session with it, unsaved
/// edits lost: the window asks first). Answers `{"ok": true}`.
fn close_view(
    stream: &mut TcpStream,
    state: &State,
    views: &Views,
    caller: &Caller,
    head: &Head,
    host: &str,
) -> Result<Done, Reject> {
    let found = view_of(views, caller, head, state.now_ms())?;
    if let Err(why) = same_origin(state, head, host, &found.token) {
        return Err(err(Status::Forbidden, why));
    }
    let gone = views
        .viewers
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&found.vid);
    views
        .assets
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&format!("view:{}", found.vid));
    // its session goes outside the lock
    drop(gone);
    views.wake();
    let _ = bicraft_http::write_response(
        stream,
        Status::Ok,
        "application/json; charset=utf-8",
        &[("Cache-Control", "no-store")],
        json!({"ok": true}).to_string().as_bytes(),
    );
    Ok(Done::Answered)
}

/// Closes the document views of workspace `id` left without a request or an event stream for
/// [`super::HttpConfig::view_idle`] (four times as long with unsaved edits, then logged: the
/// edits stay in the recovery journal when the server keeps one, and reopening the file offers them).
pub(super) fn purge_doc_views(state: &State, id: &str, views: &Views) {
    let idle = u64::try_from(state.config.view_idle.as_millis()).unwrap_or(u64::MAX);
    let now = state.now_ms();
    let gone: Vec<(String, Viewer, bool)> = {
        let mut map = views.viewers.lock().unwrap_or_else(|e| e.into_inner());
        let expired: Vec<(String, bool)> = map
            .iter()
            .filter(|(_, v)| v.doc.is_some() && v.streams == 0)
            .filter_map(|(k, v)| {
                let unused = now.saturating_sub(v.last_used);
                if unused < idle {
                    return None;
                }
                if closable(v) {
                    return Some((k.clone(), false));
                }
                let quiet = match &v.doc {
                    Some(DocView::Edit(w)) => w.pending.load(Ordering::SeqCst) == 0,
                    _ => true,
                };
                (quiet && unused >= idle.saturating_mul(4)).then(|| (k.clone(), true))
            })
            .collect();
        expired
            .into_iter()
            .filter_map(|(k, dirty)| map.remove(&k).map(|v| (k, v, dirty)))
            .collect()
    };
    if gone.is_empty() {
        return;
    }
    {
        let mut assets = views.assets.lock().unwrap_or_else(|e| e.into_inner());
        for (k, _, _) in &gone {
            assets.remove(&format!("view:{k}"));
        }
    }
    let lost = gone.iter().filter(|(_, _, dirty)| *dirty).count();
    if lost > 0 {
        let fate = if state.config.recovery_dir.is_some() {
            "kept for recovery"
        } else {
            "lost"
        };
        state.log(
            "warn",
            "view_idle",
            &format!("workspace {id}: {lost} idle document view(s) closed with unsaved edits ({fate})"),
        );
    }
    // the sessions go outside the lock
    drop(gone);
}

/// Counts an open event stream on a view while it lives.
struct StreamOpen<'a> {
    views: &'a Views,
    vid: String,
}

impl<'a> StreamOpen<'a> {
    fn enter(views: &'a Views, vid: &str) -> Self {
        if let Some(v) = views
            .viewers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(vid)
        {
            v.streams = v.streams.saturating_add(1);
        }
        Self {
            views,
            vid: vid.to_string(),
        }
    }
}

impl Drop for StreamOpen<'_> {
    fn drop(&mut self) {
        if let Some(v) = self
            .views
            .viewers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(&self.vid)
        {
            v.streams = v.streams.saturating_sub(1);
        }
    }
}

impl Views {
    /// The document view `vid` of identity `key`, checked against its token (MCP calls on the
    /// view's endpoint), and marked used.
    pub(super) fn doc_binding(
        &self,
        vid: &str,
        key: &str,
        token: Option<&str>,
        now: u64,
    ) -> Result<DocView, (Status, &'static str)> {
        let mut map = self.viewers.lock().unwrap_or_else(|e| e.into_inner());
        let v = map
            .get_mut(vid)
            .filter(|v| v.owner == key)
            .ok_or((Status::NotFound, "this view expired: open the file again"))?;
        if !token.is_some_and(|t| bicraft_http::same_token(t, &v.token)) {
            return Err((Status::Forbidden, "missing or wrong view token (X-Bicraft-Token)"));
        }
        let doc = v.doc.clone().ok_or((
            Status::NotFound,
            "not a document view: a page's view runs commands at /w/<id>/mcp",
        ))?;
        v.last_used = now;
        Ok(doc)
    }

    /// Document view `vid` changed: its event stream sends a new event (the others stay quiet).
    pub(super) fn bump_view(&self, vid: &str) {
        if let Some(v) = self
            .viewers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(vid)
        {
            v.seq = v.seq.wrapping_add(1);
        }
        self.wake();
    }

    /// The file a reader's document view reads.
    pub(super) fn view_file(&self, vid: &str) -> Option<Reopen> {
        let map = self.viewers.lock().unwrap_or_else(|e| e.into_inner());
        match map.get(vid).and_then(|v| v.doc.as_ref()) {
            Some(DocView::Read(spec)) => Some(spec.clone()),
            _ => None,
        }
    }

    /// A reader's `file.open` on its document view: the view reads that file from now on.
    pub(super) fn set_view_file(&self, vid: &str, spec: Reopen) {
        if let Some(v) = self
            .viewers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(vid)
            && matches!(v.doc, Some(DocView::Read(_)))
        {
            v.doc = Some(DocView::Read(spec));
            v.seq = v.seq.wrapping_add(1);
        }
        self.wake();
    }

    /// The sessions of this workspace's editors' document views.
    pub(super) fn doc_sessions(&self) -> Vec<Arc<Workspace>> {
        let map = self.viewers.lock().unwrap_or_else(|e| e.into_inner());
        map.values()
            .filter_map(|v| match &v.doc {
                Some(DocView::Edit(w)) => Some(Arc::clone(w)),
                _ => None,
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------------------------
// The UI control channel (docs/web-deployment.md, "UI control channel"; docs/webui.md)

/// The agent's route, the page's request stream and its replies, under `/w/<id>/view`.
const CONTROL_ROUTE: &str = "/api/control";
const CONTROL_EVENTS_ROUTE: &str = "/api/control/events";
const CONTROL_REPLY_ROUTE: &str = "/api/control/reply";
/// The header that carries the control token on the agent's route (`Authorization` carries the
/// server's own credential).
pub const CONTROL_HEADER: &str = "x-bicraft-control";
/// How long the page has to send a reply body (a screenshot of a large window).
const CONTROL_REPLY_DEADLINE: Duration = Duration::from_secs(30);

/// `v`, when it names a page's view (not a document view) of `caller`.
fn page_view(views: &Views, caller: &Caller, v: Option<&str>) -> Option<String> {
    let v = v.filter(|v| hex32(v))?;
    let map = views.viewers.lock().unwrap_or_else(|e| e.into_inner());
    map.get(v)
        .filter(|x| x.owner == caller.key && x.doc.is_none())
        .map(|_| v.to_string())
}

/// This workspace's relay, with the server's caps.
fn relay<'a>(state: &State, views: &'a Views) -> &'a bicraft_view::control::Relay {
    views.control.get_or_init(|| {
        let token = state
            .config
            .viewer_control
            .as_ref()
            .map(|t| t.reveal().to_string())
            .unwrap_or_default();
        bicraft_view::control::Relay::with_limits(
            token,
            bicraft_view::control::Limits {
                max_pages: state.config.max_viewers.max(1),
                max_queued: state
                    .config
                    .queue_per_workspace
                    .clamp(1, bicraft_view::control::MAX_QUEUED),
                per_second: bicraft_view::control::PER_SECOND,
                max_request_bytes: state
                    .config
                    .max_body_bytes
                    .min(bicraft_view::control::MAX_REQUEST_BYTES),
                reply_timeout: bicraft_view::control::REPLY_TIMEOUT,
                open_hint: "open this workspace's /w/<id>/view/app/ page",
            },
        )
    })
}

/// Wakes every workspace's page streams (the server drains or stops).
pub(super) fn wake_controls(state: &State) {
    let all: Vec<Arc<Views>> = state
        .views
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .map(Arc::clone)
        .collect();
    for v in all {
        if let Some(r) = v.control.get() {
            r.wake();
        }
    }
}

/// The control channel's routes on `/w/<id>/view`: 404 when the server runs without
/// `--viewer-control`.
///
/// - `POST …/api/control[?v=<view>]` (the agent): an editor identity (the server's credentials,
///   never the browser cookie) and the control token in [`CONTROL_HEADER`]; the request goes to
///   the caller's own app page: the one whose view `v` names, the one that opened document view
///   `v` (switched to that file's tab first), or without `v` the caller's page connected last.
/// - `GET …/api/control/events?v=<page view>` (the page): its stream of requests.
/// - `POST …/api/control/reply?v=<page view>` (the page): a reply, with the view's token and the
///   same origin checks as `/api/interact`.
#[allow(clippy::too_many_arguments)]
fn control(
    stream: &mut TcpStream,
    state: &State,
    views: &Views,
    caller: &Caller,
    head: &Head,
    body: Vec<u8>,
    host: &str,
    rest: &str,
) -> Result<Done, Reject> {
    let answer = |stream: &mut TcpStream, status: Status, error: &str| {
        let _ = bicraft_http::write_response(
            stream,
            status,
            "application/json; charset=utf-8",
            &[("Cache-Control", "no-store")],
            json!({"id": null, "ok": false, "error": error})
                .to_string()
                .as_bytes(),
        );
        Ok(Done::Answered)
    };
    let Some(token) = &state.config.viewer_control else {
        super::reqlog::view("control");
        return answer(
            stream,
            Status::NotFound,
            "the UI control channel is off on this server (bicraft-cli serve --viewer-control)",
        );
    };
    let want = if rest == CONTROL_EVENTS_ROUTE {
        "GET"
    } else {
        "POST"
    };
    // the agent's route also lists the caller's views (GET)
    let listing = rest == CONTROL_ROUTE && head.method == "GET";
    if head.method != want && !listing {
        let allow = if rest == CONTROL_ROUTE { "GET, POST" } else { want };
        bicraft_http::respond_status(stream, Status::MethodNotAllowed, &[("Allow", allow)]);
        return Ok(Done::Answered);
    }
    let ask = bicraft_view::control::Ask {
        token: head.header(CONTROL_HEADER),
        origin: head.header("origin"),
        fetch_site: head.header("sec-fetch-site"),
        content_type: head.header("content-type"),
        content_length: head.content_length,
    };
    let relay = relay(state, views);
    match rest {
        CONTROL_ROUTE => {
            super::reqlog::view("control");
            let m = &state.metrics;
            m.control_requests.fetch_add(1, Ordering::Relaxed);
            let failed = || {
                m.control_errors.fetch_add(1, Ordering::Relaxed);
            };
            if ask.origin.is_some() || ask.fetch_site.is_some() {
                super::reqlog::error("forbidden");
                failed();
                return answer(
                    stream,
                    Status::Forbidden,
                    "the control route is not for web pages",
                );
            }
            if caller.access < Access::Editor {
                super::reqlog::error("forbidden");
                failed();
                return answer(
                    stream,
                    Status::Forbidden,
                    "the UI control channel needs editor access to this workspace",
                );
            }
            if !ask
                .token
                .map(str::trim)
                .is_some_and(|t| bicraft_http::same_token(t, token.reveal()))
            {
                super::reqlog::error("unauthorized");
                failed();
                return answer(
                    stream,
                    Status::Unauthorized,
                    "unauthorized: missing or wrong control token (X-Bicraft-Control)",
                );
            }
            if listing {
                let body = json!({"ok": true, "result": control_views(views, caller, relay)});
                let _ = bicraft_http::write_response(
                    stream,
                    Status::Ok,
                    "application/json; charset=utf-8",
                    &[("Cache-Control", "no-store")],
                    body.to_string().as_bytes(),
                );
                return Ok(Done::Answered);
            }
            let to = match control_target(views, caller, relay, head.query_param("v")) {
                Ok(to) => to,
                Err((status, why)) => {
                    failed();
                    return answer(stream, status, &why);
                }
            };
            if !relay.agent(stream, &ask, body, state.config.body_deadline, &to) {
                failed();
            }
            Ok(Done::Answered)
        }
        CONTROL_EVENTS_ROUTE => {
            super::reqlog::view("control-events");
            let found = view_of(views, caller, head, state.now_ms())?;
            if found.doc.is_some() {
                return Err(err(
                    Status::NotFound,
                    "a document view has no control stream: its page's view has",
                ));
            }
            let cap = (state.config.max_connections / 2).max(1);
            let Some(_slot) = bicraft_http::Slot::acquire(&state.viewer_streams, cap) else {
                return Err(busy(state, "too many viewer streams; try again"));
            };
            let stopped = || state.stop.load(Ordering::SeqCst) || state.draining.load(Ordering::SeqCst);
            relay.events(stream, Some(&found.vid), &stopped);
            Ok(Done::Answered)
        }
        _ => {
            super::reqlog::view("control-reply");
            let found = view_of(views, caller, head, state.now_ms())?;
            if let Err(why) = same_origin(state, head, host, &found.token) {
                super::reqlog::error("forbidden");
                return answer(stream, Status::Forbidden, why);
            }
            relay.page_reply(stream, &ask, body, CONTROL_REPLY_DEADLINE, Some(&found.vid));
            Ok(Done::Answered)
        }
    }
}

/// `GET …/api/control`: the caller's views on this workspace, so an agent can address one: each
/// page's view (`connected`: its app page takes requests now), with the document views it
/// opened (their files: the page's document tabs).
fn control_views(views: &Views, caller: &Caller, relay: &bicraft_view::control::Relay) -> Value {
    let connected = relay.connected();
    let map = views.viewers.lock().unwrap_or_else(|e| e.into_inner());
    let mut pages: Vec<(&String, &Viewer)> = map
        .iter()
        .filter(|(_, v)| v.owner == caller.key && v.doc.is_none())
        .collect();
    pages.sort_by_key(|(_, v)| std::cmp::Reverse(v.last_used));
    let list: Vec<Value> = pages
        .into_iter()
        .take(64)
        .map(|(vid, _)| {
            let docs: Vec<Value> = map
                .iter()
                .filter(|(_, d)| d.owner == caller.key && d.parent.as_ref() == Some(vid))
                .map(|(dvid, d)| json!({"view": dvid, "file": d.file}))
                .collect();
            json!({"view": vid, "connected": connected.contains(vid), "docs": docs})
        })
        .collect();
    json!({"pages": list})
}

/// Where the agent's request goes ([`control`]): `v` names one of the caller's views (a page's,
/// or a document view and then the page that opened it); without `v`, the caller's app page
/// connected last.
fn control_target(
    views: &Views,
    caller: &Caller,
    relay: &bicraft_view::control::Relay,
    v: Option<&str>,
) -> Result<bicraft_view::control::To, (Status, String)> {
    use bicraft_view::control::To;
    let connected = relay.connected();
    let map = views.viewers.lock().unwrap_or_else(|e| e.into_inner());
    let Some(v) = v else {
        let page = connected.into_iter().rev().find(|k| {
            map.get(k)
                .is_some_and(|x| x.owner == caller.key && x.doc.is_none())
        });
        return match page {
            Some(page) => Ok(To {
                page: Some(page),
                doc: None,
            }),
            None => Err((
                Status::Unavailable,
                "none of your app pages is connected: open /w/<id>/view/app/ (or name a view with ?v=)"
                    .into(),
            )),
        };
    };
    let gone = || {
        (
            Status::NotFound,
            "no such view (expired, or not yours)".to_string(),
        )
    };
    if !hex32(v) {
        return Err(gone());
    }
    let x = map.get(v).filter(|x| x.owner == caller.key).ok_or_else(gone)?;
    if x.doc.is_none() {
        return Ok(To {
            page: Some(v.to_string()),
            doc: None,
        });
    }
    match (&x.parent, &x.file) {
        (Some(page), Some(file)) => Ok(To {
            page: Some(page.clone()),
            doc: Some(file.clone()),
        }),
        _ => Err((
            Status::Conflict,
            "this document view was not opened by an app page (a native client's): drive that client instead"
                .into(),
        )),
    }
}

// ---------------------------------------------------------------------------------------------
// Browser login

/// `POST /w/<id>/view/login`: a bearer-authenticated caller (an API token, an OIDC token, or
/// the proxy's headers) gets a one-time URL that logs a browser in to this workspace's viewer.
fn login(
    stream: &mut TcpStream,
    peer: IpAddr,
    state: &State,
    head: &Head,
    host: &str,
    id: &str,
) -> Result<Done, Reject> {
    if head.method != "POST" {
        bicraft_http::respond_status(stream, Status::MethodNotAllowed, &[("Allow", "POST")]);
        return Ok(Done::Answered);
    }
    let Some(auth) = &state.config.auth else {
        return Err(err(
            Status::NotFound,
            "this server has no authentication: open the viewer page directly",
        ));
    };
    // credentials only (never the cookie: a login cannot extend itself)
    let base = public_base(auth, host);
    let identity = auth
        .authenticate(
            peer,
            |n| head.header(n),
            &[format!("{base}/w/{id}/mcp"), base.clone()],
        )
        .map_err(|d| {
            unauthorized(
                auth,
                &d,
                &format!("{base}{}/w/{id}/mcp", super::RESOURCE_METADATA_PATH),
            )
            .for_files()
        })?;
    if auth.access(&identity, id).is_none() || state.workspace_dir(id).is_none() {
        return Err(Reject::plain(
            Status::NotFound,
            super::WORKSPACE_NOT_FOUND,
            "no such workspace",
        ));
    }
    let code = bicraft_auth::tokens::random_hex(32).map_err(|e| err(Status::InternalError, &e))?;
    {
        let now = Instant::now();
        let mut logins = state.logins.lock().unwrap_or_else(|e| e.into_inner());
        logins.purge(now);
        if logins.codes.len() >= MAX_CODES {
            return Err(busy(state, "too many pending logins; try again in a minute"));
        }
        logins.codes.insert(
            bicraft_auth::tokens::digest(&code),
            Pending {
                identity,
                workspace: id.to_string(),
                expires: now.checked_add(CODE_TTL).unwrap_or(now),
            },
        );
    }
    let body = json!({
        "url": format!("{base}/w/{id}/view/login/{code}"),
        "expires_in": CODE_TTL.as_secs(),
    });
    let _ = bicraft_http::write_response(
        stream,
        Status::Ok,
        "application/json",
        &[("Cache-Control", "no-store")],
        body.to_string().as_bytes(),
    );
    Ok(Done::Answered)
}

/// `GET /w/<id>/view/login/<code>`: redeems a one-time code (single use, 60 s) for the login
/// cookie, then sends the browser to the page.
fn redeem(
    stream: &mut TcpStream,
    state: &State,
    head: &Head,
    host: &str,
    id: &str,
    code: &str,
) -> Result<Done, Reject> {
    if head.method != "GET" {
        bicraft_http::respond_status(stream, Status::MethodNotAllowed, &[("Allow", "GET")]);
        return Ok(Done::Answered);
    }
    let invalid = || {
        err(
            Status::NotFound,
            "this login link is invalid, expired or already used",
        )
    };
    let Some(auth) = &state.config.auth else {
        return Err(invalid());
    };
    if !hex64(code) {
        return Err(invalid());
    }
    let sid = bicraft_auth::tokens::random_hex(32).map_err(|e| err(Status::InternalError, &e))?;
    {
        let now = Instant::now();
        let mut logins = state.logins.lock().unwrap_or_else(|e| e.into_inner());
        // single use: the code goes whatever happens next
        let pending = logins
            .codes
            .remove(&bicraft_auth::tokens::digest(code))
            .filter(|p| p.expires > now && p.workspace == id)
            .ok_or_else(invalid)?;
        if auth.access(&pending.identity, id).is_none() {
            return Err(invalid());
        }
        logins.purge(now);
        if logins.sessions.len() >= MAX_LOGINS {
            return Err(busy(state, "too many viewer logins; try again later"));
        }
        logins.sessions.insert(
            bicraft_auth::tokens::digest(&sid),
            Login {
                identity: pending.identity,
                workspace: id.to_string(),
                expires: now.checked_add(LOGIN_TTL).unwrap_or(now),
                idle_until: now.checked_add(LOGIN_IDLE).unwrap_or(now),
            },
        );
    }
    let secure = if expected_origin(state, host).starts_with("https://") {
        "; Secure"
    } else {
        ""
    };
    let cookie = format!(
        "{COOKIE}={sid}; Path=/w/{id}/; Max-Age={}; HttpOnly; SameSite=Lax{secure}",
        LOGIN_TTL.as_secs()
    );
    let location = format!("/w/{id}/view/");
    bicraft_http::respond_status(
        stream,
        Status::SeeOther,
        &[
            ("Location", location.as_str()),
            ("Set-Cookie", cookie.as_str()),
            ("Cache-Control", "no-store"),
        ],
    );
    Ok(Done::Answered)
}

/// Document views open now, per workspace (sorted): editors' (`DocView::Edit`) and readers'
/// (`/metrics`).
pub(super) fn doc_view_counts(state: &State) -> Vec<(String, usize, usize)> {
    let views: Vec<(String, Arc<Views>)> = state
        .views
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .map(|(k, v)| (k.clone(), Arc::clone(v)))
        .collect();
    let mut out: Vec<(String, usize, usize)> = views
        .into_iter()
        .filter_map(|(id, v)| {
            let map = v.viewers.lock().unwrap_or_else(|e| e.into_inner());
            let (mut edit, mut read) = (0usize, 0usize);
            for x in map.values() {
                match &x.doc {
                    Some(DocView::Edit(_)) => edit = edit.saturating_add(1),
                    Some(DocView::Read(_)) => read = read.saturating_add(1),
                    None => {}
                }
            }
            (edit > 0 || read > 0).then_some((id, edit, read))
        })
        .collect();
    out.sort();
    out
}

/// Open views and logins now (`/metrics`).
pub(super) fn counts(state: &State) -> (usize, usize) {
    let views: Vec<Arc<Views>> = state
        .views
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .map(Arc::clone)
        .collect();
    let open = views
        .iter()
        .map(|v| v.viewers.lock().unwrap_or_else(|e| e.into_inner()).len())
        .fold(0usize, usize::saturating_add);
    let logins = state
        .logins
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .sessions
        .len();
    (open, logins)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_and_cookies() {
        assert_eq!(route("/w/a/view"), Some(("a", "")));
        assert_eq!(route("/w/a/view/"), Some(("a", "/")));
        assert_eq!(route("/w/a/view/api/state"), Some(("a", "/api/state")));
        for bad in ["/w/a/viewer", "/w/../view", "/w/a", "/view", "/w//view"] {
            assert_eq!(route(bad), None, "{bad}");
        }
        let head = bicraft_http::parse_head(
            b"GET / HTTP/1.1\r\nCookie: a=b; bicraft_view=abc ; c=d\r\n\r\n",
            &bicraft_http::Limits::default(),
            &[],
        )
        .unwrap();
        assert_eq!(cookie(&head), Some("abc"));
        assert!(hex64(&"a".repeat(64)) && !hex64(&"A".repeat(64)) && !hex64("ab"));
    }

    #[test]
    fn pending_data_is_detected() {
        let snap = |data: Value| json!({"report": {"pages": [{"visuals": [{"fields": [{"name": "x"}], "data": data}]}]}});
        assert!(data_pending(&snap(json!(null))));
        assert!(!data_pending(&snap(json!({"status": "ok"}))));
        assert!(!data_pending(&json!({"report": {"pages": "x"}})));
    }
}
