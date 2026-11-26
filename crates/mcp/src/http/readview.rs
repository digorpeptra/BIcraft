//! Read views: what readers see (docs/auth.md, "Roles"; docs/kubernetes.md, "Viewer").
//!
//! A workspace has one editor session (its MCP calls, unsaved edits included). Readers must not
//! see those edits, and a reader opening a file must not replace the editor's file (and its undo
//! history). Each reader therefore reads **the saved state** of a file:
//!
//! - **the editor session itself** while it is clean (nothing unsaved) and has that file open:
//!   no extra memory in the common case. The editor session's lock is held for the whole read,
//!   so an editor's command cannot slip in between the check and the read;
//! - otherwise **one read-only copy per file**, opened from the saved file on disk and shared by
//!   every reader of it ([`HttpConfig::viewer_copies`](super::HttpConfig::viewer_copies) at most,
//!   default 1; past the cap a clear retryable error, never the dirty editor session). A copy is
//!   reloaded when the file changes: every save or upload through the server bumps the
//!   workspace's disk generation, and each acquisition compares the file's fingerprint (paths,
//!   sizes, modification times and, on Unix, inode numbers of the files that make up the model)
//!   with the one it was loaded from. Copies unused for [`COPY_IDLE`] are dropped, and so is a copy
//!   of the file the clean editor session has open. A copy is a session like any other: its memory
//!   counts against the process-wide pool (`BICRAFT_MEMORY_LIMIT`), and each copy has its own
//!   preview cache, so a result computed on the editor's unsaved state can never reach a reader.
//!
//! Which file a reader reads: the one its own `file.open` named (private, per identity), else the
//! file the workspace has open (its saved state). The view state of a browser page (slicers,
//! selections, page) is private per page and lives in the viewer ([`super::viewer`]).

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime};

use bicraft_engine::Session;
use serde_json::Value;

use super::ops::Reopen;
use super::rwgate::{ReadGuard, SessionLock, WriteGuard};
use super::{Queued, State, WorkspaceError, new_session};

/// A copy unused for this long is dropped.
pub(super) const COPY_IDLE: Duration = Duration::from_secs(600);
/// How long a read waits for the editor session or a copy (a long tool call, a copy loading).
const READ_WAIT: Duration = Duration::from_secs(60);
/// Files fingerprinted at most per file or project, and how deep.
const MAX_FINGERPRINT_FILES: usize = 20_000;
const MAX_FINGERPRINT_DEPTH: usize = 16;
/// Readers with a private file at most per workspace (the oldest entry goes beyond).
const MAX_READER_FILES: usize = 1024;
/// Files whose revisions are tracked per workspace at most.
const MAX_FILE_REVISIONS: usize = 4096;

/// The key of a file a session reads ([`Reopen::key`]).
type Key = (String, Option<String>, Option<String>);

/// One read-only copy of a saved file.
pub(super) struct Copy {
    key: Key,
    spec: Reopen,
    /// `None` until loaded (or when the load failed: then `failed` says why).
    session: SessionLock<Option<Session>>,
    failed: Mutex<Option<String>>,
    /// The file's fingerprint and the workspace's disk generation when it was loaded.
    fingerprint: u64,
    generation: u64,
    /// Milliseconds since the server started, at the last use.
    last_used: AtomicU64,
    /// BIcraft's reserved memory taken by the load (the pool's growth while loading).
    pub(super) bytes: AtomicU64,
}

/// The read views of one workspace (they outlive an idle close of its editor session).
pub(super) struct Views {
    /// Bumped when what a viewer shows may have changed (an editor's command, a save, an upload);
    /// viewers' event streams wait on it.
    seq: Mutex<u64>,
    changed: Condvar,
    /// Bumped by every write to the workspace's files through the server (saves, uploads,
    /// deletions): copies loaded before are stale.
    disk_generation: AtomicU64,
    copies: Mutex<Vec<Arc<Copy>>>,
    /// The file each reader opened (`file.open` of a reader), by identity key.
    readers: Mutex<HashMap<String, (Reopen, u64)>>,
    /// The browser pages of this workspace ([`super::viewer`]).
    pub(super) viewers: Mutex<HashMap<String, super::viewer::Viewer>>,
    /// The assets (report images) of the sessions viewers read, by source.
    pub(super) assets: Mutex<HashMap<String, super::viewer::AssetMap>>,
    /// The revision of each workspace file, by workspace-relative path: bumped by every save
    /// through a session of this server, so a session that read an older revision cannot save
    /// over it (`file_conflict`, docs/mcp.md, "Document views").
    files: Mutex<HashMap<String, u64>>,
    /// The UI control channel's relay to this workspace's app pages (`--viewer-control`,
    /// [`super::viewer`]), made on first use.
    pub(super) control: std::sync::OnceLock<bicraft_view::control::Relay>,
}

impl Views {
    pub(super) fn new() -> Self {
        Self {
            control: std::sync::OnceLock::new(),
            seq: Mutex::new(0),
            changed: Condvar::new(),
            disk_generation: AtomicU64::new(0),
            copies: Mutex::new(Vec::new()),
            readers: Mutex::new(HashMap::new()),
            viewers: Mutex::new(HashMap::new()),
            assets: Mutex::new(HashMap::new()),
            files: Mutex::new(HashMap::new()),
        }
    }

    /// What viewers show may have changed: their event streams send a new event.
    pub(super) fn bump(&self) {
        let mut s = self.seq.lock().unwrap_or_else(|e| e.into_inner());
        *s = s.wrapping_add(1);
        self.changed.notify_all();
    }

    /// The workspace's files changed through the server: copies are stale, viewers refresh.
    pub(super) fn disk_changed(&self) {
        self.disk_generation.fetch_add(1, Ordering::SeqCst);
        self.bump();
    }

    /// The disk generation (bumped by [`Views::disk_changed`]).
    pub(super) fn disk_generation(&self) -> u64 {
        self.disk_generation.load(Ordering::SeqCst)
    }

    /// The revision of workspace file `path` (0 until a session of this server saves it).
    pub(super) fn file_rev(&self, path: &str) -> u64 {
        let map = self.files.lock().unwrap_or_else(|e| e.into_inner());
        map.get(path).copied().unwrap_or(0)
    }

    /// Records the file `session` has open as read now (`refresh`: just opened or reopened; else
    /// only when it is another file than the one recorded).
    pub(super) fn note(&self, seen: &Mutex<Option<(String, u64)>>, session: &Session, refresh: bool) {
        let path = Reopen::of(session).map(|r| r.path().to_string());
        let mut seen = seen.lock().unwrap_or_else(|e| e.into_inner());
        match path {
            None => *seen = None,
            Some(p) => {
                if refresh || seen.as_ref().is_none_or(|(q, _)| *q != p) {
                    let rev = self.file_rev(&p);
                    *seen = Some((p, rev));
                }
            }
        }
    }

    /// `session` saved its file: a new revision of it, which `session` has read.
    pub(super) fn saved(&self, seen: &Mutex<Option<(String, u64)>>, session: &Session) {
        let Some(p) = Reopen::of(session).map(|r| r.path().to_string()) else {
            *seen.lock().unwrap_or_else(|e| e.into_inner()) = None;
            return;
        };
        let rev = {
            let mut map = self.files.lock().unwrap_or_else(|e| e.into_inner());
            // bounded: past the cap a new file is not tracked (a missed conflict, never a false one)
            if map.len() >= MAX_FILE_REVISIONS && !map.contains_key(&p) {
                0
            } else {
                let r = map.entry(p.clone()).or_insert(0);
                *r = r.saturating_add(1);
                *r
            }
        };
        *seen.lock().unwrap_or_else(|e| e.into_inner()) = Some((p, rev));
    }

    /// A save by `session` over its file would overwrite a revision it never read (another
    /// session saved the file since): the refusal, nothing is written.
    pub(super) fn conflict(
        &self,
        seen: &Mutex<Option<(String, u64)>>,
        session: &Session,
    ) -> Option<crate::tools::ToolResult> {
        let p = Reopen::of(session)?.path().to_string();
        let (q, read) = seen.lock().unwrap_or_else(|e| e.into_inner()).clone()?;
        let current = self.file_rev(&p);
        (q == p && read != current).then(|| {
            crate::tools::ToolResult::error_with(serde_json::json!({
                "error": format!(
                    "file conflict: {p} was saved by another session (file revision {current}) since
                     this one read it (revision {read}); nothing was saved: save it under another 
                     name, or reopen it (its unsaved edits here are lost)"
                ),
                "code": crate::tools::FILE_CONFLICT,
                "file": p,
                "file_revision": current,
                "read_revision": read,
            }))
        })
    }

    pub(super) fn seq(&self) -> u64 {
        *self.seq.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Waits until [`Views::seq`] differs from `seen`, at most `wait`; returns the current value.
    pub(super) fn wait_change(&self, seen: u64, wait: Duration) -> u64 {
        let guard = self.seq.lock().unwrap_or_else(|e| e.into_inner());
        if *guard != seen {
            return *guard;
        }
        match self.changed.wait_timeout(guard, wait) {
            Ok((g, _)) => *g,
            Err(p) => *p.into_inner().0,
        }
    }

    /// Wakes every waiting event stream (the server stops or drains).
    pub(super) fn wake(&self) {
        let _g = self.seq.lock().unwrap_or_else(|e| e.into_inner());
        self.changed.notify_all();
    }

    /// The file reader `who` opened, if any.
    pub(super) fn reader_file(&self, who: &str) -> Option<Reopen> {
        let map = self.readers.lock().unwrap_or_else(|e| e.into_inner());
        map.get(who).map(|(r, _)| r.clone())
    }

    /// Remembers the file reader `who` opened.
    pub(super) fn set_reader_file(&self, who: &str, spec: Reopen, now_ms: u64) {
        let mut map = self.readers.lock().unwrap_or_else(|e| e.into_inner());
        if map.len() >= MAX_READER_FILES
            && !map.contains_key(who)
            && let Some(oldest) = map.iter().min_by_key(|(_, (_, t))| *t).map(|(k, _)| k.clone())
        {
            map.remove(&oldest);
        }
        map.insert(who.to_string(), (spec, now_ms));
    }

    /// Copies open now, and the memory their loads took.
    pub(super) fn copy_stats(&self) -> (usize, u64) {
        let copies = self.copies.lock().unwrap_or_else(|e| e.into_inner());
        let bytes = copies
            .iter()
            .map(|c| c.bytes.load(Ordering::Relaxed))
            .fold(0u64, u64::saturating_add);
        (copies.len(), bytes)
    }

    /// Drops the copies unused for [`COPY_IDLE`] (and not in use).
    pub(super) fn drop_idle_copies(&self, now_ms: u64) -> usize {
        let idle_ms = u64::try_from(COPY_IDLE.as_millis()).unwrap_or(u64::MAX);
        let dropped: Vec<Arc<Copy>> = {
            let mut copies = self.copies.lock().unwrap_or_else(|e| e.into_inner());
            let (old, keep): (Vec<_>, Vec<_>) = copies.drain(..).partition(|c| {
                now_ms.saturating_sub(c.last_used.load(Ordering::SeqCst)) >= idle_ms && unused(c)
            });
            *copies = keep;
            old
        };
        let n = dropped.len();
        drop(dropped);
        n
    }

    /// Drops the copies that read `targets` (files about to be replaced or removed), so their
    /// file handles close first. `Err` when one of them is in use.
    pub(super) fn drop_copies_touching(&self, ws: &Path, targets: &[PathBuf]) -> Result<(), ()> {
        let dropped: Vec<Arc<Copy>> = {
            let mut copies = self.copies.lock().unwrap_or_else(|e| e.into_inner());
            let touching = |c: &Arc<Copy>| targets.iter().any(|t| c.spec.touches(ws, t));
            if copies.iter().any(|c| touching(c) && !unused(c)) {
                return Err(());
            }
            let (gone, keep): (Vec<_>, Vec<_>) = copies.drain(..).partition(touching);
            *copies = keep;
            gone
        };
        drop(dropped);
        Ok(())
    }

    /// Drops the copy with `key` when nobody uses it (the clean editor session serves that file).
    fn drop_copy(&self, key: &Key) {
        let gone: Vec<Arc<Copy>> = {
            let mut copies = self.copies.lock().unwrap_or_else(|e| e.into_inner());
            let (gone, keep): (Vec<_>, Vec<_>) = copies.drain(..).partition(|c| &c.key == key && unused(c));
            *copies = keep;
            gone
        };
        drop(gone);
    }
}

/// Nobody holds the copy (its session lock is free and only the list refers to it).
fn unused(c: &Arc<Copy>) -> bool {
    Arc::strong_count(c) == 1 && c.session.is_free()
}

/// Which file a read is about.
#[derive(Clone)]
pub(super) enum Target {
    /// The file the workspace has open (its saved state).
    Follow,
    /// A file a reader opened.
    File(Reopen),
}

/// Which session served a read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Served {
    /// The editor session (clean, or the caller is an editor).
    Editor,
    /// A read-only copy of the saved file.
    Copy,
    /// Nothing saved to read (an empty session).
    Empty,
}

impl Served {
    pub(super) fn name(&self) -> &'static str {
        match self {
            Served::Editor => "editor",
            Served::Copy => "copy",
            Served::Empty => "empty",
        }
    }
}

/// Why a read could not run.
pub(super) enum ViewError {
    /// Retryable: the editor session or a copy is busy, or every copy is in use (503).
    Busy(&'static str),
    /// The workspace cannot be opened, or the saved file cannot be loaded.
    Failed(String),
}

impl State {
    /// The read views of workspace `id` (created on first use; the caller has checked the
    /// workspace exists and the caller may read it).
    pub(super) fn views(&self, id: &str) -> Arc<Views> {
        let mut map = self.views.lock().unwrap_or_else(|e| e.into_inner());
        Arc::clone(
            map.entry(id.to_string())
                .or_insert_with(|| Arc::new(Views::new())),
        )
    }

    /// The views of `id` when they exist (nobody read it yet: `None`).
    pub(super) fn views_if_any(&self, id: &str) -> Option<Arc<Views>> {
        self.views
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .map(Arc::clone)
    }

    /// Runs `f` on the editor session of `id` (opened on first use, its pending reopen applied),
    /// holding a queue place and the session's read side: viewer reads share the session with
    /// the read-only tool calls, and an editing call waits for them (`rwgate.rs`).
    pub(super) fn with_editor<R>(
        &self,
        id: &str,
        dir: &Path,
        f: impl FnOnce(&Session) -> R,
    ) -> Result<R, ViewError> {
        let (workspace, _place) = self.checkout(id, dir).map_err(workspace_error)?;
        let _queued = Queued(&workspace.pending);
        self.reopen_first(id, &workspace)?;
        let session = workspace
            .session
            .read_within(Some(READ_WAIT))
            .ok_or(ViewError::Busy(
                "the workspace is busy with a tool call; try again",
            ))?;
        let _reading = super::ops::Active::enter(&self.metrics.reads);
        workspace.last_used.store(self.now_ms(), Ordering::SeqCst);
        let r = f(&session);
        workspace.last_used.store(self.now_ms(), Ordering::SeqCst);
        Ok(r)
    }

    /// Runs `f` on a document view's own session `doc` (its pending reopen applied first),
    /// holding a place in its queue and its read side, as [`State::with_editor`] does for the
    /// workspace's session.
    pub(super) fn with_doc<R>(
        &self,
        id: &str,
        doc: &super::Workspace,
        f: impl FnOnce(&Session) -> R,
    ) -> Result<R, ViewError> {
        doc.pending.fetch_add(1, Ordering::SeqCst);
        let _queued = Queued(&doc.pending);
        self.reopen_first(id, doc)?;
        let session = doc
            .session
            .read_within(Some(READ_WAIT))
            .ok_or(ViewError::Busy("this view is busy with a command; try again"))?;
        let _reading = super::ops::Active::enter(&self.metrics.reads);
        doc.last_used.store(self.now_ms(), Ordering::SeqCst);
        let r = f(&session);
        doc.last_used.store(self.now_ms(), Ordering::SeqCst);
        Ok(r)
    }

    /// Applies a pending reopen under the session's write side (before a read takes the read
    /// side): what a tool call does first.
    fn reopen_first(&self, id: &str, workspace: &super::Workspace) -> Result<(), ViewError> {
        let pending = workspace
            .reopen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some();
        if pending {
            let mut session = workspace
                .session
                .write_within(Some(READ_WAIT))
                .ok_or(ViewError::Busy(
                    "the workspace is busy with a tool call; try again",
                ))?;
            self.apply_reopen(id, workspace, &mut session);
        }
        Ok(())
    }

    /// Runs `f` on the session that shows `target`'s saved state to a reader (see the module
    /// docs), and says which one it was. `f` gets the session shared (its read side: other reads
    /// run beside it), or alone when `exclusive` (a call that must run alone, e.g. a read with an
    /// explicit `cache_abf`).
    pub(super) fn with_read_view<R>(
        &self,
        id: &str,
        dir: &Path,
        target: &Target,
        exclusive: bool,
        f: impl FnOnce(ViewSession<'_>, &Served) -> R,
    ) -> Result<(R, Served), ViewError> {
        let views = self.views(id);
        let now = self.now_ms();
        views.drop_idle_copies(now);
        let busy = || ViewError::Busy("the workspace is busy with a tool call; try again");
        // 1. the editor session, when it is clean and has the wanted file open: read under its
        //    read side (an editor's command waits until the read is over)
        let mut f = Some(f);
        let wanted: Option<Reopen> = {
            let (workspace, _place) = self.checkout(id, dir).map_err(workspace_error)?;
            let _queued = Queued(&workspace.pending);
            self.reopen_first(id, &workspace)?;
            let mut guard = if exclusive {
                Held::Write(workspace.session.write_within(Some(READ_WAIT)).ok_or_else(busy)?)
            } else {
                Held::Read(workspace.session.read_within(Some(READ_WAIT)).ok_or_else(busy)?)
            };
            let session = guard.get();
            let open = Reopen::of(session);
            let clean = !session.dirty && !session.report_dirty;
            let wanted = match target {
                Target::Follow => open.clone(),
                Target::File(spec) => Some(spec.clone()),
            };
            let same = match (&wanted, &open) {
                (Some(w), Some(o)) => w.key() == o.key(),
                (None, None) => session.source.is_none(),
                _ => false,
            };
            if clean && same {
                if let Some(o) = &open {
                    views.drop_copy(&o.key());
                }
                workspace.last_used.store(self.now_ms(), Ordering::SeqCst);
                let served = Served::Editor;
                let _reading = (!exclusive).then(|| super::ops::Active::enter(&self.metrics.reads));
                let r = f.take().map(|f| f(guard.view(), &served));
                return r
                    .map(|r| (r, served))
                    .ok_or(ViewError::Failed("internal error".into()));
            }
            wanted
        };
        let Some(f) = f.take() else {
            return Err(ViewError::Failed("internal error".into()));
        };
        // 2. nothing saved to read: an empty session (a model.new never saved, nothing open)
        let Some(spec) = wanted else {
            let mut empty = new_session(dir, self.config.allow_network).map_err(ViewError::Failed)?;
            let served = Served::Empty;
            let r = f(ViewSession::Alone(&mut empty), &served);
            return Ok((r, served));
        };
        // 3. the shared read-only copy of the saved file, loaded once under its write side
        let copy = self.copy_for(id, dir, &views, spec)?;
        let loading = || ViewError::Busy("the saved file is still loading; try again");
        let loaded = copy
            .session
            .read_within(Some(READ_WAIT))
            .ok_or_else(loading)?
            .is_some();
        if !loaded {
            let mut guard = copy.session.write_within(Some(READ_WAIT)).ok_or_else(loading)?;
            if guard.is_none() {
                let failed = copy.failed.lock().unwrap_or_else(|e| e.into_inner()).clone();
                if let Some(why) = failed {
                    return Err(ViewError::Failed(why));
                }
                self.load_copy(id, dir, &views, &copy, &mut guard)?;
            }
        }
        let mut guard = if exclusive {
            HeldCopy::Write(copy.session.write_within(Some(READ_WAIT)).ok_or_else(loading)?)
        } else {
            HeldCopy::Read(copy.session.read_within(Some(READ_WAIT)).ok_or_else(loading)?)
        };
        copy.last_used.store(self.now_ms(), Ordering::SeqCst);
        let served = Served::Copy;
        let _reading = (!exclusive).then(|| super::ops::Active::enter(&self.metrics.reads));
        let unloaded = || ViewError::Failed("the saved file could not be loaded".into());
        let r = match &mut guard {
            HeldCopy::Read(g) => f(ViewSession::Shared(g.as_ref().ok_or_else(unloaded)?), &served),
            HeldCopy::Write(g) => f(ViewSession::Alone(g.as_mut().ok_or_else(unloaded)?), &served),
        };
        copy.last_used.store(self.now_ms(), Ordering::SeqCst);
        Ok((r, served))
    }

    /// The pending reopen of an idle-closed (or upload-closed) workspace, applied to its fresh
    /// session (what a tool call does first).
    fn apply_reopen(&self, id: &str, workspace: &super::Workspace, session: &mut Session) {
        let reopen = workspace.reopen.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(r) = reopen {
            let ok = r.apply(session);
            let counter = if ok {
                &self.metrics.reopens
            } else {
                self.log(
                    "warn",
                    "reopen",
                    &format!("workspace {id}: its file could not be reopened after an idle close"),
                );
                &self.metrics.reopen_failures
            };
            counter.fetch_add(1, Ordering::Relaxed);
            let views = self.views(id);
            // the reopened file is read as it is on disk now
            views.note(&workspace.file_seen, session, true);
            views.bump();
        }
    }

    /// The copy of `spec`'s saved file: the current one when it is fresh, else a new (empty)
    /// entry to load. Busy when every copy slot holds another file in use.
    fn copy_for(&self, id: &str, dir: &Path, views: &Views, spec: Reopen) -> Result<Arc<Copy>, ViewError> {
        let key = spec.key();
        let fingerprint = fingerprint(dir, &spec);
        let generation = views.disk_generation.load(Ordering::SeqCst);
        let now = self.now_ms();
        let stale: Vec<Arc<Copy>>;
        let copy = {
            let mut copies = views.copies.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(c) = copies
                .iter()
                .find(|c| c.key == key && c.fingerprint == fingerprint && c.generation == generation)
            {
                c.last_used.store(now, Ordering::SeqCst);
                return Ok(Arc::clone(c));
            }
            // an older load of the same file: out of the list (a read in progress finishes on it)
            let (old, keep): (Vec<_>, Vec<_>) = copies.drain(..).partition(|c| c.key == key);
            *copies = keep;
            stale = old;
            let cap = self.config.viewer_copies.max(1);
            if copies.len() >= cap {
                // the least recently used copy nobody holds makes room
                let victim = copies
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| unused(c))
                    .min_by_key(|(_, c)| c.last_used.load(Ordering::SeqCst))
                    .map(|(i, _)| i);
                match victim {
                    Some(i) => {
                        let gone = copies.remove(i);
                        drop(gone);
                    }
                    None => {
                        self.metrics.copy_busy.fetch_add(1, Ordering::Relaxed);
                        self.log(
                            "warn",
                            "copy_busy",
                            &format!(
                                "workspace {id}: every saved-file copy is in use (--viewer-copies {cap}); \
                                 a reader was asked to retry"
                            ),
                        );
                        return Err(ViewError::Busy(
                            "every read-only copy of a saved file is in use; try again shortly",
                        ));
                    }
                }
            }
            let c = Arc::new(Copy {
                key,
                spec,
                session: SessionLock::new(None, self.config.max_concurrent_reads),
                failed: Mutex::new(None),
                fingerprint,
                generation,
                last_used: AtomicU64::new(now),
                bytes: AtomicU64::new(0),
            });
            copies.push(Arc::clone(&c));
            c
        };
        drop(stale);
        Ok(copy)
    }

    /// Loads `copy` (its session lock is held by the caller): a server-mode session that opens
    /// the saved file. A failure removes the copy and is reported to every waiting reader.
    fn load_copy(
        &self,
        id: &str,
        dir: &Path,
        views: &Views,
        copy: &Arc<Copy>,
        guard: &mut WriteGuard<'_, Option<Session>>,
    ) -> Result<(), ViewError> {
        let started = Instant::now();
        let before = bicraft_mem::stats().used;
        let opened = new_session(dir, self.config.allow_network)
            .and_then(|mut s| copy.spec.clone().apply_result(&mut s).map(|_| s));
        let took = started.elapsed();
        match opened {
            Ok(session) => {
                let grew = bicraft_mem::stats().used.saturating_sub(before);
                copy.bytes
                    .store(u64::try_from(grew).unwrap_or(u64::MAX), Ordering::Relaxed);
                **guard = Some(session);
                self.metrics.copy_loads.fetch_add(1, Ordering::Relaxed);
                self.log(
                    "info",
                    "copy_load",
                    &format!(
                        "workspace {id}: loaded a read-only copy of a saved file for readers in {} ms \
                         ({} reserved)",
                        took.as_millis(),
                        bicraft_mem::size_text(grew)
                    ),
                );
                Ok(())
            }
            Err(e) => {
                self.metrics.copy_load_failures.fetch_add(1, Ordering::Relaxed);
                *copy.failed.lock().unwrap_or_else(|p| p.into_inner()) = Some(e.clone());
                {
                    let mut copies = views.copies.lock().unwrap_or_else(|p| p.into_inner());
                    copies.retain(|c| !Arc::ptr_eq(c, copy));
                }
                self.log(
                    "warn",
                    "copy_load",
                    &format!("workspace {id}: a read-only copy of a saved file could not be loaded"),
                );
                Err(ViewError::Failed(format!("the saved file cannot be read: {e}")))
            }
        }
    }
}

fn workspace_error(e: WorkspaceError) -> ViewError {
    match e {
        WorkspaceError::TooMany => ViewError::Busy("too many open workspaces; try again later"),
        WorkspaceError::Session(why) => ViewError::Failed(format!("the workspace cannot be opened: {why}")),
    }
}

/// A fingerprint of the files `spec` reads: path, size, modification time (and inode on Unix)
/// of the file, of a project's item folders' files, and of the stored-data file. A file
/// replaced by rename (saves, uploads) changes it even with an equal size and time.
pub(super) fn fingerprint(dir: &Path, spec: &Reopen) -> u64 {
    let (path, _, cache_abf) = spec.key();
    let mut roots = vec![dir.join(&path)];
    let main = dir.join(&path);
    if let (Some(parent), Some(stem)) = (main.parent(), main.file_stem().and_then(|s| s.to_str()))
        && main
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("pbip"))
    {
        roots.push(parent.join(format!("{stem}.SemanticModel")));
        roots.push(parent.join(format!("{stem}.Report")));
    }
    if let Some(c) = cache_abf {
        roots.push(dir.join(c));
    }
    let mut files = Vec::new();
    for r in &roots {
        collect(r, 0, &mut files);
    }
    files.sort();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    files.hash(&mut h);
    h.finish()
}

fn collect(path: &Path, depth: usize, out: &mut Vec<(PathBuf, u64, u128, u64)>) {
    if out.len() >= MAX_FINGERPRINT_FILES {
        return;
    }
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        out.push((path.to_path_buf(), u64::MAX, 0, 0));
        return;
    };
    if meta.is_dir() {
        if depth >= MAX_FINGERPRINT_DEPTH {
            return;
        }
        let Ok(entries) = std::fs::read_dir(path) else {
            return;
        };
        let mut children: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        children.sort();
        for child in children {
            // Desktop's local cache and VCS metadata do not make up the model
            let skip = child
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.eq_ignore_ascii_case(".git"));
            if !skip {
                collect(&child, depth.saturating_add(1), out);
            }
            if out.len() >= MAX_FINGERPRINT_FILES {
                return;
            }
        }
        return;
    }
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    out.push((path.to_path_buf(), meta.len(), modified, inode(&meta)));
}

#[cfg(unix)]
fn inode(meta: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    meta.ino()
}

#[cfg(not(unix))]
fn inode(_meta: &std::fs::Metadata) -> u64 {
    0
}

/// The identity key of a reader's private state: the mode, the issuer and the subject (never a
/// credential).
pub(super) fn identity_key(identity: &bicraft_auth::Identity) -> String {
    match &identity.source {
        bicraft_auth::Source::Oidc { issuer } => format!("oidc\u{1}{issuer}\u{1}{}", identity.subject),
        bicraft_auth::Source::Proxy => format!("proxy\u{1}{}", identity.subject),
        bicraft_auth::Source::Token => format!("token\u{1}{}", identity.subject),
    }
}

/// The `file.open` parameters of a reader's call, when it opens a file: `Ok(None)` for a call
/// that opens nothing, `Err` for a batch that opens a file among other steps (a reader's open is
/// private and runs as a call of its own).
pub(super) fn open_params(name: &str, args: &Value) -> Result<Option<Value>, String> {
    let is_open = |id: &str| id == "file.open";
    match name {
        "run_command" => {
            let id = args.get("command").and_then(Value::as_str).unwrap_or_default();
            Ok(is_open(id).then(|| {
                args.get("params")
                    .cloned()
                    .unwrap_or_else(|| Value::Object(Default::default()))
            }))
        }
        "batch" => {
            let steps = args.get("commands").and_then(Value::as_array);
            let opens = steps
                .into_iter()
                .flatten()
                .any(|c| c.get("command").and_then(Value::as_str).is_some_and(is_open));
            if opens {
                Err(
                    "a reader's `file.open` runs as a call of its own (not in a batch): it opens the \
                     file privately for this reader, without changing the workspace's session"
                        .into(),
                )
            } else {
                Ok(None)
            }
        }
        tool => Ok(crate::tools::tool_command(tool)
            .filter(|c| is_open(c))
            .map(|_| args.clone())),
    }
}

/// Copies open in every workspace, and the memory their loads took (`/metrics`).
pub(super) fn copy_totals(state: &State) -> (usize, u64) {
    let views: Vec<Arc<Views>> = state
        .views
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .map(Arc::clone)
        .collect();
    views
        .iter()
        .map(|v| v.copy_stats())
        .fold((0, 0), |(n, b), (vn, vb)| {
            (n.saturating_add(vn), b.saturating_add(vb))
        })
}

/// The session a read view hands to its caller.
pub(super) enum ViewSession<'a> {
    /// Shared with other reads (its read side).
    Shared(&'a Session),
    /// Alone (its write side, or a private session).
    Alone(&'a mut Session),
}

impl ViewSession<'_> {
    pub(super) fn get(&self) -> &Session {
        match self {
            ViewSession::Shared(s) => s,
            ViewSession::Alone(s) => s,
        }
    }
}

/// The editor session as a read view holds it.
enum Held<'a> {
    Read(ReadGuard<'a, Session>),
    Write(WriteGuard<'a, Session>),
}

impl Held<'_> {
    fn get(&self) -> &Session {
        match self {
            Held::Read(g) => g,
            Held::Write(g) => g,
        }
    }

    fn view(&mut self) -> ViewSession<'_> {
        match self {
            Held::Read(g) => ViewSession::Shared(g),
            Held::Write(g) => ViewSession::Alone(g),
        }
    }
}

/// A copy's session as a read holds it.
enum HeldCopy<'a> {
    Read(ReadGuard<'a, Option<Session>>),
    Write(WriteGuard<'a, Option<Session>>),
}
