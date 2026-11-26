//! L0: the hardened `std` HTTP/1.1 plumbing shared by the live viewer (`bicraft-view`) and the
//! MCP Streamable HTTP endpoint (`bicraft-mcp`).
//!
//! No async runtime and no dependency: a non-blocking [`TcpListener`] polled by one accept thread
//! ([`accept_loop`]), one short-lived thread per connection under a global and a per-IP cap
//! ([`Connections`]), and bounded parsing:
//!
//! - the request head (request line + headers) is capped in bytes, header count and target
//!   length, and must arrive within a deadline ([`Limits`], [`read_request`]);
//! - bodies come with a `Content-Length` only (`Transfer-Encoding` is refused with 411) and are
//!   read within their own deadline ([`read_body`]);
//! - headers the caller relies on can be declared unique: a repeated one is a 400 (request
//!   smuggling and header confusion);
//! - after the response, a bounded drain keeps the close from turning into a reset ([`finish`]).
//!
//! Every response says `Connection: close`: one request per connection.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------------------------
// Status lines and responses

/// The statuses the servers answer with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    Created,
    Accepted,
    NoContent,
    SeeOther,
    NotModified,
    PermanentRedirect,
    BadRequest,
    Unauthorized,
    Forbidden,
    NotFound,
    MethodNotAllowed,
    NotAcceptable,
    Conflict,
    Timeout,
    LengthRequired,
    PreconditionFailed,
    PayloadTooLarge,
    UnsupportedMedia,
    UriTooLong,
    HeadersTooLarge,
    TooManyRequests,
    InternalError,
    Unavailable,
    VersionNotSupported,
    InsufficientStorage,
}

impl Status {
    /// The status line's code and reason, e.g. `"404 Not Found"`.
    pub fn line(self) -> &'static str {
        match self {
            Status::Ok => "200 OK",
            Status::Created => "201 Created",
            Status::Accepted => "202 Accepted",
            Status::NoContent => "204 No Content",
            Status::SeeOther => "303 See Other",
            Status::NotModified => "304 Not Modified",
            Status::PermanentRedirect => "308 Permanent Redirect",
            Status::BadRequest => "400 Bad Request",
            Status::Unauthorized => "401 Unauthorized",
            Status::Forbidden => "403 Forbidden",
            Status::NotFound => "404 Not Found",
            Status::MethodNotAllowed => "405 Method Not Allowed",
            Status::NotAcceptable => "406 Not Acceptable",
            Status::Conflict => "409 Conflict",
            Status::Timeout => "408 Request Timeout",
            Status::LengthRequired => "411 Length Required",
            Status::PreconditionFailed => "412 Precondition Failed",
            Status::PayloadTooLarge => "413 Content Too Large",
            Status::UnsupportedMedia => "415 Unsupported Media Type",
            Status::UriTooLong => "414 URI Too Long",
            Status::TooManyRequests => "429 Too Many Requests",
            Status::HeadersTooLarge => "431 Request Header Fields Too Large",
            Status::InternalError => "500 Internal Server Error",
            Status::Unavailable => "503 Service Unavailable",
            Status::VersionNotSupported => "505 HTTP Version Not Supported",
            Status::InsufficientStorage => "507 Insufficient Storage",
        }
    }

    /// The numeric code.
    pub fn code(self) -> u16 {
        self.line().get(..3).and_then(|c| c.parse().ok()).unwrap_or(500)
    }
}

// ---------------------------------------------------------------------------------------------
// Response notes (metrics and request logs of `bicraft serve`)

thread_local! {
    /// The status of the first response head written on this thread since the last
    /// [`take_response_note`], and the bytes written or declared since then.
    static NOTE: std::cell::Cell<(u16, u64)> = const { std::cell::Cell::new((0, 0)) };
}

/// Records a response head (`status`, the first one wins) and `bytes` sent on this thread. Every
/// writer of this crate calls it; a caller that writes body bytes itself adds them with
/// [`note_bytes`]. A connection is served by one thread, so the note is that connection's.
pub fn note_response(status: u16, bytes: u64) {
    NOTE.with(|n| {
        let (s, b) = n.get();
        n.set((if s == 0 { status } else { s }, b.saturating_add(bytes)));
    });
}

/// Adds `bytes` written by the caller (SSE events, keepalives) to this thread's note.
pub fn note_bytes(bytes: u64) {
    NOTE.with(|n| {
        let (s, b) = n.get();
        n.set((s, b.saturating_add(bytes)));
    });
}

/// This thread's note (status 0: no response head was written), reset.
pub fn take_response_note() -> (u16, u64) {
    NOTE.with(|n| n.replace((0, 0)))
}

fn len_u64(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

/// Headers sent on every response.
pub const COMMON_HEADERS: &str =
    "X-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n";

/// Writes a whole response: status line, `Content-Type`, `Content-Length`, [`COMMON_HEADERS`],
/// `extra`, then `body`.
pub fn write_response(
    stream: &mut impl Write,
    status: Status,
    content_type: &str,
    extra: &[(&str, &str)],
    body: &[u8],
) -> io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n{COMMON_HEADERS}",
        status.line(),
        body.len()
    );
    push_headers(&mut head, extra);
    head.push_str("\r\n");
    note_response(
        status.code(),
        len_u64(head.len()).saturating_add(len_u64(body.len())),
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

/// A response without a body or a `Content-Type` (`202 Accepted` to a notification).
pub fn write_empty(stream: &mut impl Write, status: Status, extra: &[(&str, &str)]) -> io::Result<()> {
    // a 204 carries no Content-Length at all (RFC 9110 §8.6)
    let length = if status == Status::NoContent {
        ""
    } else {
        "Content-Length: 0\r\n"
    };
    let mut head = format!("HTTP/1.1 {}\r\n{length}{COMMON_HEADERS}", status.line());
    push_headers(&mut head, extra);
    head.push_str("\r\n");
    note_response(status.code(), len_u64(head.len()));
    stream.write_all(head.as_bytes())?;
    stream.flush()
}

/// The head of a response whose body of `length` bytes the caller streams next (a download):
/// status line, `Content-Type`, `Content-Length`, [`COMMON_HEADERS`] and `extra`.
pub fn write_head(
    stream: &mut impl Write,
    status: Status,
    content_type: &str,
    length: u64,
    extra: &[(&str, &str)],
) -> io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {}\r\nContent-Type: {content_type}\r\nContent-Length: {length}\r\n{COMMON_HEADERS}",
        status.line()
    );
    push_headers(&mut head, extra);
    head.push_str("\r\n");
    // the body the caller streams next counts as declared (an estimate: a HEAD answer or an
    // interrupted download sends less)
    note_response(status.code(), len_u64(head.len()).saturating_add(length));
    stream.write_all(head.as_bytes())?;
    stream.flush()
}

/// The head of an open-ended Server-Sent Events response (no `Content-Length`: the stream ends
/// when the connection closes). Proxies are asked not to buffer it (`X-Accel-Buffering: no`).
pub fn write_sse_head(stream: &mut impl Write, extra: &[(&str, &str)]) -> io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream; charset=utf-8\r\nCache-Control: no-store\r\n\
         X-Accel-Buffering: no\r\n{COMMON_HEADERS}"
    );
    push_headers(&mut head, extra);
    head.push_str("\r\n");
    note_response(200, len_u64(head.len()));
    stream.write_all(head.as_bytes())?;
    stream.flush()
}

/// One SSE event carrying `data` (which must not contain a newline: serialized JSON never does)
/// with an optional `event:` name.
pub fn sse_event(event: Option<&str>, data: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len().saturating_add(32));
    if let Some(e) = event {
        out.extend_from_slice(b"event: ");
        out.extend_from_slice(e.as_bytes());
        out.push(b'\n');
    }
    out.extend_from_slice(b"data: ");
    out.extend_from_slice(data.as_bytes());
    out.extend_from_slice(b"\n\n");
    out
}

fn push_headers(head: &mut String, extra: &[(&str, &str)]) {
    for (name, value) in extra {
        // a header name or value can never smuggle a line break into the head
        if name
            .bytes()
            .chain(value.bytes())
            .any(|b| b == b'\r' || b == b'\n')
        {
            continue;
        }
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
}

/// A plain-text response whose body is the status line.
pub fn respond_status(stream: &mut impl Write, status: Status, extra: &[(&str, &str)]) {
    let body = format!("{}\n", status.line());
    let _ = write_response(
        stream,
        status,
        "text/plain; charset=utf-8",
        extra,
        body.as_bytes(),
    );
}

// ---------------------------------------------------------------------------------------------
// Request heads

/// Bounds on a request head.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Request line + headers, in bytes.
    pub max_head_bytes: usize,
    pub max_headers: usize,
    pub max_target_len: usize,
    /// The whole head must arrive within this time (slow-loris protection).
    pub head_deadline: Duration,
}

impl Default for Limits {
    /// The viewer's limits: 8 KiB, 64 headers, 2 KiB targets, 5 s.
    fn default() -> Self {
        Self {
            max_head_bytes: 8 * 1024,
            max_headers: 64,
            max_target_len: 2048,
            head_deadline: Duration::from_secs(5),
        }
    }
}

/// A parsed request head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    pub method: String,
    /// The target without its query string or fragment.
    pub path: String,
    /// The query string (after `?`, before any `#`), when the target has one.
    pub query: Option<String>,
    /// Every header, in order, values trimmed.
    pub headers: Vec<(String, String)>,
    /// `Content-Length`, when sent (always unique and decimal).
    pub content_length: Option<usize>,
}

impl Head {
    /// The first value of header `name` (case-insensitive).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// The raw value of query parameter `name` (not percent-decoded; the first one wins).
    pub fn query_param(&self, name: &str) -> Option<&str> {
        self.query
            .as_deref()?
            .split('&')
            .find_map(|kv| match kv.split_once('=') {
                Some((k, v)) if k == name => Some(v),
                None if kv == name => Some(""),
                _ => None,
            })
    }

    /// Every value of header `name`, in order.
    pub fn headers_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.headers
            .iter()
            .filter(move |(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Why a request head could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadError {
    /// Answer with this status, then close.
    Respond(Status),
    /// Close silently (peer went away or an I/O error).
    Close,
}

/// Reads the request head within `limits`, and returns it with the bytes read past it (the start
/// of a body). Headers named in `unique` may appear at most once.
pub fn read_request(
    stream: &mut TcpStream,
    limits: &Limits,
    unique: &[&str],
) -> Result<(Head, Vec<u8>), ReadError> {
    let deadline = Instant::now()
        .checked_add(limits.head_deadline)
        .unwrap_or_else(Instant::now);
    let mut buf: Vec<u8> = Vec::with_capacity(1024);
    let mut chunk = [0u8; 2048];
    loop {
        if let Some(end) = head_end(&buf) {
            let rest = buf.get(end..).unwrap_or_default().to_vec();
            return parse_head(buf.get(..end).unwrap_or_default(), limits, unique).map(|r| (r, rest));
        }
        let room = limits.max_head_bytes.saturating_sub(buf.len());
        if room == 0 {
            return Err(ReadError::Respond(Status::HeadersTooLarge));
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(ReadError::Respond(Status::Timeout));
        }
        if stream.set_read_timeout(Some(left)).is_err() {
            return Err(ReadError::Close);
        }
        let want = room.min(chunk.len());
        match stream.read(chunk.get_mut(..want).unwrap_or_default()) {
            Ok(0) => return Err(ReadError::Close),
            Ok(n) => buf.extend_from_slice(chunk.get(..n).unwrap_or_default()),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {
                return Err(ReadError::Respond(Status::Timeout));
            }
            Err(_) => return Err(ReadError::Close),
        }
    }
}

/// Length of the request head including the blank line (`\r\n\r\n` or a lenient `\n\n`).
pub fn head_end(buf: &[u8]) -> Option<usize> {
    let mut i = 0usize;
    while let Some(&byte) = buf.get(i) {
        if byte == b'\n' {
            match (buf.get(i.saturating_add(1)), buf.get(i.saturating_add(2))) {
                (Some(b'\n'), _) => return i.checked_add(2),
                (Some(b'\r'), Some(b'\n')) => return i.checked_add(3),
                _ => {}
            }
        }
        i = i.saturating_add(1);
    }
    None
}

/// An RFC 9110 `token` (method and header names).
pub fn is_token(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

/// Parses a request head (see [`read_request`]).
pub fn parse_head(head: &[u8], limits: &Limits, unique: &[&str]) -> Result<Head, ReadError> {
    let bad = ReadError::Respond(Status::BadRequest);
    let text = std::str::from_utf8(head).map_err(|_| bad)?;
    let mut lines = text
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line));
    let request_line = lines.next().ok_or(bad)?;
    let mut parts = request_line.split(' ');
    let (Some(method), Some(target), Some(version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(bad);
    };
    if !is_token(method) {
        return Err(bad);
    }
    if !version.starts_with("HTTP/") {
        return Err(bad);
    }
    if version != "HTTP/1.1" && version != "HTTP/1.0" {
        return Err(ReadError::Respond(Status::VersionNotSupported));
    }
    if target.len() > limits.max_target_len {
        return Err(ReadError::Respond(Status::UriTooLong));
    }
    if !target.starts_with('/') || target.bytes().any(|b| b <= b' ' || b == 0x7f) {
        return Err(bad);
    }
    let path = target.split(['?', '#']).next().unwrap_or("/").to_string();
    let query = target
        .split('#')
        .next()
        .and_then(|t| t.split_once('?'))
        .map(|(_, q)| q.to_string());

    let mut headers: Vec<(String, String)> = Vec::new();
    let mut content_length = None;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if headers.len() >= limits.max_headers {
            return Err(ReadError::Respond(Status::HeadersTooLarge));
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(bad);
        };
        if !is_token(name) {
            return Err(bad);
        }
        if name.eq_ignore_ascii_case("transfer-encoding") {
            // bodies come with a Content-Length only (no chunked encoding)
            return Err(ReadError::Respond(Status::LengthRequired));
        }
        let value = value.trim();
        let once = name.eq_ignore_ascii_case("content-length")
            || unique.iter().any(|u| u.eq_ignore_ascii_case(name));
        if once && headers.iter().any(|(n, _)| n.eq_ignore_ascii_case(name)) {
            return Err(bad);
        }
        if name.eq_ignore_ascii_case("content-length") {
            if value.is_empty() || value.len() > 12 || !value.bytes().all(|b| b.is_ascii_digit()) {
                return Err(bad);
            }
            content_length = Some(value.parse::<usize>().map_err(|_| bad)?);
        }
        headers.push((name.to_string(), value.to_string()));
    }
    Ok(Head {
        method: method.to_string(),
        path,
        query,
        headers,
        content_length,
    })
}

// ---------------------------------------------------------------------------------------------
// Bodies and closing

/// The request body: the bytes read with the head, then the rest up to `length`, within
/// `deadline`. More bytes than `length` already read is a 400 (pipelining is not supported).
pub fn read_body(
    stream: &mut TcpStream,
    mut body: Vec<u8>,
    length: usize,
    deadline: Duration,
) -> Result<Vec<u8>, Status> {
    if body.len() > length {
        return Err(Status::BadRequest);
    }
    body.reserve(length.saturating_sub(body.len()));
    let deadline = Instant::now().checked_add(deadline).unwrap_or_else(Instant::now);
    let mut chunk = [0u8; 16 * 1024];
    while body.len() < length {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() || stream.set_read_timeout(Some(left)).is_err() {
            return Err(Status::Timeout);
        }
        let want = length.saturating_sub(body.len()).min(chunk.len());
        match stream.read(chunk.get_mut(..want).unwrap_or_default()) {
            Ok(0) => return Err(Status::BadRequest),
            Ok(n) => body.extend_from_slice(chunk.get(..n).unwrap_or_default()),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return Err(Status::Timeout),
        }
    }
    Ok(body)
}

/// Time limits of a streamed body ([`copy_body`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamLimits {
    /// The longest wait for the next bytes (a stalled client).
    pub idle: Duration,
    /// The whole body must arrive within this time.
    pub deadline: Duration,
}

/// Why a streamed body could not be copied.
#[derive(Debug)]
pub enum CopyError {
    /// The client sent too little, too slowly, or more than announced: answer this status.
    Client(Status),
    /// Writing to the sink failed (a full disk, for example).
    Sink(io::Error),
    /// `stop` was raised (the server is shutting down).
    Stopped,
}

/// Streams a request body of exactly `length` bytes to `sink`, without holding more than one
/// 64 KiB chunk in memory: first the bytes read with the head (`rest`), then the socket. Each
/// read waits at most `limits.idle`, the whole body at most `limits.deadline`; `stop` is checked
/// between chunks. Returns the number of bytes copied (`length`).
pub fn copy_body(
    stream: &mut TcpStream,
    rest: &[u8],
    length: u64,
    limits: StreamLimits,
    stop: &AtomicBool,
    sink: &mut impl Write,
) -> Result<u64, CopyError> {
    let rest_len = u64::try_from(rest.len()).unwrap_or(u64::MAX);
    if rest_len > length {
        // more than announced: pipelining is not supported
        return Err(CopyError::Client(Status::BadRequest));
    }
    sink.write_all(rest).map_err(CopyError::Sink)?;
    let mut done = rest_len;
    let deadline = Instant::now()
        .checked_add(limits.deadline)
        .unwrap_or_else(Instant::now);
    let mut chunk = vec![0u8; 64 * 1024];
    while done < length {
        if stop.load(Ordering::SeqCst) {
            return Err(CopyError::Stopped);
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(CopyError::Client(Status::Timeout));
        }
        let wait = left.min(limits.idle).max(Duration::from_millis(1));
        if stream.set_read_timeout(Some(wait)).is_err() {
            return Err(CopyError::Client(Status::Timeout));
        }
        let want = usize::try_from(length.saturating_sub(done))
            .unwrap_or(usize::MAX)
            .min(chunk.len());
        match stream.read(chunk.get_mut(..want).unwrap_or_default()) {
            Ok(0) => return Err(CopyError::Client(Status::BadRequest)),
            Ok(n) => {
                sink.write_all(chunk.get(..n).unwrap_or_default())
                    .map_err(CopyError::Sink)?;
                done = done.saturating_add(u64::try_from(n).unwrap_or(u64::MAX));
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return Err(CopyError::Client(Status::Timeout)),
        }
    }
    Ok(done)
}

/// After answering, read and discard at most this much leftover input before closing.
const DRAIN_LIMIT: usize = 64 * 1024;
const DRAIN_TIMEOUT: Duration = Duration::from_millis(200);

/// Half-closes, then discards a bounded amount of leftover input so the close does not turn into
/// a reset that would destroy the response in flight.
pub fn finish(mut stream: TcpStream) {
    let _ = stream.flush();
    let _ = stream.shutdown(Shutdown::Write);
    if stream.set_read_timeout(Some(DRAIN_TIMEOUT)).is_err() {
        return;
    }
    let deadline = Instant::now()
        .checked_add(DRAIN_TIMEOUT)
        .unwrap_or_else(Instant::now);
    let mut sink = [0u8; 4096];
    let mut drained = 0usize;
    while drained < DRAIN_LIMIT && Instant::now() < deadline {
        match stream.read(&mut sink) {
            Ok(0) | Err(_) => break,
            Ok(n) => drained = drained.saturating_add(n),
        }
    }
}

/// Whether the peer has closed its side of the connection, checked without consuming input and
/// without blocking for more than `wait`. Data waiting to be read counts as open.
pub fn peer_closed(stream: &TcpStream, wait: Duration) -> bool {
    if stream
        .set_read_timeout(Some(wait.max(Duration::from_millis(1))))
        .is_err()
    {
        return true;
    }
    let mut probe = [0u8; 1];
    match stream.peek(&mut probe) {
        Ok(0) => true,
        Ok(_) => false,
        Err(e) => !matches!(
            e.kind(),
            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
        ),
    }
}

// ---------------------------------------------------------------------------------------------
// Hosts and tokens

/// A decimal port number.
pub fn valid_port(port: &str) -> bool {
    !port.is_empty()
        && port.len() <= 5
        && port.bytes().all(|b| b.is_ascii_digit())
        && port.parse::<u16>().is_ok()
}

/// Accepts `localhost`, `*.localhost`, `127.x.x.x` and `[::1]`, with an optional port: the
/// `Host` values a loopback server answers (DNS-rebinding protection).
pub fn host_allowed(host: &str) -> bool {
    let host = host.trim();
    if let Some(rest) = host.strip_prefix('[') {
        let Some((ip, after)) = rest.split_once(']') else {
            return false;
        };
        if !(after.is_empty() || after.strip_prefix(':').is_some_and(valid_port)) {
            return false;
        }
        return ip.parse::<Ipv6Addr>().is_ok_and(|ip| ip.is_loopback());
    }
    let name = match host.rsplit_once(':') {
        Some((name, port)) => {
            if !valid_port(port) {
                return false;
            }
            name
        }
        None => host,
    };
    let name = name.to_ascii_lowercase();
    let name = name.strip_suffix('.').unwrap_or(&name);
    if name == "localhost" || name.ends_with(".localhost") {
        return !name.starts_with('.');
    }
    name.parse::<Ipv4Addr>().is_ok_and(|ip| ip.is_loopback())
}

/// 128 random bits as 32 lowercase hex digits. `RandomState` keys are seeded from the OS random
/// generator; hashing a counter, the time and the process id under fresh keys gives tokens other
/// local web pages cannot guess (no new dependency).
pub fn random_token() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher as _, Hasher as _};
    use std::sync::atomic::AtomicU64;
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let mut out = String::with_capacity(32);
    for i in 0..2u8 {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u128(nanos);
        hasher.write_u64(count);
        hasher.write_u32(std::process::id());
        hasher.write_u8(i);
        out.push_str(&format!("{:016x}", hasher.finish()));
    }
    out
}

/// Compares in time independent of where the strings differ.
pub fn same_token(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Writes a file only its owner may read (the UI control channel's token file): any old file at
/// `path` is removed first and the new one is created, never opened through a link someone left
/// there. On Unix its mode is 0600. On Windows the new, still empty file gets an explicit DACL
/// before anything is written to it: inheritance removed and one entry, full control for the
/// current user's SID ([`restrict_to_owner`]); the file is removed again when that fails.
pub fn write_owner_only(path: &std::path::Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut f = options.open(path)?;
    #[cfg(windows)]
    if let Err(e) = restrict_to_owner(path) {
        drop(f);
        let _ = std::fs::remove_file(path);
        return Err(e);
    }
    f.write_all(bytes)?;
    f.sync_all()
}

/// Windows: replaces the DACL of `path` with one entry, full control for the current user, and
/// no inherited entries. The workspace forbids `unsafe` code, so this goes through the system's
/// own tools by their absolute paths (`%SystemRoot%\System32`): `whoami /user` for the SID,
/// `icacls /inheritance:r /grant:r *SID:(F)` for the list. In a folder without inheritable
/// entries a new file carries the creator's default DACL as explicit entries (SYSTEM,
/// Administrators, the logon session), which those two switches keep, so the list is then read
/// back as SDDL (`icacls /save`), every other trustee is removed (`/remove *SID`) and the result
/// is checked: anything but the one entry is an error.
#[cfg(windows)]
pub fn restrict_to_owner(path: &std::path::Path) -> io::Result<()> {
    use std::process::{Command, Stdio};
    let root = std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"));
    let system = root.join("System32");
    let failed = |what: &str| io::Error::other(format!("cannot restrict the file to its owner: {what}"));
    let icacls = |args: &[&std::ffi::OsStr]| -> io::Result<bool> {
        Ok(Command::new(system.join("icacls.exe"))
            .arg(path)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?
            .success())
    };
    // `icacls FILE /save ACLFILE` writes the file's name and its SDDL, in UTF-16LE.
    let read_sddl = || -> io::Result<String> {
        let name = path.file_name().ok_or_else(|| failed("no file name"))?;
        let mut save_name = name.to_os_string();
        save_name.push(format!(".acl-{}", std::process::id()));
        let save = path.with_file_name(save_name);
        let _ = std::fs::remove_file(&save);
        let ok = icacls(&["/save".as_ref(), save.as_os_str(), "/q".as_ref()]);
        let bytes = std::fs::read(&save);
        let _ = std::fs::remove_file(&save);
        if !ok? {
            return Err(failed("icacls could not read the list back"));
        }
        let bytes = bytes?;
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        Ok(String::from_utf16_lossy(&units))
    };
    let out = Command::new(system.join("whoami.exe"))
        .args(["/user", "/fo", "csv", "/nh"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()?;
    if !out.status.success() {
        return Err(failed("whoami /user failed"));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let sid = current_sid(&text).ok_or_else(|| failed("no SID in whoami's answer"))?;
    let grant = format!("*{sid}:(F)");
    if !icacls(&[
        "/inheritance:r".as_ref(),
        "/grant:r".as_ref(),
        grant.as_ref(),
        "/q".as_ref(),
    ])? {
        return Err(failed("icacls refused the new list"));
    }
    let sddl = read_sddl()?;
    let others: Vec<String> = dacl_aces(&sddl)
        .ok_or_else(|| failed("unreadable SDDL from icacls"))?
        .1
        .iter()
        .filter_map(|ace| ace_trustee(ace))
        .filter(|t| !trustee_is(t, sid))
        .map(|t| format!("*{t}"))
        .collect();
    if !others.is_empty() {
        let mut args: Vec<&std::ffi::OsStr> = vec!["/remove".as_ref()];
        args.extend(others.iter().map(|t| std::ffi::OsStr::new(t.as_str())));
        args.push("/q".as_ref());
        if !icacls(&args)? {
            return Err(failed("icacls could not remove the other entries"));
        }
    }
    let last = read_sddl()?;
    if !owner_only_dacl(&last, sid) {
        let dacl = last.find("D:").and_then(|i| last.get(i..)).unwrap_or("").trim();
        return Err(failed(&format!(
            "the list is not the user's alone (user {sid}, list {dacl})"
        )));
    }
    Ok(())
}

/// The DACL of an SDDL string (`D:PAI(A;;FA;;;S-1-…)(A;;FA;;;SY)`): its flags and its entries,
/// each without its parentheses; `None` when there is no DACL or a parenthesis is unbalanced.
#[cfg_attr(not(windows), allow(dead_code))]
fn dacl_aces(sddl: &str) -> Option<(&str, Vec<&str>)> {
    let start = sddl.find("D:")?.checked_add(2)?;
    let rest = sddl.get(start..)?;
    let flags_end = rest
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(rest.len());
    let flags = rest.get(..flags_end)?;
    let mut aces = Vec::new();
    let mut tail = rest.get(flags_end..)?;
    while tail.starts_with('(') {
        let mut depth = 0usize;
        let mut end = None;
        for (i, c) in tail.char_indices() {
            match c {
                '(' => depth = depth.checked_add(1)?,
                ')' => {
                    depth = depth.checked_sub(1)?;
                    if depth == 0 {
                        end = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let end = end?;
        aces.push(tail.get(1..end)?);
        tail = tail.get(end.checked_add(1)?..)?;
    }
    Some((flags, aces))
}

/// The trustee of an SDDL entry (`A;;FA;;;S-1-5-18`): its sixth field, a SID or an alias (`SY`).
#[cfg_attr(not(windows), allow(dead_code))]
fn ace_trustee(ace: &str) -> Option<&str> {
    let t = ace.split(';').nth(5)?.trim();
    let ok = !t.is_empty() && t.len() <= 184 && t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    ok.then_some(t)
}

/// Whether an SDDL string's DACL is protected and holds exactly one entry: allow, full control,
/// no inheritance flags, for `sid`.
#[cfg_attr(not(windows), allow(dead_code))]
fn owner_only_dacl(sddl: &str, sid: &str) -> bool {
    let Some((flags, aces)) = dacl_aces(sddl) else {
        return false;
    };
    let [ace] = aces.as_slice() else {
        return false;
    };
    let fields: Vec<&str> = ace.split(';').collect();
    flags.contains('P')
        && fields.len() == 6
        && fields.first() == Some(&"A")
        && fields.get(1) == Some(&"")
        && fields.get(2) == Some(&"FA")
        && fields.get(5).is_some_and(|t| trustee_is(t, sid))
}

/// Whether an SDDL trustee (a SID, or an alias SDDL writes for some accounts) is `sid`: `SY`,
/// `LS`, `NS` for the service accounts, `LA` and `LG` for a machine's built-in Administrator
/// (RID 500) and Guest (RID 501).
#[cfg_attr(not(windows), allow(dead_code))]
fn trustee_is(trustee: &str, sid: &str) -> bool {
    if trustee.eq_ignore_ascii_case(sid) {
        return true;
    }
    let machine_account = |rid: &str| sid.starts_with("S-1-5-21-") && sid.ends_with(rid);
    match trustee.to_ascii_uppercase().as_str() {
        "SY" => sid == "S-1-5-18",
        "LS" => sid == "S-1-5-19",
        "NS" => sid == "S-1-5-20",
        "LA" => machine_account("-500"),
        "LG" => machine_account("-501"),
        _ => false,
    }
}

/// The SID in `whoami /user /fo csv /nh`'s line (`"host\user","S-1-5-21-…"`).
#[cfg_attr(not(windows), allow(dead_code))]
fn current_sid(csv: &str) -> Option<&str> {
    let line = csv.lines().map(str::trim).find(|l| !l.is_empty())?;
    let sid = line.rsplit(',').next()?.trim().trim_matches('"');
    let ok = sid.starts_with("S-1-")
        && sid.len() <= 184
        && sid.bytes().all(|b| b.is_ascii_digit() || b == b'-' || b == b'S');
    ok.then_some(sid)
}

// ---------------------------------------------------------------------------------------------
// Connection caps and the accept loop

/// Counts a slot in a capped counter; releases it on drop.
pub struct Slot<'a>(&'a AtomicUsize);

impl<'a> Slot<'a> {
    /// Takes a slot when fewer than `cap` are taken.
    pub fn acquire(counter: &'a AtomicUsize, cap: usize) -> Option<Self> {
        let mut current = counter.load(Ordering::SeqCst);
        while current < cap {
            match counter.compare_exchange_weak(
                current,
                current.saturating_add(1),
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return Some(Slot(counter)),
                Err(actual) => current = actual,
            }
        }
        None
    }
}

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Open connections, capped in total and per client address.
#[derive(Debug)]
pub struct Connections {
    total: AtomicUsize,
    per_ip: Mutex<HashMap<IpAddr, usize>>,
    max_total: usize,
    /// `None`: no per-address cap.
    max_per_ip: Option<usize>,
}

impl Connections {
    pub fn new(max_total: usize, max_per_ip: Option<usize>) -> Arc<Self> {
        Arc::new(Self {
            total: AtomicUsize::new(0),
            per_ip: Mutex::new(HashMap::new()),
            max_total,
            max_per_ip,
        })
    }

    /// Open connections now.
    pub fn open(&self) -> usize {
        self.total.load(Ordering::SeqCst)
    }

    /// Takes a slot for a connection from `ip`; `None` when a cap is reached.
    pub fn acquire(self: &Arc<Self>, ip: IpAddr) -> Option<ConnGuard> {
        let slot = Slot::acquire(&self.total, self.max_total)?;
        if let Some(cap) = self.max_per_ip {
            let mut map = self.per_ip.lock().unwrap_or_else(|e| e.into_inner());
            let n = map.entry(ip).or_insert(0);
            if *n >= cap {
                return None; // `slot` releases the global count
            }
            *n = n.saturating_add(1);
        }
        // ownership of the global count moves to the guard
        std::mem::forget(slot);
        Some(ConnGuard {
            conns: Arc::clone(self),
            ip,
        })
    }
}

/// One counted connection; released on drop.
#[derive(Debug)]
pub struct ConnGuard {
    conns: Arc<Connections>,
    ip: IpAddr,
}

impl Drop for ConnGuard {
    fn drop(&mut self) {
        if self.conns.max_per_ip.is_some() {
            let mut map = self.conns.per_ip.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(n) = map.get_mut(&self.ip) {
                *n = n.saturating_sub(1);
                if *n == 0 {
                    map.remove(&self.ip);
                }
            }
        }
        self.conns.total.fetch_sub(1, Ordering::SeqCst);
    }
}

/// How often the accept loop polls its non-blocking listener (and checks its stop flag).
pub const ACCEPT_POLL: Duration = Duration::from_millis(15);

/// A connection handler: runs on the connection's own thread.
pub type Handler = Arc<dyn Fn(TcpStream, SocketAddr) + Send + Sync>;

/// Accepts connections until `stop` is set. Each accepted connection gets a slot in `conns`
/// (else a `503 busy` written from this thread without blocking it) and its own thread named
/// `thread_name`, where `handler` runs. `listener` must be non-blocking.
pub fn accept_loop(
    listener: TcpListener,
    stop: &AtomicBool,
    conns: &Arc<Connections>,
    thread_name: &str,
    handler: &Handler,
) {
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, peer)) => dispatch(stream, peer, conns, thread_name, handler),
            // WouldBlock, or a transient error such as running out of descriptors: back off.
            Err(_) => thread::sleep(ACCEPT_POLL),
        }
    }
}

fn dispatch(stream: TcpStream, peer: SocketAddr, conns: &Arc<Connections>, name: &str, handler: &Handler) {
    // Accepted sockets may inherit the listener's non-blocking mode (Windows).
    if stream.set_nonblocking(false).is_err() {
        return;
    }
    let _ = stream.set_nodelay(true);
    let Some(guard) = conns.acquire(peer.ip()) else {
        reject_busy(stream);
        return;
    };
    let handler = Arc::clone(handler);
    // If the spawn fails the closure (stream and guard) is dropped, closing the connection.
    let _ = thread::Builder::new().name(name.into()).spawn(move || {
        handler(stream, peer);
        drop(guard);
    });
}

/// Answers 503 from the accept thread without ever blocking it for long.
fn reject_busy(mut stream: TcpStream) {
    let _ = stream.set_write_timeout(Some(Duration::from_millis(100)));
    let _ = write_response(
        &mut stream,
        Status::Unavailable,
        "text/plain; charset=utf-8",
        &[],
        b"busy\n",
    );
    let _ = stream.shutdown(Shutdown::Both);
}

#[cfg(test)]
mod tests;
