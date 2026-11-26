//! Prometheus metrics of `bicraft serve` (docs/kubernetes.md, "Observability").
//!
//! Counters and histograms are plain atomics updated on the request path (no lock, no
//! allocation); everything else (open workspaces, memory, spill files, aggregation builds,
//! drain state) is read when `/metrics` is scraped. `/metrics` is served on its own listener
//! (`--metrics-listen`, off by default), never on the MCP listener, and needs no authentication:
//! nothing in it is model data. Label values come from fixed sets (routes, status codes, the
//! engine's command ids, outcomes); workspace ids are a label only when
//! [`HttpConfig::metrics_workspace_label`](super::HttpConfig::metrics_workspace_label) asks for it,
//! and then only for workspaces that exist and were opened by an authorized tool call (at most
//! [`MAX_WORKSPACE_SERIES`]).

use std::collections::HashMap;
use std::fmt::Write as _;
use std::net::{IpAddr, TcpStream};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bicraft_http::{Limits, ReadError, Status};

use super::State;
use crate::tools;

/// Latency buckets, in microseconds (1 ms to 10 min: a DAX query may run that long).
const BUCKETS_US: [u64; 17] = [
    1_000,
    5_000,
    10_000,
    25_000,
    50_000,
    100_000,
    250_000,
    500_000,
    1_000_000,
    2_500_000,
    5_000_000,
    10_000_000,
    30_000_000,
    60_000_000,
    120_000_000,
    300_000_000,
    600_000_000,
];

/// The status codes counted by their own label; anything else is `code="other"`.
const CODES: [u16; 26] = [
    200, 201, 202, 204, 303, 304, 308, 400, 401, 403, 404, 405, 406, 408, 409, 411, 412, 413, 414, 415, 429,
    431, 500, 503, 505, 507,
];
/// Slots per route: the codes, `other`, and "no response" (status 0, written as `none`).
const CODE_SLOTS: usize = CODES.len() + 2;

/// Workspaces with their own series at most (the opt-in label).
pub(super) const MAX_WORKSPACE_SERIES: usize = 1024;

/// What a request asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Route {
    Mcp,
    Files,
    View,
    Healthz,
    Readyz,
    Metadata,
    Other,
}

impl Route {
    const ALL: [Route; 7] = [
        Route::Mcp,
        Route::Files,
        Route::View,
        Route::Healthz,
        Route::Readyz,
        Route::Metadata,
        Route::Other,
    ];

    pub(super) fn name(self) -> &'static str {
        match self {
            Route::Mcp => "mcp",
            Route::Files => "files",
            Route::View => "view",
            Route::Healthz => "healthz",
            Route::Readyz => "readyz",
            Route::Metadata => "resource_metadata",
            Route::Other => "other",
        }
    }

    fn index(self) -> usize {
        match self {
            Route::Mcp => 0,
            Route::Files => 1,
            Route::Healthz => 2,
            Route::Readyz => 3,
            Route::Metadata => 4,
            Route::Other => 5,
            Route::View => 6,
        }
    }
}

/// How a tool call ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Outcome {
    Ok,
    Error,
    Cancelled,
    /// The caller's role refused it (a reader calling an editing command): nothing ran.
    Refused,
}

impl Outcome {
    const ALL: [Outcome; 4] = [Outcome::Ok, Outcome::Error, Outcome::Cancelled, Outcome::Refused];

    pub(super) fn name(self) -> &'static str {
        match self {
            Outcome::Ok => "ok",
            Outcome::Error => "error",
            Outcome::Cancelled => "cancelled",
            Outcome::Refused => "refused",
        }
    }

    fn index(self) -> usize {
        match self {
            Outcome::Ok => 0,
            Outcome::Error => 1,
            Outcome::Cancelled => 2,
            Outcome::Refused => 3,
        }
    }
}

fn micros(d: Duration) -> u64 {
    u64::try_from(d.as_micros()).unwrap_or(u64::MAX)
}

/// A latency histogram (cumulative buckets are computed when rendered).
#[derive(Default)]
pub(super) struct Histogram {
    /// Per bucket, observations at most its bound and above the previous one.
    buckets: [AtomicU64; BUCKETS_US.len()],
    count: AtomicU64,
    sum_us: AtomicU64,
}

impl Histogram {
    pub(super) fn observe(&self, d: Duration) {
        let us = micros(d);
        if let Some(b) = BUCKETS_US
            .iter()
            .position(|bound| us <= *bound)
            .and_then(|i| self.buckets.get(i))
        {
            b.fetch_add(1, Ordering::Relaxed);
        }
        self.count.fetch_add(1, Ordering::Relaxed);
        self.sum_us.fetch_add(us, Ordering::Relaxed);
    }

    fn count(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }

    /// The series of this histogram (`labels` without braces, empty for none).
    fn render(&self, out: &mut String, name: &str, labels: &str) {
        let sep = if labels.is_empty() { "" } else { "," };
        let mut cumulative = 0u64;
        for (bound, b) in BUCKETS_US.iter().zip(&self.buckets) {
            cumulative = cumulative.saturating_add(b.load(Ordering::Relaxed));
            let le = seconds(*bound);
            let _ = writeln!(out, "{name}_bucket{{{labels}{sep}le=\"{le}\"}} {cumulative}");
        }
        let count = self.count();
        let _ = writeln!(out, "{name}_bucket{{{labels}{sep}le=\"+Inf\"}} {count}");
        let sum = seconds(self.sum_us.load(Ordering::Relaxed));
        if labels.is_empty() {
            let _ = writeln!(out, "{name}_sum {sum}");
            let _ = writeln!(out, "{name}_count {count}");
        } else {
            let _ = writeln!(out, "{name}_sum{{{labels}}} {sum}");
            let _ = writeln!(out, "{name}_count{{{labels}}} {count}");
        }
    }
}

/// Microseconds as seconds, without trailing zeros (`0.001`, `2.5`, `600`).
fn seconds(us: u64) -> String {
    let whole = us / 1_000_000;
    let frac = us % 1_000_000;
    if frac == 0 {
        whole.to_string()
    } else {
        let f = format!("{frac:06}");
        format!("{whole}.{}", f.trim_end_matches('0'))
    }
}

/// One command id's latency and outcomes.
#[derive(Default)]
struct CommandStats {
    latency: Histogram,
    outcomes: [AtomicU64; 4],
}

/// One workspace's series (the opt-in label).
#[derive(Default)]
struct WorkspaceStats {
    calls: u64,
    seconds_us: u64,
}

/// The counters of one server.
pub(super) struct Metrics {
    started: SystemTime,
    /// Per route, per code slot.
    requests: Vec<AtomicU64>,
    request_seconds: [Histogram; 7],
    bytes_in: [AtomicU64; 7],
    bytes_out: [AtomicU64; 7],
    /// Command labels ([`tools::command_labels`]) and their stats, by index.
    labels: Vec<&'static str>,
    index: HashMap<&'static str, usize>,
    commands: Vec<CommandStats>,
    unknown: usize,
    /// Time from a tool call's queue place to its session lock.
    pub(super) queue_wait: Histogram,
    /// Tool calls waiting at their workspace's session gate (counted by `rwgate.rs` while they
    /// wait there), and running.
    pub(super) waiting: AtomicUsize,
    pub(super) running: AtomicUsize,
    /// Read-only calls (and viewer reads) running side by side on a shared session.
    pub(super) reads: AtomicUsize,
    /// Time a call that may change the session waited for its exclusive lock (`rwgate.rs`).
    pub(super) write_wait: Histogram,
    /// Calls refused because their `expected_revision` did not match.
    pub(super) conflicts: AtomicU64,
    /// Tool calls refused because the workspace's queue or the workspace table was full.
    pub(super) busy: AtomicU64,
    /// Sessions closed for idleness, and files reopened lazily after (failed reopens apart).
    pub(super) idle_closes: AtomicU64,
    pub(super) reopens: AtomicU64,
    pub(super) reopen_failures: AtomicU64,
    /// Read-only copies of saved files loaded for readers, loads that failed, and reads refused
    /// because every copy was in use (readview.rs).
    pub(super) copy_loads: AtomicU64,
    pub(super) copy_load_failures: AtomicU64,
    pub(super) copy_busy: AtomicU64,
    /// Viewer pages served, and interactions run on private views (viewer.rs).
    pub(super) viewer_pages: AtomicU64,
    pub(super) viewer_interactions: AtomicU64,
    /// Agents' requests to the UI control channel's relay (`--viewer-control`), and those that
    /// failed (refused, timed out, or answered with an error by the page).
    pub(super) control_requests: AtomicU64,
    pub(super) control_errors: AtomicU64,
    /// Per workspace (opt-in): `None` when the label is off.
    workspaces: Option<Mutex<HashMap<String, WorkspaceStats>>>,
}

impl Metrics {
    pub(super) fn new(workspace_label: bool) -> Self {
        let labels = tools::command_labels();
        let index: HashMap<&'static str, usize> = labels.iter().enumerate().map(|(i, l)| (*l, i)).collect();
        let unknown = index
            .get(tools::UNKNOWN_COMMAND)
            .copied()
            .unwrap_or(labels.len().saturating_sub(1));
        Self {
            started: SystemTime::now(),
            requests: (0..Route::ALL.len() * CODE_SLOTS)
                .map(|_| AtomicU64::new(0))
                .collect(),
            request_seconds: Default::default(),
            bytes_in: Default::default(),
            bytes_out: Default::default(),
            commands: labels.iter().map(|_| CommandStats::default()).collect(),
            labels,
            index,
            unknown,
            queue_wait: Histogram::default(),
            waiting: AtomicUsize::new(0),
            running: AtomicUsize::new(0),
            reads: AtomicUsize::new(0),
            write_wait: Histogram::default(),
            conflicts: AtomicU64::new(0),
            busy: AtomicU64::new(0),
            idle_closes: AtomicU64::new(0),
            reopens: AtomicU64::new(0),
            reopen_failures: AtomicU64::new(0),
            copy_loads: AtomicU64::new(0),
            copy_load_failures: AtomicU64::new(0),
            copy_busy: AtomicU64::new(0),
            viewer_pages: AtomicU64::new(0),
            viewer_interactions: AtomicU64::new(0),
            control_requests: AtomicU64::new(0),
            control_errors: AtomicU64::new(0),
            workspaces: workspace_label.then(|| Mutex::new(HashMap::new())),
        }
    }

    /// The index of a command label (`unknown` for any other text).
    pub(super) fn command_index(&self, label: &str) -> usize {
        self.index.get(label).copied().unwrap_or(self.unknown)
    }

    /// A finished request.
    pub(super) fn request(&self, route: Route, status: u16, took: Duration, bytes_in: u64, bytes_out: u64) {
        let r = route.index();
        let slot = if status == 0 {
            CODES.len() + 1
        } else {
            CODES.iter().position(|c| *c == status).unwrap_or(CODES.len())
        };
        if let Some(c) = self.requests.get(r * CODE_SLOTS + slot) {
            c.fetch_add(1, Ordering::Relaxed);
        }
        if let Some(h) = self.request_seconds.get(r) {
            h.observe(took);
        }
        if let Some(b) = self.bytes_in.get(r) {
            b.fetch_add(bytes_in, Ordering::Relaxed);
        }
        if let Some(b) = self.bytes_out.get(r) {
            b.fetch_add(bytes_out, Ordering::Relaxed);
        }
    }

    /// A command that ran (or was cancelled while it ran) for `took`.
    pub(super) fn command(&self, index: usize, took: Duration, outcome: Outcome) {
        if let Some(c) = self.commands.get(index) {
            c.latency.observe(took);
            if let Some(o) = c.outcomes.get(outcome.index()) {
                o.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// A tool call of workspace `id` (only with the opt-in label; takes a lock then).
    pub(super) fn workspace_call(&self, id: &str, took: Duration) {
        let Some(m) = &self.workspaces else { return };
        let mut m = m.lock().unwrap_or_else(|e| e.into_inner());
        if !m.contains_key(id) && m.len() >= MAX_WORKSPACE_SERIES {
            return;
        }
        let w = m.entry(id.to_string()).or_default();
        w.calls = w.calls.saturating_add(1);
        w.seconds_us = w.seconds_us.saturating_add(micros(took));
    }
}

/// `"` `\` and line feeds escaped (label values are from fixed sets or workspace ids, which have
/// none; escaped anyway).
fn escape(v: &str) -> String {
    let mut s = String::with_capacity(v.len());
    for c in v.chars() {
        match c {
            '\\' => s.push_str("\\\\"),
            '"' => s.push_str("\\\""),
            '\n' => s.push_str("\\n"),
            c => s.push(c),
        }
    }
    s
}

fn header(out: &mut String, name: &str, kind: &str, help: &str) {
    let _ = writeln!(out, "# HELP {name} {help}");
    let _ = writeln!(out, "# TYPE {name} {kind}");
}

fn gauge(out: &mut String, name: &str, help: &str, value: impl std::fmt::Display) {
    header(out, name, "gauge", help);
    let _ = writeln!(out, "{name} {value}");
}

fn counter(out: &mut String, name: &str, help: &str, value: impl std::fmt::Display) {
    header(out, name, "counter", help);
    let _ = writeln!(out, "{name} {value}");
}

/// Most entries read below `--recovery-dir` per scrape (one folder per workspace, a few files per
/// open file with unsaved edits).
const MAX_RECOVERY_ENTRIES: usize = 100_000;

/// Per workspace folder of `dir` (sorted), its journals (`*.jsonl`) and the bytes of every file
/// in it (journals and recovery copies). Bounded; links are not followed.
fn recovery_journals(dir: Option<&std::path::Path>) -> Vec<(String, usize, u64)> {
    let Some(dir) = dir else { return Vec::new() };
    let Ok(folders) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut seen = 0usize;
    let mut out = Vec::new();
    for f in folders.flatten() {
        if !f.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let Some(id) = f.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let Ok(files) = std::fs::read_dir(f.path()) else {
            continue;
        };
        let (mut n, mut bytes) = (0usize, 0u64);
        for e in files.flatten() {
            seen = seen.saturating_add(1);
            if seen > MAX_RECOVERY_ENTRIES {
                break;
            }
            if !e.file_type().is_ok_and(|t| t.is_file()) {
                continue;
            }
            // the file's own metadata: a directory entry's size lags behind a file still open
            // for writing (Windows)
            let Ok(meta) = std::fs::metadata(e.path()) else {
                continue;
            };
            bytes = bytes.saturating_add(meta.len());
            if e.path().extension().is_some_and(|x| x == "jsonl") {
                n = n.saturating_add(1);
            }
        }
        out.push((id, n, bytes));
        if seen > MAX_RECOVERY_ENTRIES {
            break;
        }
    }
    out.sort();
    out
}

/// The text exposition (format 0.0.4) of the server's metrics.
pub(super) fn render(state: &State) -> String {
    let m = &state.metrics;
    let mut out = String::with_capacity(16 * 1024);
    header(
        &mut out,
        "bicraft_build_info",
        "gauge",
        "The running version (always 1).",
    );
    let _ = writeln!(
        out,
        "bicraft_build_info{{version=\"{}\"}} 1",
        escape(env!("CARGO_PKG_VERSION"))
    );
    let start = m.started.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    gauge(
        &mut out,
        "bicraft_start_time_seconds",
        "When the server started (Unix time).",
        start,
    );
    let draining = state.draining.load(Ordering::SeqCst) || state.stop.load(Ordering::SeqCst);
    gauge(
        &mut out,
        "bicraft_draining",
        "1 while the server drains (shutdown: new requests get 503).",
        u8::from(draining),
    );

    // HTTP
    header(
        &mut out,
        "bicraft_http_requests_total",
        "counter",
        "Requests answered, by route and status code (code none: the connection closed first).",
    );
    for route in Route::ALL {
        for slot in 0..CODE_SLOTS {
            let n = m
                .requests
                .get(route.index() * CODE_SLOTS + slot)
                .map_or(0, |c| c.load(Ordering::Relaxed));
            if n == 0 {
                continue;
            }
            let code = match CODES.get(slot) {
                Some(c) => c.to_string(),
                None if slot == CODES.len() => "other".into(),
                None => "none".into(),
            };
            let _ = writeln!(
                out,
                "bicraft_http_requests_total{{route=\"{}\",code=\"{code}\"}} {n}",
                route.name()
            );
        }
    }
    header(
        &mut out,
        "bicraft_http_request_duration_seconds",
        "histogram",
        "Request durations by route, from the request head to the last byte of the answer.",
    );
    for route in Route::ALL {
        if let Some(h) = m.request_seconds.get(route.index())
            && h.count() > 0
        {
            h.render(
                &mut out,
                "bicraft_http_request_duration_seconds",
                &format!("route=\"{}\"", route.name()),
            );
        }
    }
    for (name, help, values) in [
        (
            "bicraft_http_request_bytes_total",
            "Request body bytes (as declared by Content-Length), by route.",
            &m.bytes_in,
        ),
        (
            "bicraft_http_response_bytes_total",
            "Response bytes written (downloads as declared), by route.",
            &m.bytes_out,
        ),
    ] {
        header(&mut out, name, "counter", help);
        for route in Route::ALL {
            let n = values.get(route.index()).map_or(0, |c| c.load(Ordering::Relaxed));
            if n > 0 {
                let _ = writeln!(out, "{name}{{route=\"{}\"}} {n}", route.name());
            }
        }
    }
    gauge(
        &mut out,
        "bicraft_http_requests_in_flight",
        "Requests in progress (probes excepted).",
        state.active.load(Ordering::SeqCst),
    );
    gauge(
        &mut out,
        "bicraft_http_connections_open",
        "Open connections of the MCP listener.",
        state.conns.open(),
    );

    // Tool calls and commands
    header(
        &mut out,
        "bicraft_command_duration_seconds",
        "histogram",
        "Tool call durations by engine command id (file.open, file.save and file.save_as are the \
         open and save durations; queue waits excluded).",
    );
    for (label, c) in m.labels.iter().zip(&m.commands) {
        if c.latency.count() > 0 {
            c.latency.render(
                &mut out,
                "bicraft_command_duration_seconds",
                &format!("command=\"{}\"", escape(label)),
            );
        }
    }
    header(
        &mut out,
        "bicraft_commands_total",
        "counter",
        "Tool calls, by engine command id and outcome (ok, error, cancelled, refused by the caller's role).",
    );
    for (label, c) in m.labels.iter().zip(&m.commands) {
        for o in Outcome::ALL {
            let n = c.outcomes.get(o.index()).map_or(0, |x| x.load(Ordering::Relaxed));
            if n > 0 {
                let _ = writeln!(
                    out,
                    "bicraft_commands_total{{command=\"{}\",outcome=\"{}\"}} {n}",
                    escape(label),
                    o.name()
                );
            }
        }
    }
    header(
        &mut out,
        "bicraft_queue_wait_seconds",
        "histogram",
        "Time a tool call waited for its workspace's session.",
    );
    m.queue_wait.render(&mut out, "bicraft_queue_wait_seconds", "");
    gauge(
        &mut out,
        "bicraft_tool_calls_queued",
        "Tool calls waiting at their workspace's session lock.",
        m.waiting.load(Ordering::Relaxed),
    );
    gauge(
        &mut out,
        "bicraft_tool_calls_running",
        "Tool calls running.",
        m.running.load(Ordering::Relaxed),
    );
    gauge(
        &mut out,
        "bicraft_concurrent_reads",
        "Read-only tool calls and viewer reads running now on shared sessions.",
        m.reads.load(Ordering::Relaxed),
    );
    header(
        &mut out,
        "bicraft_write_lock_wait_seconds",
        "histogram",
        "Time a call that may change a session waited for its exclusive lock (running reads finish first).",
    );
    m.write_wait
        .render(&mut out, "bicraft_write_lock_wait_seconds", "");
    counter(
        &mut out,
        "bicraft_revision_conflicts_total",
        "Calls refused because their expected_revision did not match the session (nothing ran).",
        m.conflicts.load(Ordering::Relaxed),
    );
    counter(
        &mut out,
        "bicraft_queue_rejections_total",
        "Tool calls refused with 503 (workspace queue full, or too many open workspaces).",
        m.busy.load(Ordering::Relaxed),
    );

    // Workspaces
    let open = state.workspaces.lock().unwrap_or_else(|e| e.into_inner()).len();
    gauge(
        &mut out,
        "bicraft_workspaces_open",
        "Workspace sessions open.",
        open,
    );
    gauge(
        &mut out,
        "bicraft_workspaces_max",
        "Workspace sessions allowed at once.",
        state.config.max_workspaces,
    );
    counter(
        &mut out,
        "bicraft_workspace_idle_closes_total",
        "Workspace sessions closed for idleness.",
        m.idle_closes.load(Ordering::Relaxed),
    );
    counter(
        &mut out,
        "bicraft_workspace_reopens_total",
        "Files reopened lazily after an idle close or an upload over the open file.",
        m.reopens.load(Ordering::Relaxed),
    );
    counter(
        &mut out,
        "bicraft_workspace_reopen_failures_total",
        "Lazy reopens that failed (the file was deleted or became unreadable).",
        m.reopen_failures.load(Ordering::Relaxed),
    );
    let (copies, copy_bytes) = super::readview::copy_totals(state);
    gauge(
        &mut out,
        "bicraft_viewer_copies_open",
        "Read-only copies of saved files open for readers.",
        copies,
    );
    gauge(
        &mut out,
        "bicraft_viewer_copy_bytes",
        "Memory reserved by the loads of the open read-only copies (bytes).",
        copy_bytes,
    );
    counter(
        &mut out,
        "bicraft_viewer_copy_loads_total",
        "Read-only copies of saved files loaded for readers.",
        m.copy_loads.load(Ordering::Relaxed),
    );
    counter(
        &mut out,
        "bicraft_viewer_copy_load_failures_total",
        "Read-only copy loads that failed.",
        m.copy_load_failures.load(Ordering::Relaxed),
    );
    counter(
        &mut out,
        "bicraft_viewer_copy_busy_total",
        "Reads refused because every read-only copy was in use.",
        m.copy_busy.load(Ordering::Relaxed),
    );
    let (views, logins) = super::viewer::counts(state);
    gauge(
        &mut out,
        "bicraft_viewer_views_open",
        "Private browser views kept.",
        views,
    );
    gauge(
        &mut out,
        "bicraft_viewer_logins_open",
        "Browser logins of the viewer kept.",
        logins,
    );
    counter(
        &mut out,
        "bicraft_viewer_pages_total",
        "Viewer pages served.",
        m.viewer_pages.load(Ordering::Relaxed),
    );
    counter(
        &mut out,
        "bicraft_viewer_interactions_total",
        "Interactions run on private views.",
        m.viewer_interactions.load(Ordering::Relaxed),
    );
    counter(
        &mut out,
        "bicraft_control_requests_total",
        "Agents' requests to the UI control channel's relay (bicraft serve --viewer-control).",
        m.control_requests.load(Ordering::Relaxed),
    );
    counter(
        &mut out,
        "bicraft_control_request_errors_total",
        "Control channel requests that failed: refused, timed out, or answered with an error by the page.",
        m.control_errors.load(Ordering::Relaxed),
    );

    // Document views (browser tabs on their own file, docs/mcp.md "Document views")
    let docs = super::viewer::doc_view_counts(state);
    let (edit, read) = docs.iter().fold((0usize, 0usize), |(e, r), (_, de, dr)| {
        (e.saturating_add(*de), r.saturating_add(*dr))
    });
    header(
        &mut out,
        "bicraft_document_views_open",
        "gauge",
        "Document views open: editors' sessions of their own (mode=edit) and readers' (mode=read).",
    );
    let _ = writeln!(out, "bicraft_document_views_open{{mode=\"edit\"}} {edit}");
    let _ = writeln!(out, "bicraft_document_views_open{{mode=\"read\"}} {read}");
    gauge(
        &mut out,
        "bicraft_document_views_max",
        "Document views allowed per workspace (--max-views).",
        state.config.max_views_per_workspace.max(1),
    );

    // Recovery journals (docs/recovery.md): read from --recovery-dir when scraped
    let journals = recovery_journals(state.config.recovery_dir.as_deref());
    gauge(
        &mut out,
        "bicraft_recovery_journals",
        "Recovery journals of unsaved edits kept in --recovery-dir (0 without it).",
        journals
            .iter()
            .map(|(_, n, _)| *n)
            .fold(0usize, usize::saturating_add),
    );
    gauge(
        &mut out,
        "bicraft_recovery_journal_bytes",
        "Bytes of the recovery journals and their recovery copies in --recovery-dir.",
        journals
            .iter()
            .map(|(_, _, b)| *b)
            .fold(0u64, u64::saturating_add),
    );

    if let Some(ws) = &m.workspaces {
        let ws = ws.lock().unwrap_or_else(|e| e.into_inner());
        let mut ids: Vec<&String> = ws.keys().collect();
        ids.sort();
        header(
            &mut out,
            "bicraft_workspace_tool_calls_total",
            "counter",
            "Tool calls by workspace (opt-in label).",
        );
        for id in &ids {
            if let Some(w) = ws.get(*id) {
                let _ = writeln!(
                    out,
                    "bicraft_workspace_tool_calls_total{{workspace=\"{}\"}} {}",
                    escape(id),
                    w.calls
                );
            }
        }
        header(
            &mut out,
            "bicraft_workspace_tool_call_seconds_total",
            "counter",
            "Time spent in tool calls by workspace (opt-in label).",
        );
        for id in &ids {
            if let Some(w) = ws.get(*id) {
                let _ = writeln!(
                    out,
                    "bicraft_workspace_tool_call_seconds_total{{workspace=\"{}\"}} {}",
                    escape(id),
                    seconds(w.seconds_us)
                );
            }
        }
        header(
            &mut out,
            "bicraft_workspace_document_views_open",
            "gauge",
            "Document views open by workspace (opt-in label).",
        );
        for (id, e, r) in docs.iter().take(MAX_WORKSPACE_SERIES) {
            let _ = writeln!(
                out,
                "bicraft_workspace_document_views_open{{workspace=\"{}\"}} {}",
                escape(id),
                e.saturating_add(*r)
            );
        }
        header(
            &mut out,
            "bicraft_workspace_recovery_journals",
            "gauge",
            "Recovery journals kept by workspace (opt-in label).",
        );
        for (id, n, _) in journals.iter().take(MAX_WORKSPACE_SERIES) {
            let _ = writeln!(
                out,
                "bicraft_workspace_recovery_journals{{workspace=\"{}\"}} {n}",
                escape(id)
            );
        }
    }

    // Memory (docs/beyond-ram.md, lot 5: counted only under BICRAFT_MEMORY_LIMIT)
    let s = bicraft_mem::stats();
    gauge(
        &mut out,
        "bicraft_memory_limit_bytes",
        "BICRAFT_MEMORY_LIMIT (0: no limit, nothing is counted).",
        s.limit.unwrap_or(0),
    );
    gauge(
        &mut out,
        "bicraft_memory_reserved_bytes",
        "Bytes reserved against the memory limit.",
        s.used,
    );
    gauge(
        &mut out,
        "bicraft_memory_peak_reserved_bytes",
        "Highest reserved bytes since the start.",
        s.peak,
    );
    header(
        &mut out,
        "bicraft_memory_reserved_class_bytes",
        "gauge",
        "Reserved bytes by class (kept, query, background, io).",
    );
    for c in bicraft_mem::Class::ALL {
        let _ = writeln!(
            out,
            "bicraft_memory_reserved_class_bytes{{class=\"{}\"}} {}",
            c.name(),
            s.class(c)
        );
    }
    counter(
        &mut out,
        "bicraft_memory_evictions_total",
        "Evictions done to make room under the memory limit.",
        s.evictions,
    );
    counter(
        &mut out,
        "bicraft_memory_refused_total",
        "Reservations refused under the memory limit (the work failed cleanly).",
        s.refused,
    );
    gauge(
        &mut out,
        "bicraft_memory_running_queries",
        "Queries sharing the memory limit now.",
        s.active_queries,
    );
    if let Some(rss) = bicraft_mem::process_resident_bytes() {
        gauge(
            &mut out,
            "bicraft_process_resident_bytes",
            "The process' resident memory.",
            rss,
        );
    }
    let e = bicraft_engine::engine_gauges();
    gauge(
        &mut out,
        "bicraft_spill_files",
        "Temporary files of backups moved out of memory (memory budget).",
        e.spill_files,
    );
    gauge(
        &mut out,
        "bicraft_spill_bytes",
        "Bytes in those temporary files.",
        e.spill_bytes,
    );
    gauge(
        &mut out,
        "bicraft_aggregation_builds_running",
        "Background aggregation builds running.",
        e.aggregation_builds_running,
    );
    header(
        &mut out,
        "bicraft_aggregation_builds_total",
        "counter",
        "Background aggregation builds ended, by result (built, stopped: cancelled, refused or failed).",
    );
    let _ = writeln!(
        out,
        "bicraft_aggregation_builds_total{{result=\"built\"}} {}",
        e.aggregation_builds_done
    );
    let _ = writeln!(
        out,
        "bicraft_aggregation_builds_total{{result=\"stopped\"}} {}",
        e.aggregation_builds_stopped
    );
    out
}

/// The metrics listener's request head bounds.
const LIMITS: Limits = Limits {
    max_head_bytes: 8 * 1024,
    max_headers: 32,
    max_target_len: 256,
    head_deadline: Duration::from_secs(10),
};

/// One connection of the metrics listener: `GET /metrics` (or `HEAD`), 404 for any other path.
/// No authentication and no `Host` check (Prometheus scrapes the pod address); the listener
/// serves nothing else.
pub(super) fn serve_connection(mut stream: TcpStream, _peer: IpAddr, state: &State) {
    if stream.set_write_timeout(Some(Duration::from_secs(10))).is_err() {
        return;
    }
    let head = match bicraft_http::read_request(&mut stream, &LIMITS, &["host"]) {
        Ok((head, _)) => head,
        Err(ReadError::Close) => return,
        Err(ReadError::Respond(status)) => {
            bicraft_http::respond_status(&mut stream, status, &[]);
            bicraft_http::finish(stream);
            return;
        }
    };
    let path = head.path.split('?').next().unwrap_or_default();
    if path != "/metrics" {
        bicraft_http::respond_status(&mut stream, Status::NotFound, &[]);
    } else if head.method != "GET" && head.method != "HEAD" {
        bicraft_http::respond_status(&mut stream, Status::MethodNotAllowed, &[("Allow", "GET, HEAD")]);
    } else {
        let body = render(state);
        let content_type = "text/plain; version=0.0.4; charset=utf-8";
        let headers = [("Cache-Control", "no-store")];
        if head.method == "HEAD" {
            let len = u64::try_from(body.len()).unwrap_or(0);
            let _ = bicraft_http::write_head(&mut stream, Status::Ok, content_type, len, &headers);
        } else {
            let _ = bicraft_http::write_response(
                &mut stream,
                Status::Ok,
                content_type,
                &headers,
                body.as_bytes(),
            );
        }
    }
    // the scrape's own note is not a request of the MCP listener
    let _ = bicraft_http::take_response_note();
    bicraft_http::finish(stream);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histograms_render_cumulative_buckets() {
        let h = Histogram::default();
        h.observe(Duration::from_micros(500));
        h.observe(Duration::from_millis(7));
        h.observe(Duration::from_secs(3600));
        let mut out = String::new();
        h.render(&mut out, "x_seconds", "command=\"dax.evaluate\"");
        assert!(
            out.contains("x_seconds_bucket{command=\"dax.evaluate\",le=\"0.001\"} 1\n"),
            "{out}"
        );
        assert!(
            out.contains("x_seconds_bucket{command=\"dax.evaluate\",le=\"0.01\"} 2\n"),
            "{out}"
        );
        assert!(
            out.contains("x_seconds_bucket{command=\"dax.evaluate\",le=\"600\"} 2\n"),
            "{out}"
        );
        assert!(
            out.contains("x_seconds_bucket{command=\"dax.evaluate\",le=\"+Inf\"} 3\n"),
            "{out}"
        );
        assert!(
            out.contains("x_seconds_count{command=\"dax.evaluate\"} 3\n"),
            "{out}"
        );
        assert!(
            out.contains("x_seconds_sum{command=\"dax.evaluate\"} 3600.0075\n"),
            "{out}"
        );
        let mut out = String::new();
        Histogram::default().render(&mut out, "q", "");
        assert!(
            out.contains("q_bucket{le=\"+Inf\"} 0\nq_sum 0\nq_count 0\n"),
            "{out}"
        );
    }

    /// The cost of what a tool call adds to the request path (run with `--ignored --nocapture`).
    #[test]
    #[ignore = "micro-benchmark"]
    fn cost_per_request() {
        let m = Metrics::new(false);
        let n = 1_000_000u32;
        let t = std::time::Instant::now();
        for i in 0..n {
            let idx = m.command_index("dax.evaluate");
            let d = Duration::from_micros(u64::from(i % 5000));
            m.command(idx, d, Outcome::Ok);
            m.queue_wait.observe(d);
            m.request(Route::Mcp, 200, d, 300, 900);
        }
        let per = t.elapsed() / n;
        eprintln!("metrics per tool call: {per:?}");
    }

    #[test]
    fn labels_are_bounded() {
        let m = Metrics::new(false);
        // a client's text never becomes a label: unknown commands share one series
        assert_eq!(
            m.command_index("EVALUATE ROW(\"x\", 1)"),
            m.command_index("unknown")
        );
        assert_ne!(m.command_index("dax.evaluate"), m.command_index("unknown"));
        assert_eq!(
            tools::command_label("run_command", &serde_json::json!({"command": "no.such"})),
            "unknown"
        );
        assert_eq!(
            tools::command_label(
                "run_command",
                &serde_json::json!({"command": "dax.evaluate", "params": {"query": "x"}})
            ),
            "dax.evaluate"
        );
        assert_eq!(
            tools::command_label("evaluate_dax_query", &serde_json::json!({})),
            "dax.evaluate"
        );
        // every status lands in a slot, unknown ones in `other`
        m.request(Route::Mcp, 418, Duration::ZERO, 1, 2);
        m.request(Route::Mcp, 0, Duration::ZERO, 0, 0);
        assert_eq!(
            m.requests.get(CODES.len()).map(|c| c.load(Ordering::Relaxed)),
            Some(1)
        );
        assert_eq!(
            m.requests.get(CODES.len() + 1).map(|c| c.load(Ordering::Relaxed)),
            Some(1)
        );
        // the opt-in workspace label is capped
        let m = Metrics::new(true);
        for i in 0..MAX_WORKSPACE_SERIES + 10 {
            m.workspace_call(&format!("w{i}"), Duration::from_millis(1));
        }
        let n = m.workspaces.as_ref().map(|w| w.lock().unwrap().len());
        assert_eq!(n, Some(MAX_WORKSPACE_SERIES));
        assert_eq!(seconds(1), "0.000001");
        assert_eq!(seconds(2_500_000), "2.5");
        assert_eq!(escape("a\"b\\c\nd"), "a\\\"b\\\\c\\nd");
    }
}
