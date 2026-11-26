//! The UI control channel (ROADMAP M11.6, docs/desktop.md "Control channel", docs/webui.md): an
//! agent drives the running app and looks at it, through the same code as a person's clicks.
//! Opt-in in every shell, never OS-level synthetic input.
//!
//! The protocol is the Crafting Apps' JSON-lines shape (DeckCraft's `docs/control-protocol.md`):
//! one request object `{"id", "method", "params", "token"}`, one reply
//! `{"id", "ok": true, "result"}` or `{"id", "ok": false, "error"}`. This module is the part both
//! shells share: [`parse`] checks a request, [`Job`] carries it to the window, and [`pump`] runs
//! it on the window's thread between frames (from [`App::logic`]). The transports are the
//! shells': a loopback socket with a token on the desktop (`apps/bicraft-desktop/src/control.rs`),
//! the viewer server's relay to the page in the browser (`crates/view`, `apps/bicraft-web`). The
//! token is checked by the transport, never here.
//!
//! What runs is what the window's buttons run: a command goes through [`ribbon::invoke`] (the
//! File view's flows, the parameters form when values are missing, else the command on the
//! active document's source), a tab through [`App::select_ribbon_tab`], a page through the page
//! tab's `view.interact` action, the palette and the forms through [`palette`] and [`form`]. A
//! screenshot is egui's [`egui::ViewportCommand::Screenshot`]: the frame as the GPU drew it, as
//! PNG, base64 in the reply.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex, PoisonError};

use egui::{ScreenshotCallback, ViewportCommand};
use serde_json::{Map, Value, json};

use crate::app::{App, Tab as View};
use crate::backstage::{self, Backstage};
use crate::model_view::Sel;
use crate::registry::{self, Kind, Tab as RTab};
use crate::ribbon::{self, VisualSel};
use crate::theme::Brightness;
use crate::{form, palette};

/// Largest request (one line on the socket, one body over HTTP).
pub const MAX_REQUEST_BYTES: usize = 1 << 20;
/// Largest reply a client reads (a screenshot of a large window, base64).
pub const MAX_REPLY_BYTES: usize = 48 << 20;
/// Largest PNG a screenshot returns (before base64).
pub const MAX_PNG_BYTES: usize = 32 << 20;
/// How long a transport waits for the window's reply, in seconds.
pub const REPLY_TIMEOUT_SECS: u64 = 60;
/// Requests queued for the window at most (more are answered `busy`).
pub const MAX_QUEUED: usize = 64;
/// Requests started per frame at most (the rest wait for the next frame).
const PER_FRAME: usize = 16;
/// How long a screenshot may take, in the window's seconds.
const SHOT_SECS: f64 = 10.0;
/// How long `ui.wait` and the waits of `command`, `app.open` and `ui.set` last at most.
const WAIT_SECS: f64 = 55.0;

/// The methods, their parameters and what they do (`methods` returns this list;
/// docs/desktop.md has the same table).
pub const METHODS: &[(&str, &str, &str)] = &[
    ("methods", "", "this list"),
    (
        "ui.inspect",
        "",
        "the window's state: views, ribbon tab, pages, selection, File view, palette, form, answer window, documents, title, error, status",
    ),
    (
        "ui.commands",
        "{query?}",
        "the palette's rows: id, label, place, shortcut, enabled or why not",
    ),
    (
        "command",
        "{command, params?}",
        "with every required value in params, runs the command as the form's Run does; without params, clicks its button (ribbon::invoke: the selection's values, the form or the File view when values are missing); waits for its answer",
    ),
    (
        "app.open",
        "{path}",
        "opens a file (the desktop) or a workspace file (the browser); waits for its first snapshot",
    ),
    (
        "ui.set",
        "{doc?, theme?, view?, backstage?, page?, visual?, model?, ribbonTab?}",
        "switches what a click would switch; returns ui.inspect",
    ),
    (
        "ui.palette",
        "{open?, query?}",
        "opens (or closes) the command palette, with a query",
    ),
    (
        "ui.palette.choose",
        "{command, form?}",
        "Enter (or Shift+Enter with form) on a palette row",
    ),
    (
        "ui.form.open",
        "{command, prefill?}",
        "opens a command's parameters form (prefill defaults to the selection's)",
    ),
    (
        "ui.form.set",
        "{fields: {name: value}, json?}",
        "types values into the open form's fields",
    ),
    (
        "ui.form.run",
        "",
        "Run in the open form: the problems, or the command's answer",
    ),
    (
        "ui.close",
        "{what?}",
        "closes form, palette, answer, confirm, popup, backstage or all (default: the topmost)",
    ),
    (
        "ui.wait",
        "{timeout?}",
        "waits until the document is loaded, every command answered, the data computed and the page drawn",
    ),
    (
        "ui.screenshot",
        "",
        "the window as drawn: {png (base64), width, height}",
    ),
];

// --- requests and replies -------------------------------------------------------------------

/// One request.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    /// The client's id, echoed in the reply (a number, a string or null).
    pub id: Value,
    pub method: String,
    /// An object (`{}` when absent).
    pub params: Value,
}

/// Reads one request: a JSON object with a string `method`, an optional `id` (number, string or
/// null), optional object `params` and the transport's `token` (returned, not checked here).
/// Anything else is an error, and the transport closes the connection.
pub fn parse(bytes: &[u8]) -> Result<(Request, Option<String>), String> {
    if bytes.len() > MAX_REQUEST_BYTES {
        return Err(format!("request larger than {MAX_REQUEST_BYTES} bytes"));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "the request is not UTF-8".to_string())?;
    let v: Value = serde_json::from_str(text.trim())
        .map_err(|_| "the request is not JSON: one object per line".to_string())?;
    let Value::Object(mut o) = v else {
        return Err("the request must be a JSON object".into());
    };
    let method = match o.remove("method") {
        Some(Value::String(m)) if !m.is_empty() && m.len() <= 64 => m,
        _ => return Err("the request needs a string `method`".into()),
    };
    let id = match o.remove("id") {
        None | Some(Value::Null) => Value::Null,
        Some(v @ (Value::Number(_) | Value::String(_))) => v,
        Some(_) => return Err("`id` must be a number or a string".into()),
    };
    let params = match o.remove("params") {
        None | Some(Value::Null) => Value::Object(Map::new()),
        Some(v @ Value::Object(_)) => v,
        Some(_) => return Err("`params` must be an object".into()),
    };
    let token = match o.remove("token") {
        None => None,
        Some(Value::String(t)) => Some(t),
        Some(_) => return Err("`token` must be a string".into()),
    };
    if let Some(k) = o.keys().next() {
        return Err(format!("unknown key `{}`", crate::app::clip(k, 40)));
    }
    Ok((Request { id, method, params }, token))
}

/// A success reply.
pub fn ok(id: &Value, result: Value) -> Value {
    json!({"id": id, "ok": true, "result": result})
}

/// An error reply.
pub fn err(id: &Value, error: impl Into<String>) -> Value {
    json!({"id": id, "ok": false, "error": error.into()})
}

const QUEUED: u8 = 0;
const STARTED: u8 = 1;
const ABANDONED: u8 = 2;

/// A request on its way to the window, with where its reply goes.
pub struct Job {
    pub request: Request,
    /// The server workspace file whose document tab the request runs in (the browser relay's
    /// request addressed to a document view): the window switches to that tab first, as a click
    /// on it does.
    pub doc: Option<String>,
    reply: SyncSender<Value>,
    state: Arc<AtomicU8>,
}

/// The transport's half of a [`Job`]: the reply when it comes.
pub struct Ticket {
    id: Value,
    rx: Receiver<Value>,
    state: Arc<AtomicU8>,
}

impl Job {
    /// A job for `request` and the ticket its reply comes back on.
    pub fn new(request: Request) -> (Job, Ticket) {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let state = Arc::new(AtomicU8::new(QUEUED));
        let ticket = Ticket {
            id: request.id.clone(),
            rx,
            state: Arc::clone(&state),
        };
        (
            Job {
                request,
                doc: None,
                reply: tx,
                state,
            },
            ticket,
        )
    }
}

impl Job {
    /// The same job, run in the document tab of workspace file `file`.
    pub fn in_doc(mut self, file: Option<String>) -> Job {
        self.doc = file;
        self
    }
}

impl Job {
    /// Answers the job without running it in a window (a transport's own refusal, tests); false
    /// when its transport gave up on it.
    pub fn answer(self, reply: Value) -> bool {
        let start = self
            .state
            .compare_exchange(QUEUED, STARTED, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok();
        start && self.reply.try_send(reply).is_ok()
    }
}

impl Ticket {
    /// The reply, if it has come.
    pub fn try_take(&self) -> Option<Value> {
        self.rx.try_recv().ok()
    }

    /// The reply, or a timeout error after `timeout`: a request the window had not started yet
    /// is given up (it will never run, so a retry cannot run it twice); one it had started may
    /// still have had its effect.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn wait(self, timeout: std::time::Duration) -> Value {
        if let Ok(v) = self.rx.recv_timeout(timeout) {
            return v;
        }
        self.give_up(timeout.as_secs())
    }

    /// The timeout error, the request given up when the window had not started it.
    pub fn give_up(&self, secs: u64) -> Value {
        // a reply that came at the last moment wins
        if let Ok(v) = self.rx.try_recv() {
            return v;
        }
        let not_started = self
            .state
            .compare_exchange(QUEUED, ABANDONED, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok();
        if not_started {
            err(
                &self.id,
                format!(
                    "timeout: the window did not take the request within {secs} s (it will not run; is the window minimized?)"
                ),
            )
        } else {
            err(
                &self.id,
                format!(
                    "timeout: no answer within {secs} s; the request had started, its outcome is unknown"
                ),
            )
        }
    }
}

// --- the window's side ----------------------------------------------------------------------

/// What a request waits for before its reply.
#[derive(Debug)]
enum Wait {
    /// The screenshot with this number.
    Shot(u64),
    /// The answer of the command sent with `tag` (and, after it, the window idle).
    Call { tag: u64, id: &'static str },
    /// The window idle, from the frame after `frame` on.
    Idle { frame: u64 },
    /// The page `name` shown.
    Page {
        name: String,
        frame: u64,
        /// The keys of `ui.set` applied once the page is shown (a visual on it, a ribbon tab).
        then: Value,
    },
    /// The open form's run with `tag` answered.
    Form { tag: u64 },
}

struct Waiting {
    id: Value,
    reply: SyncSender<Value>,
    wait: Wait,
    /// The window's time (seconds) after which it is answered with a timeout.
    until: f64,
    /// What to answer with `ui.inspect`'s state (`ui.set`).
    inspect: bool,
}

/// The control channel's state in the window: the inbox the shell attached and the requests
/// waiting for something (a screenshot, an answer).
#[derive(Default)]
pub struct Control {
    inbox: Option<Receiver<Job>>,
    waiting: Vec<Waiting>,
    shots: u64,
    /// The screenshots egui's callbacks delivered, which the next [`pump`] answers.
    taken: Taken,
}

impl Control {
    /// Takes requests from `inbox` from now on (the shell's transport sends them).
    pub fn attach(&mut self, inbox: Receiver<Job>) {
        self.inbox = Some(inbox);
    }

    /// Whether a transport is attached.
    pub fn attached(&self) -> bool {
        self.inbox.is_some()
    }

    /// Requests waiting for their reply.
    pub fn waiting(&self) -> usize {
        self.waiting.len()
    }
}

/// The screenshots taken and not yet answered: each one's number and the frame, filled by the
/// callback of egui's [`ScreenshotCallback`] (on whichever thread the shell reads the pixels).
type Taken = Arc<Mutex<Vec<(u64, Arc<egui::ColorImage>)>>>;

enum Outcome {
    Done(Result<Value, String>),
    Later(Wait, f64),
}

/// Runs the requests that arrived and answers the ones whose wait is over. Called by
/// [`App::logic`] on every frame; does nothing without an attached transport.
pub fn pump(app: &mut App, ctx: &egui::Context) {
    if !app.control.attached() {
        return;
    }
    let mut c = std::mem::take(&mut app.control);
    let now = ctx.input(|i| i.time);
    let frame = ctx.cumulative_frame_nr();
    // screenshots taken since the last frame
    let shots = std::mem::take(&mut *c.taken.lock().unwrap_or_else(PoisonError::into_inner));
    for (n, image) in shots {
        if let Some(pos) = c
            .waiting
            .iter()
            .position(|w| matches!(w.wait, Wait::Shot(s) if s == n))
        {
            let w = c.waiting.remove(pos);
            let r = shot_result(&image);
            let _ = w.reply.try_send(reply(&w.id, r));
        }
    }
    // new requests
    let mut taken = 0;
    while taken < PER_FRAME {
        let job = match c.inbox.as_ref().map(Receiver::try_recv) {
            Some(Ok(j)) => j,
            Some(Err(TryRecvError::Disconnected)) => {
                c.inbox = None;
                break;
            }
            _ => break,
        };
        taken += 1;
        // a request the transport gave up on never runs
        let start = job
            .state
            .compare_exchange(QUEUED, STARTED, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok();
        if !start {
            continue;
        }
        let Job {
            request,
            reply: tx,
            doc,
            ..
        } = job;
        if let Some(file) = doc {
            match app.workspace_doc(&file) {
                Some(i) => app.switch_doc(i),
                None => {
                    let why = format!(
                        "the document tab of `{}` is not open in this page",
                        crate::app::clip(&file, 120)
                    );
                    let _ = tx.try_send(err(&request.id, why));
                    continue;
                }
            }
        }
        let inspect = request.method == "ui.set";
        match handle(app, &mut c, ctx, &request, frame) {
            Outcome::Done(r) => {
                let _ = tx.try_send(reply(&request.id, r));
            }
            Outcome::Later(wait, secs) => c.waiting.push(Waiting {
                id: request.id,
                reply: tx,
                wait,
                until: now + secs,
                inspect,
            }),
        }
    }
    // waits that are over
    let mut i = 0;
    while i < c.waiting.len() {
        let Some(w) = c.waiting.get(i) else { break };
        let done = match &w.wait {
            Wait::Shot(_) => None,
            Wait::Call { tag, id } => (!app.waiting_for(*tag)).then(|| call_result(app, *tag, id)),
            Wait::Idle { frame: f } => (frame > *f && idle(app)).then(|| Ok(inspect(app, ctx))),
            Wait::Page { name, frame: f, then } => {
                if frame > *f && page_shown(app, name) && idle(app) {
                    let then = then.clone();
                    Some(match set(app, &then) {
                        Ok(_) => Ok(inspect(app, ctx)),
                        Err(e) => Err(e),
                    })
                } else {
                    None
                }
            }
            Wait::Form { tag } => match &app.form {
                Some(f) if f.running == Some(*tag) => None,
                Some(f) => Some(match &f.result {
                    Some(Ok(v)) => Ok(json!({"command": f.id, "result": v})),
                    Some(Err(e)) => Err(e.clone()),
                    None => Ok(json!({"command": f.id})),
                }),
                None => Some(Err("the form was closed before its answer".into())),
            },
        };
        let done = match done {
            Some(r) => Some(if w.inspect { r.map(|_| inspect(app, ctx)) } else { r }),
            None if now > w.until => Some(Err(match w.wait {
                Wait::Shot(_) => "no frame was captured: the window draws no frame (minimized, hidden or without a GPU renderer)".to_string(),
                _ => "timeout: the window is still busy".to_string(),
            })),
            None => None,
        };
        match done {
            Some(r) => {
                let w = c.waiting.remove(i);
                let _ = w.reply.try_send(reply(&w.id, r));
            }
            None => i += 1,
        }
    }
    if !c.waiting.is_empty() {
        ctx.request_repaint_after(std::time::Duration::from_millis(30));
    }
    app.control = c;
}

fn reply(id: &Value, r: Result<Value, String>) -> Value {
    match r {
        Ok(v) => ok(id, v),
        Err(e) => err(id, e),
    }
}

/// Runs one request: its reply now, or what it waits for.
fn handle(app: &mut App, c: &mut Control, ctx: &egui::Context, r: &Request, frame: u64) -> Outcome {
    let p = &r.params;
    let done = Outcome::Done;
    match r.method.as_str() {
        "methods" => done(Ok(Value::Array(
            METHODS
                .iter()
                .map(|(m, params, what)| json!({"method": m, "params": params, "does": what}))
                .collect(),
        ))),
        "ui.inspect" => done(Ok(inspect(app, ctx))),
        "ui.commands" => {
            let q = p.get("query").and_then(Value::as_str).unwrap_or("");
            let rows = palette::rows(app, &crate::app::clip(q, 200));
            done(Ok(Value::Array(
                rows.iter()
                    .map(|row| {
                        json!({
                            "id": row.id, "label": row.label, "place": row.path,
                            "shortcut": row.shortcut, "enabled": row.state.is_ok(),
                            "reason": row.state.as_ref().err(),
                        })
                    })
                    .collect(),
            )))
        }
        "command" | "engine.execute" => command(app, p, frame),
        "app.open" => match p.get("path").and_then(Value::as_str) {
            Some(path) => open(app, path, frame),
            None => done(Err("app.open needs a string `path`".into())),
        },
        "ui.set" => match set(app, p) {
            Ok(Some((page, then))) => Outcome::Later(
                Wait::Page {
                    name: page,
                    frame,
                    then,
                },
                WAIT_SECS,
            ),
            Ok(None) => Outcome::Later(Wait::Idle { frame }, WAIT_SECS),
            Err(e) => done(Err(e)),
        },
        "ui.palette" => {
            let open = p.get("open").and_then(Value::as_bool).unwrap_or(true);
            if open != app.palette.is_some() {
                palette::toggle(app);
            }
            if let (Some(pal), Some(q)) = (&mut app.palette, p.get("query").and_then(Value::as_str)) {
                pal.query = crate::app::clip(q, 200);
                pal.sel = 0;
            }
            done(Ok(inspect(app, ctx)))
        }
        "ui.palette.choose" => {
            let Some(id) = p.get("command").and_then(Value::as_str) else {
                return done(Err("ui.palette.choose needs a string `command`".into()));
            };
            let Some(cmd) = registry::find(id) else {
                return done(Err(format!("unknown command `{}`", crate::app::clip(id, 80))));
            };
            if let Err(why) = ribbon::state(app, cmd.id) {
                return done(Err(format!("{}: {why}", cmd.id)));
            }
            let before = Before::of(app);
            palette::choose(
                app,
                cmd.id,
                p.get("form").and_then(Value::as_bool).unwrap_or(false),
            );
            after_invoke(app, cmd.id, &before)
        }
        "ui.form.open" => {
            let Some(id) = p.get("command").and_then(Value::as_str) else {
                return done(Err("ui.form.open needs a string `command`".into()));
            };
            let Some(cmd) = registry::find(id) else {
                return done(Err(format!("unknown command `{}`", crate::app::clip(id, 80))));
            };
            let pre = match p.get("prefill") {
                Some(v @ Value::Object(_)) => v.clone(),
                None | Some(Value::Null) => ribbon::prefill(app, cmd, None),
                Some(_) => return done(Err("`prefill` must be an object".into())),
            };
            form::open(app, cmd.id, pre);
            done(Ok(form_state(app)))
        }
        "ui.form.set" => done(form_set(app, p).map(|()| form_state(app))),
        "ui.form.run" => {
            let Some(f) = &app.form else {
                return done(Err("no form is open".into()));
            };
            let id = f.id;
            if let Err(errs) = form::params(f, app.lang) {
                let names: Vec<Value> = errs
                    .iter()
                    .map(|(i, e)| {
                        let name = f.inputs.get(*i).map_or("json", |inp| inp.field.name);
                        json!({"field": name, "problem": e})
                    })
                    .collect();
                form::run(app);
                return done(Err(format!(
                    "the form's values are not valid: {}",
                    Value::Array(names)
                )));
            }
            if let Err(why) = ribbon::state(app, id) {
                return done(Err(format!("{id}: {why}")));
            }
            let tag = app.next_tag();
            form::run(app);
            if app.form.as_ref().and_then(|f| f.running) == Some(tag) {
                Outcome::Later(Wait::Form { tag }, WAIT_SECS)
            } else {
                done(Err(app
                    .error
                    .clone()
                    .unwrap_or_else(|| "the form's command was not sent".into())))
            }
        }
        "ui.close" => done(close(app, p.get("what").and_then(Value::as_str)).map(|()| inspect(app, ctx))),
        "ui.wait" => {
            let secs = p
                .get("timeout")
                .and_then(Value::as_f64)
                .filter(|t| t.is_finite() && *t > 0.0)
                .map_or(WAIT_SECS, |t| t.min(WAIT_SECS));
            Outcome::Later(Wait::Idle { frame }, secs)
        }
        "ui.screenshot" => {
            c.shots = c.shots.wrapping_add(1);
            let n = c.shots;
            let (taken, repaint) = (Arc::clone(&c.taken), ctx.clone());
            ctx.send_viewport_cmd(ViewportCommand::Screenshot(ScreenshotCallback::new(
                move |image| {
                    taken
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .push((n, image));
                    repaint.request_repaint();
                },
            )));
            ctx.request_repaint();
            Outcome::Later(Wait::Shot(n), SHOT_SECS)
        }
        m => done(Err(format!(
            "unknown method `{}` (`methods` lists them)",
            crate::app::clip(m, 64)
        ))),
    }
}

/// `command`: the button's path; `file.open` with a path opens it (never a dialog here).
fn command(app: &mut App, p: &Value, frame: u64) -> Outcome {
    let Some(id) = p.get("command").and_then(Value::as_str) else {
        return Outcome::Done(Err(
            "command needs a string `command` (an engine command id)".into()
        ));
    };
    let Some(cmd) = registry::find(id) else {
        return Outcome::Done(Err(format!(
            "unknown command `{}` (ui.commands lists them)",
            crate::app::clip(id, 80)
        )));
    };
    let (params, given) = match p.get("params") {
        Some(v @ Value::Object(_)) => (v.clone(), true),
        None | Some(Value::Null) => (ribbon::prefill(app, cmd, None), false),
        Some(_) => return Outcome::Done(Err("`params` must be an object".into())),
    };
    if cmd.id == "file.open" {
        return match params.get("path").and_then(Value::as_str) {
            Some(path) => open(app, path, frame),
            None => Outcome::Done(Err(
                "file.open needs a `path` here: the control channel never opens a dialog".into(),
            )),
        };
    }
    let before = Before::of(app);
    // every required value given: run it as the form's Run does; else the button's click (the
    // File view's flows, the form when values are missing)
    if given && !cmd.needs_values(&params) {
        if let Err(why) = ribbon::state(app, cmd.id) {
            return Outcome::Done(Err(format!("{}: {why}", cmd.id)));
        }
        ribbon::send(app, cmd, params);
        return after_invoke(app, cmd.id, &before);
    }
    ribbon::invoke(app, cmd.id, params);
    if app.next_tag() == before.tag && app.status != before.status && ribbon::state(app, cmd.id).is_err() {
        return Outcome::Done(Err(app.status.clone()));
    }
    after_invoke(app, cmd.id, &before)
}

/// What a click may change, taken before it.
struct Before {
    tag: u64,
    status: String,
    error: Option<String>,
    backstage: bool,
}

impl Before {
    fn of(app: &App) -> Before {
        Before {
            tag: app.next_tag(),
            status: app.status.clone(),
            error: app.error.clone(),
            backstage: app.backstage.is_some(),
        }
    }
}

/// What a button's click did: a command sent (wait for its answer), a form or the File view
/// opened, or an error.
fn after_invoke(app: &mut App, id: &'static str, before: &Before) -> Outcome {
    let sent = app.next_tag() > before.tag;
    if sent {
        // the last command sent (a duplicate sends a second one when the first answers)
        let tag = app.next_tag().saturating_sub(1);
        return Outcome::Later(Wait::Call { tag, id }, WAIT_SECS);
    }
    if let Some(f) = &app.form
        && f.id == id
    {
        return Outcome::Done(Ok(
            json!({"command": id, "opened": "form", "form": form_state(app)}),
        ));
    }
    if id == "measure.add"
        && let Some(t) = app.formula.target.as_ref()
        && t.new
    {
        return Outcome::Done(Ok(
            json!({"command": id, "opened": "formula_bar", "text": app.formula.text}),
        ));
    }
    if let Some(bs) = &app.backstage
        && !before.backstage
    {
        return Outcome::Done(Ok(
            json!({"command": id, "opened": "backstage", "page": page_name(bs.page)}),
        ));
    }
    if let Some(e) = &app.error
        && before.error.as_ref() != Some(e)
    {
        return Outcome::Done(Err(e.clone()));
    }
    Outcome::Done(Ok(json!({"command": id})))
}

fn call_result(app: &App, tag: u64, id: &str) -> Result<Value, String> {
    match app.answered.iter().rev().find(|(t, _)| *t == tag) {
        Some((_, Ok(v))) => Ok(json!({"command": id, "result": v})),
        Some((_, Err(e))) => Err(format!("{id}: {e}")),
        None => Ok(json!({"command": id})),
    }
}

/// `app.open`: a path on the desktop, a workspace file in the browser.
fn open(app: &mut App, path: &str, frame: u64) -> Outcome {
    if path.is_empty() || path.len() > 4096 || path.contains('\0') {
        return Outcome::Done(Err("`path` must be a file path".into()));
    }
    if app.caps().open_files {
        app.error = None;
        app.open_path(std::path::Path::new(path));
    } else if app.workspace().is_some() {
        if !crate::remote::safe_relative(path) {
            return Outcome::Done(Err("a workspace-relative path (no drive, no `..`)".into()));
        }
        app.error = None;
        app.open_ws_file(path);
    } else {
        return Outcome::Done(Err("this page opens no other file (bicraft view)".into()));
    }
    Outcome::Later(Wait::Idle { frame }, WAIT_SECS)
}

/// The window has nothing left to do: no source about to start, the document's snapshot here
/// (or its error), every command answered, every bound visual's data computed, and the page
/// shown drawn.
fn idle(app: &App) -> bool {
    if app.opening() || app.calls_pending() > 0 {
        return false;
    }
    if !app.has_doc() {
        return true;
    }
    let Some(l) = &app.loaded else {
        return app.error.is_some();
    };
    if data_pending(&l.snap) {
        return false;
    }
    let drawn = app.shown.as_ref().is_some_and(|s| s.0.seq == l.seq);
    app.tab != View::Report || app.backstage.is_some() || app.page_names().is_empty() || drawn
}

/// A visual with bound fields has no data yet (`bicraft_engine::watch::data_pending`).
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

fn page_shown(app: &App, name: &str) -> bool {
    app.page_names()
        .get(app.active_page())
        .is_some_and(|p| p.0 == name)
}

// --- ui.set ---------------------------------------------------------------------------------

const SET_KEYS: [&str; 9] = [
    "doc",
    "theme",
    "view",
    "backstage",
    "page",
    "visual",
    "model",
    "ribbonTab",
    "buildTab",
];

/// `ui.set`: applies each key as its click does, in a fixed order. Returns the page asked for
/// (the reply waits until it is shown).
fn set(app: &mut App, p: &Value) -> Result<Option<(String, Value)>, String> {
    let Some(o) = p.as_object() else {
        return Err("params must be an object".into());
    };
    if let Some(k) = o.keys().find(|k| !SET_KEYS.contains(&k.as_str())) {
        return Err(format!(
            "unknown key `{}` (ui.set takes {})",
            crate::app::clip(k, 40),
            SET_KEYS.join(", ")
        ));
    }
    if let Some(d) = o.get("doc") {
        let n = d.as_u64().ok_or("`doc` is a document tab's index")?;
        let count = app.doc_tabs().0.len();
        let i = usize::try_from(n)
            .ok()
            .filter(|i| *i < count)
            .ok_or_else(|| format!("no document tab {n} ({count} open)"))?;
        app.switch_doc(i);
    }
    if let Some(t) = o.get("theme") {
        let b = match t.as_str() {
            Some("light") => Brightness::Light,
            Some("dark") => Brightness::Dark,
            Some("high-contrast") => Brightness::HighContrast,
            _ => return Err("`theme` is light, dark or high-contrast".into()),
        };
        app.set_theme(b);
    }
    if let Some(v) = o.get("view") {
        let tab = match v.as_str() {
            Some("report") => View::Report,
            Some("data") => View::Data,
            Some("model") => View::Model,
            Some("query") => View::Query,
            Some("tmdl") => View::Tmdl,
            Some("validation") => View::Validation,
            _ => return Err("`view` is report, data, model, query, tmdl or validation".into()),
        };
        app.backstage = None;
        app.show_tab(tab);
    }
    if let Some(b) = o.get("backstage") {
        match b {
            Value::Null | Value::Bool(false) => app.backstage = None,
            Value::String(s) => {
                let page = [
                    backstage::Page::Home,
                    backstage::Page::Open,
                    backstage::Page::Info,
                    backstage::Page::Export,
                    backstage::Page::Options,
                ]
                .into_iter()
                .find(|pg| page_name(*pg) == s)
                .ok_or("`backstage` is home, open, info, export, options, or null to close it")?;
                if page.needs_doc() && !app.has_doc() {
                    return Err(format!("the File view's {s} page needs an open document"));
                }
                app.backstage = Some(Backstage::new(page));
            }
            _ => return Err("`backstage` is home, open, info, export, options, or null".into()),
        }
    }
    let mut wait_page = None;
    if let Some(pg) = o.get("page") {
        let pages = app.page_names();
        let found = match pg {
            Value::Number(n) => n
                .as_u64()
                .and_then(|i| usize::try_from(i).ok())
                .and_then(|i| pages.get(i)),
            Value::String(s) => pages
                .iter()
                .find(|x| &x.0 == s)
                .or_else(|| pages.iter().find(|x| &x.1 == s)),
            _ => None,
        };
        let name = found
            .map(|x| x.0.clone())
            .ok_or("`page` is a page's name, display name or index in the report")?;
        if !page_shown(app, &name) {
            app.interact(json!({"action": "page", "page": name}));
        }
        app.backstage = None;
        if app.tab != View::Report {
            app.show_tab(View::Report);
        }
        if !page_shown(app, &name) {
            // the selection belongs to the page shown: the rest waits for the new one
            let rest: Map<String, Value> = o
                .iter()
                .filter(|(k, _)| ["visual", "model", "ribbonTab", "buildTab"].contains(&k.as_str()))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            return Ok(Some((name, Value::Object(rest))));
        }
        wait_page = Some(name);
    }
    if let Some(v) = o.get("visual") {
        match v {
            Value::Null => app.cmd.visual = None,
            Value::String(_) | Value::Object(_) => {
                let (page, visual) = match v {
                    Value::String(s) => (None, s.as_str()),
                    _ => (
                        v.get("page").and_then(Value::as_str),
                        v.get("visual").and_then(Value::as_str).unwrap_or(""),
                    ),
                };
                let page = page
                    .map(str::to_string)
                    .or_else(|| wait_page.clone())
                    .or_else(|| app.page_names().get(app.active_page()).map(|x| x.0.clone()))
                    .ok_or("there is no page to select a visual on")?;
                if !visual_exists(app, &page, visual) {
                    return Err(format!(
                        "no visual `{}` on page `{}`",
                        crate::app::clip(visual, 80),
                        crate::app::clip(&page, 80)
                    ));
                }
                app.backstage = None;
                app.show_tab(View::Report);
                app.cmd.visual = Some(VisualSel {
                    page,
                    visual: visual.to_string(),
                });
            }
            _ => return Err("`visual` is a visual's name, {page, visual}, or null".into()),
        }
    }
    if let Some(m) = o.get("model") {
        app.model_view.sel = model_sel(m)?;
        app.backstage = None;
        app.show_tab(View::Model);
    }
    if let Some(t) = o.get("ribbonTab") {
        let rt = t
            .as_str()
            .and_then(|s| RTab::ALL.into_iter().find(|x| tab_name(*x) == s))
            .ok_or(
                "`ribbonTab` is file, home, report, model, data, analyze, view, visual, table or measure",
            )?;
        if rt != RTab::File && !ribbon::tabs(app).contains(&rt) {
            return Err(format!(
                "the {} tab is not shown now (a contextual tab needs its selection)",
                tab_name(rt)
            ));
        }
        app.select_ribbon_tab(rt);
    }
    if let Some(b) = o.get("buildTab") {
        match b.as_str() {
            Some("build") => app.panes.format.tab = crate::format_pane::Tab::Build,
            Some("format") => {
                if app.access.as_deref() == Some("reader") {
                    return Err("a reader has no Format visual tab".into());
                }
                crate::format_pane::open_format(app);
            }
            Some("analytics") => {
                if app.access.as_deref() == Some("reader") {
                    return Err("a reader has no Analytics tab".into());
                }
                crate::format_pane::open_format(app);
                app.panes.format.tab = crate::format_pane::Tab::Analytics;
            }
            _ => {
                return Err("`buildTab` is build, format or analytics (the Build pane's sub-tab)".into());
            }
        }
    }
    Ok(wait_page.map(|p| (p, json!({}))))
}

fn model_sel(m: &Value) -> Result<Option<Sel>, String> {
    if m.is_null() {
        return Ok(None);
    }
    let s = |k: &str| m.get(k).and_then(Value::as_str).map(str::to_string);
    if let Some(i) = m.get("relationship").and_then(Value::as_u64) {
        return Ok(Some(Sel::Rel(
            usize::try_from(i).map_err(|_| "a relationship index")?,
        )));
    }
    let table = s("table")
        .ok_or("`model` is {table}, {table, column}, {table, measure}, {relationship: index} or null")?;
    Ok(Some(match (s("column"), s("measure")) {
        (Some(name), None) => Sel::Column { table, name },
        (None, Some(name)) => Sel::Measure { table, name },
        (None, None) => Sel::Table(table),
        _ => return Err("`model` takes a column or a measure, not both".into()),
    }))
}

fn visual_exists(app: &App, page: &str, visual: &str) -> bool {
    let Some(l) = &app.loaded else { return false };
    l.snap
        .pointer("/report/pages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|p| p.get("name").and_then(Value::as_str) == Some(page))
        .flat_map(|p| {
            p.get("visuals")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .any(|v| v.get("name").and_then(Value::as_str) == Some(visual))
}

// --- forms ----------------------------------------------------------------------------------

fn form_set(app: &mut App, p: &Value) -> Result<(), String> {
    let Some(f) = &mut app.form else {
        return Err("no form is open (ui.form.open)".into());
    };
    if let Some(k) = p
        .as_object()
        .and_then(|o| o.keys().find(|k| !["fields", "json"].contains(&k.as_str())))
    {
        return Err(format!(
            "unknown key `{}` (ui.form.set takes fields, json)",
            crate::app::clip(k, 40)
        ));
    }
    if let Some(j) = p.get("json") {
        if f.extra.is_none() {
            return Err(format!("the form of {} has no JSON field", f.id));
        }
        f.extra = Some(match j {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        });
    }
    let fields = match p.get("fields") {
        None => return Ok(()),
        Some(Value::Object(o)) => o,
        Some(_) => return Err("`fields` is an object {name: value}".into()),
    };
    for (name, v) in fields {
        let id = f.id;
        let Some(input) = f.input_mut(name) else {
            return Err(format!(
                "the form of {id} has no field `{}`",
                crate::app::clip(name, 60)
            ));
        };
        match (&input.field.kind, v) {
            (Kind::Bool, Value::Bool(b)) => input.flag = Some(*b),
            (Kind::Bool, Value::Null) => input.flag = None,
            (Kind::Bool, _) => return Err(format!("`{name}` is true, false or null")),
            (_, Value::Null) => input.text.clear(),
            (_, Value::String(s)) => input.text = crate::app::clip(s, 1 << 20),
            (Kind::List | Kind::Paths, Value::Array(a)) if a.iter().all(Value::is_string) => {
                input.text = a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n");
            }
            (_, Value::Array(_) | Value::Object(_)) => {
                input.text = serde_json::to_string_pretty(v).unwrap_or_default();
            }
            (_, other) => input.text = other.to_string(),
        }
    }
    f.tried = false;
    Ok(())
}

fn form_state(app: &App) -> Value {
    let Some(f) = &app.form else { return Value::Null };
    let fields: Map<String, Value> = f
        .inputs
        .iter()
        .map(|i| {
            let v = match i.field.kind {
                Kind::Bool => i.flag.map_or(Value::Null, Value::Bool),
                _ => Value::String(crate::app::clip(&i.text, 2000)),
            };
            (i.field.name.to_string(), v)
        })
        .collect();
    let required: Vec<&str> = f
        .inputs
        .iter()
        .filter(|i| i.field.required)
        .map(|i| i.field.name)
        .collect();
    json!({
        "command": f.id,
        "fields": fields,
        "required": required,
        "json": f.extra,
        "running": f.running.is_some(),
        "result": match &f.result {
            None => Value::Null,
            Some(Ok(v)) => json!({"ok": true, "result": v}),
            Some(Err(e)) => json!({"ok": false, "error": e}),
        },
    })
}

// --- ui.close -------------------------------------------------------------------------------

fn close(app: &mut App, what: Option<&str>) -> Result<(), String> {
    let what = match what {
        Some(w) => w,
        None if app.form.is_some() => "form",
        None if app.palette.is_some() => "palette",
        None if app.cmd.result.is_some() => "answer",
        None if app.confirm.is_some() => "confirm",
        None if app.popup.is_some() => "popup",
        None if app.backstage.is_some() => "backstage",
        None => return Ok(()),
    };
    match what {
        "form" => app.form = None,
        "palette" => app.palette = None,
        "answer" => app.cmd.result = None,
        "confirm" => app.confirm = None,
        "popup" => app.popup = None,
        "backstage" => app.backstage = None,
        "all" => {
            app.form = None;
            app.palette = None;
            app.cmd.result = None;
            app.confirm = None;
            app.popup = None;
            app.backstage = None;
        }
        _ => return Err("`what` is form, palette, answer, confirm, popup, backstage or all".into()),
    }
    Ok(())
}

// --- ui.inspect -----------------------------------------------------------------------------

/// The registry tab's name in the protocol (`file`, `home`, â€¦ `measure`).
pub fn tab_name(t: RTab) -> &'static str {
    match t {
        RTab::File => "file",
        RTab::Home => "home",
        RTab::Insert => "insert",
        RTab::Report => "report",
        RTab::Model => "model",
        RTab::Data => "data",
        RTab::Analyze => "analyze",
        RTab::View => "view",
        RTab::Visual => "visual",
        RTab::Table => "table",
        RTab::Measure => "measure",
    }
}

fn page_name(p: backstage::Page) -> &'static str {
    match p {
        backstage::Page::Home => "home",
        backstage::Page::Open => "open",
        backstage::Page::Info => "info",
        backstage::Page::Export => "export",
        backstage::Page::Options => "options",
    }
}

/// The window's state, as `ui.inspect` returns it.
pub fn inspect(app: &App, ctx: &egui::Context) -> Value {
    let rect = ctx.content_rect();
    let (docs, active_doc) = app.doc_tabs();
    let pages = app.page_names();
    let snap = app.loaded.as_ref().map(|l| l.snap.as_ref());
    let model_sel = match &app.model_view.sel {
        None => Value::Null,
        Some(Sel::Table(t)) => json!({"table": t}),
        Some(Sel::Column { table, name }) => json!({"table": table, "column": name}),
        Some(Sel::Measure { table, name }) => json!({"table": table, "measure": name}),
        Some(Sel::Rel(i)) => json!({"relationship": i}),
        Some(Sel::Item(item)) => {
            use crate::model_view::Item;
            match item {
                Item::CalcItem { group, name } => {
                    json!({"calculation_group": group, "calculation_item": name})
                }
                Item::Hierarchy { table, name } => json!({"table": table, "hierarchy": name}),
                Item::Role(n) => json!({"role": n}),
                Item::Culture(n) => json!({"culture": n}),
                Item::Expression(n) => json!({"expression": n}),
                Item::DataSource(n) => json!({"data_source": n}),
            }
        }
    };
    json!({
        "title": app.window_title(),
        "shell": if app.caps().open_files { "desktop" } else { "browser" },
        "lang": match app.lang { crate::i18n::Lang::En => "en", crate::i18n::Lang::Fr => "fr" },
        "theme": app.prefs.theme.name(),
        "window": {"width": rect.width(), "height": rect.height(), "pixelsPerPoint": ctx.pixels_per_point()},
        "view": app.tab.name(),
        "ribbonTab": tab_name(app.ribbon_tab),
        "ribbonTabs": ribbon::tabs(app).into_iter().map(tab_name).collect::<Vec<_>>(),
        "ribbonCollapsed": app.prefs.ribbon_collapsed,
        "backstage": app.backstage.as_ref().map(|b| page_name(b.page)),
        "docs": docs.iter().map(|(label, unsaved)| json!({"label": label, "unsaved": unsaved})).collect::<Vec<_>>(),
        "activeDoc": active_doc,
        "loading": app.has_doc() && app.loaded.is_none(),
        "revision": snap.and_then(|s| s.get("revision")).cloned(),
        "dirty": snap.and_then(|s| s.get("dirty")).cloned(),
        "pages": pages.iter().map(|(n, l, h)| json!({"name": n, "label": l, "hidden": h})).collect::<Vec<_>>(),
        "page": pages.get(app.active_page()).map(|p| p.0.clone()),
        "visual": app.cmd.visual.as_ref().map(|v| json!({"page": v.page, "visual": v.visual})),
        "model": model_sel,
        "palette": app.palette.as_ref().map(|p| json!({
            "query": p.query,
            "rows": palette::rows(app, &p.query).iter().take(20).map(|r| r.id).collect::<Vec<_>>(),
        })),
        "form": form_state(app),
        "answer": app.cmd.result.as_ref().map(|(id, r)| json!({
            "command": id,
            "done": r.is_some(),
            "error": r.as_ref().and_then(|x| x.as_ref().err()),
        })),
        "confirm": app.confirm.is_some(),
        "popup": app.popup.is_some(),
        "access": app.access,
        "error": app.error,
        "status": app.status,
        "pendingCommands": app.calls_pending(),
        "idle": idle(app),
    })
}

// --- screenshots ----------------------------------------------------------------------------

fn shot_result(image: &egui::ColorImage) -> Result<Value, String> {
    let png = png(image)?;
    if png.len() > MAX_PNG_BYTES {
        return Err(format!("the screenshot is larger than {MAX_PNG_BYTES} bytes"));
    }
    Ok(json!({
        "width": image.size[0],
        "height": image.size[1],
        "png": base64(&png),
    }))
}

/// A frame as PNG (RGBA).
pub fn png(image: &egui::ColorImage) -> Result<Vec<u8>, String> {
    let [w, h] = image.size;
    let (w32, h32) = (
        u32::try_from(w).map_err(|_| "image too wide")?,
        u32::try_from(h).map_err(|_| "image too tall")?,
    );
    let size = tiny_skia::IntSize::from_wh(w32, h32).ok_or("an empty frame")?;
    let n = w
        .checked_mul(h)
        .filter(|n| *n == image.pixels.len())
        .ok_or("a damaged frame")?;
    let mut data = Vec::with_capacity(n.saturating_mul(4));
    // egui's colours are premultiplied, as tiny-skia's pixmaps
    for c in &image.pixels {
        data.extend_from_slice(&c.to_array());
    }
    let pm = tiny_skia::Pixmap::from_vec(data, size).ok_or("a damaged frame")?;
    pm.encode_png().map_err(|e| e.to_string())
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding (RFC 4648 Â§4).
pub fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3).saturating_mul(4));
    for chunk in bytes.chunks(3) {
        let b = [
            chunk.first().copied().unwrap_or(0),
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let sym = |shift: u32| char::from(B64.get(((n >> shift) & 63) as usize).copied().unwrap_or(b'A'));
        out.push(sym(18));
        out.push(sym(12));
        out.push(if chunk.len() > 1 { sym(6) } else { '=' });
        out.push(if chunk.len() > 2 { sym(0) } else { '=' });
    }
    out
}

/// Decodes standard base64 (padding optional; whitespace refused).
pub fn unbase64(s: &str) -> Option<Vec<u8>> {
    let s = s.trim_end_matches('=');
    let mut out = Vec::with_capacity(s.len().saturating_mul(3) / 4);
    let mut acc: u32 = 0;
    let mut bits = 0u32;
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
#[path = "control_tests.rs"]
mod tests;
