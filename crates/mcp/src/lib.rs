//! L6: bicraft's MCP server.
//!
//! [Model Context Protocol](https://modelcontextprotocol.io) over stdio: newline-delimited
//! JSON-RPC 2.0, hand-written (no async runtime). Every tool forwards to an in-process
//! [`bicraft_engine::Session`], so the MCP server and the CLI always behave the same. The same
//! tools are served over Streamable HTTP (revision 2026-07-28 only, one session per workspace)
//! by [`http::HttpServer`].
//!
//! Entry points: [`serve`] (the stdio loop) and [`Server::handle_line`] / [`handle`] (one
//! message, for tests). Logs go to stderr only; stdout is the protocol stream.
//!
//! An optional [`Observer`] ([`Server::with_observer`], [`serve_with_observer`]) sees the session
//! after every `tools/call`, successful or not; the live viewer uses it to publish snapshots.
//! An optional [`IdleHook`] ([`Server::with_idle`], [`serve_with_idle`]) runs while no message
//! waits, until it says it is done; the viewer uses it to keep publishing visuals whose data did
//! not fit a snapshot's time budget, without holding up the next tool call.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

pub mod http;
mod tools;

use std::io::{BufRead, Write};
use std::sync::mpsc;
use std::time::Duration;

use bicraft_engine::Session;
use serde_json::{Value, json};

pub use bicraft_http::StreamLimits;
pub use http::{
    ControlToken, DrainReport, HostPolicy, HttpConfig, HttpServer, LogFormat, LogSink, ShutdownSignal,
    VIEWER_CONTROL_HEADER, ZipLimits, log_event,
};
pub use tools::{Role, ToolResult, call_tool, call_tool_as, tool_definitions};

/// The MCP revision we implement (the latest one we know).
pub const PROTOCOL_VERSION: &str = "2025-06-18";
/// Every revision we can speak; a client asking for one of these gets it echoed back.
pub const SUPPORTED_VERSIONS: &[&str] = &[PROTOCOL_VERSION, "2025-03-26", "2024-11-05"];
/// Longest accepted message line, in bytes. Longer lines are discarded and answered with a parse
/// error, so a hostile client cannot make the server buffer unbounded input.
pub const MAX_LINE_BYTES: usize = 64 * 1024 * 1024;

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;

const INSTRUCTIONS: &str = "bicraft reads, validates and edits Power BI semantic models and reports \
(PBIX, PBIT, PBIP projects, model.bim) and validates DAX. Typical flow: open_file {path} (or new_model \
{name}) -> inspect_file / model_summary -> edit with add_table, add_column, add_measure, set_measure, \
add_relationship (every DAX expression is validated against the model before the edit is applied; \
invalid DAX is rejected with its diagnostics unless force: true) -> validate_model -> save, save_as \
{path} or new_project {dir, name}. Renames rewrite every DAX expression that references the \
renamed object (`dax_rewritten`); removals are refused while DAX uses the object (force: true \
removes it anyway). Both return the expressions that no longer validate in `broken`, and the report \
bindings they affect in `report` (renames and measure moves rewrite those with update_report: true). model_usage shows where an object is used (DAX, RLS, relationships, the report), model_unused lists what nothing uses and remove_unused removes it in one undoable edit. import_objects copies measures, tables, calculation groups, pages, visuals or the theme from another file into the open one (what they use comes along; on_conflict: fail, skip, replace or rename). Reports: report_summary lists pages and visuals; \
add_page, add_visual {page, visual_type, x, y, width, height, fields: [{role, field}]} and bind_field \
edit them, with fields written Table[Column], [Measure] or Sum(Table[Column]) and checked against the \
model; validate_report checks schemas and every field; save writes a PBIP report as PBIR. After \
new_model, run_command report.new starts a blank report. Every action is an engine command: \
list_commands shows ids and parameters (table.*, column.*, measure.*, relationship.*, role.*, \
report.*, page.*, visual.*, file.*), run_command runs one, batch runs several and stops at the first \
error (atomic by default: a failure rolls the whole batch back). run_command edit.undo / edit.redo \
{steps?} step back and forth through the edits; edit.history lists them. validate_dax checks an expression without changing anything. run_tests runs a project's CI gates (DAX assertions, model and report rules from bicraft.tests.yaml) and returns pass/fail per test, as JSON, text, JUnit or SARIF. list_sources shows the M data sources (files, databases, Databricks, web); rebase_sources moves file paths to another root or makes them relative to a parameter, and maps servers and hosts; apply_env / check_env apply or check a profile of bicraft.env.yaml.";

/// Called with the session after every `tools/call` that reached a tool (failed calls too, so
/// [`Session::last_command`] shows errors). It must not block for long: the client waits.
pub type Observer = Box<dyn FnMut(&Session) + Send>;

/// Called by [`serve_with_idle`] with the session while no message waits, after a `tools/call`
/// armed it and every poll interval after that, until it returns `false`. Each run should be
/// short (a bounded snapshot): a message that arrives meanwhile is read once it returns.
pub type IdleHook = Box<dyn FnMut(&Session) -> bool + Send>;

/// An MCP server bound to one engine session.
#[derive(Default)]
pub struct Server {
    session: Session,
    initialized: bool,
    observer: Option<Observer>,
    idle: Option<IdleHook>,
    /// The idle hook asked to run again (set after every `tools/call`).
    idle_armed: bool,
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Server")
            .field("session", &self.session)
            .field("initialized", &self.initialized)
            .field("observer", &self.observer.is_some())
            .field("idle", &self.idle.is_some())
            .finish()
    }
}

/// What a request returns: a plain result, or a tool's result (serialized by the caller).
enum Answer {
    Value(Value),
    Tool(ToolResult),
}

/// The reply to one message: a whole JSON-RPC message, or a tool's result and the request id.
enum Reply {
    Value(Value),
    Tool(Value, ToolResult),
}

fn response(id: Value, result: Value) -> Value {
    // built by hand: `json!` would copy `result` (it serializes its values)
    let mut o = serde_json::Map::with_capacity(3);
    o.insert("jsonrpc".into(), Value::String("2.0".into()));
    o.insert("id".into(), id);
    o.insert("result".into(), result);
    Value::Object(o)
}

fn error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message.into()}})
}

fn to_line(v: &Value) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| {
        r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32603,"message":"serialization failed"}}"#.to_string()
    })
}

impl Server {
    pub fn new() -> Self {
        Self::default()
    }

    /// This server, with `observer` called after every `tools/call` (see [`Observer`]).
    #[must_use]
    pub fn with_observer(mut self, observer: impl FnMut(&Session) + Send + 'static) -> Self {
        self.set_observer(Some(Box::new(observer)));
        self
    }

    /// Sets or removes the observer.
    pub fn set_observer(&mut self, observer: Option<Observer>) {
        self.observer = observer;
    }

    /// This server, with `hook` run while idle (see [`IdleHook`]); armed from the start.
    #[must_use]
    pub fn with_idle(mut self, hook: impl FnMut(&Session) -> bool + Send + 'static) -> Self {
        self.idle = Some(Box::new(hook));
        self.idle_armed = true;
        self
    }

    /// Whether the idle hook wants to run.
    pub fn idle_pending(&self) -> bool {
        self.idle.is_some() && self.idle_armed
    }

    /// Runs the idle hook once (if armed); it stays armed while the hook returns `true`. A
    /// panicking hook is reported on stderr and disarmed.
    pub fn run_idle(&mut self) {
        if !self.idle_armed {
            return;
        }
        let Some(hook) = self.idle.as_mut() else {
            self.idle_armed = false;
            return;
        };
        let session = &self.session;
        self.idle_armed = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| hook(session))) {
            Ok(again) => again,
            Err(_) => {
                eprintln!("bicraft-mcp: the idle hook panicked");
                false
            }
        };
    }

    /// Runs the observer on the current session. A panicking observer is reported on stderr and
    /// never takes the server down.
    fn notify(&mut self) {
        if let Some(observer) = self.observer.as_mut() {
            let session = &self.session;
            let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| observer(session)));
            if run.is_err() {
                eprintln!("bicraft-mcp: the session observer panicked");
            }
        }
    }

    /// The engine session the tools act on.
    pub fn session(&mut self) -> &mut Session {
        &mut self.session
    }

    /// Whether the client has sent `notifications/initialized`.
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Handles one line; returns the reply line (`None` for notifications, responses and blank
    /// lines).
    pub fn handle_line(&mut self, line: &str) -> Option<String> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        let reply = match serde_json::from_str::<Value>(line) {
            Ok(Value::Array(batch)) => {
                // JSON-RPC batches were removed in 2025-06-18; still answer older clients.
                if batch.is_empty() {
                    Some(error(Value::Null, INVALID_REQUEST, "empty batch"))
                } else {
                    let replies: Vec<Value> =
                        batch.into_iter().filter_map(|m| self.handle_message(m)).collect();
                    (!replies.is_empty()).then_some(Value::Array(replies))
                }
            }
            Ok(msg) => {
                // a tool's reply is written straight from its result (the same bytes)
                return match self.reply(msg)? {
                    Reply::Tool(id, r) => Some(
                        r.response_line(&id)
                            .unwrap_or_else(|| to_line(&response(id, r.into_value()))),
                    ),
                    Reply::Value(v) => Some(to_line(&v)),
                };
            }
            Err(e) => Some(error(Value::Null, PARSE_ERROR, format!("parse error: {e}"))),
        };
        reply.as_ref().map(to_line)
    }

    /// Handles one JSON-RPC message; `None` for notifications and responses.
    pub fn handle_message(&mut self, msg: Value) -> Option<Value> {
        self.reply(msg).map(|r| match r {
            Reply::Tool(id, r) => response(id, r.into_value()),
            Reply::Value(v) => v,
        })
    }

    /// [`Server::handle_message`], with a tool's result kept apart from its envelope.
    fn reply(&mut self, msg: Value) -> Option<Reply> {
        let Value::Object(o) = &msg else {
            return Some(Reply::Value(error(
                Value::Null,
                INVALID_REQUEST,
                "message must be a JSON object",
            )));
        };
        let id = o.get("id").cloned();
        let Some(method) = o.get("method").and_then(Value::as_str) else {
            // A response to a server-to-client request (we send none): ignore it.
            if o.contains_key("result") || o.contains_key("error") {
                return None;
            }
            return Some(Reply::Value(error(
                id.filter(valid_id).unwrap_or(Value::Null),
                INVALID_REQUEST,
                "missing string `method`",
            )));
        };
        let Some(id) = id else {
            self.notification(method);
            return None;
        };
        if !valid_id(&id) {
            return Some(Reply::Value(error(
                Value::Null,
                INVALID_REQUEST,
                "`id` must be a string or a number",
            )));
        }
        let params = o.get("params").cloned().unwrap_or(Value::Null);
        if !(params.is_object() || params.is_null()) {
            return Some(Reply::Value(error(
                id,
                INVALID_PARAMS,
                "`params` must be an object",
            )));
        }
        Some(match self.request(method, &params) {
            Ok(Answer::Value(r)) => Reply::Value(response(id, r)),
            Ok(Answer::Tool(r)) => Reply::Tool(id, r),
            Err((code, m)) => Reply::Value(error(id, code, m)),
        })
    }

    fn notification(&mut self, method: &str) {
        match method {
            "notifications/initialized" => self.initialized = true,
            "notifications/cancelled" | "notifications/progress" | "notifications/roots/list_changed" => {}
            other => eprintln!("bicraft-mcp: ignoring notification `{other}`"),
        }
    }

    fn request(&mut self, method: &str, params: &Value) -> Result<Answer, (i64, String)> {
        match method {
            "initialize" => {
                let asked = params.get("protocolVersion").and_then(Value::as_str);
                let version = asked
                    .and_then(|a| SUPPORTED_VERSIONS.iter().find(|v| **v == a))
                    .copied()
                    .unwrap_or(PROTOCOL_VERSION);
                Ok(Answer::Value(json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "bicraft", "title": "BIcraft", "version": env!("CARGO_PKG_VERSION")},
                    "instructions": INSTRUCTIONS,
                })))
            }
            "ping" => Ok(Answer::Value(json!({}))),
            "tools/list" => Ok(Answer::Value(json!({"tools": tool_definitions()}))),
            "tools/call" => {
                let name = params
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or((INVALID_PARAMS, "missing string `name`".to_string()))?;
                let args = match params.get("arguments") {
                    None | Some(Value::Null) => json!({}),
                    Some(a @ Value::Object(_)) => a.clone(),
                    Some(_) => return Err((INVALID_PARAMS, "`arguments` must be an object".to_string())),
                };
                if !tools::is_tool(name) {
                    return Err((INVALID_PARAMS, format!("unknown tool `{name}` (see tools/list)")));
                }
                let result = call_tool(&mut self.session, name, &args);
                self.notify();
                self.idle_armed = self.idle.is_some();
                Ok(Answer::Tool(result))
            }
            other => Err((METHOD_NOT_FOUND, format!("method not found: {other}"))),
        }
    }
}

fn valid_id(id: &Value) -> bool {
    id.is_string() || id.is_number()
}

/// Handles one line on `server`; see [`Server::handle_line`].
pub fn handle(server: &mut Server, line: &str) -> Option<String> {
    server.handle_line(line)
}

/// Serves newline-delimited JSON-RPC on a fresh session until `input` closes.
pub fn serve<R: BufRead, W: Write>(input: R, output: W) -> std::io::Result<()> {
    serve_with_limit(&mut Server::new(), input, output, MAX_LINE_BYTES)
}

/// [`serve`] with an [`Observer`] called after every `tools/call`.
pub fn serve_with_observer<R: BufRead, W: Write>(
    input: R,
    output: W,
    observer: impl FnMut(&Session) + Send + 'static,
) -> std::io::Result<()> {
    serve_with_limit(
        &mut Server::new().with_observer(observer),
        input,
        output,
        MAX_LINE_BYTES,
    )
}

/// [`serve`] with an explicit server (which may carry an observer) and line-length cap.
pub fn serve_with_limit<R: BufRead, W: Write>(
    server: &mut Server,
    mut input: R,
    mut output: W,
    max_line: usize,
) -> std::io::Result<()> {
    let mut buf = Vec::new();
    loop {
        let reply = match read_line_capped(&mut input, &mut buf, max_line)? {
            Line::Eof => return Ok(()),
            Line::TooLong => Some(to_line(&error(
                Value::Null,
                PARSE_ERROR,
                format!("message longer than {max_line} bytes"),
            ))),
            Line::Complete => match std::str::from_utf8(&buf) {
                Ok(line) => server.handle_line(line),
                Err(_) => Some(to_line(&error(
                    Value::Null,
                    PARSE_ERROR,
                    "message is not valid UTF-8",
                ))),
            },
        };
        if let Some(reply) = reply {
            output.write_all(reply.as_bytes())?;
            output.write_all(b"\n")?;
            output.flush()?;
            // after the answer: background work (aggregation jobs) the command asked for
            server.session().background_work();
        }
    }
}

/// [`serve_with_limit`] that runs the server's idle hook ([`Server::with_idle`]) every `poll`
/// while it is armed and no message waits. Input is read on its own thread, so a message is
/// handled as soon as the running hook returns.
pub fn serve_with_idle<R: BufRead + Send + 'static, W: Write>(
    server: &mut Server,
    mut input: R,
    mut output: W,
    max_line: usize,
    poll: Duration,
) -> std::io::Result<()> {
    // at most a few lines read ahead of the server
    let (tx, rx) = mpsc::sync_channel::<std::io::Result<(Line, Vec<u8>)>>(4);
    std::thread::Builder::new()
        .name("mcp-stdin".into())
        .spawn(move || {
            let mut buf = Vec::new();
            loop {
                let item =
                    read_line_capped(&mut input, &mut buf, max_line).map(|l| (l, std::mem::take(&mut buf)));
                let stop = !matches!(item, Ok((Line::Complete | Line::TooLong, _)));
                if tx.send(item).is_err() || stop {
                    return;
                }
            }
        })?;
    loop {
        let item = if server.idle_pending() {
            match rx.recv_timeout(poll) {
                Ok(item) => item,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    server.run_idle();
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
            }
        } else {
            match rx.recv() {
                Ok(item) => item,
                Err(_) => return Ok(()),
            }
        };
        let (line, buf) = item?;
        let reply = match line {
            Line::Eof => return Ok(()),
            Line::TooLong => Some(to_line(&error(
                Value::Null,
                PARSE_ERROR,
                format!("message longer than {max_line} bytes"),
            ))),
            Line::Complete => match std::str::from_utf8(&buf) {
                Ok(line) => server.handle_line(line),
                Err(_) => Some(to_line(&error(
                    Value::Null,
                    PARSE_ERROR,
                    "message is not valid UTF-8",
                ))),
            },
        };
        if let Some(reply) = reply {
            output.write_all(reply.as_bytes())?;
            output.write_all(b"\n")?;
            output.flush()?;
            // after the answer: background work (aggregation jobs) the command asked for
            server.session().background_work();
        }
    }
}

enum Line {
    Eof,
    Complete,
    TooLong,
}

/// Reads one `\n`-terminated line into `buf` (without the terminator), keeping at most `max`
/// bytes. An over-long line is read to its end and discarded.
fn read_line_capped(input: &mut impl BufRead, buf: &mut Vec<u8>, max: usize) -> std::io::Result<Line> {
    buf.clear();
    let mut too_long = false;
    let mut any = false;
    loop {
        let available = match input.fill_buf() {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if available.is_empty() {
            return Ok(match (any, too_long) {
                (false, _) => Line::Eof,
                (true, true) => Line::TooLong,
                (true, false) => Line::Complete,
            });
        }
        any = true;
        let (chunk, done) = match available.iter().position(|&b| b == b'\n') {
            Some(i) => (available.get(..i).unwrap_or_default(), true),
            None => (available, false),
        };
        let used = chunk.len().saturating_add(usize::from(done));
        if !too_long {
            if buf.len().saturating_add(chunk.len()) > max {
                too_long = true;
                buf.clear();
                buf.shrink_to_fit();
            } else {
                buf.extend_from_slice(chunk);
            }
        }
        input.consume(used);
        if done {
            return Ok(if too_long { Line::TooLong } else { Line::Complete });
        }
    }
}

#[cfg(test)]
mod idle_tests;
#[cfg(test)]
mod observer_tests;
#[cfg(test)]
#[path = "../../../testsupport/tempdir.rs"]
mod tempdir;
#[cfg(test)]
mod tests;
