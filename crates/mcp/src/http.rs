//! MCP over Streamable HTTP, revision `2026-07-28` only (docs/web-deployment.md, "MCP over HTTP").
//!
//! One endpoint per workspace, `/w/<workspace-id>/mcp`, **POST only** (GET and DELETE answer 405:
//! this revision has no GET stream and no protocol session). Each POST carries one JSON-RPC
//! request or notification:
//!
//! - a notification is answered `202 Accepted` with no body;
//! - a request is answered with one JSON object (`application/json`), or, for `tools/call` when
//!   the client accepts `text/event-stream`, with an SSE stream scoped to that request (comment
//!   keepalives, then the response as the one event, then the stream closes).
//!
//! The revision is stateless: there is no `initialize` handshake. Every request carries its
//! protocol version and client capabilities in `params._meta`
//! (`io.modelcontextprotocol/protocolVersion`, `io.modelcontextprotocol/clientCapabilities`),
//! mirrored in the `MCP-Protocol-Version` header; `Mcp-Method` mirrors `method` and `Mcp-Name`
//! mirrors `params.name` (`tools/call`) or `params.uri`. Any header that is missing, malformed or
//! disagrees with the body is a `400` with JSON-RPC error `-32020` (`HeaderMismatch`). Methods:
//! `server/discover`, `tools/list` and `tools/call`, with the stdio server's tools unchanged.
//!
//! BIcraft is stateful, so the workspace is named by the URL: each workspace (a subfolder of the
//! server root) has one engine [`Session`] in server mode rooted at that folder ([`new_session`],
//! `docs/server-mode.md`: tool paths are relative to the workspace and cannot leave it, commands
//! denied on a server are hidden from `tools/list` and `list_commands` and refused, network
//! access only with [`HttpConfig::allow_network`]), and its tool calls run one at a time behind a mutex,
//! with a bounded queue (503 beyond); different workspaces run in parallel. Closing the response
//! stream cancels the request: the flag reaches the engine ([`Session::set_cancel`]), which stops
//! a running DAX evaluation.
//!
//! Hardening comes from `bicraft-http` (bounded heads and bodies, deadlines, connection caps per
//! server and per client address), plus a `Host` check (DNS rebinding), an `Origin` allow-list
//! (403 otherwise) and a body cap (8 MiB by default, 413 beyond).
//!
//! Operations (`docs/web-deployment.md`, "Operating `bicraft serve`"): file upload, download,
//! listing and deletion per workspace at `/w/<id>/files/…` ([`files`]), the probes `/healthz`
//! and `/readyz`, a graceful drain ([`HttpServer::drain`]) and the idle close of workspaces
//! ([`HttpConfig::idle_timeout`]) live in the child modules.

mod files;
mod metrics;
mod ops;
mod paths;
mod readview;
mod reqlog;
mod rwgate;
mod viewer;

pub use ops::{DrainReport, ShutdownSignal};
pub use reqlog::{LogFormat, LogSink, log_event};
pub use viewer::CONTROL_HEADER as VIEWER_CONTROL_HEADER;
pub use viewer::COOKIE as VIEWER_COOKIE;

use std::collections::HashMap;
use std::io::{self, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use bicraft_auth::{Access, Authenticator, Denial};
use bicraft_engine::{RecoveryConfig, ServerConfig, Session};
use bicraft_http::{Connections, Handler, Head, Limits, ReadError, Status, StreamLimits};
pub use bicraft_opc::ZipLimits;
use serde_json::{Map, Value, json};

use crate::{INSTRUCTIONS, INVALID_PARAMS, INVALID_REQUEST, METHOD_NOT_FOUND, PARSE_ERROR, tools};

/// The only MCP revision served over HTTP (the latest; stdio keeps [`crate::SUPPORTED_VERSIONS`]).
pub const HTTP_PROTOCOL_VERSION: &str = "2026-07-28";
/// Every revision served over HTTP.
pub const HTTP_SUPPORTED_VERSIONS: &[&str] = &[HTTP_PROTOCOL_VERSION];

/// The HTTP headers do not match the body, or a required one is missing or malformed.
pub const HEADER_MISMATCH: i64 = -32020;
/// The requested protocol version is not served (`data: {supported, requested}`).
pub const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;
/// BIcraft's own codes, outside the JSON-RPC reserved range (the MCP spec asks for that).
/// The workspace's queue is full (HTTP 503).
pub const WORKSPACE_BUSY: i64 = -31001;
/// No such workspace (HTTP 404).
pub const WORKSPACE_NOT_FOUND: i64 = -31002;
/// Authentication failed or is missing (HTTP 401, 403 or 400 with `WWW-Authenticate`).
pub const UNAUTHORIZED: i64 = -31003;
/// The server is shutting down (HTTP 503 with `Retry-After`).
pub const SHUTTING_DOWN: i64 = -31004;

/// The protected resource metadata path (RFC 9728 §3), at the root or followed by the endpoint
/// path.
pub const RESOURCE_METADATA_PATH: &str = "/.well-known/oauth-protected-resource";

/// `_meta` keys of the revision.
pub const META_PROTOCOL_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
pub const META_CLIENT_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
pub const META_SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";

/// Request headers of the revision.
pub const H_PROTOCOL_VERSION: &str = "MCP-Protocol-Version";
pub const H_METHOD: &str = "Mcp-Method";
pub const H_NAME: &str = "Mcp-Name";

/// Freshness hint of `tools/list` (the tools never change while the server runs).
const TOOLS_TTL_MS: u64 = 3_600_000;
/// A client that does not drain its socket within this time is dropped.
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);
/// How often a running request checks whether its client went away.
const WATCH_POLL: Duration = Duration::from_millis(50);

/// Which `Host` values the server answers (DNS-rebinding protection).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostPolicy {
    /// `localhost`, `*.localhost`, `127.x.x.x`, `[::1]` (with any port): a loopback server.
    Loopback,
    /// Exactly these `host[:port]` values (ASCII case-insensitive).
    Names(Vec<String>),
}

/// Limits and policies of the HTTP endpoint.
#[derive(Debug, Clone)]
pub struct HttpConfig {
    /// Largest request body (413 beyond). Default 8 MiB.
    pub max_body_bytes: usize,
    /// Browser origins allowed to call the endpoint (`scheme://host[:port]`, exact, ASCII
    /// case-insensitive). A request whose `Origin` is not listed gets 403; agents send none.
    pub allowed_origins: Vec<String>,
    pub hosts: HostPolicy,
    /// Tool calls per workspace, running and waiting (503 beyond). Default 8.
    pub queue_per_workspace: usize,
    /// Open sessions at most (503 for a new workspace beyond). Default 64.
    pub max_workspaces: usize,
    /// Open connections, in total and per client address. Defaults 128 and 32.
    pub max_connections: usize,
    pub max_connections_per_ip: Option<usize>,
    /// Request head bounds (16 KiB, 64 headers, 2 KiB targets, 10 s).
    pub head_limits: Limits,
    /// The body must arrive within this time. Default 60 s.
    pub body_deadline: Duration,
    /// SSE comment keepalive interval. Default 15 s.
    pub keepalive: Duration,
    /// Network access of the workspaces' sessions (the Databricks connector of `model.refresh`,
    /// `docs/server-mode.md`). Default off; `bicraft serve --allow-network` or
    /// `BICRAFT_SERVER_NETWORK=1` turns it on.
    pub allow_network: bool,
    /// Authentication and the workspace ACL (`docs/auth.md`). `None`: no authentication, and the
    /// server only listens on loopback addresses.
    pub auth: Option<Arc<Authenticator>>,
    /// Largest file upload (`PUT /w/<id>/files/…`, a zipped project too), 413 beyond. Default
    /// 4 GiB.
    pub max_upload_bytes: u64,
    /// Time limits of an upload body: the longest stall (default 30 s) and the whole upload
    /// (default 2 h).
    pub upload_limits: StreamLimits,
    /// Caps of a zipped project's extraction (entries, bytes, expansion ratio).
    pub zip_limits: ZipLimits,
    /// How long an upload or a deletion waits for the workspace's running tool call before it
    /// answers 503. Default 30 s.
    pub commit_wait: Duration,
    /// Close a workspace's session after this long without a tool call (`None`, the default:
    /// never). A session with unsaved changes is never closed for idleness; the next call reopens
    /// the file lazily.
    pub idle_timeout: Option<Duration>,
    /// `/readyz` answers 503 while BIcraft's reserved memory is above this percentage of
    /// `BICRAFT_MEMORY_LIMIT` (`None`, the default: memory is not checked; without a limit it
    /// never is).
    pub ready_memory_percent: Option<u8>,
    /// Where Prometheus scrapes `/metrics` (`None`, the default: no metrics listener). A listener
    /// of its own, without authentication, serving `/metrics` only; the MCP listener never serves
    /// it ([`metrics`]).
    pub metrics_listen: Option<SocketAddr>,
    /// Per-workspace metric series (label `workspace`; off by default: bounded by the workspace
    /// folders, capped at 1024).
    pub metrics_workspace_label: bool,
    /// The log: text (the default; no per-request line) or one JSON line per request
    /// ([`reqlog`]).
    pub log_format: LogFormat,
    /// Where log lines go (`None`: stderr).
    pub log_sink: Option<LogSink>,
    /// Serve the live viewer of each workspace at `/w/<id>/view/` ([`viewer`]). Default on.
    pub viewer: bool,
    /// Read-only copies of saved files kept for readers at most (one per file; [`readview`]).
    /// Default 1.
    pub viewer_copies: usize,
    /// Browser views (pages) kept per workspace at most (the least recently used goes beyond).
    /// Default 256.
    pub max_viewers: usize,
    /// Read-only tool calls (and viewer reads) running at once on one workspace's session
    /// (`--max-concurrent-reads`, default 4; 1 runs them one at a time). Calls that may change the
    /// session always run alone, and a waiting one goes before new reads.
    pub max_concurrent_reads: usize,
    /// Document views (a private session on one workspace file, [`viewer`]: the browser's
    /// document tabs) one identity may hold open at once, over every workspace. Default 8 (the
    /// UI's cap of document tabs).
    pub max_views_per_user: usize,
    /// Document views open at once on one workspace, whoever holds them. Default 32.
    pub max_views_per_workspace: usize,
    /// A document view without a request or an open event stream for this long is closed; one
    /// with unsaved edits is kept four times as long, then closed (its edits stay in the recovery
    /// journal when [`HttpConfig::recovery_dir`] is set, else they are lost). Default 30 min.
    pub view_idle: Duration,
    /// Auto-recovery (`docs/recovery.md`): the folder where editor sessions journal their unsaved
    /// edits, one subfolder per workspace (`--recovery-dir`). A view that expires, a session
    /// dropped with unsaved edits or a server restart then loses nothing: reopening the file offers
    /// them back. `None` (the default): no journal.
    pub recovery_dir: Option<PathBuf>,
    /// The UI may save PBIP projects automatically (`--allow-autosave`; the snapshot's
    /// `view.limits.autosave`). Off by default.
    pub allow_autosave: bool,
    /// The UI control channel relayed to the browser app pages of each workspace
    /// (`--viewer-control`, [`viewer`]): agents with an editor identity and this token drive and
    /// screenshot their own app pages. `None`, the default: off, the routes answer 404.
    pub viewer_control: Option<ControlToken>,
}

/// The control token of [`HttpConfig::viewer_control`]: never printed (`Debug` hides it).
#[derive(Clone, PartialEq, Eq)]
pub struct ControlToken(String);

impl ControlToken {
    /// A token of 32 to 256 visible ASCII characters (`bicraft serve` makes 32 random hex
    /// digits).
    pub fn new(token: &str) -> Result<ControlToken, String> {
        let t = token.trim();
        if t.len() < 32 || t.len() > 256 || !t.bytes().all(|b| b.is_ascii_graphic()) {
            return Err("a control token is 32 to 256 visible ASCII characters".into());
        }
        Ok(ControlToken(t.to_string()))
    }

    /// A new random token (128 bits, hex).
    pub fn random() -> Result<ControlToken, String> {
        bicraft_auth::tokens::random_hex(16).map(ControlToken)
    }

    /// The token, to write to its owner-only file.
    pub fn reveal(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for ControlToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ControlToken(***)")
    }
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            max_body_bytes: 8 * 1024 * 1024,
            allowed_origins: Vec::new(),
            hosts: HostPolicy::Loopback,
            queue_per_workspace: 8,
            max_workspaces: 64,
            max_connections: 128,
            max_connections_per_ip: Some(32),
            head_limits: Limits {
                max_head_bytes: 16 * 1024,
                max_headers: 64,
                max_target_len: 2048,
                head_deadline: Duration::from_secs(10),
            },
            body_deadline: Duration::from_secs(60),
            keepalive: Duration::from_secs(15),
            allow_network: false,
            auth: None,
            max_upload_bytes: 4 * 1024 * 1024 * 1024,
            upload_limits: StreamLimits {
                idle: Duration::from_secs(30),
                deadline: Duration::from_secs(2 * 3600),
            },
            zip_limits: ZipLimits::default(),
            commit_wait: Duration::from_secs(30),
            idle_timeout: None,
            ready_memory_percent: None,
            metrics_listen: None,
            metrics_workspace_label: false,
            log_format: LogFormat::Text,
            log_sink: None,
            viewer: true,
            viewer_copies: 1,
            max_viewers: 256,
            max_concurrent_reads: 4,
            max_views_per_user: 8,
            max_views_per_workspace: 32,
            view_idle: Duration::from_secs(30 * 60),
            recovery_dir: None,
            allow_autosave: false,
            viewer_control: None,
        }
    }
}

/// Headers the endpoint reads: each may appear once.
const UNIQUE_HEADERS: &[&str] = &[
    "host",
    "origin",
    "cookie",
    "sec-fetch-site",
    bicraft_view::TOKEN_HEADER,
    viewer::CONTROL_HEADER,
    "content-type",
    "content-length",
    H_PROTOCOL_VERSION,
    H_METHOD,
    H_NAME,
];

/// One workspace: its session, and how many tool calls (and file commits) run or wait for it.
struct Workspace {
    /// Read-only calls share it, the others run alone (`rwgate.rs`).
    session: rwgate::SessionLock<Session>,
    /// Incremented under the workspace map's lock ([`State::checkout`]), so the idle reaper,
    /// which checks it under the same lock, never closes a workspace someone is about to use.
    pending: AtomicUsize,
    /// Milliseconds since [`State::epoch`] at the last tool call's start or end.
    last_used: AtomicU64,
    /// The file to reopen before the next tool call (after an idle close, or after an upload
    /// replaced the open file). Locked after `session`, never before.
    reopen: Mutex<Option<ops::Reopen>>,
    /// The idle reaper already said this workspace stays open because of unsaved changes.
    warned: AtomicBool,
    /// The file the session has open and the file revision it read ([`readview::Views::file_rev`]):
    /// a `file.save` over a file another session saved since is refused (`file_conflict`).
    file_seen: Mutex<Option<(String, u64)>>,
}

impl Workspace {
    /// A session with its own queue (a workspace's, or a document view's).
    fn new(session: Session, max_readers: usize, now_ms: u64) -> Self {
        Self {
            session: rwgate::SessionLock::new(session, max_readers),
            pending: AtomicUsize::new(0),
            last_used: AtomicU64::new(now_ms),
            reopen: Mutex::new(None),
            warned: AtomicBool::new(false),
            file_seen: Mutex::new(None),
        }
    }
}

struct State {
    config: HttpConfig,
    /// The server root, canonical.
    root: PathBuf,
    workspaces: Mutex<HashMap<String, Arc<Workspace>>>,
    /// Workspaces closed for idleness, with the file their next session reopens. Locked after
    /// `workspaces`, never before.
    closed: Mutex<HashMap<String, ops::Reopen>>,
    conns: Arc<Connections>,
    /// Headers that may appear once (lowercase): [`UNIQUE_HEADERS`], `Authorization` and the
    /// identity headers of the trusted-proxy mode.
    unique: Vec<String>,
    stop: AtomicBool,
    /// Draining (shutdown): `/readyz` answers 503 and new requests get 503.
    draining: AtomicBool,
    /// The grace period is over: running uploads and downloads stop.
    abort: AtomicBool,
    /// Requests in progress (probes excepted).
    active: AtomicUsize,
    /// Cancel flags of the running tool calls, raised when the grace period ends.
    running: Mutex<HashMap<u64, Arc<AtomicBool>>>,
    next_call: AtomicU64,
    /// Serializes the final step of uploads and deletions (checks, then renames).
    commit: Mutex<()>,
    epoch: Instant,
    /// Counters of `/metrics` ([`metrics`]).
    metrics: metrics::Metrics,
    /// Stops the metrics listener (after the drain, or when the server is dropped).
    metrics_stop: AtomicBool,
    /// The read views and browser views of each workspace ([`readview`], [`viewer`]).
    views: Mutex<HashMap<String, Arc<readview::Views>>>,
    /// The viewer's browser logins.
    logins: Mutex<viewer::Logins>,
    /// Viewer event streams open now.
    viewer_streams: AtomicUsize,
    /// The last revision of each workspace's closed sessions ([`State::revision_floor`]).
    last_revisions: Mutex<HashMap<String, u64>>,
}

/// A running MCP HTTP endpoint. Dropping it stops accepting connections.
pub struct HttpServer {
    addr: SocketAddr,
    state: Arc<State>,
    accept: Option<JoinHandle<()>>,
    /// The idle reaper, when [`HttpConfig::idle_timeout`] is set.
    reaper: Option<JoinHandle<()>>,
    /// The metrics listener's address and accept loop, when [`HttpConfig::metrics_listen`] is set.
    metrics_addr: Option<SocketAddr>,
    metrics_accept: Option<JoinHandle<()>>,
}

impl HttpServer {
    /// Listens on `addr` and serves the workspaces found under `root` (each subfolder is one).
    /// With [`HostPolicy::Loopback`] or without [`HttpConfig::auth`], a non-loopback `addr` is
    /// refused ([`io::ErrorKind::InvalidInput`]).
    pub fn start(addr: SocketAddr, root: impl Into<PathBuf>, config: HttpConfig) -> io::Result<Self> {
        if config.hosts == HostPolicy::Loopback && !addr.ip().is_loopback() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "a loopback server only listens on loopback addresses, not {}",
                    addr.ip()
                ),
            ));
        }
        if config.auth.is_none() && !addr.ip().is_loopback() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "a server without authentication only listens on loopback addresses, not {}",
                    addr.ip()
                ),
            ));
        }
        let root = root.into();
        if !root.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("the workspace root {} is not a folder", root.display()),
            ));
        }
        // canonical (links resolved), so a workspace folder can be checked to stay under it
        let root = std::fs::canonicalize(&root)?;
        let listener = TcpListener::bind(addr)?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let mut unique: Vec<String> = UNIQUE_HEADERS.iter().map(|h| h.to_ascii_lowercase()).collect();
        if let Some(a) = &config.auth {
            unique.extend(a.unique_headers());
        }
        // the metrics listener: its own port, never the MCP one
        let metrics_listener = match config.metrics_listen {
            Some(m) => {
                if m == addr || (m.port() == addr.port() && m.port() != 0 && m.ip().is_unspecified()) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "the metrics listener needs its own address (--metrics-listen)",
                    ));
                }
                let l = TcpListener::bind(m)?;
                l.set_nonblocking(true)?;
                Some(l)
            }
            None => None,
        };
        let metrics_addr = match &metrics_listener {
            Some(l) => Some(l.local_addr()?),
            None => None,
        };
        // upload leftovers of an earlier run that did not shut down cleanly
        ops::remove_temp_dirs(&root);
        let metrics = metrics::Metrics::new(config.metrics_workspace_label);
        let state = Arc::new(State {
            metrics,
            metrics_stop: AtomicBool::new(false),
            views: Mutex::new(HashMap::new()),
            logins: Mutex::new(viewer::Logins::default()),
            viewer_streams: AtomicUsize::new(0),
            last_revisions: Mutex::new(HashMap::new()),
            conns: Connections::new(config.max_connections, config.max_connections_per_ip),
            unique,
            config,
            root,
            workspaces: Mutex::new(HashMap::new()),
            closed: Mutex::new(HashMap::new()),
            stop: AtomicBool::new(false),
            draining: AtomicBool::new(false),
            abort: AtomicBool::new(false),
            active: AtomicUsize::new(0),
            running: Mutex::new(HashMap::new()),
            next_call: AtomicU64::new(0),
            commit: Mutex::new(()),
            epoch: Instant::now(),
        });
        let reaper = match state.config.idle_timeout {
            Some(timeout) => {
                let reaper_state = Arc::clone(&state);
                Some(
                    thread::Builder::new()
                        .name("bicraft-mcp-idle".into())
                        .spawn(move || ops::idle_loop(&reaper_state, timeout))?,
                )
            }
            None => None,
        };
        let accept_state = Arc::clone(&state);
        let accept = thread::Builder::new()
            .name("bicraft-mcp-http".into())
            .spawn(move || {
                let conn_state = Arc::clone(&accept_state);
                let handler: Handler =
                    Arc::new(move |stream, peer| handle_connection(stream, peer.ip(), &conn_state));
                bicraft_http::accept_loop(
                    listener,
                    &accept_state.stop,
                    &accept_state.conns,
                    "bicraft-mcp-http-conn",
                    &handler,
                );
            })?;
        let metrics_accept = match metrics_listener {
            Some(listener) => {
                let metrics_state = Arc::clone(&state);
                Some(
                    thread::Builder::new()
                        .name("bicraft-metrics".into())
                        .spawn(move || {
                            // a few scrapers at most: Prometheus, an operator's curl
                            let conns = Connections::new(8, Some(4));
                            let conn_state = Arc::clone(&metrics_state);
                            let handler: Handler = Arc::new(move |stream, peer| {
                                metrics::serve_connection(stream, peer.ip(), &conn_state)
                            });
                            bicraft_http::accept_loop(
                                listener,
                                &metrics_state.metrics_stop,
                                &conns,
                                "bicraft-metrics-conn",
                                &handler,
                            );
                        })?,
                )
            }
            None => None,
        };
        Ok(Self {
            addr,
            state,
            accept: Some(accept),
            reaper,
            metrics_addr,
            metrics_accept,
        })
    }

    /// The address of the metrics listener (`None`: no [`HttpConfig::metrics_listen`]).
    pub fn metrics_addr(&self) -> Option<SocketAddr> {
        self.metrics_addr
    }

    /// The Prometheus text exposition `/metrics` serves (also without a metrics listener).
    pub fn metrics_text(&self) -> String {
        metrics::render(&self.state)
    }

    /// Writes a server event to the log in the configured format ([`log_event`]).
    pub fn log(&self, level: &str, event: &str, message: &str) {
        log_event(
            self.state.config.log_format,
            self.state.config.log_sink.as_ref(),
            level,
            event,
            message,
        );
    }

    /// The URL of workspace `id`'s files (`…/files/`).
    pub fn files_url(&self, id: &str) -> String {
        format!("http://{}/w/{id}/files/", self.addr)
    }

    /// The URL of workspace `id`'s live viewer (`…/view/`).
    pub fn view_url(&self, id: &str) -> String {
        format!("http://{}/w/{id}/view/", self.addr)
    }

    /// Whether the server is draining ([`HttpServer::drain`] started).
    pub fn is_draining(&self) -> bool {
        self.state.draining.load(Ordering::SeqCst)
    }

    /// Open workspace sessions now.
    pub fn open_workspaces(&self) -> usize {
        self.state
            .workspaces
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .len()
    }

    /// The address the server listens on (port 0 at start = OS-chosen).
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The endpoint URL of workspace `id`.
    pub fn url(&self, id: &str) -> String {
        format!("http://{}/w/{id}/mcp", self.addr)
    }

    /// Open connections now.
    pub fn open_connections(&self) -> usize {
        self.state.conns.open()
    }

    /// Blocks until the accept loop ends (it ends when the server is dropped from another
    /// thread, or never: `bicraft serve` runs until killed).
    pub fn wait(mut self) {
        if let Some(accept) = self.accept.take() {
            let _ = accept.join();
        }
    }
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        self.state.stop.store(true, Ordering::SeqCst);
        viewer::wake_controls(&self.state);
        if let Some(accept) = self.accept.take() {
            let _ = accept.join();
        }
        if let Some(reaper) = self.reaper.take() {
            let _ = reaper.join();
        }
        self.state.metrics_stop.store(true, Ordering::SeqCst);
        if let Some(m) = self.metrics_accept.take() {
            let _ = m.join();
        }
    }
}

/// Workspace ids: 1 to 64 ASCII letters, digits, `-`, `_` or `.`, not starting with `.`. They name
/// a subfolder of the root, so they can never be a path (`/`, `\`, `..`).
pub fn valid_workspace_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && !id.starts_with('.')
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// The workspace id of an endpoint path `/w/<id>/mcp`.
fn route(path: &str) -> Option<&str> {
    let id = path.strip_prefix("/w/")?.strip_suffix("/mcp")?;
    valid_workspace_id(id).then_some(id)
}

/// The workspace id and the document view of a view's endpoint `/w/<id>/mcp/view/<view>`
/// (docs/mcp.md, "Document views"): the same MCP endpoint, on that view's own session.
fn view_route(path: &str) -> Option<(&str, &str)> {
    let (id, vid) = path.strip_prefix("/w/")?.split_once("/mcp/view/")?;
    let hex = vid.len() == 32
        && vid
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    (valid_workspace_id(id) && hex).then_some((id, vid))
}

/// The server's public origin: the configured public URL, else `http://<Host>` (a loopback or
/// development server; the `Host` value has passed the host policy).
fn public_base(auth: &Authenticator, host: &str) -> String {
    match auth.public_url() {
        Some(u) => u.to_string(),
        None => format!("http://{}", host.to_ascii_lowercase()),
    }
}

/// The answer to a request that is not authenticated: 401 (or 403 `insufficient_scope`, 400
/// `invalid_request`) with the `WWW-Authenticate` challenge naming the resource metadata.
fn unauthorized(auth: &Authenticator, denial: &Denial, metadata_url: &str) -> Reject {
    let (code, challenge) = auth.challenge(denial, Some(metadata_url));
    let (status, message) = match code {
        403 => (Status::Forbidden, "insufficient scope"),
        400 => (Status::BadRequest, "malformed credentials"),
        _ => (Status::Unauthorized, "authentication required"),
    };
    let mut r = Reject::new(status, None, UNAUTHORIZED, message);
    r.extra.push(("WWW-Authenticate", challenge));
    r
}

/// `GET /.well-known/oauth-protected-resource[/w/<id>/mcp]`: the protected resource metadata
/// (RFC 9728) of the server (root) or of one workspace endpoint. Public (no authentication, any
/// origin), the same for any well-formed workspace id so it reveals nothing; 404 without OIDC.
fn resource_metadata(
    stream: &mut TcpStream,
    state: &State,
    head: &Head,
    host: &str,
    suffix: &str,
) -> Result<Done, Reject> {
    let Some(auth) = state.config.auth.as_ref().filter(|a| a.has_oidc()) else {
        bicraft_http::respond_status(stream, Status::NotFound, &[]);
        return Ok(Done::Answered);
    };
    let base = public_base(auth, host);
    let resource = if suffix.is_empty() {
        base
    } else if let Some(id) = route(suffix) {
        format!("{base}/w/{id}/mcp")
    } else {
        bicraft_http::respond_status(stream, Status::NotFound, &[]);
        return Ok(Done::Answered);
    };
    if head.method != "GET" {
        bicraft_http::respond_status(stream, Status::MethodNotAllowed, &[("Allow", "GET")]);
        return Ok(Done::Answered);
    }
    let doc = auth.resource_metadata(&resource).unwrap_or_default();
    let bytes = serde_json::to_vec(&doc).unwrap_or_default();
    let _ = bicraft_http::write_response(
        stream,
        Status::Ok,
        "application/json",
        &[
            ("Cache-Control", "public, max-age=3600"),
            ("Access-Control-Allow-Origin", "*"),
        ],
        &bytes,
    );
    Ok(Done::Answered)
}

impl State {
    fn host_ok(&self, host: &str) -> bool {
        match &self.config.hosts {
            HostPolicy::Loopback => bicraft_http::host_allowed(host),
            HostPolicy::Names(names) => names.iter().any(|n| n.eq_ignore_ascii_case(host.trim())),
        }
    }

    fn origin_ok(&self, origin: &str) -> bool {
        self.config
            .allowed_origins
            .iter()
            .any(|o| o.eq_ignore_ascii_case(origin.trim()))
    }

    /// The workspace folder of `id`: an existing subfolder of the root whose real location (links
    /// and junctions resolved) is the root's direct child named exactly `id`, so a workspace can
    /// never be another one (a link to it, or another spelling on a case-insensitive file
    /// system) or a folder outside the root. `None`: no such workspace.
    fn workspace_dir(&self, id: &str) -> Option<PathBuf> {
        let dir = std::fs::canonicalize(self.root.join(id)).ok()?;
        let exact =
            dir.parent() == Some(self.root.as_path()) && dir.file_name() == Some(std::ffi::OsStr::new(id));
        (exact && dir.is_dir()).then_some(dir)
    }

    /// A server event in the configured log format ([`log_event`]).
    fn log(&self, level: &str, event: &str, message: &str) {
        log_event(
            self.config.log_format,
            self.config.log_sink.as_ref(),
            level,
            event,
            message,
        );
    }

    /// Milliseconds since the server started.
    fn now_ms(&self) -> u64 {
        u64::try_from(self.epoch.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// The workspace's session, created on first use (with the file an idle close left to
    /// reopen), and its queue place: the number of calls that held one before. The place is
    /// taken under the map's lock (see [`Workspace::pending`]); the caller releases it with
    /// [`Queued`].
    fn checkout(&self, id: &str, dir: &Path) -> Result<(Arc<Workspace>, usize), WorkspaceError> {
        let mut map = self.workspaces.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(w) = map.get(id) {
            let place = w.pending.fetch_add(1, Ordering::SeqCst);
            return Ok((Arc::clone(w), place));
        }
        if map.len() >= self.config.max_workspaces {
            return Err(WorkspaceError::TooMany);
        }
        let mut session = new_session(dir, self.config.allow_network).map_err(WorkspaceError::Session)?;
        session.set_recovery(self.recovery_config(id, None));
        // revisions never repeat for a workspace: an `expected_revision` read from an older
        // session (before an idle close, or before a restart) never matches this one
        session.revision = self.revision_floor(id);
        let reopen = self.closed.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
        let w = Workspace::new(session, self.config.max_concurrent_reads, self.now_ms());
        w.pending.store(1, Ordering::SeqCst);
        *w.reopen.lock().unwrap_or_else(|e| e.into_inner()) = reopen;
        let w = Arc::new(w);
        map.insert(id.to_string(), Arc::clone(&w));
        Ok((w, 0))
    }

    /// The first revision of a new session of workspace `id`: above every revision its earlier
    /// sessions in this process reached, and above the wall clock in milliseconds (above what
    /// an earlier process can have reached, unless it ran more than one command per millisecond).
    fn revision_floor(&self, id: &str) -> u64 {
        let clock = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
        let last = self
            .last_revisions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .copied()
            .unwrap_or(0);
        clock.max(last.saturating_add(1))
    }

    /// The journal settings of an editor session of workspace `id` (`owner`: a document view's
    /// identity; `None`: the workspace's shared session), when auto-recovery is on.
    pub(super) fn recovery_config(&self, id: &str, owner: Option<&str>) -> Option<RecoveryConfig> {
        let dir = self.config.recovery_dir.as_ref()?.join(id);
        let cfg = RecoveryConfig::new(dir);
        Some(match owner {
            Some(o) => cfg.with_owner_of(o),
            None => cfg,
        })
    }

    /// Remembers the revision a closing session of `id` reached ([`State::revision_floor`]).
    pub(super) fn note_revision(&self, id: &str, revision: u64) {
        let mut map = self.last_revisions.lock().unwrap_or_else(|e| e.into_inner());
        let e = map.entry(id.to_string()).or_insert(0);
        *e = (*e).max(revision);
    }

    /// The workspace's session when one is open, with a queue place taken (released by
    /// [`Queued`]); `None` when it is not open.
    fn checkout_open(&self, id: &str) -> Option<Arc<Workspace>> {
        let map = self.workspaces.lock().unwrap_or_else(|e| e.into_inner());
        let w = map.get(id)?;
        w.pending.fetch_add(1, Ordering::SeqCst);
        Some(Arc::clone(w))
    }
}

/// Why a workspace's session is not available.
enum WorkspaceError {
    /// Too many open workspaces (503).
    TooMany,
    /// The folder cannot be a server root (it vanished, or cannot be read).
    Session(String),
}

/// A fresh server-mode session rooted at the workspace folder `dir` (`docs/server-mode.md`):
/// every path parameter is resolved inside `dir` (absolute paths, `..` and links leaving it are
/// refused), commands denied in server mode are refused, and network access follows
/// `allow_network`.
pub fn new_session(dir: &Path, allow_network: bool) -> Result<Session, String> {
    Ok(Session::server(
        ServerConfig::new(dir)?.allow_network(allow_network),
    ))
}

// ---------------------------------------------------------------------------------------------
// Responses

fn server_info() -> Value {
    json!({"name": "bicraft", "title": "BIcraft", "version": env!("CARGO_PKG_VERSION")})
}

/// A JSON-RPC error message; `id` is left out when `None` (the request's id was unreadable).
fn rpc_error(id: Option<&Value>, code: i64, message: &str, data: Option<Value>) -> Value {
    let mut error = Map::new();
    error.insert("code".into(), json!(code));
    error.insert("message".into(), json!(message));
    if let Some(d) = data {
        error.insert("data".into(), d);
    }
    let mut o = Map::new();
    o.insert("jsonrpc".into(), json!("2.0"));
    if let Some(id) = id {
        o.insert("id".into(), id.clone());
    }
    o.insert("error".into(), Value::Object(error));
    Value::Object(o)
}

/// A result object completed with the revision's fields: `resultType` and the server's
/// identity in `_meta`.
fn complete(result: Value) -> Value {
    let mut o = match result {
        Value::Object(o) => o,
        other => {
            let mut o = Map::new();
            o.insert("value".into(), other);
            o
        }
    };
    o.insert("resultType".into(), json!("complete"));
    let meta = o.entry("_meta").or_insert_with(|| json!({}));
    if let Value::Object(m) = meta {
        m.insert(META_SERVER_INFO.into(), server_info());
    }
    Value::Object(o)
}

fn rpc_result(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": complete(result)})
}

fn write_json(stream: &mut TcpStream, status: Status, body: &Value, extra: &[(&str, &str)]) {
    let bytes = serde_json::to_vec(body).unwrap_or_else(|_| {
        br#"{"jsonrpc":"2.0","error":{"code":-32603,"message":"serialization failed"}}"#.to_vec()
    });
    let mut headers = vec![("Cache-Control", "no-store")];
    headers.extend_from_slice(extra);
    let _ = bicraft_http::write_response(stream, status, "application/json", &headers, &bytes);
}

/// An error answer: `status` with a JSON-RPC error body.
struct Reject {
    status: Status,
    body: Value,
    extra: Vec<(&'static str, String)>,
}

impl Reject {
    fn new(status: Status, id: Option<&Value>, code: i64, message: &str) -> Self {
        Self {
            status,
            body: rpc_error(id, code, message, None),
            extra: Vec::new(),
        }
    }

    fn with_data(mut self, data: Value) -> Self {
        if let Some(e) = self.body.get_mut("error").and_then(Value::as_object_mut) {
            e.insert("data".into(), data);
        }
        self
    }

    /// A plain JSON error for the file routes: `{"error": {"code", "message"}}`.
    fn plain(status: Status, code: i64, message: &str) -> Self {
        Self {
            status,
            body: json!({"error": {"code": code, "message": message}}),
            extra: Vec::new(),
        }
    }

    /// The same error without the JSON-RPC envelope (the file routes are not JSON-RPC).
    fn for_files(mut self) -> Self {
        if let Some(e) = self.body.get("error").cloned() {
            self.body = json!({"error": e});
        }
        self
    }

    fn send(self, stream: &mut TcpStream) {
        let extra: Vec<(&str, &str)> = self.extra.iter().map(|(k, v)| (*k, v.as_str())).collect();
        write_json(stream, self.status, &self.body, &extra);
    }
}

// ---------------------------------------------------------------------------------------------
// Connections

/// What to do with a connection once its request is answered.
enum Close {
    /// Nothing was read: just drop it.
    Drop,
    /// Half-close and discard leftover input ([`bicraft_http::finish`]).
    Finish,
    /// A cancelled request: nothing more is sent.
    Reset,
}

fn handle_connection(mut stream: TcpStream, peer: IpAddr, state: &State) {
    if stream.set_write_timeout(Some(WRITE_TIMEOUT)).is_err() {
        return;
    }
    reqlog::begin();
    let close = respond(&mut stream, peer, state);
    // counted and logged before the lingering close, which is not the request's time
    if !matches!(close, Close::Drop) {
        reqlog::end(state);
    }
    match close {
        Close::Drop => {}
        Close::Finish => bicraft_http::finish(stream),
        Close::Reset => {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
    }
}

fn respond(stream: &mut TcpStream, peer: IpAddr, state: &State) -> Close {
    let unique: Vec<&str> = state.unique.iter().map(String::as_str).collect();
    let (head, rest) = match bicraft_http::read_request(stream, &state.config.head_limits, &unique) {
        Ok(r) => r,
        Err(ReadError::Close) => return Close::Drop,
        Err(ReadError::Respond(status)) => {
            bicraft_http::respond_status(stream, status, &[]);
            return Close::Finish;
        }
    };
    reqlog::head(&head);
    // probes: before the Host check (kubelet sends the pod address), no authentication, no data
    if ops::probe(stream, state, &head) {
        return Close::Finish;
    }
    // counted first, then the drain flag checked: a drain that sees no active request never
    // misses one that is about to start
    let _active = ops::Active::enter(&state.active);
    if state.draining.load(Ordering::SeqCst) {
        reqlog::error("shutting_down");
        ops::refuse_draining(stream, &head);
        return Close::Finish;
    }
    match serve(stream, peer, state, &head, rest) {
        Ok(Done::Answered) => {}
        Ok(Done::Cancelled) => {
            reqlog::outcome("cancelled", Some("cancelled"));
            return Close::Reset;
        }
        Err(mut reject) => {
            paths::scrub(&mut reject.body, &paths::prefixes(&state.root));
            let code = reject
                .body
                .get("error")
                .and_then(|e| e.get("code"))
                .and_then(Value::as_i64);
            // a known JSON-RPC code names the error; anything else falls back to the status class
            if let Some(kind) = code.map(reqlog::error_kind).filter(|k| *k != "other") {
                reqlog::error(kind);
            }
            reject.send(stream);
        }
    }
    Close::Finish
}

enum Done {
    Answered,
    Cancelled,
}

/// Checks the request in the order of the spec and serves it.
fn serve(
    stream: &mut TcpStream,
    peer: IpAddr,
    state: &State,
    head: &Head,
    rest: Vec<u8>,
) -> Result<Done, Reject> {
    let plain = |stream: &mut TcpStream, status: Status, extra: &[(&str, &str)]| {
        bicraft_http::respond_status(stream, status, extra);
        Ok(Done::Answered)
    };
    let host = match head.header("host") {
        None => return plain(stream, Status::BadRequest, &[]),
        Some(h) if !state.host_ok(h) => return plain(stream, Status::Forbidden, &[]),
        Some(h) => h.trim(),
    };
    if let Some(suffix) = head.path.strip_prefix(RESOURCE_METADATA_PATH) {
        return resource_metadata(stream, state, head, host, suffix);
    }
    if let Some((id, rest_path)) = viewer::route(&head.path) {
        return viewer::serve(stream, peer, state, head, rest, host, id, rest_path);
    }
    if let Some((id, rest_path)) = files::route(&head.path) {
        // reads may use the viewer's login cookie (its download link); writes never do
        let reading = matches!(head.method.as_str(), "GET" | "HEAD");
        let access = if state.config.auth.is_none() {
            reqlog::caller("local");
            None
        } else {
            Some(viewer::authenticate(state, peer, head, host, id, reading)?.access)
        };
        return files::serve(stream, state, head, rest, id, rest_path, access);
    }
    let (id, view) = match (route(&head.path), view_route(&head.path)) {
        (Some(id), _) => (id, None),
        (None, Some((id, vid))) => (id, Some(vid)),
        (None, None) => return plain(stream, Status::NotFound, &[]),
    };
    // no authentication: every caller is an editor
    let caller = authorize(state, peer, head, host, id)?;
    let role = caller
        .as_ref()
        .map_or(tools::Role::Editor, |(a, _)| tools::Role::from(*a));
    let who = caller.map(|(_, k)| k).unwrap_or_default();
    let not_found = || Reject::new(Status::NotFound, None, WORKSPACE_NOT_FOUND, "no such workspace");
    if head.method != "POST" {
        // no GET stream and no DELETE in this revision (no protocol session)
        return plain(stream, Status::MethodNotAllowed, &[("Allow", "POST")]);
    }
    if let Some(origin) = head.header("origin")
        && !state.origin_ok(origin)
    {
        return Err(Reject::new(
            Status::Forbidden,
            None,
            INVALID_REQUEST,
            "this Origin is not allowed",
        ));
    }
    let Some(dir) = state.workspace_dir(id) else {
        return Err(not_found());
    };
    let json_type = head.header("content-type").is_some_and(|t| {
        t.split(';')
            .next()
            .is_some_and(|m| m.trim().eq_ignore_ascii_case("application/json"))
    });
    if !json_type {
        return Err(Reject::new(
            Status::UnsupportedMedia,
            None,
            INVALID_REQUEST,
            "the body must be application/json",
        ));
    }
    let Some(length) = head.content_length else {
        return Err(Reject::new(
            Status::LengthRequired,
            None,
            INVALID_REQUEST,
            "Content-Length is required",
        ));
    };
    if length > state.config.max_body_bytes {
        return Err(Reject::new(
            Status::PayloadTooLarge,
            None,
            INVALID_REQUEST,
            &format!("the body is larger than {} bytes", state.config.max_body_bytes),
        ));
    }
    let accept = Accept::of(head);
    if !accept.json && !accept.sse {
        return Err(Reject::new(
            Status::NotAcceptable,
            None,
            INVALID_REQUEST,
            "Accept must list application/json and text/event-stream",
        ));
    }
    let body = bicraft_http::read_body(stream, rest, length, state.config.body_deadline)
        .map_err(|status| Reject::new(status, None, INVALID_REQUEST, status.line()))?;
    let msg: Value = serde_json::from_slice(&body).map_err(|e| {
        Reject::new(
            Status::BadRequest,
            None,
            PARSE_ERROR,
            &format!("parse error: {e}"),
        )
    })?;
    drop(body);
    let Value::Object(o) = &msg else {
        let why = if msg.is_array() {
            "JSON-RPC batches are not supported: one message per POST"
        } else {
            "the body must be one JSON-RPC message (an object)"
        };
        return Err(Reject::new(Status::BadRequest, None, INVALID_REQUEST, why));
    };
    let Some(method) = o.get("method").and_then(Value::as_str) else {
        let why = if o.contains_key("result") || o.contains_key("error") {
            "clients must not send JSON-RPC responses"
        } else {
            "missing string `method`"
        };
        let id = o.get("id").filter(|i| valid_id(i));
        return Err(Reject::new(Status::BadRequest, id, INVALID_REQUEST, why));
    };
    reqlog::rpc(method);
    let Some(rid) = o.get("id") else {
        // a notification: accepted (this revision defines none on HTTP); a mirrored method
        // header, when sent, must still agree
        if let Some(h) = head.header(H_METHOD)
            && h != method
        {
            return Err(Reject::new(
                Status::BadRequest,
                None,
                HEADER_MISMATCH,
                &format!(
                    "Header mismatch: {H_METHOD} header value '{h}' does not match body value '{method}'"
                ),
            ));
        }
        let _ = bicraft_http::write_empty(stream, Status::Accepted, &[]);
        return Ok(Done::Answered);
    };
    if !valid_id(rid) {
        return Err(Reject::new(
            Status::BadRequest,
            None,
            INVALID_REQUEST,
            "`id` must be a string or a number",
        ));
    }
    let params = match o.get("params") {
        None | Some(Value::Null) => Value::Object(Map::new()),
        Some(p @ Value::Object(_)) => p.clone(),
        Some(_) => {
            return Err(Reject::new(
                Status::BadRequest,
                Some(rid),
                INVALID_PARAMS,
                "`params` must be an object",
            ));
        }
    };
    if method == "initialize" {
        // a legacy client: no fall-forward exists, so say which versions are served
        let requested = params.get("protocolVersion").cloned().unwrap_or(Value::Null);
        return Err(Reject::new(
            Status::BadRequest,
            Some(rid),
            UNSUPPORTED_PROTOCOL_VERSION,
            "this endpoint speaks MCP 2026-07-28 only: no initialize handshake; send each request \
             with _meta io.modelcontextprotocol/protocolVersion and the MCP-Protocol-Version header",
        )
        .with_data(json!({"supported": HTTP_SUPPORTED_VERSIONS, "requested": requested})));
    }
    check_headers(head, rid, method, &params)?;
    check_meta(head, rid, &params)?;
    match method {
        "server/discover" => {
            write_json(
                stream,
                Status::Ok,
                &rpc_result(
                    rid,
                    json!({
                        "supportedVersions": HTTP_SUPPORTED_VERSIONS,
                        "capabilities": {"tools": {}},
                        "instructions": INSTRUCTIONS,
                    }),
                ),
                &[],
            );
            Ok(Done::Answered)
        }
        "tools/list" => {
            write_json(
                stream,
                Status::Ok,
                &rpc_result(
                    rid,
                    json!({
                        "tools": tools::server_tool_definitions_for(role),
                        "ttlMs": TOOLS_TTL_MS,
                        "cacheScope": "private",
                    }),
                ),
                &[],
            );
            Ok(Done::Answered)
        }
        "tools/call" => {
            let view = view.map(|vid| (vid, head.header(bicraft_view::TOKEN_HEADER)));
            call(
                stream, state, id, &dir, rid, &params, accept.sse, role, &who, view,
            )
        }
        other => Err(Reject::new(
            Status::NotFound,
            Some(rid),
            METHOD_NOT_FOUND,
            &format!("method not found: {other}"),
        )),
    }
}

/// Authenticates a request to workspace `id` (MCP endpoint or files): `None` without
/// authentication, else the identity's access level and its key ([`readview::identity_key`]).
/// Authentication comes before anything that
/// could tell whether the workspace exists; a workspace the identity may not open is answered
/// exactly like a missing one (404). The accepted token audiences are the workspace's MCP
/// endpoint and the server's base URL, for the file routes too, so one token serves both.
fn authorize(
    state: &State,
    peer: IpAddr,
    head: &Head,
    host: &str,
    id: &str,
) -> Result<Option<(Access, String)>, Reject> {
    let Some(auth) = &state.config.auth else {
        reqlog::caller("local");
        return Ok(None);
    };
    let base = public_base(auth, host);
    let resource = format!("{base}/w/{id}/mcp");
    reqlog::caller("unauthenticated");
    let identity = auth
        .authenticate(peer, |n| head.header(n), &[resource, base.clone()])
        .map_err(|d| unauthorized(auth, &d, &format!("{base}{RESOURCE_METADATA_PATH}/w/{id}/mcp")))?;
    // the mode only (the label's prefix): never the subject
    reqlog::caller(match identity.label().split(':').next() {
        Some("oidc") => "oidc",
        Some("proxy") => "proxy",
        Some("token") => "token",
        _ => "other",
    });
    auth.access(&identity, id)
        .map(|a| Some((a, readview::identity_key(&identity))))
        .ok_or_else(|| Reject::new(Status::NotFound, None, WORKSPACE_NOT_FOUND, "no such workspace"))
}

fn valid_id(id: &Value) -> bool {
    id.is_string() || id.is_number()
}

/// What the client's `Accept` lists.
struct Accept {
    json: bool,
    sse: bool,
}

impl Accept {
    fn of(head: &Head) -> Self {
        let mut any = false;
        let (mut json, mut sse, mut wildcard) = (false, false, false);
        for value in head.headers_named("accept") {
            for item in value.split(',') {
                any = true;
                let media = item.split(';').next().unwrap_or_default().trim();
                if media.eq_ignore_ascii_case("application/json") {
                    json = true;
                } else if media.eq_ignore_ascii_case("text/event-stream") {
                    sse = true;
                } else if media == "*/*" || media.eq_ignore_ascii_case("application/*") {
                    wildcard = true;
                }
            }
        }
        // no Accept at all accepts anything; a wildcard gets plain JSON
        Self {
            json: json || wildcard || !any,
            sse,
        }
    }
}

/// A header value as RFC 9110 allows it: visible ASCII, spaces and tabs.
fn header_safe(v: &str) -> bool {
    v.bytes().all(|b| b == b'\t' || (b' '..=b'~').contains(&b))
}

/// Checks the mirrored headers against the body (`-32020` otherwise).
fn check_headers(head: &Head, rid: &Value, method: &str, params: &Value) -> Result<(), Reject> {
    let mismatch = |why: String| Reject::new(Status::BadRequest, Some(rid), HEADER_MISMATCH, &why);
    for name in [H_PROTOCOL_VERSION, H_METHOD, H_NAME] {
        if head.header(name).is_some_and(|v| !header_safe(v)) {
            return Err(mismatch(format!(
                "Header mismatch: {name} has invalid characters"
            )));
        }
    }
    if head.header(H_PROTOCOL_VERSION).is_none_or(str::is_empty) {
        return Err(mismatch(format!(
            "Header mismatch: the {H_PROTOCOL_VERSION} header is required"
        )));
    }
    match head.header(H_METHOD) {
        None => {
            return Err(mismatch(format!(
                "Header mismatch: the {H_METHOD} header is required"
            )));
        }
        Some(h) if h != method => {
            return Err(mismatch(format!(
                "Header mismatch: {H_METHOD} header value '{h}' does not match body value '{method}'"
            )));
        }
        Some(_) => {}
    }
    let named = match method {
        "tools/call" | "prompts/get" => Some(params.get("name")),
        "resources/read" => Some(params.get("uri")),
        _ => None,
    };
    if let Some(body_name) = named {
        let Some(raw) = head.header(H_NAME) else {
            return Err(mismatch(format!(
                "Header mismatch: the {H_NAME} header is required for {method}"
            )));
        };
        let Some(value) = decode_header_value(raw) else {
            return Err(mismatch(format!("Header mismatch: {H_NAME} is not valid base64")));
        };
        if body_name.and_then(Value::as_str) != Some(value.as_str()) {
            return Err(mismatch(format!(
                "Header mismatch: {H_NAME} header value does not match the body's {}",
                if method == "resources/read" {
                    "params.uri"
                } else {
                    "params.name"
                }
            )));
        }
    }
    Ok(())
}

/// Checks the per-request `_meta` fields (`-32602` when missing), that the version header agrees
/// with them (`-32020`) and that the version is served (`-32022`).
fn check_meta(head: &Head, rid: &Value, params: &Value) -> Result<(), Reject> {
    let invalid = |why: &str| Reject::new(Status::BadRequest, Some(rid), INVALID_PARAMS, why);
    let meta = params.get("_meta").and_then(Value::as_object);
    let Some(version) = meta
        .and_then(|m| m.get(META_PROTOCOL_VERSION))
        .and_then(Value::as_str)
    else {
        return Err(invalid(&format!(
            "params._meta[\"{META_PROTOCOL_VERSION}\"] is required"
        )));
    };
    if !meta
        .and_then(|m| m.get(META_CLIENT_CAPABILITIES))
        .is_some_and(Value::is_object)
    {
        return Err(invalid(&format!(
            "params._meta[\"{META_CLIENT_CAPABILITIES}\"] is required"
        )));
    }
    let header = head.header(H_PROTOCOL_VERSION).unwrap_or_default();
    if header != version {
        return Err(Reject::new(
            Status::BadRequest,
            Some(rid),
            HEADER_MISMATCH,
            &format!(
                "Header mismatch: {H_PROTOCOL_VERSION} header value '{header}' does not match body value '{version}'"
            ),
        ));
    }
    if !HTTP_SUPPORTED_VERSIONS.contains(&version) {
        return Err(Reject::new(
            Status::BadRequest,
            Some(rid),
            UNSUPPORTED_PROTOCOL_VERSION,
            "Unsupported protocol version",
        )
        .with_data(json!({"supported": HTTP_SUPPORTED_VERSIONS, "requested": version})));
    }
    Ok(())
}

/// A header value, decoded when it uses the `=?base64?…?=` sentinel; `None` when that encoding
/// is broken or does not decode to UTF-8.
fn decode_header_value(raw: &str) -> Option<String> {
    match raw.strip_prefix("=?base64?").and_then(|r| r.strip_suffix("?=")) {
        Some(b64) => String::from_utf8(base64_decode(b64)?).ok(),
        None => Some(raw.to_string()),
    }
}

/// Standard base64 with padding (RFC 4648 §4).
fn base64_decode(s: &str) -> Option<Vec<u8>> {
    fn sextet(c: u8) -> Option<u32> {
        Some(u32::from(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        }))
    }
    let bytes = s.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    let groups = bytes.chunks(4).count();
    for (i, chunk) in bytes.chunks(4).enumerate() {
        let last = i + 1 == groups;
        let pad = chunk.iter().rev().take_while(|&&c| c == b'=').count();
        if pad > 2 || (pad > 0 && !last) {
            return None;
        }
        let mut n = 0u32;
        for (j, &c) in chunk.iter().enumerate() {
            let v = if j >= 4 - pad { 0 } else { sextet(c)? };
            n = (n << 6) | v;
        }
        let [_, a, b, c] = n.to_be_bytes();
        out.push(a);
        if pad < 2 {
            out.push(b);
        }
        if pad < 1 {
            out.push(c);
        }
    }
    Some(out)
}

// ---------------------------------------------------------------------------------------------
// tools/call

/// Releases a queue place on drop.
struct Queued<'a>(&'a AtomicUsize);

impl Drop for Queued<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

#[allow(clippy::too_many_arguments)]
fn call(
    stream: &mut TcpStream,
    state: &State,
    id: &str,
    dir: &Path,
    rid: &Value,
    params: &Value,
    sse: bool,
    role: tools::Role,
    who: &str,
    view: Option<(&str, Option<&str>)>,
) -> Result<Done, Reject> {
    let name = params.get("name").and_then(Value::as_str).ok_or_else(|| {
        Reject::new(
            Status::BadRequest,
            Some(rid),
            INVALID_PARAMS,
            "missing string `name`",
        )
    })?;
    let args = match params.get("arguments") {
        None | Some(Value::Null) => json!({}),
        Some(a @ Value::Object(_)) => a.clone(),
        Some(_) => {
            return Err(Reject::new(
                Status::BadRequest,
                Some(rid),
                INVALID_PARAMS,
                "`arguments` must be an object",
            ));
        }
    };
    if !tools::is_tool(name) {
        return Err(Reject::new(
            Status::BadRequest,
            Some(rid),
            INVALID_PARAMS,
            &format!("unknown tool `{name}` (see tools/list)"),
        ));
    }
    // fixed labels only: the tool's static name and the engine command id, never a parameter
    let label = tools::command_label(name, &args);
    if let Some(tool) = tools::static_tool_name(name) {
        reqlog::tool(tool, label);
    }
    let command = state.metrics.command_index(label);
    let busy = |why: &str| {
        state.metrics.busy.fetch_add(1, Ordering::Relaxed);
        let mut r = Reject::new(Status::Unavailable, Some(rid), WORKSPACE_BUSY, why);
        r.extra.push(("Retry-After", "1".into()));
        r
    };
    let reader = role == tools::Role::Reader && state.config.auth.is_some();
    // a document view's endpoint runs on that view's own session (docs/mcp.md, "Document views")
    let mut bound: Option<&str> = None;
    let (workspace, place) = if let Some((vid, token)) = view {
        let views = state.views(id);
        viewer::purge_doc_views(state, id, &views);
        let key = if who.is_empty() { "local" } else { who };
        match views.doc_binding(vid, key, token, state.now_ms()) {
            Err((status, why)) => {
                let code = if status == Status::NotFound {
                    WORKSPACE_NOT_FOUND
                } else {
                    INVALID_REQUEST
                };
                return Err(Reject::new(status, Some(rid), code, why));
            }
            // a reader's view reads the saved state of its file
            Ok(viewer::DocView::Read(_)) => {
                return reader_call(
                    stream,
                    state,
                    id,
                    dir,
                    rid,
                    name,
                    &args,
                    sse,
                    who,
                    command,
                    Some(vid),
                );
            }
            Ok(viewer::DocView::Edit(_)) if reader => {
                return Err(Reject::new(
                    Status::Forbidden,
                    Some(rid),
                    INVALID_REQUEST,
                    "this view was opened with editor access, which this identity no longer has: reload it",
                ));
            }
            Ok(viewer::DocView::Edit(w)) => {
                bound = Some(vid);
                let place = w.pending.fetch_add(1, Ordering::SeqCst);
                (w, place)
            }
        }
    } else {
        // a reader reads the saved state, never the editor session's unsaved edits (readview.rs)
        if reader {
            return reader_call(stream, state, id, dir, rid, name, &args, sse, who, command, None);
        }
        state.checkout(id, dir).map_err(|e| match e {
            WorkspaceError::TooMany => busy("too many open workspaces; try again later"),
            WorkspaceError::Session(why) => Reject::new(
                Status::InternalError,
                Some(rid),
                -32603,
                &format!("the workspace cannot be opened: {why}"),
            ),
        })?
    };
    let _queued = Queued(&workspace.pending);
    if place >= state.config.queue_per_workspace {
        return Err(busy("the workspace's queue is full; try again"));
    }
    let queued_at = Instant::now();
    let Ok(watch_stream) = stream.try_clone() else {
        return Err(Reject::new(
            Status::InternalError,
            Some(rid),
            -32603,
            "internal error",
        ));
    };
    if sse && bicraft_http::write_sse_head(stream, &[]).is_err() {
        return Ok(Done::Cancelled);
    }
    let cancel = Arc::new(AtomicBool::new(false));
    // the drain raises every running call's flag when the grace period ends
    let _registered = ops::Running::register(state, Arc::clone(&cancel));
    let keepalive = sse.then_some(state.config.keepalive);
    // read-only calls share the session (`rwgate.rs`); a call that may change it, or a pending
    // reopen to apply first, runs alone
    let reopen_pending = workspace
        .reopen
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_some();
    let shared = !reopen_pending && tools::runs_shared(name, &args);
    // the file revisions of the workspace's files (per-file save checks)
    let files = state.views(id);
    let mut guard: Option<SessionGuard<'_>> = None;
    let mut result = None;
    let mut ran: Option<Duration> = None;
    let mut changed = false;
    let mut refused = false;
    let mut revision = 0u64;
    thread::scope(|_| {
        let watcher = Watcher::start(watch_stream, &cancel, keepalive);
        let session = if shared {
            SessionGuard::Read(workspace.session.read(Some(&state.metrics.waiting)))
        } else {
            let t = Instant::now();
            let g = workspace.session.write(Some(&state.metrics.waiting));
            state.metrics.write_wait.observe(t.elapsed());
            SessionGuard::Write(g)
        };
        let session = guard.insert(session);
        state.metrics.queue_wait.observe(queued_at.elapsed());
        let _running = ops::Active::enter(&state.metrics.running);
        let _reading = shared.then(|| ops::Active::enter(&state.metrics.reads));
        workspace.last_used.store(state.now_ms(), Ordering::SeqCst);
        workspace.warned.store(false, Ordering::SeqCst);
        if !cancel.load(Ordering::SeqCst) {
            let before = session.session().revision;
            let t = Instant::now();
            let (r, was_refused) = match session {
                SessionGuard::Read(s) => {
                    tools::call_tool_shared(s, name, &args, role, Some(Arc::clone(&cancel)))
                }
                SessionGuard::Write(session) => {
                    // a file an idle close (or an upload over the open file) left to reopen
                    let reopen = workspace.reopen.lock().unwrap_or_else(|e| e.into_inner()).take();
                    if let Some(r) = reopen {
                        let t = Instant::now();
                        changed = true;
                        let ok = r.apply(session);
                        // a lazy reopen is a file.open too
                        let outcome = if ok {
                            metrics::Outcome::Ok
                        } else {
                            metrics::Outcome::Error
                        };
                        let open = state.metrics.command_index("file.open");
                        state.metrics.command(open, t.elapsed(), outcome);
                        let counter = if ok {
                            &state.metrics.reopens
                        } else {
                            state.log(
                                "warn",
                                "reopen",
                                &format!(
                                    "workspace {id}: its file could not be reopened after an idle close"
                                ),
                            );
                            &state.metrics.reopen_failures
                        };
                        counter.fetch_add(1, Ordering::Relaxed);
                        // the reopened file is read as it is on disk now
                        files.note(&workspace.file_seen, session, true);
                    }
                    // a save over a file another session saved since this one read it: refused,
                    // nothing written (docs/mcp.md, "Document views")
                    // (a refresh of the open PBIX writes it in place too)
                    let refreshes = tools::refreshes_in_place(name, &args, session);
                    let conflict = (tools::saves_in_place(name, &args) || refreshes)
                        .then(|| files.conflict(&workspace.file_seen, session))
                        .flatten();
                    match conflict {
                        Some(c) => {
                            state.metrics.conflicts.fetch_add(1, Ordering::Relaxed);
                            (c, false)
                        }
                        None => {
                            let before = session.revision;
                            session.set_cancel(Some(Arc::clone(&cancel)));
                            let r = tools::call_tool_role(session, name, &args, role);
                            session.set_cancel(None);
                            changed |= session.revision != before;
                            let ok = !r.0.is_error && !r.1;
                            if ok && (tools::saves(name, &args) || refreshes) {
                                files.saved(&workspace.file_seen, session);
                            } else {
                                files.note(&workspace.file_seen, session, ok && label == "file.open");
                            }
                            r
                        }
                    }
                }
            };
            if tools::is_conflict(&r) {
                state.metrics.conflicts.fetch_add(1, Ordering::Relaxed);
            }
            changed |= session.session().revision != before;
            revision = session.session().revision;
            result = Some(r);
            refused = was_refused;
            ran = Some(t.elapsed());
        }
        workspace.last_used.store(state.now_ms(), Ordering::SeqCst);
        watcher.finish();
    });
    let cancelled = cancel.load(Ordering::SeqCst);
    if let Some(took) = ran {
        let outcome = match &result {
            _ if cancelled => metrics::Outcome::Cancelled,
            _ if refused => metrics::Outcome::Refused,
            Some(r) if r.is_error => metrics::Outcome::Error,
            _ => metrics::Outcome::Ok,
        };
        state.metrics.command(command, took, outcome);
        state.metrics.workspace_call(id, took);
        // what viewers show: a changed session refreshes them; a write makes copies stale. A
        // document view's change refreshes that view only.
        let wrote = outcome == metrics::Outcome::Ok && WRITING_LABELS.contains(&label);
        match bound {
            Some(vid) => {
                if wrote {
                    files.disk_changed();
                }
                if changed || wrote {
                    files.bump_view(vid);
                }
            }
            None => {
                if wrote {
                    files.disk_changed();
                } else if changed {
                    files.bump();
                }
            }
        }
        match outcome {
            metrics::Outcome::Ok => reqlog::outcome("ok", None),
            metrics::Outcome::Error => reqlog::outcome("error", Some("command_error")),
            // the role refused it before anything ran (a reader calling an editing command)
            metrics::Outcome::Refused => reqlog::outcome("refused", Some("forbidden")),
            metrics::Outcome::Cancelled => {}
        }
    }
    let Some(result) = result.filter(|_| !cancelled) else {
        return Ok(Done::Cancelled);
    };
    // workspace-relative paths only (paths.rs)
    let mut value = result.into_value();
    paths::scrub(&mut value, &paths::workspace_prefixes(dir, &state.root));
    // the session's revision after the call (optimistic concurrency: `expected_revision`)
    with_revision(&mut value, revision);
    let message = rpc_result(rid, value);
    let text = serde_json::to_string(&message).unwrap_or_default();
    if sse {
        let event = bicraft_http::sse_event(None, &text);
        bicraft_http::note_bytes(u64::try_from(event.len()).unwrap_or(u64::MAX));
        let _ = stream.write_all(&event).and_then(|()| stream.flush());
    } else {
        let _ = bicraft_http::write_response(
            stream,
            Status::Ok,
            "application/json",
            &[("Cache-Control", "no-store")],
            text.as_bytes(),
        );
    }
    // after the answer, still holding the session: background work the command asked for
    if let Some(session) = guard.as_ref() {
        session.session().background_work();
    }
    drop(guard);
    Ok(Done::Answered)
}

/// The session a tool call runs on: shared (read-only calls) or alone.
enum SessionGuard<'a> {
    Read(rwgate::ReadGuard<'a, Session>),
    Write(rwgate::WriteGuard<'a, Session>),
}

impl SessionGuard<'_> {
    fn session(&self) -> &Session {
        match self {
            SessionGuard::Read(g) => g,
            SessionGuard::Write(g) => g,
        }
    }
}

/// The `_meta` key of a tool result carrying the session's revision after the call.
pub const META_REVISION: &str = "io.bicraft/revision";

/// Adds the session's revision to a tool result's `_meta` ([`META_REVISION`]).
fn with_revision(value: &mut Value, revision: u64) {
    if let Value::Object(o) = value
        && let Value::Object(m) = o.entry("_meta").or_insert_with(|| json!({}))
    {
        m.insert(META_REVISION.into(), json!(revision));
    }
}

/// Commands that write the workspace's files (a batch may hold one): read-only copies loaded
/// before them are stale ([`readview`]).
const WRITING_LABELS: &[&str] = &[
    "file.save",
    "file.save_as",
    "file.convert",
    "project.new",
    "model.export_vpax",
    "theme.export",
    "batch",
];

/// A reader's tool call (`docs/auth.md`, "Roles"): it runs on the reader's read view, the saved
/// state of its file ([`readview`]), never on the editor session's unsaved edits. A reader's
/// `file.open` is private: it names the file this reader reads from now on (served from the
/// shared read-only copy of that saved file) and never touches the editor session or its undo
/// history. Every copy busy: 503 with `Retry-After` (or the same JSON-RPC error as the stream's
/// event).
#[allow(clippy::too_many_arguments)]
fn reader_call(
    stream: &mut TcpStream,
    state: &State,
    id: &str,
    dir: &Path,
    rid: &Value,
    name: &str,
    args: &Value,
    sse: bool,
    who: &str,
    command: usize,
    view: Option<&str>,
) -> Result<Done, Reject> {
    let Ok(watch_stream) = stream.try_clone() else {
        return Err(Reject::new(
            Status::InternalError,
            Some(rid),
            -32603,
            "internal error",
        ));
    };
    if sse && bicraft_http::write_sse_head(stream, &[]).is_err() {
        return Ok(Done::Cancelled);
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let _registered = ops::Running::register(state, Arc::clone(&cancel));
    let keepalive = sse.then_some(state.config.keepalive);
    let mut outcome: Option<Result<(tools::ToolResult, bool), readview::ViewError>> = None;
    let mut ran = Duration::ZERO;
    thread::scope(|_| {
        let watcher = Watcher::start(watch_stream, &cancel, keepalive);
        let _running = ops::Active::enter(&state.metrics.running);
        let t = Instant::now();
        outcome = Some(reader_exec(state, id, dir, name, args, who, &cancel, view));
        ran = t.elapsed();
        watcher.finish();
    });
    let cancelled = cancel.load(Ordering::SeqCst);
    let message = match outcome {
        None => return Ok(Done::Cancelled),
        Some(Ok((result, refused))) => {
            let kind = match &result {
                _ if cancelled => metrics::Outcome::Cancelled,
                _ if refused => metrics::Outcome::Refused,
                r if r.is_error => metrics::Outcome::Error,
                _ => metrics::Outcome::Ok,
            };
            state.metrics.command(command, ran, kind);
            state.metrics.workspace_call(id, ran);
            match kind {
                metrics::Outcome::Ok => reqlog::outcome("ok", None),
                metrics::Outcome::Error => reqlog::outcome("error", Some("command_error")),
                metrics::Outcome::Refused => reqlog::outcome("refused", Some("forbidden")),
                metrics::Outcome::Cancelled => return Ok(Done::Cancelled),
            }
            rpc_result(rid, result.into_value())
        }
        Some(Err(readview::ViewError::Busy(why))) => {
            state.metrics.busy.fetch_add(1, Ordering::Relaxed);
            if !sse {
                let mut r = Reject::new(Status::Unavailable, Some(rid), WORKSPACE_BUSY, why);
                r.extra.push(("Retry-After", "2".into()));
                return Err(r);
            }
            reqlog::outcome("rejected", Some("busy"));
            rpc_error(
                Some(rid),
                WORKSPACE_BUSY,
                why,
                Some(json!({"retry_after_secs": 2})),
            )
        }
        Some(Err(readview::ViewError::Failed(why))) => {
            reqlog::outcome("error", Some("command_error"));
            rpc_result(rid, tools::ToolResult::error(why).into_value())
        }
    };
    let mut message = message;
    paths::scrub(&mut message, &paths::workspace_prefixes(dir, &state.root));
    if cancelled {
        return Ok(Done::Cancelled);
    }
    let text = serde_json::to_string(&message).unwrap_or_default();
    if sse {
        let event = bicraft_http::sse_event(None, &text);
        bicraft_http::note_bytes(u64::try_from(event.len()).unwrap_or(u64::MAX));
        let _ = stream.write_all(&event).and_then(|()| stream.flush());
    } else {
        let _ = bicraft_http::write_response(
            stream,
            Status::Ok,
            "application/json",
            &[("Cache-Control", "no-store")],
            text.as_bytes(),
        );
    }
    Ok(Done::Answered)
}

/// Runs a reader's call on its read view, or on the file of its document view `view`;
/// `(result, refused)`.
#[allow(clippy::too_many_arguments)]
fn reader_exec(
    state: &State,
    id: &str,
    dir: &Path,
    name: &str,
    args: &Value,
    who: &str,
    cancel: &Arc<AtomicBool>,
    view: Option<&str>,
) -> Result<(tools::ToolResult, bool), readview::ViewError> {
    let views = state.views(id);
    match readview::open_params(name, args) {
        Err(why) => Ok((tools::ToolResult::error(why), true)),
        Ok(Some(params)) => {
            let cfg = ServerConfig::new(dir).map_err(readview::ViewError::Failed)?;
            let spec = match ops::Reopen::from_open_params(&cfg, &params) {
                Ok(s) => s,
                Err(e) => return Ok((tools::ToolResult::error(e), false)),
            };
            let target = readview::Target::File(spec.clone());
            let (summary, served) = state.with_read_view(id, dir, &target, false, |s, _| {
                s.get().execute_shared("model.summary", &json!({}), None)
            })?;
            let path = spec.path().to_string();
            match view {
                // the document view reads this file from now on
                Some(vid) => views.set_view_file(vid, spec),
                None => views.set_reader_file(who, spec, state.now_ms()),
            }
            Ok((
                tools::ToolResult::ok(json!({
                    "path": path,
                    "private": true,
                    "session": served.name(),
                    "summary": summary.ok(),
                    "note": "a reader's file.open is private: this reader now reads the saved state \
                             of this file; the workspace's session is unchanged",
                })),
                false,
            ))
        }
        Ok(None) => {
            let file = match view {
                Some(vid) => views.view_file(vid),
                None => views.reader_file(who),
            };
            let target = match file {
                Some(spec) => readview::Target::File(spec),
                None => readview::Target::Follow,
            };
            // a read-only call shares the session it reads; one that must run alone (a read with an
            // explicit `cache_abf`) takes it alone
            let exclusive = !tools::runs_shared(name, args);
            let (r, _) = state.with_read_view(id, dir, &target, exclusive, |s, _| match s {
                readview::ViewSession::Shared(s) => {
                    tools::call_tool_shared(s, name, args, tools::Role::Reader, Some(Arc::clone(cancel)))
                }
                readview::ViewSession::Alone(s) => {
                    s.set_cancel(Some(Arc::clone(cancel)));
                    let r = tools::call_tool_role(s, name, args, tools::Role::Reader);
                    s.set_cancel(None);
                    r
                }
            })?;
            Ok(r)
        }
    }
}
/// Watches a request's connection on a thread of its own while the call runs: raises `cancel`
/// when the client closes it, and writes an SSE comment every `keepalive` (when the reply is a
/// stream). [`Watcher::finish`] returns at once: the call never waits for the watcher's poll
/// (it used to join it, which added up to [`WATCH_POLL`] to every tool call); the watcher sees
/// it is finished at its next poll and ends.
struct Watcher {
    /// Set when the call is over; held while a keepalive is written, so the answer is never
    /// interleaved with one.
    finished: Arc<Mutex<bool>>,
}

impl Watcher {
    fn start(stream: TcpStream, cancel: &Arc<AtomicBool>, keepalive: Option<Duration>) -> Watcher {
        let finished = Arc::new(Mutex::new(false));
        let (f, c) = (Arc::clone(&finished), Arc::clone(cancel));
        // without a thread (none left), the call still runs; it cannot be cancelled
        let _ = thread::Builder::new()
            .name("bicraft-watch".into())
            .spawn(move || watch(stream, &c, &f, keepalive));
        Watcher { finished }
    }

    /// The call is over: no keepalive and no cancel from now on (a keepalive being written ends
    /// first).
    fn finish(self) {
        *self.finished.lock().unwrap_or_else(|e| e.into_inner()) = true;
    }
}

fn watch(stream: TcpStream, cancel: &AtomicBool, finished: &Mutex<bool>, keepalive: Option<Duration>) {
    let mut stream = stream;
    let mut last = Instant::now();
    let over = || *finished.lock().unwrap_or_else(|e| e.into_inner());
    while !over() {
        if bicraft_http::peer_closed(&stream, WATCH_POLL) {
            if !over() {
                cancel.store(true, Ordering::SeqCst);
            }
            return;
        }
        if let Some(every) = keepalive
            && last.elapsed() >= every
        {
            let f = finished.lock().unwrap_or_else(|e| e.into_inner());
            if *f {
                return;
            }
            if stream.write_all(b":\n\n").and_then(|()| stream.flush()).is_err() {
                cancel.store(true, Ordering::SeqCst);
                return;
            }
            drop(f);
            last = Instant::now();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_ids_and_routes() {
        for ok in ["a", "sales-2026", "A_b.c", &"x".repeat(64)] {
            assert!(valid_workspace_id(ok), "{ok}");
        }
        for bad in [
            "",
            ".",
            "..",
            ".hidden",
            "a/b",
            "a\\b",
            "é",
            &"x".repeat(65),
            "a b",
        ] {
            assert!(!valid_workspace_id(bad), "{bad}");
        }
        assert_eq!(route("/w/sales/mcp"), Some("sales"));
        for bad in ["/w//mcp", "/w/../mcp", "/w/a/b/mcp", "/mcp", "/w/a/mcp/", "/w/a"] {
            assert_eq!(route(bad), None, "{bad}");
        }
    }

    #[test]
    fn base64_sentinel() {
        assert_eq!(decode_header_value("plain").as_deref(), Some("plain"));
        assert_eq!(
            decode_header_value("=?base64?SGVsbG8sIOS4lueVjA==?=").as_deref(),
            Some("Hello, 世界")
        );
        assert_eq!(
            decode_header_value("=?base64?IHBhZGRlZCA=?=").as_deref(),
            Some(" padded ")
        );
        assert_eq!(
            decode_header_value("=?base64?PT9iYXNlNjQ/bGl0ZXJhbD89?=").as_deref(),
            Some("=?base64?literal?=")
        );
        for bad in [
            "=?base64?abc?=",
            "=?base64?a===?=",
            "=?base64?ab=c?=",
            "=?base64?////?=",
        ] {
            assert!(decode_header_value(bad).is_none(), "{bad}");
        }
        assert_eq!(decode_header_value("=?base64??=").as_deref(), Some(""));
    }

    #[test]
    fn results_carry_the_revision_fields() {
        let r = complete(json!({"tools": []}));
        assert_eq!(r["resultType"], "complete");
        assert_eq!(r["_meta"][META_SERVER_INFO]["name"], "bicraft");
        assert_eq!(complete(json!(3))["value"], 3);
    }
}
