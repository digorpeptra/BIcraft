//! The gateway: the controller's HTTP endpoint, in front of every workspace pod.
//!
//! - `/healthz`, `/readyz`: probes (no authentication, before the `Host` check).
//! - `/.well-known/oauth-protected-resource[/w/<id>/mcp]`: the protected resource metadata
//!   (RFC 9728), answered here so it never wakes a workspace.
//! - `/api/v1/workspaces[/<id>[/wake|/stop]]`: the admin and agent API.
//! - `/w/<id>/…`: the workspace's MCP endpoint and files, forwarded to its pod after
//!   authentication and the workspace's grants; a workspace at zero replicas is woken up and the
//!   request held until the pod is ready.
//!
//! Forwarding streams both directions with one thread each (no body is buffered: 4 GiB uploads,
//! downloads and SSE pass through as they come). The request head is rebuilt: `Host` becomes the
//! workspace Service's name, the credentials (`Authorization`, identity headers) are removed and
//! the identity the gateway checked is sent in the `X-Bicraft-Gateway-*` headers, which the pod
//! accepts from the gateway's address only.

use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use bicraft_auth::acl::{Access, Acl};
use bicraft_auth::{Authenticator, Denial, Identity};
use bicraft_http::{
    Connections, Handler, Head, Limits, ReadError, Status, accept_loop, finish, head_end, parse_head,
    read_body, respond_status, write_response,
};
use serde_json::{Value, json};

use crate::config::{
    ACCESS_GROUP_PREFIX, EDITOR_GROUP, EMAIL_HEADER, GROUPS_HEADER, KIND, READER_GROUP, Reclaim, USER_HEADER,
    VERSION,
};
use crate::kube::Resource;
use crate::reconcile::{Controller, Phase};
use crate::render;
use crate::spec::{Object, WorkspaceSpec, claim_name, recovery_claim_name, resolve};

/// The protected resource metadata path (RFC 9728), as `bicraft serve` serves it.
pub const RESOURCE_METADATA_PATH: &str = "/.well-known/oauth-protected-resource";
/// Largest API request body.
const MAX_API_BODY: usize = 64 * 1024;
/// Time to read an API body.
const API_BODY_DEADLINE: Duration = Duration::from_secs(10);
/// How often a waking workspace's `/readyz` is polled.
const READY_POLL: Duration = Duration::from_millis(500);
/// Buffer of the byte pump.
const PUMP_BUF: usize = 64 * 1024;

/// Upstream address of a workspace (`bws-<id>.<ns>.svc:7878` in a cluster).
pub type Upstream = Arc<dyn Fn(&str) -> String + Send + Sync>;

/// The gateway's settings.
#[derive(Clone)]
pub struct GatewayConfig {
    pub auth: Arc<Authenticator>,
    /// Who may create and delete workspaces (grants on `*`).
    pub admins: Acl,
    /// The `Host` values answered; empty: loopback names only.
    pub hosts: Vec<String>,
    /// Browser origins allowed on the API (requests with another `Origin` get 403).
    pub allowed_origins: Vec<String>,
    pub max_connections: usize,
    pub max_per_ip: Option<usize>,
    pub upstream: Upstream,
    /// Longest silence of the upstream (the longest DAX query is 600 s).
    pub upstream_idle: Duration,
    /// Longest wait to write to a slow client.
    pub client_write: Duration,
}

impl std::fmt::Debug for GatewayConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GatewayConfig({} hosts)", self.hosts.len())
    }
}

impl GatewayConfig {
    pub fn new(auth: Arc<Authenticator>, admins: Acl, upstream: Upstream) -> Self {
        Self {
            auth,
            admins,
            hosts: Vec::new(),
            allowed_origins: Vec::new(),
            max_connections: 512,
            max_per_ip: Some(64),
            upstream,
            upstream_idle: Duration::from_secs(660),
            client_write: Duration::from_secs(120),
        }
    }
}

/// A running gateway.
pub struct Gateway {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    conns: Arc<Connections>,
    accept: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for Gateway {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Gateway({})", self.addr)
    }
}

struct Ctx {
    ctl: Arc<Controller>,
    cfg: GatewayConfig,
    draining: Arc<AtomicBool>,
}

impl Gateway {
    pub fn start(listen: SocketAddr, ctl: Arc<Controller>, cfg: GatewayConfig) -> io::Result<Gateway> {
        let listener = TcpListener::bind(listen)?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let conns = Connections::new(cfg.max_connections, cfg.max_per_ip);
        let ctx = Arc::new(Ctx {
            ctl,
            cfg,
            draining: Arc::clone(&stop),
        });
        let handler: Handler = Arc::new(move |stream, peer| handle(&ctx, stream, peer));
        let (s, c) = (Arc::clone(&stop), Arc::clone(&conns));
        let accept = thread::Builder::new()
            .name("bicraft-gateway-accept".into())
            .spawn(move || accept_loop(listener, &s, &c, "bicraft-gateway", &handler))?;
        Ok(Gateway {
            addr,
            stop,
            conns,
            accept: Some(accept),
        })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Open connections now.
    pub fn open_connections(&self) -> usize {
        self.conns.open()
    }

    /// Stops accepting, then waits up to `grace` for the open connections; whether they all
    /// finished.
    pub fn stop(mut self, grace: Duration) -> bool {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.accept.take() {
            let _ = t.join();
        }
        let start = Instant::now();
        while self.conns.open() > 0 && start.elapsed() < grace {
            thread::sleep(Duration::from_millis(50));
        }
        self.conns.open() == 0
    }
}

impl Drop for Gateway {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.accept.take() {
            let _ = t.join();
        }
    }
}

fn limits() -> Limits {
    Limits {
        max_head_bytes: 32 * 1024,
        max_headers: 100,
        max_target_len: 8192,
        head_deadline: Duration::from_secs(10),
    }
}

/// Reads a request head and keeps its raw bytes: (raw head, parsed head, bytes read past it).
fn read_head(
    stream: &mut TcpStream,
    limits: &Limits,
    unique: &[&str],
) -> Result<(Vec<u8>, Head, Vec<u8>), ReadError> {
    let deadline = Instant::now()
        .checked_add(limits.head_deadline)
        .unwrap_or_else(Instant::now);
    let mut buf: Vec<u8> = Vec::with_capacity(2048);
    let mut chunk = [0u8; 4096];
    loop {
        if let Some(end) = head_end(&buf) {
            let raw = buf.get(..end).unwrap_or_default().to_vec();
            let rest = buf.get(end..).unwrap_or_default().to_vec();
            let head = parse_head(&raw, limits, unique)?;
            return Ok((raw, head, rest));
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

fn json_response(stream: &mut TcpStream, status: Status, body: &Value, extra: &[(&str, &str)]) {
    let mut headers = vec![("Cache-Control", "no-store")];
    headers.extend_from_slice(extra);
    let _ = write_response(
        stream,
        status,
        "application/json",
        &headers,
        format!("{body}\n").as_bytes(),
    );
}

fn error(stream: &mut TcpStream, status: Status, msg: &str) {
    json_response(stream, status, &json!({"error": msg}), &[]);
}

/// Whether `host` is one of the answered names (ASCII case-insensitive).
fn host_ok(cfg: &GatewayConfig, host: Option<&str>) -> bool {
    let Some(h) = host else { return false };
    if cfg.hosts.is_empty() {
        return bicraft_http::host_allowed(h);
    }
    cfg.hosts.iter().any(|n| n.eq_ignore_ascii_case(h.trim()))
}

/// The workspace id of `/w/<id>/…` and the rest of the path.
fn workspace_of(path: &str) -> Option<(&str, &str)> {
    let rest = path.strip_prefix("/w/")?;
    let (id, tail) = match rest.split_once('/') {
        Some((id, tail)) => (id, tail),
        None => (rest, ""),
    };
    (!id.is_empty()).then_some((id, tail))
}

fn handle(ctx: &Arc<Ctx>, mut stream: TcpStream, peer: SocketAddr) {
    let unique_owned = ctx.cfg.auth.unique_headers();
    let mut unique: Vec<&str> = unique_owned.iter().map(String::as_str).collect();
    unique.push("host");
    let (raw, head, rest) = match read_head(&mut stream, &limits(), &unique) {
        Ok(r) => r,
        Err(ReadError::Respond(s)) => {
            respond_status(&mut stream, s, &[]);
            finish(stream);
            return;
        }
        Err(ReadError::Close) => return,
    };
    let _ = stream.set_write_timeout(Some(ctx.cfg.client_write));
    match head.path.as_str() {
        "/healthz" => {
            let _ = write_response(&mut stream, Status::Ok, "text/plain; charset=utf-8", &[], b"ok\n");
            finish(stream);
            return;
        }
        "/readyz" => {
            let (s, body): (Status, &[u8]) = if ctx.draining.load(Ordering::SeqCst) {
                (Status::Unavailable, b"draining\n")
            } else if ctx.ctl.is_ready() {
                (Status::Ok, b"ok\n")
            } else {
                (Status::Unavailable, b"starting\n")
            };
            let _ = write_response(&mut stream, s, "text/plain; charset=utf-8", &[], body);
            finish(stream);
            return;
        }
        _ => {}
    }
    if !host_ok(&ctx.cfg, head.header("host")) {
        error(&mut stream, Status::Forbidden, "unknown Host");
        finish(stream);
        return;
    }
    let public = ctx.cfg.auth.public_url().map(str::to_string);
    if let Some(suffix) = head.path.strip_prefix(RESOURCE_METADATA_PATH) {
        let resource = format!("{}{suffix}", public.clone().unwrap_or_default());
        match (public.is_some(), ctx.cfg.auth.resource_metadata(&resource)) {
            (true, Some(doc)) => json_response(&mut stream, Status::Ok, &doc, &[]),
            _ => error(&mut stream, Status::NotFound, "no protected resource metadata"),
        }
        finish(stream);
        return;
    }
    let ws = workspace_of(&head.path).map(|(id, _)| id.to_string());
    let resources: Vec<String> = match (&public, &ws) {
        (Some(p), Some(id)) => vec![format!("{p}/w/{id}/mcp")],
        _ => Vec::new(),
    };
    let id = match ctx
        .cfg
        .auth
        .authenticate(peer.ip(), |n| head.header(n), &resources)
    {
        Ok(id) => id,
        Err(denial) => {
            deny(ctx, &mut stream, &denial, ws.as_deref());
            finish(stream);
            return;
        }
    };
    if let Some(ws) = ws {
        proxy(ctx, stream, &raw, rest, &id, &ws);
    } else if head.path == "/api/v1/workspaces" || head.path.starts_with("/api/v1/workspaces/") {
        if let Some(o) = head.header("origin")
            && !ctx.cfg.allowed_origins.iter().any(|a| a.eq_ignore_ascii_case(o))
        {
            error(&mut stream, Status::Forbidden, "Origin not allowed");
        } else {
            api(ctx, &mut stream, &head, rest, &id);
        }
        finish(stream);
    } else {
        error(&mut stream, Status::NotFound, "not found");
        finish(stream);
    }
}

fn deny(ctx: &Ctx, stream: &mut TcpStream, denial: &Denial, ws: Option<&str>) {
    let metadata = ctx.cfg.auth.public_url().map(|p| match ws {
        Some(id) => format!("{p}{RESOURCE_METADATA_PATH}/w/{id}/mcp"),
        None => format!("{p}{RESOURCE_METADATA_PATH}"),
    });
    let (code, challenge) = ctx.cfg.auth.challenge(denial, metadata.as_deref());
    let status = match code {
        403 => Status::Forbidden,
        400 => Status::BadRequest,
        _ => Status::Unauthorized,
    };
    json_response(
        stream,
        status,
        &json!({"error": "authentication required"}),
        &[("WWW-Authenticate", &challenge)],
    );
}

fn is_admin(ctx: &Ctx, id: &Identity) -> bool {
    // admin grants are on every workspace (`SUBJECT=*`), so any name matches them
    ctx.cfg.admins.access(id, "*").is_some()
}

/// Whether `id` may use workspace `ws`: an admin, a controller-wide grant, or the workspace's own
/// grants.
/// The access `id` has on workspace `ws`: editor for an admin, else the highest of the
/// controller-wide grants and the workspace's own `grants` (editor) and `readers` (reader).
fn access(ctx: &Ctx, id: &Identity, ws: &str) -> Option<Access> {
    if is_admin(ctx, id) {
        return Some(Access::Editor);
    }
    let own = ctx
        .ctl
        .workspace(ws)
        .and_then(|w| w.info)
        .and_then(|i| Acl { grants: i.grants }.access(id, ws));
    ctx.cfg.auth.access(id, ws).max(own)
}

fn can_open(ctx: &Ctx, id: &Identity, ws: &str) -> bool {
    access(ctx, id, ws).is_some()
}

/// Grants of a workspace object that is not (yet) in the controller's state.
fn can_open_object(ctx: &Ctx, id: &Identity, o: &Object) -> bool {
    if can_open(ctx, id, &o.name) {
        return true;
    }
    let Ok(spec) = &o.spec else { return false };
    resolve(&ctx.ctl.cfg, &o.name, &o.uid, o.generation, spec)
        .is_ok_and(|w| Acl { grants: w.grants }.access(id, &o.name).is_some())
}

/// The API's view of a workspace.
fn view(o: &Object) -> Value {
    let spec = o
        .spec
        .as_ref()
        .map_or(Value::Null, |s| serde_json::to_value(s).unwrap_or(Value::Null));
    json!({
        "name": o.name,
        "phase": o.status.get("phase").cloned().unwrap_or(json!("Pending")),
        "message": o.status.get("message").cloned().unwrap_or(json!("")),
        "endpoint": o.status.get("endpoint").cloned().unwrap_or(Value::Null),
        "lastActivity": o.status.get("lastActivity").cloned().unwrap_or(Value::Null),
        "volumeClaim": o.status.get("volumeClaim").cloned().unwrap_or(Value::Null),
        "spec": spec,
    })
}

fn get_object(ctx: &Ctx, name: &str) -> Result<Option<Object>, String> {
    ctx.ctl
        .kube
        .get(Resource::Workspace, name)
        .map(|v| v.as_ref().and_then(Object::from_json))
        .map_err(|e| e.to_string())
}

fn api(ctx: &Ctx, stream: &mut TcpStream, head: &Head, rest: Vec<u8>, id: &Identity) {
    let tail = head
        .path
        .strip_prefix("/api/v1/workspaces")
        .unwrap_or_default()
        .trim_start_matches('/');
    let mut parts = tail.splitn(2, '/');
    let name = parts.next().unwrap_or_default();
    let action = parts.next();
    let method = head.method.as_str();
    match (name, action, method) {
        ("", None, "GET") => match ctx.ctl.kube.list(Resource::Workspace, None) {
            Ok(items) => {
                let list: Vec<Value> = items
                    .iter()
                    .filter_map(Object::from_json)
                    .filter(|o| can_open_object(ctx, id, o))
                    .map(|o| view(&o))
                    .collect();
                json_response(stream, Status::Ok, &json!({"workspaces": list}), &[]);
            }
            Err(e) => error(stream, Status::InternalError, &e.to_string()),
        },
        ("", None, "POST") => create(ctx, stream, head, rest, id),
        ("", None, _) => error(stream, Status::MethodNotAllowed, "GET or POST"),
        (n, None, "GET") => match get_object(ctx, n) {
            Ok(Some(o)) if can_open_object(ctx, id, &o) => json_response(stream, Status::Ok, &view(&o), &[]),
            Ok(_) => error(stream, Status::NotFound, "no such workspace"),
            Err(e) => error(stream, Status::InternalError, &e),
        },
        (n, None, "DELETE") => {
            if !is_admin(ctx, id) {
                return error(stream, Status::Forbidden, "deleting a workspace needs an admin");
            }
            let o = match get_object(ctx, n) {
                Ok(Some(o)) => o,
                Ok(None) => return error(stream, Status::NotFound, "no such workspace"),
                Err(e) => return error(stream, Status::InternalError, &e),
            };
            let reclaim = o
                .spec
                .as_ref()
                .ok()
                .and_then(|s| s.volume.reclaim_policy)
                .unwrap_or(ctx.ctl.cfg.default_reclaim_policy);
            match ctx.ctl.kube.delete(Resource::Workspace, n) {
                Ok(_) => {
                    eprintln!("bicraft controller: workspace {n} deleted by {}", id.label());
                    ctx.ctl.kick();
                    let volume = match reclaim {
                        Reclaim::Retain if ctx.ctl.cfg.recovery.enabled => {
                            format!("retained: {} and {}", claim_name(n), recovery_claim_name(n))
                        }
                        Reclaim::Retain => format!("retained: {}", claim_name(n)),
                        Reclaim::Delete => "deleted with the workspace".to_string(),
                    };
                    json_response(
                        stream,
                        Status::Accepted,
                        &json!({"name": n, "deleted": true, "volume": volume}),
                        &[],
                    );
                }
                Err(e) => error(stream, Status::InternalError, &e.to_string()),
            }
        }
        (n, Some(a @ ("wake" | "stop")), "POST") => {
            let Some(level) = access(ctx, id, n) else {
                return error(stream, Status::NotFound, "no such workspace");
            };
            let awake = a == "wake";
            if !awake && level < Access::Editor {
                return error(
                    stream,
                    Status::Forbidden,
                    "stopping a workspace needs editor access",
                );
            }
            if let Err(e) = ctx.ctl.set_awake(n, awake) {
                return error(stream, Status::Conflict, &e);
            }
            if awake && !wait_ready(ctx, n) {
                return error(
                    stream,
                    Status::Unavailable,
                    "the workspace did not become ready in time",
                );
            }
            match get_object(ctx, n) {
                Ok(Some(o)) => json_response(stream, Status::Ok, &view(&o), &[]),
                Ok(None) => error(stream, Status::NotFound, "no such workspace"),
                Err(e) => error(stream, Status::InternalError, &e),
            }
        }
        (_, Some(_), _) | (_, None, _) => error(stream, Status::NotFound, "not found"),
    }
}

fn create(ctx: &Ctx, stream: &mut TcpStream, head: &Head, rest: Vec<u8>, id: &Identity) {
    if !is_admin(ctx, id) {
        return error(stream, Status::Forbidden, "creating a workspace needs an admin");
    }
    let Some(len) = head.content_length else {
        return error(stream, Status::LengthRequired, "Content-Length required");
    };
    if len > MAX_API_BODY {
        return error(stream, Status::PayloadTooLarge, "body larger than 64 KiB");
    }
    let body = match read_body(stream, rest, len, API_BODY_DEADLINE) {
        Ok(b) => b,
        Err(s) => return error(stream, s, "could not read the body"),
    };
    let mut doc: Value = match serde_json::from_slice(&body) {
        Ok(v @ Value::Object(_)) => v,
        _ => return error(stream, Status::BadRequest, "the body is a JSON object"),
    };
    let name = match doc.as_object_mut().and_then(|o| o.remove("name")) {
        Some(Value::String(s)) => s,
        _ => return error(stream, Status::BadRequest, "`name` (a string) is required"),
    };
    let spec: WorkspaceSpec = match serde_json::from_value(doc) {
        Ok(s) => s,
        Err(e) => return error(stream, Status::BadRequest, &format!("spec: {e}")),
    };
    if let Err(e) = resolve(&ctx.ctl.cfg, &name, "", 0, &spec) {
        return error(stream, Status::BadRequest, &e);
    }
    let object = json!({
        "apiVersion": format!("{}/{VERSION}", ctx.ctl.cfg.crd_group),
        "kind": KIND,
        "metadata": {"name": name, "namespace": ctx.ctl.cfg.namespace},
        "spec": spec,
    });
    match ctx.ctl.kube.create(Resource::Workspace, &object) {
        Ok(v) => {
            eprintln!("bicraft controller: workspace {name} created by {}", id.label());
            if let Err(e) = ctx.ctl.reconcile_named(&name) {
                eprintln!("bicraft controller: workspace {name}: {e}");
            }
            let o = get_object(ctx, &name)
                .ok()
                .flatten()
                .or_else(|| Object::from_json(&v));
            let body = o.as_ref().map_or_else(|| json!({"name": name}), view);
            json_response(stream, Status::Created, &body, &[]);
        }
        Err(e) if e.status == Some(409) => error(stream, Status::Conflict, "the workspace exists"),
        Err(e) if e.status == Some(422) || e.status == Some(400) => {
            error(stream, Status::BadRequest, &e.message)
        }
        Err(e) => error(stream, Status::InternalError, &e.to_string()),
    }
}

// ---------------------------------------------------------------------------------------------
// Forwarding

fn resolve_addr(addr: &str) -> Option<SocketAddr> {
    addr.to_socket_addrs().ok()?.next()
}

/// Whether the workspace pod answers `/readyz` with 200.
fn upstream_ready(addr: &str) -> bool {
    let Some(sa) = resolve_addr(addr) else {
        return false;
    };
    let Ok(mut s) = TcpStream::connect_timeout(&sa, Duration::from_secs(2)) else {
        return false;
    };
    let _ = s.set_read_timeout(Some(Duration::from_secs(3)));
    let _ = s.set_write_timeout(Some(Duration::from_secs(3)));
    if s.write_all(format!("GET /readyz HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n").as_bytes())
        .is_err()
    {
        return false;
    }
    let mut buf = [0u8; 64];
    let mut n = 0;
    while n < 12 {
        match s.read(buf.get_mut(n..).unwrap_or_default()) {
            Ok(0) | Err(_) => break,
            Ok(k) => n = n.saturating_add(k),
        }
    }
    let head = buf.get(..n).unwrap_or_default();
    head.starts_with(b"HTTP/1.1 200") || head.starts_with(b"HTTP/1.0 200")
}

/// Waits for the workspace's pod to be ready, within the configured wake timeout.
fn wait_ready(ctx: &Ctx, ws: &str) -> bool {
    let addr = (ctx.cfg.upstream)(ws);
    let limit = Duration::from_secs(ctx.ctl.cfg.wake_timeout_seconds);
    let start = Instant::now();
    loop {
        if upstream_ready(&addr) {
            return true;
        }
        if start.elapsed() >= limit || ctx.draining.load(Ordering::SeqCst) {
            return false;
        }
        thread::sleep(READY_POLL);
    }
}

/// A header value without control characters (identity values come from tokens and claims).
fn clean(v: &str) -> String {
    v.chars().filter(|c| !c.is_control()).take(4096).collect()
}

/// The request head sent to the pod.
fn upstream_head(ctx: &Ctx, raw: &[u8], id: &Identity, level: Access, host: &str) -> Vec<u8> {
    let text = String::from_utf8_lossy(raw);
    let mut lines = text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l));
    let request_line = lines.next().unwrap_or_default();
    let mut drop: Vec<String> = ctx.cfg.auth.unique_headers();
    for h in [
        "host",
        USER_HEADER,
        EMAIL_HEADER,
        GROUPS_HEADER,
        "x-forwarded-user",
        "x-forwarded-email",
        "x-forwarded-groups",
    ] {
        drop.push(h.to_ascii_lowercase());
    }
    let mut out = String::with_capacity(raw.len().saturating_add(256));
    out.push_str(request_line);
    out.push_str("\r\n");
    out.push_str(&format!("Host: {host}\r\n"));
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let name = line
            .split(':')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        if drop.iter().any(|d| d.eq_ignore_ascii_case(&name)) {
            continue;
        }
        out.push_str(line);
        out.push_str("\r\n");
    }
    out.push_str(&format!("{USER_HEADER}: {}\r\n", clean(&id.label())));
    if let Some(e) = &id.email {
        out.push_str(&format!("{EMAIL_HEADER}: {}\r\n", clean(e)));
    }
    // The identity's groups, minus any that could pose as the gateway's access group, then the
    // access group the pod maps to editor or reader.
    let mut groups: Vec<String> = id
        .groups
        .iter()
        .filter(|g| !g.contains(','))
        .map(|g| clean(g))
        .filter(|g| !g.is_empty() && !g.trim().to_ascii_lowercase().starts_with(ACCESS_GROUP_PREFIX))
        .collect();
    groups.push(
        match level {
            Access::Editor => EDITOR_GROUP,
            Access::Reader => READER_GROUP,
        }
        .to_string(),
    );
    out.push_str(&format!("{GROUPS_HEADER}: {}\r\n", groups.join(",")));
    out.push_str("\r\n");
    out.into_bytes()
}

fn proxy(ctx: &Arc<Ctx>, mut stream: TcpStream, raw: &[u8], rest: Vec<u8>, id: &Identity, ws: &str) {
    let Some(level) = access(ctx, id, ws) else {
        error(&mut stream, Status::NotFound, "no such workspace");
        finish(stream);
        return;
    };
    // Counted as activity from now on (before the state is read): the idle stop never races a
    // request.
    let _activity = ctx.ctl.begin(ws);
    let Some(state) = ctx.ctl.workspace(ws) else {
        error(&mut stream, Status::NotFound, "no such workspace");
        finish(stream);
        return;
    };
    if !state.awake
        && let Err(e) = ctx.ctl.set_awake(ws, true)
    {
        json_response(
            &mut stream,
            Status::Unavailable,
            &json!({"error": e}),
            &[("Retry-After", "10")],
        );
        finish(stream);
        return;
    }
    let addr = (ctx.cfg.upstream)(ws);
    if (state.phase != Phase::Running || !state.awake) && !wait_ready(ctx, ws) {
        json_response(
            &mut stream,
            Status::Unavailable,
            &json!({"error": "the workspace is starting; retry"}),
            &[("Retry-After", "10")],
        );
        finish(stream);
        return;
    }
    let connect =
        || resolve_addr(&addr).and_then(|sa| TcpStream::connect_timeout(&sa, Duration::from_secs(10)).ok());
    let upstream = match connect() {
        Some(u) => u,
        // The pod may be restarting: wait for it once.
        None if wait_ready(ctx, ws) => match connect() {
            Some(u) => u,
            None => {
                error(&mut stream, Status::Unavailable, "the workspace is unreachable");
                finish(stream);
                return;
            }
        },
        None => {
            error(&mut stream, Status::Unavailable, "the workspace is unreachable");
            finish(stream);
            return;
        }
    };
    let host = render::upstream_host(&ctx.ctl.cfg, ws);
    let mut up = upstream;
    let _ = up.set_nodelay(true);
    let _ = up.set_write_timeout(Some(ctx.cfg.upstream_idle));
    let new_head = upstream_head(ctx, raw, id, level, &host);
    if up
        .write_all(&new_head)
        .and_then(|()| up.write_all(&rest))
        .is_err()
    {
        error(
            &mut stream,
            Status::Unavailable,
            "the workspace closed the connection",
        );
        finish(stream);
        return;
    }
    pump(stream, up, ctx.cfg.upstream_idle, ctx.cfg.client_write);
}

/// Copies until EOF or an error (a timeout ends the copy too).
fn copy(mut from: TcpStream, mut to: TcpStream) -> u64 {
    let mut buf = vec![0u8; PUMP_BUF];
    let mut total = 0u64;
    loop {
        match from.read(&mut buf) {
            Ok(0) => return total,
            Ok(n) => {
                if to.write_all(buf.get(..n).unwrap_or_default()).is_err() {
                    return total;
                }
                total = total.saturating_add(n as u64);
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return total,
        }
    }
}

/// Streams the client's bytes to the pod and the pod's answer back, each on its own thread, until
/// the pod closes (it answers `Connection: close`).
fn pump(client: TcpStream, upstream: TcpStream, upstream_idle: Duration, client_write: Duration) {
    let _ = upstream.set_read_timeout(Some(upstream_idle));
    // The client side is read without a timeout: after its body the client is silent while the
    // pod works (up to 600 s); the copy ends when the pod's answer is complete and both sockets
    // are shut down below.
    let _ = client.set_read_timeout(None);
    let _ = client.set_write_timeout(Some(client_write));
    let (Ok(c_read), Ok(u_write)) = (client.try_clone(), upstream.try_clone()) else {
        return;
    };
    let upload = thread::Builder::new()
        .name("bicraft-gateway-upload".into())
        .spawn(move || {
            let u2 = u_write.try_clone();
            copy(c_read, u_write);
            // The client finished sending (or left): tell the pod.
            if let Ok(u) = u2 {
                let _ = u.shutdown(Shutdown::Write);
            }
        });
    if let Ok(c_write) = client.try_clone() {
        copy(upstream.try_clone().unwrap_or(upstream), c_write);
    }
    let _ = client.shutdown(Shutdown::Both);
    if let Ok(t) = upload {
        let _ = t.join();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tests::sample;
    use crate::kube::{FakeKube, Kube};
    use bicraft_auth::{AuthConfig, Grant, TokenEntry};
    use std::sync::Mutex;

    const ADMIN: &str = "bicraft_admin_token_0000000000000000000000";
    const AGENT: &str = "bicraft_agent_token_0000000000000000000000";
    const OTHER: &str = "bicraft_other_token_0000000000000000000000";
    const VIEWER: &str = "bicraft_viewer_token_000000000000000000000";

    fn token_entry(name: &str, token: &str) -> TokenEntry {
        TokenEntry {
            name: name.into(),
            sha256: bicraft_auth::tokens::digest(token),
            roles: Vec::new(),
            groups: Vec::new(),
        }
    }

    /// A fake pod: answers /readyz when `ready`, echoes what it received otherwise.
    struct FakePod {
        addr: SocketAddr,
        ready: Arc<AtomicBool>,
        seen: Arc<Mutex<Vec<String>>>,
    }

    fn fake_pod() -> FakePod {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let ready = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (r, s) = (ready.clone(), seen.clone());
        thread::spawn(move || {
            for c in l.incoming() {
                let Ok(mut c) = c else { continue };
                let (r, s) = (r.clone(), s.clone());
                thread::spawn(move || {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 4096];
                    let mut total = None;
                    loop {
                        let n = c.read(&mut chunk).unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                        if let Some(end) = head_end(&buf) {
                            let text = String::from_utf8_lossy(&buf[..end]).to_string();
                            let len = text
                                .lines()
                                .find_map(|l| l.strip_prefix("Content-Length: "))
                                .and_then(|v| v.trim().parse::<usize>().ok())
                                .unwrap_or(0);
                            total = Some(end + len);
                        }
                        if total.is_some_and(|t| buf.len() >= t) {
                            break;
                        }
                    }
                    let text = String::from_utf8_lossy(&buf).to_string();
                    if text.starts_with("GET /readyz") {
                        let ok = r.load(Ordering::SeqCst);
                        let _ = c.write_all(if ok {
                            b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nok\n" as &[u8]
                        } else {
                            b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        });
                        return;
                    }
                    s.lock().unwrap().push(text.clone());
                    let body = format!("echo {} bytes", text.len());
                    let _ = c.write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                        .as_bytes(),
                    );
                });
            }
        });
        FakePod { addr, ready, seen }
    }

    struct Env {
        kube: Arc<FakeKube>,
        ctl: Arc<Controller>,
        gw: Gateway,
        pod: FakePod,
    }

    fn env() -> Env {
        let kube = Arc::new(FakeKube::new());
        let mut cfg = sample();
        cfg.wake_timeout_seconds = 5;
        let ctl = Controller::new(cfg, kube.clone(), vec!["127.0.0.1/32".into()]);
        let mut ac = AuthConfig {
            tokens: vec![
                token_entry("admin", ADMIN),
                token_entry("agent", AGENT),
                token_entry("other", OTHER),
                // a group posing as the gateway's access group: dropped by the gateway
                TokenEntry {
                    groups: vec!["bicraft-gateway-editor".into()],
                    ..token_entry("viewer", VIEWER)
                },
            ],
            ..AuthConfig::default()
        };
        ac.acl.grants.push(Grant::parse_text("token:admin=*").unwrap());
        let auth = Arc::new(Authenticator::new(ac).unwrap());
        let admins = Acl {
            grants: vec![Grant::parse_text("token:admin=*").unwrap()],
        };
        let pod = fake_pod();
        let pod_addr = pod.addr.to_string();
        let upstream: Upstream = Arc::new(move |_ws: &str| pod_addr.clone());
        let gw = Gateway::start(
            "127.0.0.1:0".parse().unwrap(),
            ctl.clone(),
            GatewayConfig::new(auth, admins, upstream),
        )
        .unwrap();
        Env { kube, ctl, gw, pod }
    }

    fn send(
        gw: &Gateway,
        method: &str,
        path: &str,
        token: Option<&str>,
        extra: &str,
        body: &str,
    ) -> (u16, String) {
        let mut s = TcpStream::connect(gw.addr()).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
        let auth = token
            .map(|t| format!("Authorization: Bearer {t}\r\n"))
            .unwrap_or_default();
        write!(
            s,
            "{method} {path} HTTP/1.1\r\nHost: localhost\r\n{auth}{extra}Content-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        let mut out = Vec::new();
        let _ = s.read_to_end(&mut out);
        let text = String::from_utf8_lossy(&out).to_string();
        let status = text.get(9..12).and_then(|c| c.parse().ok()).unwrap_or(0);
        let body = text
            .split_once("\r\n\r\n")
            .map(|(_, b)| b.to_string())
            .unwrap_or_default();
        (status, body)
    }

    #[test]
    fn probes_hosts_and_authentication() {
        let e = env();
        assert_eq!(send(&e.gw, "GET", "/healthz", None, "", "").0, 200);
        assert_eq!(send(&e.gw, "GET", "/readyz", None, "", "").0, 503);
        e.ctl.reconcile_all(Instant::now(), 0).unwrap();
        assert_eq!(send(&e.gw, "GET", "/readyz", None, "", "").0, 200);
        let (s, _) = send(&e.gw, "GET", "/api/v1/workspaces", None, "", "");
        assert_eq!(s, 401);
        let (s, _) = send(
            &e.gw,
            "GET",
            "/api/v1/workspaces",
            Some("bicraft_wrong_0000000000000000000000000"),
            "",
            "",
        );
        assert_eq!(s, 401);
        let mut st = TcpStream::connect(e.gw.addr()).unwrap();
        write!(
            st,
            "GET /api/v1/workspaces HTTP/1.1\r\nHost: evil.example\r\nAuthorization: Bearer {ADMIN}\r\n\r\n"
        )
        .unwrap();
        let mut out = String::new();
        let _ = st.read_to_string(&mut out);
        assert!(out.starts_with("HTTP/1.1 403"), "{out}");
        let (s, _) = send(
            &e.gw,
            "GET",
            "/api/v1/workspaces",
            Some(ADMIN),
            "Origin: https://evil.example\r\n",
            "",
        );
        assert_eq!(s, 403);
        assert_eq!(send(&e.gw, "GET", "/elsewhere", Some(ADMIN), "", "").0, 404);
    }

    #[test]
    fn the_api_creates_lists_and_deletes() {
        let e = env();
        let body = r#"{"name": "sales", "grants": ["token:agent"], "idleTimeoutSeconds": 60}"#;
        assert_eq!(
            send(&e.gw, "POST", "/api/v1/workspaces", Some(AGENT), "", body).0,
            403
        );
        let (s, b) = send(&e.gw, "POST", "/api/v1/workspaces", Some(ADMIN), "", body);
        assert_eq!(s, 201, "{b}");
        let v: Value = serde_json::from_str(&b).unwrap();
        assert_eq!(v["phase"], "Idle");
        assert!(e.kube.object(Resource::Deployment, "bws-sales").is_some());
        assert_eq!(
            send(&e.gw, "POST", "/api/v1/workspaces", Some(ADMIN), "", body).0,
            409
        );
        let bad = r#"{"name": "big", "sizeClass": "huge"}"#;
        let (s, b) = send(&e.gw, "POST", "/api/v1/workspaces", Some(ADMIN), "", bad);
        assert_eq!(s, 400);
        assert!(b.contains("sizeClass"));
        assert_eq!(
            send(
                &e.gw,
                "POST",
                "/api/v1/workspaces",
                Some(ADMIN),
                "",
                r#"{"name": "x", "memory": 1}"#
            )
            .0,
            400
        );
        assert_eq!(
            send(&e.gw, "POST", "/api/v1/workspaces", Some(ADMIN), "", "[1]").0,
            400
        );
        // Lists show what the caller may open.
        let (_, b) = send(&e.gw, "GET", "/api/v1/workspaces", Some(AGENT), "", "");
        assert!(b.contains("\"sales\""));
        let (_, b) = send(&e.gw, "GET", "/api/v1/workspaces", Some(OTHER), "", "");
        assert!(!b.contains("\"sales\""));
        assert_eq!(
            send(&e.gw, "GET", "/api/v1/workspaces/sales", Some(OTHER), "", "").0,
            404
        );
        assert_eq!(
            send(&e.gw, "GET", "/api/v1/workspaces/sales", Some(AGENT), "", "").0,
            200
        );
        assert_eq!(
            send(&e.gw, "DELETE", "/api/v1/workspaces/sales", Some(AGENT), "", "").0,
            403
        );
        let (s, b) = send(&e.gw, "DELETE", "/api/v1/workspaces/sales", Some(ADMIN), "", "");
        assert_eq!(s, 202);
        assert!(b.contains("retained: bws-sales-data"), "{b}");
        assert!(e.kube.object(Resource::Deployment, "bws-sales").is_none());
        assert!(e.kube.object(Resource::Claim, "bws-sales-data").is_some());
        assert_eq!(
            send(&e.gw, "DELETE", "/api/v1/workspaces/sales", Some(ADMIN), "", "").0,
            404
        );
    }

    #[test]
    fn a_request_wakes_the_workspace_and_is_forwarded_with_the_identity() {
        let e = env();
        e.kube
            .create(
                Resource::Workspace,
                &json!({"metadata": {"name": "sales"}, "spec": {"grants": ["token:agent"], "readers": ["token:viewer"]}}),
            )
            .unwrap();
        e.ctl.reconcile_all(Instant::now(), 0).unwrap();
        // Not granted: answered as missing, nothing wakes.
        assert_eq!(send(&e.gw, "POST", "/w/sales/mcp", Some(OTHER), "", "{}").0, 404);
        assert_eq!(
            send(&e.gw, "POST", "/w/nowhere/mcp", Some(ADMIN), "", "{}").0,
            404
        );
        assert!(!e.ctl.workspace("sales").unwrap().awake);
        // The pod becomes ready a moment after the wake-up.
        let ready = e.pod.ready.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(700));
            ready.store(true, Ordering::SeqCst);
        });
        let (s, b) = send(
            &e.gw,
            "POST",
            "/w/sales/mcp?x=1",
            Some(AGENT),
            "X-Bicraft-Gateway-User: forged\r\nX-Forwarded-User: forged\r\n",
            "{\"jsonrpc\":\"2.0\"}",
        );
        assert_eq!(s, 200, "{b}");
        assert!(b.starts_with("echo"));
        let d = e.kube.object(Resource::Deployment, "bws-sales").unwrap();
        assert_eq!(d["spec"]["replicas"], 1);
        let seen = e.pod.seen.lock().unwrap().clone();
        let req = seen.last().unwrap();
        assert!(req.starts_with("POST /w/sales/mcp?x=1 HTTP/1.1\r\n"), "{req}");
        assert!(req.contains("Host: bws-sales.bi.svc:7878\r\n"));
        assert!(req.contains("X-Bicraft-Gateway-User: token:agent\r\n"));
        assert!(
            req.contains("X-Bicraft-Gateway-Groups: bicraft-gateway-editor\r\n"),
            "{req}"
        );
        assert!(!req.contains("forged"));
        assert!(!req.contains("Authorization"));
        assert!(req.ends_with("{\"jsonrpc\":\"2.0\"}"));
        // Activity was recorded and released (the handler thread ends just after the answer).
        let t = Instant::now();
        while e.ctl.workspace("sales").unwrap().inflight > 0 && t.elapsed() < Duration::from_secs(5) {
            thread::sleep(Duration::from_millis(20));
        }
        let w = e.ctl.workspace("sales").unwrap();
        assert_eq!(w.inflight, 0);
        assert!(w.awake);
        // A reader is forwarded as a reader, whatever groups its identity claims, and may not stop it.
        let (s, _) = send(&e.gw, "POST", "/w/sales/mcp", Some(VIEWER), "", "{}");
        assert_eq!(s, 200);
        let seen = e.pod.seen.lock().unwrap().clone();
        let req = seen.last().unwrap();
        assert!(req.contains("X-Bicraft-Gateway-User: token:viewer\r\n"), "{req}");
        assert!(
            req.contains("X-Bicraft-Gateway-Groups: bicraft-gateway-reader\r\n"),
            "{req}"
        );
        assert!(!req.contains("bicraft-gateway-editor"), "{req}");
        assert_eq!(
            send(
                &e.gw,
                "POST",
                "/api/v1/workspaces/sales/stop",
                Some(VIEWER),
                "",
                ""
            )
            .0,
            403
        );
        assert_eq!(
            send(
                &e.gw,
                "POST",
                "/api/v1/workspaces/sales/wake",
                Some(VIEWER),
                "",
                ""
            )
            .0,
            200
        );
        assert_eq!(
            send(&e.gw, "DELETE", "/api/v1/workspaces/sales", Some(VIEWER), "", "").0,
            403
        );
    }

    /// Document views, their MCP endpoints and the UI control channel (`bicraft serve
    /// --viewer-control`) go through as they are: paths, queries and the control token's header,
    /// with the checked identity instead of the credential.
    #[test]
    fn view_and_control_routes_reach_the_pod_with_the_control_header() {
        let e = env();
        e.kube
            .create(
                Resource::Workspace,
                &json!({"metadata": {"name": "sales"}, "spec": {"grants": ["token:agent"]}}),
            )
            .unwrap();
        e.ctl.reconcile_all(Instant::now(), 0).unwrap();
        e.pod.ready.store(true, Ordering::SeqCst);
        let view = "0123456789abcdef0123456789abcdef";
        for (method, path, extra) in [
            (
                "POST",
                "/w/sales/view/api/session".to_string(),
                "Content-Type: application/json\r\n",
            ),
            (
                "POST",
                format!("/w/sales/mcp/view/{view}"),
                "Mcp-Method: tools/call\r\n",
            ),
            (
                "GET",
                "/w/sales/view/api/control".to_string(),
                "X-Bicraft-Control: ctl-token-0123456789abcdef0123456789\r\n",
            ),
            (
                "POST",
                format!("/w/sales/view/api/control?v={view}"),
                "X-Bicraft-Control: ctl-token-0123456789abcdef0123456789\r\n",
            ),
            (
                "GET",
                format!("/w/sales/view/api/control/events?v={view}"),
                "Accept: text/event-stream\r\n",
            ),
            (
                "POST",
                format!("/w/sales/view/api/control/reply?v={view}"),
                "X-Bicraft-Token: page-token\r\n",
            ),
        ] {
            let (st, b) = send(&e.gw, method, &path, Some(AGENT), extra, "{}");
            assert_eq!(st, 200, "{path}: {b}");
            let seen = e.pod.seen.lock().unwrap().clone();
            let req = seen.last().unwrap();
            assert!(req.starts_with(&format!("{method} {path} HTTP/1.1\r\n")), "{req}");
            assert!(req.contains(extra), "{path}: the header goes through: {req}");
            assert!(req.contains("X-Bicraft-Gateway-User: token:agent\r\n"), "{req}");
            assert!(!req.contains("Authorization"), "{req}");
        }
    }

    #[test]
    fn a_workspace_that_never_gets_ready_answers_503() {
        let e = env();
        e.kube
            .create(
                Resource::Workspace,
                &json!({"metadata": {"name": "slow"}, "spec": {}}),
            )
            .unwrap();
        e.ctl.reconcile_all(Instant::now(), 0).unwrap();
        let t = Instant::now();
        let (s, _) = send(&e.gw, "POST", "/w/slow/mcp", Some(ADMIN), "", "{}");
        assert_eq!(s, 503);
        assert!(t.elapsed() >= Duration::from_secs(4));
        // Stop on request.
        assert_eq!(
            send(&e.gw, "POST", "/api/v1/workspaces/slow/stop", Some(ADMIN), "", "").0,
            200
        );
        assert!(!e.ctl.workspace("slow").unwrap().awake);
        assert_eq!(
            e.kube.object(Resource::Deployment, "bws-slow").unwrap()["spec"]["replicas"],
            0
        );
    }

    #[test]
    fn large_bodies_stream_through() {
        let e = env();
        e.kube
            .create(
                Resource::Workspace,
                &json!({"metadata": {"name": "big"}, "spec": {}}),
            )
            .unwrap();
        e.ctl.reconcile_all(Instant::now(), 0).unwrap();
        e.pod.ready.store(true, Ordering::SeqCst);
        let body = "x".repeat(3 * 1024 * 1024);
        let (s, b) = send(&e.gw, "PUT", "/w/big/files/a.bin", Some(ADMIN), "", &body);
        assert_eq!(s, 200);
        let n: usize = b
            .trim_start_matches("echo ")
            .trim_end_matches(" bytes")
            .parse()
            .unwrap();
        assert!(n > body.len());
        assert!(e.gw.stop(Duration::from_secs(1)));
    }

    #[test]
    fn paths() {
        assert_eq!(workspace_of("/w/sales/mcp"), Some(("sales", "mcp")));
        assert_eq!(workspace_of("/w/sales"), Some(("sales", "")));
        assert_eq!(workspace_of("/w/"), None);
        assert_eq!(workspace_of("/api"), None);
    }
}
