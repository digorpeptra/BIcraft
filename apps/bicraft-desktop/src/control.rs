//! The desktop's UI control transport (docs/desktop.md, "Control channel"): JSON lines on a
//! loopback socket, each request carrying the launch's random token. Off unless
//! `--control PORT` or `BICRAFT_CONTROL_PORT` asks for it.
//!
//! One request per line, one reply per line, in order (the Crafting Apps' shape,
//! `bicraft_ui::control`). The token is written to an owner-only file ([`Server::write_file`]),
//! never printed. A line that is not a request (an HTTP request, text that isn't JSON, invalid
//! UTF-8, a line past [`Limits::line_bytes`]) or carries a wrong token gets one error reply and
//! the connection is closed: nothing sent after it runs, so a web page's cross-origin `fetch` to
//! the port cannot smuggle a request in its body. Requests reach the window through the inbox
//! [`bicraft_ui::App::control`] reads between frames, and run there as the window's own clicks.
//!
//! Bounds: [`Limits`] (connections at once, requests per second per connection, line size,
//! idle time, write time, how long a reply is waited for) and [`bicraft_ui::control::MAX_QUEUED`]
//! requests queued for the window (more are answered `busy`).

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use bicraft_http::{Slot, random_token, same_token};
use bicraft_ui::control::{self, Job};
use serde_json::{Value, json};

/// Wakes the window when a request arrives (`egui::Context::request_repaint`).
pub type Notify = Arc<dyn Fn() + Send + Sync>;

/// The transport's bounds.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Connections served at once (more get an error line and are closed).
    pub connections: usize,
    /// Requests per second on one connection (more are answered `busy`).
    pub per_second: u32,
    /// Longest request line, its newline included.
    pub line_bytes: usize,
    /// A connection that sends nothing for this long is closed.
    pub idle: Duration,
    /// A reply the client does not read within this time closes the connection.
    pub write: Duration,
    /// How long a reply is waited for.
    pub reply: Duration,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            connections: 16,
            per_second: 200,
            line_bytes: control::MAX_REQUEST_BYTES,
            idle: Duration::from_secs(300),
            write: Duration::from_secs(30),
            reply: Duration::from_secs(control::REPLY_TIMEOUT_SECS),
        }
    }
}

struct Shared {
    token: String,
    jobs: SyncSender<Job>,
    notify: Notify,
    limits: Limits,
    open: AtomicUsize,
    stop: AtomicBool,
    /// Wrong tokens so far: each refusal waits a little longer (at most 2 s).
    failures: AtomicU32,
}

/// A running control channel. Dropping it stops listening and removes its token file.
pub struct Server {
    addr: SocketAddr,
    shared: Arc<Shared>,
    file: Mutex<Option<PathBuf>>,
    accept: Option<JoinHandle<()>>,
}

/// Starts the channel on `127.0.0.1:port` (0: a free port), with a new random token. Requests go
/// to `jobs`; `notify` wakes the window.
pub fn start(port: u16, jobs: SyncSender<Job>, notify: Notify, limits: Limits) -> io::Result<Server> {
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))?;
    listener.set_nonblocking(true)?;
    let addr = listener.local_addr()?;
    let shared = Arc::new(Shared {
        token: random_token(),
        jobs,
        notify,
        limits,
        open: AtomicUsize::new(0),
        stop: AtomicBool::new(false),
        failures: AtomicU32::new(0),
    });
    let s = Arc::clone(&shared);
    let accept = thread::Builder::new()
        .name("bicraft-control".into())
        .spawn(move || accept_loop(&listener, &s))?;
    Ok(Server {
        addr,
        shared,
        file: Mutex::new(None),
        accept: Some(accept),
    })
}

impl Server {
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The launch's token (tests; agents read the file).
    pub fn token(&self) -> &str {
        &self.shared.token
    }

    /// What the token file holds: `{"kind": "desktop", "addr", "token", "pid"}`.
    pub fn file_json(&self) -> Value {
        json!({
            "kind": "desktop",
            "addr": self.addr.to_string(),
            "token": self.shared.token,
            "pid": std::process::id(),
        })
    }

    /// Writes the token file at `path`, readable by its owner only (removed on drop).
    pub fn write_file(&self, path: &Path) -> io::Result<()> {
        let text = serde_json::to_string_pretty(&self.file_json()).map_err(io::Error::other)?;
        bicraft_http::write_owner_only(path, text.as_bytes())?;
        *self.file.lock().unwrap_or_else(|e| e.into_inner()) = Some(path.to_path_buf());
        Ok(())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        if let Some(a) = self.accept.take() {
            let _ = a.join();
        }
        let file = self.file.lock().unwrap_or_else(|e| e.into_inner()).take();
        // only our own file: a later launch may have written its token there since
        if let Some(path) = file
            && std::fs::read_to_string(&path).is_ok_and(|t| t.contains(&self.shared.token))
        {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// The token file's default place: next to the settings file (`%APPDATA%\BIcraft\control.json`,
/// `~/.config/bicraft/control.json`).
pub fn default_file() -> Option<PathBuf> {
    crate::prefs::default_path().map(|p| p.with_file_name("control.json"))
}

fn accept_loop(listener: &TcpListener, shared: &Arc<Shared>) {
    while !shared.stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _peer)) => {
                if stream.set_nonblocking(false).is_err() {
                    continue;
                }
                let _ = stream.set_nodelay(true);
                let s = Arc::clone(shared);
                let _ = thread::Builder::new()
                    .name("bicraft-control-conn".into())
                    .spawn(move || {
                        let Some(_slot) = Slot::acquire(&s.open, s.limits.connections) else {
                            let mut stream = stream;
                            let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
                            let _ = write_line(
                                &mut stream,
                                &control::err(
                                    &Value::Null,
                                    format!(
                                        "busy: {} connections at once; closing the connection",
                                        s.limits.connections
                                    ),
                                ),
                            );
                            return;
                        };
                        serve(stream, &s);
                    });
            }
            Err(_) => thread::sleep(bicraft_http::ACCEPT_POLL),
        }
    }
}

fn write_line(stream: &mut TcpStream, v: &Value) -> io::Result<()> {
    let mut text = v.to_string();
    text.push('\n');
    stream.write_all(text.as_bytes())?;
    stream.flush()
}

/// One connection: requests in order until the client closes, idles, or sends what is not a
/// request.
fn serve(stream: TcpStream, shared: &Shared) {
    let limits = shared.limits;
    if stream.set_read_timeout(Some(limits.idle)).is_err()
        || stream.set_write_timeout(Some(limits.write)).is_err()
    {
        return;
    }
    let Ok(read_half) = stream.try_clone() else { return };
    let mut out = stream;
    let mut reader = BufReader::with_capacity(64 * 1024, read_half);
    let mut window = (Instant::now(), 0u32);
    let cap = u64::try_from(limits.line_bytes).unwrap_or(u64::MAX);
    loop {
        if shared.stop.load(Ordering::SeqCst) {
            return;
        }
        let mut line = Vec::new();
        let read = (&mut reader).take(cap).read_until(b'\n', &mut line);
        match read {
            Ok(0) => return,
            Ok(_) => {}
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {
                let _ = write_line(
                    &mut out,
                    &json!({"ok": false, "error": "control connection idle; closing the connection", "closed": "idle"}),
                );
                return;
            }
            Err(_) => return,
        }
        if line.last() != Some(&b'\n') {
            if line.len() >= limits.line_bytes {
                let _ = write_line(
                    &mut out,
                    &control::err(
                        &Value::Null,
                        format!(
                            "request longer than {} bytes; closing the connection",
                            limits.line_bytes
                        ),
                    ),
                );
            }
            // an unfinished line at the end is never run
            return;
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let (request, token) = match control::parse(&line) {
            Ok(r) => r,
            Err(e) => {
                let _ = write_line(
                    &mut out,
                    &control::err(&Value::Null, format!("{e}; closing the connection")),
                );
                return;
            }
        };
        if !token.as_deref().is_some_and(|t| same_token(t, &shared.token)) {
            let n = shared.failures.fetch_add(1, Ordering::SeqCst).saturating_add(1);
            thread::sleep(Duration::from_millis(u64::from(n.min(20)) * 100));
            let _ = write_line(
                &mut out,
                &control::err(
                    &request.id,
                    "unauthorized: missing or wrong token; closing the connection",
                ),
            );
            return;
        }
        // requests per second on this connection
        if window.0.elapsed() >= Duration::from_secs(1) {
            window = (Instant::now(), 0);
        }
        window.1 = window.1.saturating_add(1);
        if window.1 > limits.per_second {
            if write_line(
                &mut out,
                &control::err(&request.id, "busy: too many requests; slow down"),
            )
            .is_err()
            {
                return;
            }
            continue;
        }
        let id = request.id.clone();
        let (job, ticket) = Job::new(request);
        let reply = match shared.jobs.try_send(job) {
            Ok(()) => {
                (shared.notify)();
                ticket.wait(limits.reply)
            }
            Err(TrySendError::Full(_)) => control::err(&id, "busy: the window has too many requests queued"),
            Err(TrySendError::Disconnected(_)) => control::err(&id, "the window is closed"),
        };
        if write_line(&mut out, &reply).is_err() {
            return;
        }
    }
}

#[cfg(test)]
#[path = "tests/control_tests.rs"]
mod tests;
