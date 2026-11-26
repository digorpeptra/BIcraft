//! File transfer per workspace (`docs/web-deployment.md`, "Files"): authenticated and
//! ACL-checked like the MCP endpoint, every path resolved by the engine's workspace sandbox
//! (`docs/server-mode.md`).
//!
//! | Request | Effect |
//! |---|---|
//! | `GET /w/<id>/files/` | the workspace's entries, recursively (JSON) |
//! | `GET /w/<id>/files/<folder>` | the folder's entries, recursively (JSON) |
//! | `GET` or `HEAD /w/<id>/files/<file>` | the file, streamed (`Content-Disposition: attachment`) |
//! | `PUT /w/<id>/files/<file>` | stores the raw body as the file |
//! | `POST /w/<id>/files/[<folder>]` with `Content-Type: application/zip` | extracts the archive into the folder (a zipped PBIP project) |
//! | `DELETE /w/<id>/files/<path>` | removes a file, an empty folder, or a folder tree with `Depth: infinity` |
//!
//! Writes:
//!
//! - **Streamed, never buffered whole**: the body goes in 64 KiB chunks to a temp file in the
//!   workspace's reserved `.bicraft-tmp/` folder (same file system), is synced, then renamed over
//!   the target: a reader sees the old file or the new one, never half of it.
//! - **Plain files only**: files are created with `create_new`, folders with `create_dir_all`;
//!   no link is ever created, and archive entries that are links are refused.
//! - **No overwrite by default**: an existing target answers 412 unless the request says
//!   `Overwrite: T` (the WebDAV header, RFC 4918 §10.6); a folder in the way answers 409.
//! - **Caps**: `Content-Length` is required (411) and at most
//!   [`HttpConfig::max_upload_bytes`](super::HttpConfig::max_upload_bytes) (413, refused before
//!   reading); an archive's extraction under [`ZipLimits`](super::ZipLimits) (413 past a cap).
//! - **Sessions**: the final step (checks and renames) runs under one server-wide lock and the
//!   workspace's session lock, so it never interleaves with a tool call. When it would replace
//!   or remove a file the session has open: 409 if the session holds unsaved changes (save or
//!   reopen first); otherwise the session is closed (`session_closed: true` in the answer) and
//!   the next tool call reopens the file, now the new one (a deleted file is not reopened).

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

use bicraft_auth::Access;
use bicraft_engine::{Refusal, ServerConfig};
use bicraft_http::{CopyError, Head, Status};
use serde_json::{Value, json};

use super::ops::{Reopen, TEMP_DIR, open_paths, touches_open};
use super::{Done, Queued, Reject, State, new_session, valid_workspace_id, write_json};

/// Error codes of the file routes (outside the JSON-RPC reserved range, like the endpoint's).
pub const FILE_ERROR: i64 = -31010;
/// Listing caps.
const MAX_LIST_ENTRIES: usize = 10_000;
const MAX_LIST_DEPTH: usize = 32;
/// Entries a recursive delete removes at most.
const MAX_DELETE_ENTRIES: usize = 200_000;

/// `/w/<id>/files` or `/w/<id>/files/<rest>`: the workspace id and the raw rest (still
/// percent-encoded; empty for the workspace itself).
pub(super) fn route(path: &str) -> Option<(&str, &str)> {
    let after = path.strip_prefix("/w/")?;
    let (id, tail) = after.split_once('/')?;
    if !valid_workspace_id(id) {
        return None;
    }
    let rest = match tail.strip_prefix("files") {
        Some("") => "",
        Some(r) => r.strip_prefix('/')?,
        None => return None,
    };
    Some((id, rest))
}

fn err(status: Status, message: impl AsRef<str>) -> Reject {
    Reject::plain(status, FILE_ERROR, message.as_ref())
}

/// Decodes `%XX` escapes; `None` for a broken escape, bytes that are not UTF-8, or control
/// characters.
fn percent_decode(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while let Some(&b) = bytes.get(i) {
        if b == b'%' {
            let hex = bytes.get(i.checked_add(1)?..i.checked_add(3)?)?;
            let s = std::str::from_utf8(hex).ok()?;
            out.push(u8::from_str_radix(s, 16).ok()?);
            i = i.checked_add(3)?;
        } else {
            out.push(b);
            i = i.checked_add(1)?;
        }
    }
    let s = String::from_utf8(out).ok()?;
    (!s.chars().any(char::is_control)).then_some(s)
}

/// A client path: decoded, `\` and `/` both separators, `.` and empty components dropped,
/// `/`-joined. `None` when it names the reserved temp folder. A rooted path (`/…`, `\…`) is kept
/// as is, and `..` and the other refused classes stay in: the sandbox refuses them with its
/// message.
fn normalize(decoded: &str) -> Option<String> {
    if decoded.starts_with(['/', '\\']) {
        return Some(decoded.to_string());
    }
    let parts: Vec<&str> = decoded
        .split(['/', '\\'])
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    if parts.first().is_some_and(|p| p.eq_ignore_ascii_case(TEMP_DIR)) {
        return None;
    }
    Some(parts.join("/"))
}

/// The entry a write or a deletion acts on: the sandbox checks the whole path (lexical rules,
/// links on the way), but a link in the last place is the entry itself, not its target (a
/// deletion removes the link; an upload replaces it with a plain file). Otherwise the canonical
/// path (the real spelling on a case-insensitive file system, to compare with the open file).
fn entry_path(cfg: &ServerConfig, rel: &str) -> Result<PathBuf, Reject> {
    let full = cfg.resolve(rel).map_err(|r| refusal(&r))?;
    let (parent, last) = match rel.rsplit_once('/') {
        Some((p, l)) => (cfg.resolve(p).map_err(|r| refusal(&r))?, l),
        None => (cfg.root().to_path_buf(), rel),
    };
    let lexical = parent.join(last);
    match std::fs::symlink_metadata(&lexical) {
        Ok(m) if m.file_type().is_symlink() => Ok(lexical),
        _ => Ok(full),
    }
}

fn refusal(r: &Refusal) -> Reject {
    let status = match r {
        Refusal::Escapes | Refusal::Unresolvable(_) => Status::Forbidden,
        _ => Status::BadRequest,
    };
    err(status, format!("path refused: {r}"))
}

/// Serves a file route (after authentication).
pub(super) fn serve(
    stream: &mut TcpStream,
    state: &State,
    head: &Head,
    rest: Vec<u8>,
    id: &str,
    raw: &str,
    access: Option<Access>,
) -> Result<Done, Reject> {
    if let Some(origin) = head.header("origin")
        && !state.origin_ok(origin)
    {
        return Err(err(Status::Forbidden, "this Origin is not allowed"));
    }
    let not_found = || Reject::plain(Status::NotFound, super::WORKSPACE_NOT_FOUND, "no such workspace");
    let Some(dir) = state.workspace_dir(id) else {
        return Err(not_found());
    };
    let decoded = percent_decode(raw).ok_or_else(|| err(Status::BadRequest, "malformed path"))?;
    let folder_form = decoded.is_empty() || decoded.ends_with('/') || decoded.ends_with('\\');
    let Some(rel) = normalize(&decoded) else {
        return Err(err(Status::NotFound, "no such file"));
    };
    let cfg = ServerConfig::new(&dir).map_err(|e| err(Status::InternalError, e))?;
    let writer = access.is_none_or(|a| a >= Access::Editor);
    match head.method.as_str() {
        "GET" | "HEAD" => read(stream, state, head, &cfg, &rel),
        "PUT" | "POST" | "DELETE" if !writer => Err(err(
            Status::Forbidden,
            "this identity may not change the workspace's files",
        )),
        "PUT" => {
            if folder_form {
                return Err(err(Status::BadRequest, "PUT needs a file path (no trailing `/`)"));
            }
            put(stream, state, head, &rest, id, &dir, &cfg, &rel)
        }
        "POST" => extract(stream, state, head, &rest, id, &dir, &cfg, &rel),
        "DELETE" => delete(stream, state, head, id, &dir, &cfg, &rel),
        _ => {
            bicraft_http::respond_status(
                stream,
                Status::MethodNotAllowed,
                &[("Allow", "GET, HEAD, PUT, POST, DELETE")],
            );
            Ok(Done::Answered)
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Reading

fn read(
    stream: &mut TcpStream,
    state: &State,
    head: &Head,
    cfg: &ServerConfig,
    rel: &str,
) -> Result<Done, Reject> {
    let path = if rel.is_empty() {
        cfg.root().to_path_buf()
    } else {
        cfg.resolve(rel).map_err(|r| refusal(&r))?
    };
    let meta = std::fs::metadata(&path).map_err(|_| err(Status::NotFound, "no such file"))?;
    if meta.is_dir() {
        let (entries, truncated) = list(&path, rel);
        let body = json!({"path": rel, "entries": entries, "truncated": truncated});
        if head.method == "HEAD" {
            let len = serde_json::to_vec(&body).map_or(0, |b| b.len());
            let _ = bicraft_http::write_head(
                stream,
                Status::Ok,
                "application/json",
                u64::try_from(len).unwrap_or(0),
                &[("Cache-Control", "no-store")],
            );
        } else {
            write_json(stream, Status::Ok, &body, &[]);
        }
        return Ok(Done::Answered);
    }
    if !meta.is_file() {
        return Err(err(Status::NotFound, "no such file"));
    }
    let mut file = File::open(&path).map_err(|_| err(Status::NotFound, "no such file"))?;
    let len = meta.len();
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("download");
    let disposition = content_disposition(name);
    let headers = [
        ("Content-Disposition", disposition.as_str()),
        ("Cache-Control", "no-store"),
        // never rendered as a page from this origin, whatever the bytes are
        ("Content-Security-Policy", "sandbox; default-src 'none'"),
    ];
    if bicraft_http::write_head(stream, Status::Ok, content_type(name), len, &headers).is_err() {
        return Ok(Done::Cancelled);
    }
    if head.method == "HEAD" {
        return Ok(Done::Answered);
    }
    let mut chunk = vec![0u8; 64 * 1024];
    let mut left = len;
    while left > 0 {
        if state.abort.load(Ordering::SeqCst) {
            return Ok(Done::Cancelled);
        }
        let want = usize::try_from(left).unwrap_or(usize::MAX).min(chunk.len());
        let n = match file.read(chunk.get_mut(..want).unwrap_or_default()) {
            Ok(0) | Err(_) => return Ok(Done::Cancelled), // the file shrank: cut the response short
            Ok(n) => n,
        };
        if stream.write_all(chunk.get(..n).unwrap_or_default()).is_err() {
            return Ok(Done::Cancelled);
        }
        left = left.saturating_sub(u64::try_from(n).unwrap_or(u64::MAX));
    }
    let _ = stream.flush();
    Ok(Done::Answered)
}

/// `Content-Type` by extension: JSON-like project files, text, images; anything else (PBIX,
/// PBIT, ABF) is opaque bytes.
fn content_type(name: &str) -> &'static str {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "json" | "pbir" | "pbism" | "pbip" | "bim" | "platform" | "vpax" => "application/json",
        "tmdl" | "txt" | "csv" | "m" | "dax" | "md" => "text/plain; charset=utf-8",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
}

/// `attachment` with an ASCII fallback name and the exact UTF-8 name (RFC 6266, RFC 8187).
fn content_disposition(name: &str) -> String {
    let ascii: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "._- ()[]".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut encoded = String::new();
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() || b"._-".contains(&b) {
            encoded.push(char::from(b));
        } else {
            encoded.push_str(&format!("%{b:02X}"));
        }
    }
    format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{encoded}")
}

/// RFC 3339 UTC (`2026-10-26T10:00:00Z`) of a modification time.
fn rfc3339(t: SystemTime) -> Option<String> {
    let secs = i64::try_from(t.duration_since(UNIX_EPOCH).ok()?.as_secs()).ok()?;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (year, month, day) = bicraft_locale::calendar::civil_from_days(days);
    Some(format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    ))
}

/// The entries below `dir`, recursively (bounded), sorted by path; links are listed as such and
/// never followed; the reserved temp folder is left out.
fn list(dir: &Path, rel: &str) -> (Vec<Value>, bool) {
    let mut out: Vec<(String, Value)> = Vec::new();
    let mut truncated = false;
    let prefix = if rel.is_empty() {
        String::new()
    } else {
        format!("{rel}/")
    };
    let mut stack: Vec<(PathBuf, String, usize)> = vec![(dir.to_path_buf(), prefix, 0)];
    'walk: while let Some((d, prefix, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if prefix.is_empty() && name.eq_ignore_ascii_case(TEMP_DIR) {
                continue;
            }
            if out.len() >= MAX_LIST_ENTRIES {
                truncated = true;
                break 'walk;
            }
            let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
                continue;
            };
            let path = format!("{prefix}{name}");
            let modified = meta.modified().ok().and_then(rfc3339);
            let kind = if meta.file_type().is_symlink() {
                "link"
            } else if meta.is_dir() {
                "dir"
            } else if meta.is_file() {
                "file"
            } else {
                "other"
            };
            let mut v = json!({"path": path, "type": kind, "modified": modified});
            if kind == "file" {
                v["size"] = json!(meta.len());
            }
            if kind == "dir" {
                if depth < MAX_LIST_DEPTH {
                    stack.push((entry.path(), format!("{path}/"), depth.saturating_add(1)));
                } else {
                    truncated = true;
                }
            }
            out.push((path, v));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    (out.into_iter().map(|(_, v)| v).collect(), truncated)
}

// ---------------------------------------------------------------------------------------------
// Writing

/// A temp file or folder in the workspace's reserved folder, removed on drop unless taken.
struct Temp {
    path: PathBuf,
    dir: bool,
}

impl Temp {
    /// The reserved folder, created when missing; refused when something else (a link) is there.
    fn base(ws: &Path) -> Result<PathBuf, Reject> {
        let base = ws.join(TEMP_DIR);
        match std::fs::create_dir(&base) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => {
                return Err(err(
                    Status::InternalError,
                    format!("cannot create the temp folder: {e}"),
                ));
            }
        }
        let meta = std::fs::symlink_metadata(&base).map_err(|e| err(Status::InternalError, e.to_string()))?;
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return Err(err(
                Status::InternalError,
                "the workspace's temp folder is not a plain folder",
            ));
        }
        Ok(base)
    }

    fn file(ws: &Path, kind: &str) -> Result<(Self, File), Reject> {
        let path = Self::base(ws)?.join(format!("{kind}-{}.part", bicraft_http::random_token()));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| err(Status::InternalError, format!("cannot create a temp file: {e}")))?;
        Ok((Self { path, dir: false }, file))
    }

    fn folder(ws: &Path) -> Result<Self, Reject> {
        let path = Self::base(ws)?.join(format!("extract-{}", bicraft_http::random_token()));
        std::fs::create_dir(&path)
            .map_err(|e| err(Status::InternalError, format!("cannot create a temp folder: {e}")))?;
        Ok(Self { path, dir: true })
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = if self.dir {
            std::fs::remove_dir_all(&self.path)
        } else {
            std::fs::remove_file(&self.path)
        };
    }
}

/// `Overwrite: T` / `F` (RFC 4918); absent: `F`.
fn overwrite(head: &Head) -> Result<bool, Reject> {
    match head.header("overwrite").map(str::trim) {
        None => Ok(false),
        Some(v) if v.eq_ignore_ascii_case("t") => Ok(true),
        Some(v) if v.eq_ignore_ascii_case("f") => Ok(false),
        Some(_) => Err(err(Status::BadRequest, "Overwrite must be T or F")),
    }
}

/// The announced body length, within the upload cap.
fn upload_length(head: &Head, state: &State) -> Result<u64, Reject> {
    let length = head
        .content_length
        .ok_or_else(|| err(Status::LengthRequired, "Content-Length is required"))?;
    let length = u64::try_from(length).unwrap_or(u64::MAX);
    if length > state.config.max_upload_bytes {
        return Err(err(
            Status::PayloadTooLarge,
            format!(
                "the upload is larger than {} bytes",
                state.config.max_upload_bytes
            ),
        ));
    }
    Ok(length)
}

fn sink_error(e: &io::Error) -> Reject {
    if e.kind() == io::ErrorKind::StorageFull {
        err(Status::InsufficientStorage, "the workspace volume is full")
    } else {
        err(Status::InternalError, format!("cannot write the upload: {e}"))
    }
}

/// Streams the body into a new temp file, synced.
fn receive(
    stream: &mut TcpStream,
    state: &State,
    rest: &[u8],
    length: u64,
    ws: &Path,
) -> Result<Temp, Reject> {
    let (temp, mut file) = Temp::file(ws, "upload")?;
    bicraft_http::copy_body(
        stream,
        rest,
        length,
        state.config.upload_limits,
        &state.abort,
        &mut file,
    )
    .map_err(|e| match e {
        CopyError::Client(status) => err(status, status.line()),
        CopyError::Sink(e) => sink_error(&e),
        CopyError::Stopped => err(Status::Unavailable, "the server is shutting down"),
    })?;
    file.sync_all().map_err(|e| sink_error(&e))?;
    Ok(temp)
}

/// What a commit did.
struct Committed {
    replaced: usize,
    session_closed: bool,
}

/// The final step of an upload or a deletion, under the server-wide commit lock and the
/// workspace's session lock (module docs): `check` validates the targets, then `act` writes.
/// `targets` are the resolved paths the step writes or removes; `keep_reopen` says whether a
/// closed session should reopen its file on the next call (not after a deletion).
fn commit(
    state: &State,
    id: &str,
    ws: &Path,
    targets: &[PathBuf],
    keep_reopen: bool,
    act: impl FnOnce() -> Result<usize, Reject>,
) -> Result<Committed, Reject> {
    let _commit = state.commit.lock().unwrap_or_else(|e| e.into_inner());
    let mut session_closed = false;
    let workspace = state.checkout_open(id);
    let _queued = workspace.as_ref().map(|w| Queued(&w.pending));
    let mut guard = match &workspace {
        Some(w) => Some(
            w.session
                .write_within(Some(state.config.commit_wait))
                .ok_or_else(|| {
                    let mut r = err(
                        Status::Unavailable,
                        "the workspace is busy with a tool call; try again",
                    );
                    r.extra.push(("Retry-After", "5".into()));
                    r
                })?,
        ),
        None => None,
    };
    if let (Some(w), Some(session)) = (&workspace, guard.as_mut()) {
        let open = open_paths(session);
        if targets.iter().any(|t| touches_open(&open, t)) {
            if session.dirty || session.report_dirty {
                return Err(err(
                    Status::Conflict,
                    "the workspace's open session has unsaved changes to this file: save it (or reopen \
                     the file to discard them) first",
                ));
            }
            let reopen = if keep_reopen { Reopen::of(session) } else { None };
            // the old session goes now (its file handles close before the rename)
            let revision = session.revision;
            let journal = session.recovery_config().cloned();
            **session =
                new_session(ws, state.config.allow_network).map_err(|e| err(Status::InternalError, e))?;
            session.set_recovery(journal);
            // revisions keep going up: an `expected_revision` of the old session never matches
            session.revision = revision.saturating_add(1);
            *w.reopen.lock().unwrap_or_else(|e| e.into_inner()) = reopen;
            session_closed = true;
        }
    }
    // the document views' own sessions (docs/mcp.md, "Document views"), the same way
    let docs: Vec<std::sync::Arc<super::Workspace>> = state
        .views_if_any(id)
        .map(|v| v.doc_sessions())
        .unwrap_or_default();
    let mut doc_guards = Vec::with_capacity(docs.len());
    for w in &docs {
        let mut g = w
            .session
            .write_within(Some(state.config.commit_wait))
            .ok_or_else(|| {
                let mut r = err(
                    Status::Unavailable,
                    "a browser tab of this workspace is busy with a command; try again",
                );
                r.extra.push(("Retry-After", "5".into()));
                r
            })?;
        let open = open_paths(&g);
        if targets.iter().any(|t| touches_open(&open, t)) {
            if g.dirty || g.report_dirty {
                return Err(err(
                    Status::Conflict,
                    "a document view (a browser tab) has unsaved changes to this file: save or close \
                     it first",
                ));
            }
            let reopen = if keep_reopen { Reopen::of(&g) } else { None };
            let revision = g.revision;
            let journal = g.recovery_config().cloned();
            *g = new_session(ws, state.config.allow_network).map_err(|e| err(Status::InternalError, e))?;
            g.set_recovery(journal);
            g.revision = revision.saturating_add(1);
            *w.reopen.lock().unwrap_or_else(|e| e.into_inner()) = reopen;
            session_closed = true;
        }
        doc_guards.push(g);
    }
    if !keep_reopen {
        // an idle-closed workspace must not try to reopen a file that is going away
        let mut closed = state.closed.lock().unwrap_or_else(|e| e.into_inner());
        if closed
            .get(id)
            .is_some_and(|r| targets.iter().any(|t| r.touches(ws, t)))
        {
            closed.remove(id);
        }
    }
    // readers' copies of a file going away close first (their handles), and go stale after
    let views = state.views_if_any(id);
    if let Some(v) = &views {
        v.drop_copies_touching(ws, targets).map_err(|()| {
            let mut r = err(
                Status::Unavailable,
                "readers are reading this file right now; try again",
            );
            r.extra.push(("Retry-After", "5".into()));
            r
        })?;
    }
    let replaced = act()?;
    if let Some(v) = &views {
        v.disk_changed();
    }
    drop(doc_guards);
    drop(guard);
    Ok(Committed {
        replaced,
        session_closed,
    })
}

/// Checks that `targets` can be written: no folder in the way (409), nothing there unless
/// `overwrite` (412). Returns how many exist.
fn check_targets(targets: &[PathBuf], overwrite: bool) -> Result<usize, Reject> {
    let mut existing = 0usize;
    for t in targets {
        match std::fs::symlink_metadata(t) {
            Ok(m) if m.is_dir() => return Err(err(Status::Conflict, "a folder is in the way")),
            Ok(_) if !overwrite => {
                return Err(err(
                    Status::PreconditionFailed,
                    "the file exists: send `Overwrite: T` to replace it",
                ));
            }
            Ok(_) => existing = existing.saturating_add(1),
            Err(_) => {}
        }
    }
    Ok(existing)
}

/// Moves `src` over `dst`, creating `dst`'s missing parent folders (plain folders).
fn place(src: &Path, dst: &Path) -> io::Result<()> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(src, dst)
}

#[allow(clippy::too_many_arguments)]
fn put(
    stream: &mut TcpStream,
    state: &State,
    head: &Head,
    rest: &[u8],
    id: &str,
    ws: &Path,
    cfg: &ServerConfig,
    rel: &str,
) -> Result<Done, Reject> {
    let overwrite = overwrite(head)?;
    let length = upload_length(head, state)?;
    // fail fast, before reading the body; checked again under the commit lock
    let target = entry_path(cfg, rel)?;
    check_targets(std::slice::from_ref(&target), overwrite)?;
    let temp = receive(stream, state, rest, length, ws)?;
    let done = commit(state, id, ws, std::slice::from_ref(&target), true, || {
        // resolved again: the folders on the way may have changed while the body arrived
        let target = entry_path(cfg, rel)?;
        let replaced = check_targets(std::slice::from_ref(&target), overwrite)?;
        place(&temp.path, &target).map_err(|e| sink_error(&e))?;
        Ok(replaced)
    })?;
    let status = if done.replaced > 0 {
        Status::Ok
    } else {
        Status::Created
    };
    write_json(
        stream,
        status,
        &json!({
            "path": rel,
            "size": length,
            "replaced": done.replaced > 0,
            "session_closed": done.session_closed,
        }),
        &[],
    );
    Ok(Done::Answered)
}

/// `dest/name`, or `name` at the workspace root.
fn join(dest: &str, name: &str) -> String {
    if dest.is_empty() {
        name.to_string()
    } else {
        format!("{dest}/{name}")
    }
}

#[allow(clippy::too_many_arguments)]
fn extract(
    stream: &mut TcpStream,
    state: &State,
    head: &Head,
    rest: &[u8],
    id: &str,
    ws: &Path,
    cfg: &ServerConfig,
    dest: &str,
) -> Result<Done, Reject> {
    let zip = head.header("content-type").is_some_and(|t| {
        t.split(';').next().is_some_and(|m| {
            let m = m.trim();
            m.eq_ignore_ascii_case("application/zip")
                || m.eq_ignore_ascii_case("application/x-zip-compressed")
        })
    });
    if !zip {
        return Err(err(
            Status::UnsupportedMedia,
            "POST extracts an archive: send Content-Type: application/zip (PUT stores a file as is)",
        ));
    }
    let overwrite = overwrite(head)?;
    let length = upload_length(head, state)?;
    if !dest.is_empty() {
        let d = cfg.resolve(dest).map_err(|r| refusal(&r))?;
        if d.exists() && !d.is_dir() {
            return Err(err(Status::Conflict, "the destination is a file"));
        }
    }
    let archive = receive(stream, state, rest, length, ws)?;
    let staging = Temp::folder(ws)?;
    let staged = ServerConfig::new(&staging.path).map_err(|e| err(Status::InternalError, e))?;
    let mut files: Vec<(PathBuf, String)> = Vec::new();
    let mut dirs: Vec<String> = Vec::new();
    let reader = File::open(&archive.path).map_err(|e| err(Status::InternalError, e.to_string()))?;
    let stats = bicraft_opc::for_each_zip_entry(
        io::BufReader::new(reader),
        &state.config.zip_limits,
        |entry, data| {
            if state.abort.load(Ordering::SeqCst) {
                return Err("the server is shutting down".into());
            }
            let target = join(dest, &entry.name);
            if normalize(&target).is_none_or(|n| n != target) {
                return Err(format!("`{}`: a reserved or non-canonical name", entry.name));
            }
            let path = staged
                .resolve(&entry.name)
                .map_err(|r| format!("`{}`: {r}", entry.name))?;
            if entry.is_dir {
                std::fs::create_dir_all(&path).map_err(|e| format!("`{}`: {e}", entry.name))?;
                dirs.push(target);
                return Ok(());
            }
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("`{}`: {e}", entry.name))?;
            }
            let mut out = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(|e| format!("`{}`: {e}", entry.name))?;
            io::copy(data, &mut out).map_err(|e| format!("`{}`: {e}", entry.name))?;
            out.sync_all().map_err(|e| format!("`{}`: {e}", entry.name))?;
            files.push((path, target));
            Ok(())
        },
    )
    .map_err(|e| match e {
        bicraft_opc::ExtractError::Invalid(_) | bicraft_opc::ExtractError::Refused(_) => {
            err(Status::BadRequest, e.to_string())
        }
        bicraft_opc::ExtractError::TooLarge(_) => err(Status::PayloadTooLarge, e.to_string()),
        bicraft_opc::ExtractError::Sink(m) => err(Status::BadRequest, m),
    })?;
    drop(archive);
    // final targets, resolved in the workspace
    let mut targets = Vec::with_capacity(files.len());
    for (_, rel) in &files {
        targets.push(entry_path(cfg, rel)?);
    }
    let mut dir_targets = Vec::with_capacity(dirs.len().saturating_add(1));
    for rel in dirs
        .iter()
        .map(String::as_str)
        .chain((!dest.is_empty()).then_some(dest))
    {
        dir_targets.push(cfg.resolve(rel).map_err(|r| refusal(&r))?);
    }
    let done = commit(state, id, ws, &targets, true, || {
        for d in &dir_targets {
            if d.exists() && !d.is_dir() {
                return Err(err(Status::Conflict, "a file is in the way of a folder"));
            }
        }
        let replaced = check_targets(&targets, overwrite)?;
        for d in &dir_targets {
            std::fs::create_dir_all(d).map_err(|e| sink_error(&e))?;
        }
        for (i, ((src, _), dst)) in files.iter().zip(&targets).enumerate() {
            place(src, dst).map_err(|e| {
                err(
                    Status::InternalError,
                    format!("{i} of {} files were placed, then: {e}", files.len()),
                )
            })?;
        }
        Ok(replaced)
    })?;
    drop(staging);
    write_json(
        stream,
        Status::Created,
        &json!({
            "path": dest,
            "files": stats.files,
            "dirs": stats.dirs,
            "bytes": stats.bytes,
            "replaced": done.replaced,
            "session_closed": done.session_closed,
        }),
        &[],
    );
    Ok(Done::Answered)
}

fn delete(
    stream: &mut TcpStream,
    state: &State,
    head: &Head,
    id: &str,
    ws: &Path,
    cfg: &ServerConfig,
    rel: &str,
) -> Result<Done, Reject> {
    if rel.is_empty() {
        return Err(err(
            Status::MethodNotAllowed,
            "the workspace folder itself cannot be deleted",
        ));
    }
    let recursive = match head.header("depth").map(str::trim) {
        None => false,
        Some(v) if v.eq_ignore_ascii_case("infinity") => true,
        Some("0") => false,
        Some(_) => return Err(err(Status::BadRequest, "Depth must be 0 or infinity")),
    };
    let target = entry_path(cfg, rel)?;
    commit(state, id, ws, std::slice::from_ref(&target), false, || {
        let meta = std::fs::symlink_metadata(&target).map_err(|_| err(Status::NotFound, "no such file"))?;
        let result = if meta.is_dir() {
            if recursive {
                if count_tree(&target) > MAX_DELETE_ENTRIES {
                    return Err(err(Status::PayloadTooLarge, "the folder holds too many entries"));
                }
                // links inside are removed themselves, never followed
                std::fs::remove_dir_all(&target)
            } else {
                std::fs::remove_dir(&target)
                    .map_err(|_| io::Error::new(io::ErrorKind::DirectoryNotEmpty, "the folder is not empty"))
            }
        } else {
            std::fs::remove_file(&target)
        };
        result.map_err(|e| {
            if e.kind() == io::ErrorKind::DirectoryNotEmpty {
                err(
                    Status::Conflict,
                    "the folder is not empty: send `Depth: infinity` to remove it",
                )
            } else {
                err(Status::InternalError, format!("cannot delete: {e}"))
            }
        })?;
        Ok(1)
    })?;
    let _ = bicraft_http::write_empty(stream, Status::NoContent, &[]);
    Ok(Done::Answered)
}

/// Entries below `dir`, counted up to just past [`MAX_DELETE_ENTRIES`] (links not followed).
fn count_tree(dir: &Path) -> usize {
    let mut n = 0usize;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            n = n.saturating_add(1);
            if n > MAX_DELETE_ENTRIES {
                return n;
            }
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                stack.push(e.path());
            }
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_and_paths() {
        assert_eq!(route("/w/a/files"), Some(("a", "")));
        assert_eq!(route("/w/a/files/"), Some(("a", "")));
        assert_eq!(route("/w/a/files/x/y.pbix"), Some(("a", "x/y.pbix")));
        assert_eq!(route("/w/a/filesx"), None);
        assert_eq!(route("/w/../files/x"), None);
        assert_eq!(route("/w/a/mcp"), None);
        assert_eq!(percent_decode("a%20b%2Fc").as_deref(), Some("a b/c"));
        assert_eq!(percent_decode("%C3%A9").as_deref(), Some("é"));
        for bad in ["%", "%2", "%zz", "%FF", "a%0Ab", "%00"] {
            assert!(percent_decode(bad).is_none(), "{bad}");
        }
        assert_eq!(normalize("a//./b\\c/").as_deref(), Some("a/b/c"));
        assert_eq!(normalize(".bicraft-tmp/x"), None);
        assert_eq!(normalize(".BICRAFT-TMP"), None);
        assert_eq!(normalize("x/.bicraft-tmp").as_deref(), Some("x/.bicraft-tmp"));
        assert_eq!(normalize("../x").as_deref(), Some("../x"));
        assert_eq!(normalize("/etc/passwd").as_deref(), Some("/etc/passwd"));
        assert_eq!(normalize("./.bicraft-tmp/x"), None);
    }

    #[test]
    fn headers_of_downloads() {
        assert_eq!(content_type("Sales.PBIX"), "application/octet-stream");
        assert_eq!(content_type("model.bim"), "application/json");
        assert_eq!(content_type("t.tmdl"), "text/plain; charset=utf-8");
        assert_eq!(
            content_disposition("Ventes été.pbix"),
            "attachment; filename=\"Ventes _t_.pbix\"; filename*=UTF-8''Ventes%20%C3%A9t%C3%A9.pbix"
        );
        let d = content_disposition("a\"b\r\n.pbix");
        assert!(!d.contains(['\r', '\n']), "{d}");
        assert_eq!(d.matches('"').count(), 2, "{d}");
        let t = UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        assert_eq!(rfc3339(t).as_deref(), Some("2023-11-14T22:13:20Z"));
        assert_eq!(rfc3339(UNIX_EPOCH).as_deref(), Some("1970-01-01T00:00:00Z"));
        // year boundaries, leap days, a century that is not leap (G37)
        let at = |s: u64| rfc3339(UNIX_EPOCH + std::time::Duration::from_secs(s));
        assert_eq!(at(946_684_799).as_deref(), Some("1999-12-31T23:59:59Z"));
        assert_eq!(at(951_782_400).as_deref(), Some("2000-02-29T00:00:00Z"));
        assert_eq!(at(1_709_251_199).as_deref(), Some("2024-02-29T23:59:59Z"));
        assert_eq!(at(4_107_542_399).as_deref(), Some("2100-02-28T23:59:59Z"));
        assert_eq!(at(4_107_542_400).as_deref(), Some("2100-03-01T00:00:00Z"));
    }
}
