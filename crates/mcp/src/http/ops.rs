//! Operations of `bicraft serve`: the probes, the graceful drain, the idle close of workspaces
//! and the lazy reopen of their file (`docs/web-deployment.md`, "Operating `bicraft serve`").
//!
//! - `GET /healthz` (liveness) answers 200 while the accept loop dispatches connections, and
//!   `GET /readyz` (readiness) 200 unless the server drains (or, with
//!   [`HttpConfig::ready_memory_percent`](super::HttpConfig::ready_memory_percent), its reserved
//!   memory is above that share of `BICRAFT_MEMORY_LIMIT`). Both skip the `Host` check (the
//!   kubelet sends the pod's address) and authentication, and their bodies carry nothing but
//!   `ok` or the reason (`draining`, `memory`): no workspace name, no count.
//! - [`HttpServer::drain`]: new requests get 503 and `/readyz` 503 at once; running requests
//!   finish within the grace period; then running tool calls are cancelled (the engine's cancel
//!   flag stops a DAX evaluation) and uploads and downloads stop; the listener closes; every
//!   session is dropped **without saving** (sessions with unsaved changes are reported), which
//!   also deletes their spill files; the upload temp folders are removed.
//! - Idle close: a session unused for [`HttpConfig::idle_timeout`](super::HttpConfig::idle_timeout)
//!   is dropped, unless it holds unsaved changes (then it stays open and the reaper says so once).
//!   The file it had open is remembered ([`Reopen`]) and reopened by the next tool call.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use bicraft_engine::{Session, Source, UiLanguage};
use bicraft_http::{Head, Status};
use serde_json::{Value, json};

use super::{HttpServer, SHUTTING_DOWN, State, Workspace, route, rpc_error, view_route, write_json};

/// The folder of a workspace that holds upload temp files and extraction staging. Reserved:
/// the file routes never list, read or write it.
pub(super) const TEMP_DIR: &str = ".bicraft-tmp";

/// How long the drain waits, after cancelling, for the cancelled requests to end.
const CANCEL_WAIT: Duration = Duration::from_secs(5);
/// How long the drain waits for a session it closes (a call that ignores its cancel flag).
const CLOSE_WAIT: Duration = Duration::from_secs(1);
/// Retry hint of a 503 during the drain (another replica, or this pod restarted).
const DRAIN_RETRY_AFTER: &str = "5";

// ---------------------------------------------------------------------------------------------
// Counters

/// One request in progress ([`State::active`]); released on drop.
pub(super) struct Active<'a>(&'a AtomicUsize);

impl<'a> Active<'a> {
    pub(super) fn enter(counter: &'a AtomicUsize) -> Self {
        counter.fetch_add(1, Ordering::SeqCst);
        Self(counter)
    }
}

impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// A running tool call's cancel flag in [`State::running`]; removed on drop.
pub(super) struct Running<'a> {
    state: &'a State,
    key: u64,
}

impl<'a> Running<'a> {
    pub(super) fn register(state: &'a State, flag: Arc<AtomicBool>) -> Self {
        let key = state.next_call.fetch_add(1, Ordering::SeqCst);
        state
            .running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key, flag);
        Self { state, key }
    }
}

impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.state
            .running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.key);
    }
}

// ---------------------------------------------------------------------------------------------
// Probes

/// Answers `/healthz` and `/readyz` (and returns `true`); anything else is left alone.
pub(super) fn probe(stream: &mut std::net::TcpStream, state: &State, head: &Head) -> bool {
    let ready = match head.path.as_str() {
        "/healthz" => None,
        "/readyz" => Some(()),
        _ => return false,
    };
    if head.method != "GET" && head.method != "HEAD" {
        bicraft_http::respond_status(stream, Status::MethodNotAllowed, &[("Allow", "GET, HEAD")]);
        return true;
    }
    let verdict: Result<(), &str> = match ready {
        None => {
            if state.stop.load(Ordering::SeqCst) {
                Err("stopping")
            } else {
                Ok(())
            }
        }
        Some(()) => readiness(state),
    };
    let (status, body) = match verdict {
        Ok(()) => (Status::Ok, "ok\n".to_string()),
        Err(why) => (Status::Unavailable, format!("{why}\n")),
    };
    let headers = [("Cache-Control", "no-store")];
    if head.method == "HEAD" {
        let len = u64::try_from(body.len()).unwrap_or(0);
        let _ = bicraft_http::write_head(stream, status, "text/plain; charset=utf-8", len, &headers);
    } else {
        let _ = bicraft_http::write_response(
            stream,
            status,
            "text/plain; charset=utf-8",
            &headers,
            body.as_bytes(),
        );
    }
    true
}

/// `Ok` when the server takes new work.
fn readiness(state: &State) -> Result<(), &'static str> {
    if state.draining.load(Ordering::SeqCst) || state.stop.load(Ordering::SeqCst) {
        return Err("draining");
    }
    if let Some(percent) = state.config.ready_memory_percent {
        let stats = bicraft_mem::stats();
        if let Some(limit) = stats.limit {
            let allowed = u128::from(u64::try_from(limit).unwrap_or(u64::MAX)) * u128::from(percent) / 100;
            if u128::from(u64::try_from(stats.used).unwrap_or(u64::MAX)) > allowed {
                return Err("memory");
            }
        }
    }
    Ok(())
}

/// The 503 a request gets while the server drains: JSON-RPC for the MCP endpoint, plain JSON
/// for the file routes, text otherwise.
pub(super) fn refuse_draining(stream: &mut std::net::TcpStream, head: &Head) {
    let extra = [("Retry-After", DRAIN_RETRY_AFTER)];
    let message = "the server is shutting down; retry later";
    if route(&head.path).is_some() || view_route(&head.path).is_some() {
        write_json(
            stream,
            Status::Unavailable,
            &rpc_error(None, SHUTTING_DOWN, message, None),
            &extra,
        );
    } else if super::files::route(&head.path).is_some() {
        write_json(
            stream,
            Status::Unavailable,
            &json!({"error": {"code": SHUTTING_DOWN, "message": message}}),
            &extra,
        );
    } else {
        bicraft_http::respond_status(stream, Status::Unavailable, &extra);
    }
}

// ---------------------------------------------------------------------------------------------
// The file a session had open

/// What a new session needs to reopen the file an earlier one had open: the paths relative to
/// the workspace folder (tool paths in server mode are relative) and the display settings.
#[derive(Debug, Clone)]
pub(super) struct Reopen {
    path: String,
    data_root: Option<String>,
    cache_abf: Option<String>,
    ui_language: UiLanguage,
    display_culture: Option<String>,
    as_of: Option<f64>,
}

/// `path` relative to `root`, `/`-separated; `None` outside it or not UTF-8.
pub(super) fn relative(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let parts: Option<Vec<&str>> = rel.components().map(|c| c.as_os_str().to_str()).collect();
    let joined = parts?.join("/");
    (!joined.is_empty()).then_some(joined)
}

impl Reopen {
    /// The file `session` has open; `None` when none (or a `model.new` model never saved).
    pub(super) fn of(session: &Session) -> Option<Self> {
        let root = session.server_config()?.root();
        let path = match session.source.as_ref()? {
            Source::Package { path, .. } | Source::Project { path, .. } | Source::Bim { path } => path,
        };
        Some(Self {
            path: relative(root, path)?,
            data_root: session.data_root.as_deref().and_then(|p| relative(root, p)),
            cache_abf: session.cache_abf.as_deref().and_then(|p| relative(root, p)),
            ui_language: session.ui_language.clone(),
            display_culture: session.display_culture.clone(),
            as_of: session.as_of,
        })
    }

    /// The identity of the file a session would read: path, data root and stored-data file (the
    /// display settings do not change the model). Two sessions with equal keys show the same
    /// saved file.
    pub(super) fn key(&self) -> (String, Option<String>, Option<String>) {
        (self.path.clone(), self.data_root.clone(), self.cache_abf.clone())
    }

    /// The workspace-relative path of the file.
    pub(super) fn path(&self) -> &str {
        &self.path
    }

    /// The file a reader's `file.open` names (`docs/auth.md`, "Roles": a reader's open is
    /// private), each path resolved by the workspace sandbox exactly as `file.open` would, so the
    /// key matches [`Reopen::of`] of a session that opened it. Unknown parameters and paths the
    /// sandbox refuses are errors (nothing is read yet).
    pub(super) fn from_open_params(
        cfg: &bicraft_engine::ServerConfig,
        params: &Value,
    ) -> Result<Self, String> {
        let o = params.as_object().ok_or("`params` must be an object")?;
        if let Some(k) = o
            .keys()
            .find(|k| !matches!(k.as_str(), "path" | "data_root" | "cache_abf"))
        {
            return Err(format!("file.open: unknown parameter `{k}`"));
        }
        let rel = |name: &str| -> Result<Option<String>, String> {
            match o.get(name) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(raw)) => {
                    let abs = cfg.resolve(raw).map_err(|e| format!("`{name}`: {e}"))?;
                    relative(cfg.root(), &abs)
                        .map(Some)
                        .ok_or_else(|| format!("`{name}` must name a file inside the workspace"))
                }
                Some(_) => Err(format!("`{name}` must be a string")),
            }
        };
        let path = rel("path")?.ok_or("file.open needs `path`")?;
        Ok(Self {
            path,
            data_root: rel("data_root")?,
            cache_abf: rel("cache_abf")?,
            ui_language: UiLanguage::default(),
            display_culture: None,
            as_of: None,
        })
    }

    /// [`Reopen::apply`] with `file.open`'s answer or error.
    pub(super) fn apply_result(self, session: &mut Session) -> Result<Value, String> {
        let mut params = json!({"path": self.path});
        if let Some(d) = &self.data_root {
            params["data_root"] = Value::String(d.clone());
        }
        if let Some(c) = &self.cache_abf {
            params["cache_abf"] = Value::String(c.clone());
        }
        let v = session.execute("file.open", &params)?;
        session.ui_language = self.ui_language;
        session.display_culture = self.display_culture;
        session.as_of = self.as_of;
        Ok(v)
    }

    /// Opens the file in `session` again (`file.open`) and restores the display settings. A
    /// failure (the file was deleted) leaves the session empty, as a new one: the tool call
    /// that triggered the reopen runs anyway and reports what it finds.
    pub(super) fn apply(self, session: &mut Session) -> bool {
        self.apply_result(session).is_ok()
    }

    /// Whether reopening would read `target` (or a file below it).
    pub(super) fn touches(&self, root: &Path, target: &Path) -> bool {
        let path = root.join(&self.path);
        path.starts_with(target) || target.starts_with(&path)
    }
}

/// The files and folders `session` has open: `(path, is a tree)`.
pub(super) fn open_paths(session: &Session) -> Vec<(PathBuf, bool)> {
    match &session.source {
        None => Vec::new(),
        Some(Source::Package { path, .. } | Source::Bim { path }) => vec![(path.clone(), false)],
        Some(Source::Project { path, project }) => {
            let mut v = vec![(path.clone(), false)];
            v.extend(project.semantic_model.iter().map(|p| (p.clone(), true)));
            v.extend(project.report.iter().map(|p| (p.clone(), true)));
            v
        }
    }
}

/// Whether writing or removing `target` touches one of `open` (the same file, a file inside an
/// open project folder, or a folder holding an open file).
pub(super) fn touches_open(open: &[(PathBuf, bool)], target: &Path) -> bool {
    open.iter()
        .any(|(p, tree)| p == target || p.starts_with(target) || (*tree && target.starts_with(p)))
}

// ---------------------------------------------------------------------------------------------
// Idle close

/// The idle reaper's loop: checks every quarter of `timeout` (between 10 ms and 30 s) until the
/// server stops.
pub(super) fn idle_loop(state: &State, timeout: Duration) {
    let period = (timeout / 4).clamp(Duration::from_millis(10), Duration::from_secs(30));
    let mut last = Instant::now();
    while !state.stop.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(10).min(period));
        if last.elapsed() >= period {
            reap_idle(state, timeout);
            let now = state.now_ms();
            let views: Vec<_> = state
                .views
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .map(|(id, v)| (id.clone(), Arc::clone(v)))
                .collect();
            for (id, v) in views {
                v.drop_idle_copies(now);
                super::viewer::purge_doc_views(state, &id, &v);
            }
            last = Instant::now();
        }
    }
}

/// Closes the sessions unused for `timeout` and not in use (see the module docs).
pub(super) fn reap_idle(state: &State, timeout: Duration) {
    let now = state.now_ms();
    let timeout_ms = u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX);
    let mut dropped: Vec<Arc<Workspace>> = Vec::new();
    {
        let mut map = state.workspaces.lock().unwrap_or_else(|e| e.into_inner());
        let idle: Vec<String> = map
            .iter()
            .filter(|(_, w)| {
                w.pending.load(Ordering::SeqCst) == 0
                    && now.saturating_sub(w.last_used.load(Ordering::SeqCst)) >= timeout_ms
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in idle {
            let Some(w) = map.get(&id) else { continue };
            let Some(session) = w.session.try_write() else {
                continue;
            };
            if session.dirty || session.report_dirty {
                if !w.warned.swap(true, Ordering::SeqCst) {
                    state.log(
                        "warn",
                        "idle",
                        &format!(
                            "workspace {id} is idle but has unsaved changes: kept open \
                             (save or reopen to discard; it is closed without saving at shutdown)"
                        ),
                    );
                }
                continue;
            }
            let pending_reopen = w.reopen.lock().unwrap_or_else(|e| e.into_inner()).take();
            let reopen = Reopen::of(&session).or(pending_reopen);
            state.note_revision(&id, session.revision);
            drop(session);
            if let Some(w) = map.remove(&id) {
                state.metrics.idle_closes.fetch_add(1, Ordering::Relaxed);
                if let Some(r) = reopen {
                    state
                        .closed
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(id, r);
                }
                dropped.push(w);
            }
        }
    }
    // the sessions (decoded columns, spill files) go outside the map's lock
    drop(dropped);
}

// ---------------------------------------------------------------------------------------------
// Temp folders

/// Removes every workspace's [`TEMP_DIR`] under `root` (at start and at shutdown); returns how
/// many were removed. A link with that name is removed itself, never followed.
pub(super) fn remove_temp_dirs(root: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(root) else {
        return 0;
    };
    let mut removed = 0usize;
    for entry in entries.flatten().take(100_000) {
        let temp = entry.path().join(TEMP_DIR);
        let Ok(meta) = std::fs::symlink_metadata(&temp) else {
            continue;
        };
        let ok = if meta.is_dir() {
            std::fs::remove_dir_all(&temp).is_ok()
        } else {
            std::fs::remove_file(&temp).is_ok()
        };
        if ok {
            removed = removed.saturating_add(1);
        }
    }
    removed
}

// ---------------------------------------------------------------------------------------------
// Shutdown

/// A one-shot shutdown request: raised from a signal handler (`bicraft serve` uses the `ctrlc`
/// crate) or from a test, awaited by the thread that then calls [`HttpServer::drain`].
#[derive(Debug, Default)]
pub struct ShutdownSignal {
    raised: Mutex<bool>,
    cv: Condvar,
}

impl ShutdownSignal {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Raises the signal (idempotent; never blocks for long: safe from a signal-handling thread).
    pub fn raise(&self) {
        *self.raised.lock().unwrap_or_else(|e| e.into_inner()) = true;
        self.cv.notify_all();
    }

    pub fn is_raised(&self) -> bool {
        *self.raised.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Blocks until the signal is raised.
    pub fn wait(&self) {
        let mut raised = self.raised.lock().unwrap_or_else(|e| e.into_inner());
        while !*raised {
            raised = self.cv.wait(raised).unwrap_or_else(|e| e.into_inner());
        }
    }
}

/// What [`HttpServer::drain`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DrainReport {
    /// Every request ended within the grace period.
    pub finished_in_grace: bool,
    /// Tool calls cancelled when the grace period ended.
    pub cancelled: usize,
    /// Requests still running when the server stopped waiting (they are abandoned).
    pub abandoned: usize,
    /// Workspaces closed with unsaved changes, which were **not** saved (sorted).
    pub unsaved: Vec<String>,
    /// Workspace sessions closed.
    pub closed: usize,
    /// Upload temp folders removed.
    pub temp_removed: usize,
}

impl DrainReport {
    /// Nothing was lost: every request finished in time and no session held unsaved changes.
    pub fn clean(&self) -> bool {
        self.finished_in_grace && self.cancelled == 0 && self.abandoned == 0 && self.unsaved.is_empty()
    }
}

/// Waits until no request is active, at most `wait`; `true` when none is.
fn wait_idle(state: &State, wait: Duration) -> bool {
    let deadline = Instant::now().checked_add(wait).unwrap_or_else(Instant::now);
    loop {
        if state.active.load(Ordering::SeqCst) == 0 {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

impl HttpServer {
    /// Shuts the server down gracefully (module docs): from now on `/readyz` answers 503 and new
    /// requests 503 (`Retry-After`); running requests get `grace` to finish; then running tool
    /// calls are cancelled and transfers stop; the listener closes; every session is dropped
    /// without saving; the upload temp folders are removed. Never saves anything.
    pub fn drain(mut self, grace: Duration) -> DrainReport {
        let state = Arc::clone(&self.state);
        state.draining.store(true, Ordering::SeqCst);
        // the control channel's page streams end now (they would hold the drain)
        super::viewer::wake_controls(&state);
        let mut report = DrainReport {
            finished_in_grace: wait_idle(&state, grace),
            ..DrainReport::default()
        };
        if !report.finished_in_grace {
            state.abort.store(true, Ordering::SeqCst);
            let flags: Vec<Arc<AtomicBool>> = state
                .running
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .values()
                .cloned()
                .collect();
            for f in &flags {
                f.store(true, Ordering::SeqCst);
            }
            report.cancelled = flags.len();
            if !wait_idle(&state, CANCEL_WAIT) {
                report.abandoned = state.active.load(Ordering::SeqCst);
            }
        }
        // the listener closes: from here connections are refused
        state.stop.store(true, Ordering::SeqCst);
        super::viewer::wake_controls(&state);
        if let Some(accept) = self.accept.take() {
            let _ = accept.join();
        }
        if let Some(reaper) = self.reaper.take() {
            let _ = reaper.join();
        }
        let workspaces: Vec<(String, Arc<Workspace>)> = state
            .workspaces
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain()
            .collect();
        state.closed.lock().unwrap_or_else(|e| e.into_inner()).clear();
        for (id, w) in workspaces {
            match w.session.write_within(Some(CLOSE_WAIT)) {
                Some(session) => {
                    if session.dirty || session.report_dirty {
                        report.unsaved.push(id);
                    }
                }
                // still running a command that ignored its cancel flag: cannot be inspected
                None => report.unsaved.push(id),
            }
            report.closed = report.closed.saturating_add(1);
            // dropping the last handle drops the session (and deletes its spill files); a
            // session an abandoned request still holds goes when the process exits
            drop(w);
        }
        // readers' copies (clean by construction) and the browser views go too
        let views: Vec<_> = state
            .views
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain()
            .collect();
        for (id, v) in &views {
            v.wake();
            // document views with unsaved edits are closed without saving too
            let unsaved = v
                .doc_sessions()
                .iter()
                .any(|w| match w.session.write_within(Some(CLOSE_WAIT)) {
                    Some(s) => s.dirty || s.report_dirty,
                    None => true,
                });
            if unsaved && !report.unsaved.contains(id) {
                report.unsaved.push(id.clone());
            }
        }
        drop(views);
        report.unsaved.sort();
        report.temp_removed = remove_temp_dirs(&state.root);
        report
    }
}
