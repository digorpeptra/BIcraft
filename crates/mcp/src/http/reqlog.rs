//! Request logs of `bicraft serve` (docs/kubernetes.md, "Observability"): with
//! [`LogFormat::Json`], one JSON line per request on stderr (or a [`LogSink`]), and the server's
//! own events (start, idle, drain) as JSON lines too. The text format (the default) keeps the
//! human messages and writes no per-request line.
//!
//! **Logs never carry model data** (docs/web-deployment.md): a request line holds the time, the
//! route class, the HTTP method, the workspace id (an operator-chosen folder name, validated), the
//! caller *kind* (`local`, `oidc`, `proxy`, `token`, `unauthenticated`, `none`), the JSON-RPC
//! method and tool name and the engine command id (each from a fixed set, never the client's
//! text), the status, outcome and error kind, the duration and the byte counts. Never a DAX
//! expression, a parameter, a value, a result, a file name, a token or a user id: user ids are
//! **omitted**, not hashed (a hash of an e-mail address is reversible by dictionary, and a salted
//! one cannot be correlated with the identity provider's audit log, which is where per-user audit
//! belongs).
//!
//! The context of the request being served lives in a thread-local: each connection is served by
//! its own thread (`bicraft_http::accept_loop`), so it needs no lock and no plumbing through the
//! handlers.

use std::cell::RefCell;
use std::io::Write as _;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use bicraft_http::Head;
use serde_json::{Map, Value, json};

use super::State;
use super::metrics::Route;

/// How `bicraft serve` writes its log.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LogFormat {
    /// Human messages for the server's events; no per-request line (the default).
    #[default]
    Text,
    /// One JSON object per line: every request (probes excepted) and every server event.
    Json,
}

impl LogFormat {
    /// `text` or `json` (ASCII case-insensitive).
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            other => Err(format!("log format `{other}`: expected text or json")),
        }
    }
}

/// Where log lines go instead of stderr (tests, embedding): called with each line, without its
/// line feed.
#[derive(Clone)]
pub struct LogSink(pub Arc<dyn Fn(&str) + Send + Sync>);

impl std::fmt::Debug for LogSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LogSink")
    }
}

/// Writes one line to `sink`, or to stderr in one write (lines of different threads never mix).
fn emit(sink: Option<&LogSink>, line: &str) {
    match sink {
        Some(s) => (s.0)(line),
        None => {
            let mut e = std::io::stderr().lock();
            let mut buf = Vec::with_capacity(line.len().saturating_add(1));
            buf.extend_from_slice(line.as_bytes());
            buf.push(b'\n');
            let _ = e.write_all(&buf);
        }
    }
}

/// A server event (`event`: `start`, `idle`, `drain`, …) in the given format: a JSON line, or
/// `bicraft serve: <message>` in text. The message is the operator's text: it must carry no
/// model data either.
pub fn log_event(format: LogFormat, sink: Option<&LogSink>, level: &str, event: &str, message: &str) {
    match format {
        LogFormat::Text => emit(sink, &format!("bicraft serve: {message}")),
        LogFormat::Json => {
            let line = json!({
                "ts": timestamp(SystemTime::now()),
                "level": level,
                "event": event,
                "msg": message,
            });
            emit(sink, &line.to_string());
        }
    }
}

/// `2026-10-27T08:15:02.123Z`.
pub(super) fn timestamp(t: SystemTime) -> String {
    let d = t.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = d.as_secs();
    let millis = d.subsec_millis();
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let rem = secs % 86_400;
    let (y, m, day) = bicraft_locale::calendar::civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// The request being served on this thread.
struct Ctx {
    start: Instant,
    route: Route,
    method: &'static str,
    workspace: Option<String>,
    caller: &'static str,
    rpc: Option<&'static str>,
    tool: Option<&'static str>,
    command: Option<&'static str>,
    /// The viewer route kind (`page`, `state`, `events`, `asset`, `interact`, `login`, `other`).
    view: Option<&'static str>,
    outcome: Option<&'static str>,
    error: Option<&'static str>,
    bytes_in: u64,
}

thread_local! {
    static CTX: RefCell<Option<Ctx>> = const { RefCell::new(None) };
}

fn with(f: impl FnOnce(&mut Ctx)) {
    CTX.with(|c| {
        if let Ok(mut c) = c.try_borrow_mut()
            && let Some(ctx) = c.as_mut()
        {
            f(ctx);
        }
    });
}

/// A request starts on this thread (before its head is read).
pub(super) fn begin() {
    let _ = bicraft_http::take_response_note();
    CTX.with(|c| {
        if let Ok(mut c) = c.try_borrow_mut() {
            *c = Some(Ctx {
                start: Instant::now(),
                route: Route::Other,
                method: "-",
                workspace: None,
                caller: "none",
                rpc: None,
                tool: None,
                command: None,
                view: None,
                outcome: None,
                error: None,
                bytes_in: 0,
            });
        }
    });
}

/// The request's head arrived: its route class, method, workspace id and declared body size.
pub(super) fn head(head: &Head) {
    let path = head.path.as_str();
    let (route, workspace) = if path == "/healthz" {
        (Route::Healthz, None)
    } else if path == "/readyz" {
        (Route::Readyz, None)
    } else if path.starts_with(super::RESOURCE_METADATA_PATH) {
        (Route::Metadata, None)
    } else if let Some((id, _)) = super::viewer::route(path) {
        (Route::View, Some(id))
    } else if let Some((id, _)) = super::files::route(path) {
        (Route::Files, Some(id))
    } else if let Some(id) = super::route(path) {
        (Route::Mcp, Some(id))
    } else {
        (Route::Other, None)
    };
    let method = match head.method.as_str() {
        "GET" => "GET",
        "HEAD" => "HEAD",
        "POST" => "POST",
        "PUT" => "PUT",
        "DELETE" => "DELETE",
        "OPTIONS" => "OPTIONS",
        _ => "other",
    };
    let bytes_in = head
        .content_length
        .map_or(0, |n| u64::try_from(n).unwrap_or(u64::MAX));
    with(|c| {
        c.route = route;
        c.method = method;
        // ids are validated (1 to 64 of [A-Za-z0-9._-]): never a path or a client's free text
        c.workspace = workspace.map(str::to_string);
        c.bytes_in = bytes_in;
    });
}

/// The caller's kind (never its identity).
pub(super) fn caller(kind: &'static str) {
    with(|c| c.caller = kind);
}

/// The JSON-RPC method, from the fixed set of the revision (`other` for anything else).
pub(super) fn rpc(method: &str) {
    let m = match method {
        "server/discover" => "server/discover",
        "tools/list" => "tools/list",
        "tools/call" => "tools/call",
        "initialize" => "initialize",
        _ => "other",
    };
    with(|c| c.rpc = Some(m));
}

/// The tool (a static name) and the engine command label of a tool call.
pub(super) fn tool(tool: &'static str, command: &'static str) {
    with(|c| {
        c.tool = Some(tool);
        c.command = Some(command);
    });
}

/// The viewer route kind (a fixed name).
pub(super) fn view(kind: &'static str) {
    with(|c| c.view = Some(kind));
}

/// How the request ended, when the status alone does not say it (a tool call's error result is
/// a 200).
pub(super) fn outcome(outcome: &'static str, error: Option<&'static str>) {
    with(|c| {
        c.outcome = Some(outcome);
        if error.is_some() {
            c.error = error;
        }
    });
}

/// The error kind of a rejected request.
pub(super) fn error(kind: &'static str) {
    with(|c| c.error = Some(kind));
}

/// The error kind of a JSON-RPC error code (BIcraft's and the revision's), from a fixed set.
pub(super) fn error_kind(code: i64) -> &'static str {
    match code {
        crate::PARSE_ERROR => "parse_error",
        crate::INVALID_REQUEST => "invalid_request",
        crate::METHOD_NOT_FOUND => "method_not_found",
        crate::INVALID_PARAMS => "invalid_params",
        -32603 => "internal_error",
        super::HEADER_MISMATCH => "header_mismatch",
        super::UNSUPPORTED_PROTOCOL_VERSION => "unsupported_protocol_version",
        super::WORKSPACE_BUSY => "busy",
        super::WORKSPACE_NOT_FOUND => "not_found",
        super::UNAUTHORIZED => "unauthorized",
        super::SHUTTING_DOWN => "shutting_down",
        _ => "other",
    }
}

/// The request is over: counts it, and logs it (JSON format, probes excepted).
pub(super) fn end(state: &State) {
    let (status, bytes_out) = bicraft_http::take_response_note();
    let Some(ctx) = CTX.with(|c| c.try_borrow_mut().ok().and_then(|mut c| c.take())) else {
        return;
    };
    let took = ctx.start.elapsed();
    state
        .metrics
        .request(ctx.route, status, took, ctx.bytes_in, bytes_out);
    if state.config.log_format != LogFormat::Json || matches!(ctx.route, Route::Healthz | Route::Readyz) {
        return;
    }
    let outcome = ctx.outcome.unwrap_or(match status {
        0 => "closed",
        1..=399 => "ok",
        _ => "rejected",
    });
    let error = ctx.error.or(match status {
        0..=399 => None,
        400..=499 => Some("client_error"),
        _ => Some("server_error"),
    });
    let mut o = Map::new();
    o.insert("ts".into(), json!(timestamp(SystemTime::now())));
    o.insert("level".into(), json!(if status >= 500 { "warn" } else { "info" }));
    o.insert("event".into(), json!("request"));
    o.insert("route".into(), json!(ctx.route.name()));
    o.insert("method".into(), json!(ctx.method));
    if let Some(w) = &ctx.workspace {
        o.insert("workspace".into(), json!(w));
    }
    o.insert("caller".into(), json!(ctx.caller));
    if let Some(r) = ctx.rpc {
        o.insert("rpc".into(), json!(r));
    }
    if let Some(t) = ctx.tool {
        o.insert("tool".into(), json!(t));
    }
    if let Some(c) = ctx.command {
        o.insert("command".into(), json!(c));
    }
    if let Some(v) = ctx.view {
        o.insert("view".into(), json!(v));
    }
    o.insert("status".into(), json!(status));
    o.insert("outcome".into(), json!(outcome));
    if let Some(e) = error {
        o.insert("error".into(), json!(e));
    }
    let ms = (took.as_micros() as f64) / 1000.0;
    o.insert("duration_ms".into(), json!((ms * 1000.0).round() / 1000.0));
    o.insert("bytes_in".into(), json!(ctx.bytes_in));
    o.insert("bytes_out".into(), json!(bytes_out));
    emit(state.config.log_sink.as_ref(), &Value::Object(o).to_string());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn timestamps_are_rfc3339_utc() {
        assert_eq!(timestamp(UNIX_EPOCH), "1970-01-01T00:00:00.000Z");
        let t = UNIX_EPOCH + Duration::from_millis(1_793_088_902_123);
        assert_eq!(timestamp(t), "2026-10-27T08:15:02.123Z");
        // a leap day
        let t = UNIX_EPOCH + Duration::from_secs(951_782_400);
        assert_eq!(timestamp(t), "2000-02-29T00:00:00.000Z");
        // year boundaries, a leap day's last millisecond, a century that is not leap (G37)
        let at = |ms: u64| timestamp(UNIX_EPOCH + Duration::from_millis(ms));
        assert_eq!(at(946_684_799_999), "1999-12-31T23:59:59.999Z");
        assert_eq!(at(946_684_800_000), "2000-01-01T00:00:00.000Z");
        assert_eq!(at(1_709_251_199_999), "2024-02-29T23:59:59.999Z");
        assert_eq!(at(4_107_499_200_007), "2100-02-28T12:00:00.007Z");
        assert_eq!(at(4_107_542_400_000), "2100-03-01T00:00:00.000Z");
        assert_eq!(LogFormat::parse("JSON"), Ok(LogFormat::Json));
        assert_eq!(LogFormat::parse("text"), Ok(LogFormat::Text));
        assert!(LogFormat::parse("xml").is_err());
        assert_eq!(error_kind(super::super::WORKSPACE_BUSY), "busy");
        assert_eq!(error_kind(12345), "other");
    }
}
