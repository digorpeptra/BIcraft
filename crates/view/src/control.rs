//! The UI control channel's relay to the browser app (ROADMAP M11.6, docs/webui.md "Control
//! channel"): a page cannot listen, so the server takes an agent's request and hands it to an app
//! page, which runs it as the desktop does (`bicraft_ui::control`) and posts the reply back.
//! `bicraft-cli view --control` uses it ([`crate::ViewOptions::control`]; off, the three routes
//! answer 404), and so does `bicraft-cli serve --viewer-control` (`crates/mcp`, one relay per
//! workspace, behind the server's authentication).
//!
//! - The agent's route (`POST /api/control` here): the request object `{"id", "method",
//!   "params"}` as the body and the control token ([`Ask::token`]: a bearer on `bicraft-cli view`);
//!   the answer is the reply object. Refused: no or a wrong token (401), a request from a web page
//!   (`Origin` or `Sec-Fetch-Site`: 403), more than [`Limits::per_second`] a second (429), a body
//!   that is not a JSON object with a string `method` (400, 413, 415, 411), no app page connected
//!   or [`Limits::max_queued`] already waiting (503). The reply is waited for
//!   [`Limits::reply_timeout`]; a request no page took by then is dropped (it never runs).
//! - The page's stream (`GET /api/control/events`): Server-Sent Events, `event: control` with
//!   `{"seq", "request"}` (and `"doc"`, the workspace file whose document tab the request runs
//!   in, when the agent addressed a document view). Each request goes to one page: the first
//!   connected stream of the page it is addressed to ([`To::page`]; `None`, any page).
//! - The page's reply (`POST /api/control/reply`): `{"seq", "reply"}`, after the transport checked
//!   the page's token and origin; at most [`MAX_REPLY_BYTES`]. Only the page that took a request
//!   may answer it.
//!
//! The control token is the agent's credential: it is written to an owner-only file and never
//! printed. The page token only lets the page answer requests it was given.

use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::net::TcpStream;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use bicraft_http::{COMMON_HEADERS, Slot, Status, same_token, write_response};
use serde_json::{Value, json};

/// The agent's route.
pub const ROUTE: &str = "/api/control";
/// The page's request stream.
pub const EVENTS_ROUTE: &str = "/api/control/events";
/// The page's replies.
pub const REPLY_ROUTE: &str = "/api/control/reply";
/// Largest request body.
pub const MAX_REQUEST_BYTES: usize = 1 << 20;
/// Largest reply body (a screenshot, base64).
pub const MAX_REPLY_BYTES: usize = 48 << 20;
/// Requests waiting for a page at most.
pub const MAX_QUEUED: usize = 64;
/// Agent requests per second at most.
pub const PER_SECOND: u32 = 200;
/// How long an agent waits for the page's reply.
pub const REPLY_TIMEOUT: Duration = Duration::from_secs(60);
/// Pages connected at once at most (`bicraft-cli view`).
const MAX_PAGES: usize = 8;
const KEEPALIVE: Duration = Duration::from_secs(15);

/// A relay's bounds (the defaults are `bicraft-cli view`'s; a server passes its own caps).
#[derive(Debug, Clone)]
pub struct Limits {
    /// Page streams at once.
    pub max_pages: usize,
    /// Requests waiting for a page.
    pub max_queued: usize,
    /// Agent requests per second.
    pub per_second: u32,
    /// Largest request body.
    pub max_request_bytes: usize,
    /// How long an agent waits for the page's reply.
    pub reply_timeout: Duration,
    /// Where the agent is told to open a page when none is connected.
    pub open_hint: &'static str,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_pages: MAX_PAGES,
            max_queued: MAX_QUEUED,
            per_second: PER_SECOND,
            max_request_bytes: MAX_REQUEST_BYTES,
            reply_timeout: REPLY_TIMEOUT,
            open_hint: "open this viewer's /app/ page",
        }
    }
}

/// Where an agent's request goes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct To {
    /// The page (its key, a server's page view id); `None`: any connected page.
    pub page: Option<String>,
    /// The workspace file whose document tab the page switches to first (a document view).
    pub doc: Option<String>,
}

#[derive(Clone)]
struct Item {
    seq: u64,
    page: Option<String>,
    data: String,
}

struct Waiter {
    tx: SyncSender<Value>,
    /// The page that took the request (`Some(None)`: a page without a key).
    taken: Option<Option<String>>,
}

/// The relay's state.
pub struct Relay {
    /// The agent's token.
    pub(crate) token: String,
    limits: Limits,
    queue: Mutex<VecDeque<Item>>,
    changed: Condvar,
    waiting: Mutex<HashMap<u64, Waiter>>,
    next: AtomicU64,
    pages: AtomicUsize,
    /// The keys of the connected page streams, oldest first.
    keys: Mutex<Vec<String>>,
    rate: Mutex<(Instant, u32)>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// What a route needs from the request.
pub struct Ask<'a> {
    /// The control token the agent presented (`bicraft-cli view`: `Authorization: Bearer`;
    /// `bicraft-cli serve`: `X-Bicraft-Control`, the bearer being the server's credential).
    pub token: Option<&'a str>,
    pub origin: Option<&'a str>,
    pub fetch_site: Option<&'a str>,
    pub content_type: Option<&'a str>,
    pub content_length: Option<usize>,
}

fn reply(stream: &mut TcpStream, status: Status, body: &Value) {
    let _ = write_response(
        stream,
        status,
        "application/json; charset=utf-8",
        &[("Cache-Control", "no-store")],
        body.to_string().as_bytes(),
    );
}

fn refuse(stream: &mut TcpStream, status: Status, id: &Value, error: &str) {
    reply(stream, status, &json!({"id": id, "ok": false, "error": error}));
}

/// [`refuse`], for the agent's route: the request failed.
fn refused(stream: &mut TcpStream, status: Status, id: &Value, error: &str) -> bool {
    refuse(stream, status, id, error);
    false
}

fn json_type(t: Option<&str>) -> bool {
    t.is_some_and(|t| {
        t.split(';')
            .next()
            .is_some_and(|m| m.trim().eq_ignore_ascii_case("application/json"))
    })
}

/// Keeps a connected page stream's key listed while it lives.
struct Connected<'a> {
    relay: &'a Relay,
    key: Option<String>,
}

impl Drop for Connected<'_> {
    fn drop(&mut self) {
        if let Some(k) = &self.key {
            let mut keys = lock(&self.relay.keys);
            if let Some(i) = keys.iter().rposition(|x| x == k) {
                keys.remove(i);
            }
        }
    }
}

impl Relay {
    /// A relay with `bicraft-cli view`'s bounds.
    pub fn new(token: String) -> Relay {
        Relay::with_limits(token, Limits::default())
    }

    /// A relay with its own bounds (a server's caps).
    pub fn with_limits(token: String, limits: Limits) -> Relay {
        Relay {
            token,
            limits,
            queue: Mutex::new(VecDeque::new()),
            changed: Condvar::new(),
            waiting: Mutex::new(HashMap::new()),
            next: AtomicU64::new(0),
            pages: AtomicUsize::new(0),
            keys: Mutex::new(Vec::new()),
            rate: Mutex::new((Instant::now(), 0)),
        }
    }

    /// Wakes the page streams (the server stops).
    pub fn wake(&self) {
        let _q = lock(&self.queue);
        self.changed.notify_all();
    }

    /// App pages connected now.
    pub fn pages(&self) -> usize {
        self.pages.load(Ordering::SeqCst)
    }

    /// The keys of the pages connected now, the most recently connected last.
    pub fn connected(&self) -> Vec<String> {
        lock(&self.keys).clone()
    }

    /// The agent's route: an agent's request, answered with the page's reply. Returns whether the
    /// page answered it without an error (`false`: refused, timed out, or the page's reply says
    /// `"ok": false`), for the server's metrics.
    pub fn agent(
        &self,
        stream: &mut TcpStream,
        ask: &Ask<'_>,
        rest: Vec<u8>,
        deadline: Duration,
        to: &To,
    ) -> bool {
        let none = Value::Null;
        // a web page cannot use this route, whatever it holds
        if ask.origin.is_some() || ask.fetch_site.is_some() {
            return refused(
                stream,
                Status::Forbidden,
                &none,
                "the control route is not for web pages",
            );
        }
        if !ask
            .token
            .map(str::trim)
            .is_some_and(|t| same_token(t, &self.token))
        {
            return refused(
                stream,
                Status::Unauthorized,
                &none,
                "unauthorized: missing or wrong control token",
            );
        }
        {
            let mut r = lock(&self.rate);
            if r.0.elapsed() >= Duration::from_secs(1) {
                *r = (Instant::now(), 0);
            }
            r.1 = r.1.saturating_add(1);
            if r.1 > self.limits.per_second {
                drop(r);
                return refused(
                    stream,
                    Status::TooManyRequests,
                    &none,
                    "busy: too many requests; slow down",
                );
            }
        }
        if !json_type(ask.content_type) {
            return refused(
                stream,
                Status::UnsupportedMedia,
                &none,
                "the body must be application/json",
            );
        }
        let Some(length) = ask.content_length else {
            return refused(
                stream,
                Status::LengthRequired,
                &none,
                "Content-Length is required",
            );
        };
        if length > self.limits.max_request_bytes {
            return refused(stream, Status::PayloadTooLarge, &none, "the request is too large");
        }
        let body = match bicraft_http::read_body(stream, rest, length, deadline) {
            Ok(b) => b,
            Err(status) => {
                bicraft_http::respond_status(stream, status, &[]);
                return false;
            }
        };
        let mut request = match serde_json::from_slice::<Value>(&body) {
            Ok(v @ Value::Object(_)) if v.get("method").is_some_and(Value::is_string) => v,
            _ => {
                return refused(
                    stream,
                    Status::BadRequest,
                    &none,
                    "the body must be a JSON object with a string `method`",
                );
            }
        };
        // the page never sees the agent's token
        if let Some(o) = request.as_object_mut() {
            o.remove("token");
        }
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let connected = match &to.page {
            None => self.pages() > 0,
            Some(k) => lock(&self.keys).contains(k),
        };
        if !connected {
            let why = match &to.page {
                None => format!("no app page is connected: {}", self.limits.open_hint),
                Some(_) => "the page of this view is not connected (closed, or reloading)".to_string(),
            };
            return refused(stream, Status::Unavailable, &id, &why);
        }
        let seq = self.next.fetch_add(1, Ordering::SeqCst).saturating_add(1);
        let mut data = format!("{{\"seq\":{seq},\"request\":{request}");
        if let Some(doc) = &to.doc {
            data.push_str(&format!(",\"doc\":{}", Value::String(doc.clone())));
        }
        data.push('}');
        let (tx, rx) = sync_channel(1);
        {
            let mut q = lock(&self.queue);
            if q.len() >= self.limits.max_queued {
                drop(q);
                return refused(stream, Status::Unavailable, &id, "busy: too many requests queued");
            }
            lock(&self.waiting).insert(seq, Waiter { tx, taken: None });
            q.push_back(Item {
                seq,
                page: to.page.clone(),
                data,
            });
            self.changed.notify_all();
        }
        let answer = rx.recv_timeout(self.limits.reply_timeout);
        lock(&self.waiting).remove(&seq);
        match answer {
            Ok(v) => {
                reply(stream, Status::Ok, &v);
                v.get("ok") != Some(&Value::Bool(false))
            }
            Err(_) => {
                let mut q = lock(&self.queue);
                let queued = q.iter().position(|i| i.seq == seq);
                if let Some(i) = queued {
                    q.remove(i);
                }
                drop(q);
                let why = if queued.is_some() {
                    "timeout: no page took the request (it will not run)"
                } else {
                    "timeout: the page did not answer; the request had started, its outcome is unknown"
                };
                refused(stream, Status::Unavailable, &id, why)
            }
        }
    }

    /// The page's stream of requests until it goes or `stopped`: the requests addressed to
    /// `key` (`None`: the ones addressed to no page in particular). The caller made the request's
    /// checks; this writes the stream's head.
    pub fn events(&self, stream: &mut TcpStream, key: Option<&str>, stopped: &dyn Fn() -> bool) {
        let Some(_page) = Slot::acquire(&self.pages, self.limits.max_pages.max(1)) else {
            return bicraft_http::respond_status(stream, Status::Unavailable, &[("Retry-After", "5")]);
        };
        if let Some(k) = key {
            lock(&self.keys).push(k.to_string());
        }
        let _connected = Connected {
            relay: self,
            key: key.map(str::to_string),
        };
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream; charset=utf-8\r\nCache-Control: no-store\r\n\
             X-Accel-Buffering: no\r\n{COMMON_HEADERS}\r\nretry: 1000\n\n"
        );
        if stream.write_all(head.as_bytes()).is_err() {
            return;
        }
        let mine = |i: &Item| i.page.as_deref() == key;
        let mut last = Instant::now();
        loop {
            let next = {
                let mut q = lock(&self.queue);
                loop {
                    if stopped() {
                        return;
                    }
                    if let Some(at) = q.iter().position(mine) {
                        break q.remove(at);
                    }
                    let Some(left) = KEEPALIVE.checked_sub(last.elapsed()).filter(|d| !d.is_zero()) else {
                        break None;
                    };
                    q = match self.changed.wait_timeout(q, left) {
                        Ok((g, _)) => g,
                        Err(p) => p.into_inner().0,
                    };
                }
            };
            // a page that went without a word (a closed tab) leaves the request to another one
            if let Some(item) = &next
                && bicraft_http::peer_closed(stream, Duration::from_millis(1))
            {
                self.put_back(item.clone());
                return;
            }
            let message = match &next {
                Some(item) => {
                    if let Some(w) = lock(&self.waiting).get_mut(&item.seq) {
                        w.taken = Some(key.map(str::to_string));
                    }
                    format!("event: control\ndata: {}\n\n", item.data)
                }
                None => ": keepalive\n\n".to_string(),
            };
            if stream.write_all(message.as_bytes()).is_err() || stream.flush().is_err() {
                // the page went: another one takes the request
                if let Some(item) = next {
                    self.put_back(item);
                }
                return;
            }
            last = Instant::now();
        }
    }

    /// A request a page did not get goes back to the head of the queue.
    fn put_back(&self, item: Item) {
        if let Some(w) = lock(&self.waiting).get_mut(&item.seq) {
            w.taken = None;
        }
        lock(&self.queue).push_front(item);
        self.changed.notify_all();
    }

    /// The page's reply to request `seq`, from page `key` (its stream's key). The caller
    /// checked the page token and the origin.
    pub fn page_reply(
        &self,
        stream: &mut TcpStream,
        ask: &Ask<'_>,
        rest: Vec<u8>,
        deadline: Duration,
        key: Option<&str>,
    ) {
        let none = Value::Null;
        if !json_type(ask.content_type) {
            return refuse(
                stream,
                Status::UnsupportedMedia,
                &none,
                "the body must be application/json",
            );
        }
        let Some(length) = ask.content_length else {
            return refuse(
                stream,
                Status::LengthRequired,
                &none,
                "Content-Length is required",
            );
        };
        if length > MAX_REPLY_BYTES {
            return refuse(stream, Status::PayloadTooLarge, &none, "the reply is too large");
        }
        let body = match bicraft_http::read_body(stream, rest, length, deadline) {
            Ok(b) => b,
            Err(status) => return bicraft_http::respond_status(stream, status, &[]),
        };
        let v: Value = match serde_json::from_slice(&body) {
            Ok(v @ Value::Object(_)) => v,
            _ => return refuse(stream, Status::BadRequest, &none, "the body must be {seq, reply}"),
        };
        let (Some(seq), Some(answer)) = (v.get("seq").and_then(Value::as_u64), v.get("reply")) else {
            return refuse(stream, Status::BadRequest, &none, "the body must be {seq, reply}");
        };
        let waiter = {
            let mut waiting = lock(&self.waiting);
            // only the page the request went to answers it
            let theirs = waiting
                .get(&seq)
                .is_some_and(|w| w.taken.as_ref().is_some_and(|t| t.as_deref() == key));
            if theirs { waiting.remove(&seq) } else { None }
        };
        match waiter {
            Some(w) => {
                let _ = w.tx.try_send(answer.clone());
                reply(stream, Status::Ok, &json!({"ok": true}));
            }
            None => refuse(stream, Status::NotFound, &none, "no request waits for this reply"),
        }
    }
}
