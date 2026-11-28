//! Auto-recovery: a journal of the commands applied since the last save (`docs/recovery.md`).
//!
//! **Off by default.** A host turns it on with [`Session::set_recovery`] and a per-user folder
//! ([`RecoveryConfig::default_dir`]: `%LOCALAPPDATA%\bicraft\recovery` on Windows,
//! `$XDG_STATE_HOME/bicraft/recovery` elsewhere; `bicraft-cli serve` uses its own `--recovery-dir`).
//! The journal is never written next to the user's file.
//!
//! **The journal.** Every successful command that edits the open file and whose registry row
//! says [`Replay::Exact`] appends one JSON line `{seq, at_ms, rev, cmd, params, guids}` (the
//! GUIDs the command generated, so a replay gives the same lineage tags and names). An atomic
//! batch is one line (`batch`), written when it commits. Lines are written with one `write` each
//! and synced to disk every [`RecoveryConfig::sync_every`] lines or
//! [`RecoveryConfig::sync_interval`], whichever comes first. The first line is a header with the
//! file's path and its identity at open or last save (size and modification time; for a PBIP
//! project, a hash of the names, sizes and times of its item files, `.pbi` left out).
//!
//! **Recovery copies.** A command with external inputs ([`Replay::Copy`]: imports, refresh, a
//! theme file), an undo or redo that goes back past the journal's start, and a journal past
//! [`RecoveryConfig::max_bytes`] write a full copy of the session's model and report instead
//! (`<journal>.copy.json`, the session's own JSON form) and restart the journal after it. The
//! journal never grows beyond its cap.
//!
//! **Lifetime.** A save deletes the journal (the file holds the edits); so does a session whose
//! edits are all undone (clean again), [`Session::recovery_close`] and `recovery.discard`. A
//! crash, a dropped session or opening another file over unsaved edits leaves it: the next
//! `file.open` of that file offers it (`recovery.list`, the snapshot's `recovery.offers`), and
//! `recovery.apply` replays it through [`Session::execute`], stopping at the first failure.
//! A live session holds an exclusive lock on its journal, so another session never offers it.
//!
//! **Privacy.** Parameters are the user's own DAX, M, names and values. They stay in the recovery
//! folder on the user's machine (or the server's); BIcraft never logs or sends them elsewhere.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bicraft_model::SemanticModel;
use bicraft_report::Report;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::params::{allow, opt_bool, opt_str};
use crate::{Session, Source, find_command, now_ms};

/// How a command is journaled: every row of [`crate::COMMANDS`] says it, next to its `server`
/// policy, so a new command has to decide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Replay {
    /// Its parameters (and the GUIDs it generates) are enough to run it again with the same
    /// result: journaled as one line.
    Exact,
    /// It reads outside inputs (a file, a CSV, the network): a full recovery copy is written
    /// after it instead of a line.
    Copy,
    /// It does not edit the open file (read-only commands, view settings, caches, exports):
    /// not journaled.
    Skip,
    /// It changes the journal's base: opens, saves or replaces the file, or manages recovery
    /// itself.
    Base,
}

impl Replay {
    pub fn name(self) -> &'static str {
        match self {
            Replay::Exact => "exact",
            Replay::Copy => "copy",
            Replay::Skip => "skip",
            Replay::Base => "base",
        }
    }
}

/// Journal format version (the header's `bicraft_recovery`).
pub const JOURNAL_VERSION: u64 = 1;
/// Default cap of a journal file before a recovery copy replaces it.
pub const DEFAULT_MAX_JOURNAL_BYTES: u64 = 4 << 20;
/// Default number of lines written between two syncs to disk.
pub const DEFAULT_SYNC_EVERY: u32 = 256;
/// Default longest time between a line and its sync to disk.
pub const DEFAULT_SYNC_INTERVAL: Duration = Duration::from_secs(1);
/// Environment variable naming the recovery folder (overrides the per-user default).
pub const RECOVERY_DIR_ENV: &str = "BICRAFT_RECOVERY_DIR";

/// Largest journal read back.
const MAX_READ_BYTES: u64 = 256 << 20;
/// Largest recovery copy read back.
const MAX_COPY_BYTES: u64 = 4 << 30;
/// Most journal files a listing looks at.
const MAX_LISTED: usize = 1000;
/// Most files hashed for a project's identity, and the deepest folder walked.
const MAX_WALK: usize = 200_000;
const MAX_DEPTH: usize = 32;
/// Most steps replayed from one journal.
const MAX_STEPS: usize = 1_000_000;
const JOURNAL_EXT: &str = ".jsonl";
const COPY_EXT: &str = ".copy.json";

/// Where and how a session journals ([`Session::set_recovery`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryConfig {
    /// The recovery folder (created on the first journal line).
    pub dir: PathBuf,
    /// Whose journals these are (a server's user); listings show only the same owner's.
    pub owner: Option<String>,
    /// Cap of a journal file: past it, a recovery copy replaces the lines.
    pub max_bytes: u64,
    /// Lines between two syncs to disk.
    pub sync_every: u32,
    /// Longest time a line waits for its sync.
    pub sync_interval: Duration,
}

impl RecoveryConfig {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            owner: None,
            max_bytes: DEFAULT_MAX_JOURNAL_BYTES,
            sync_every: DEFAULT_SYNC_EVERY,
            sync_interval: DEFAULT_SYNC_INTERVAL,
        }
    }

    pub fn with_owner(mut self, owner: Option<String>) -> Self {
        self.owner = owner;
        self
    }

    /// The owner as a hash of `identity` (a server's user key): journals never hold the user's
    /// name or id.
    pub fn with_owner_of(mut self, identity: &str) -> Self {
        self.owner = Some(format!("u{:016x}", fnv(identity.as_bytes(), FNV_START)));
        self
    }

    pub fn with_max_bytes(mut self, max: u64) -> Self {
        self.max_bytes = max;
        self
    }

    /// The per-user recovery folder: `BICRAFT_RECOVERY_DIR` when set, else
    /// `%LOCALAPPDATA%\bicraft\recovery` on Windows, `$XDG_STATE_HOME/bicraft/recovery` (else
    /// `~/.local/state/bicraft/recovery`) elsewhere. `None` when none of them is known.
    pub fn default_dir() -> Option<PathBuf> {
        if let Some(d) = bicraft_env::var_os(RECOVERY_DIR_ENV).filter(|d| !d.is_empty()) {
            return Some(PathBuf::from(d));
        }
        let base = if cfg!(windows) {
            bicraft_env::var_os("LOCALAPPDATA")
                .filter(|d| !d.is_empty())
                .map(PathBuf::from)
        } else {
            bicraft_env::var_os("XDG_STATE_HOME")
                .filter(|d| !d.is_empty())
                .map(PathBuf::from)
                .or_else(|| {
                    bicraft_env::var_os("HOME")
                        .filter(|d| !d.is_empty())
                        .map(|h| PathBuf::from(h).join(".local").join("state"))
                })
        };
        base.map(|b| b.join("bicraft").join("recovery"))
    }
}

// --- GUID tape ------------------------------------------------------------------------------

#[derive(Default)]
struct Tape {
    /// GUIDs to hand out first (a replay).
    play: VecDeque<String>,
    /// GUIDs handed out while a journaled command runs.
    record: Option<Vec<String>>,
}

thread_local! {
    static TAPE: RefCell<Tape> = RefCell::new(Tape::default());
}

/// A new GUID: the next recorded one during a replay, else `fresh()`; recorded while a journaled
/// command runs (`edit::new_guid`).
pub(crate) fn taped_guid(fresh: impl FnOnce() -> String) -> String {
    let played = TAPE.with(|t| t.borrow_mut().play.pop_front());
    let g = played.unwrap_or_else(fresh);
    TAPE.with(|t| {
        if let Some(r) = t.borrow_mut().record.as_mut()
            && r.len() < 1_000_000
        {
            r.push(g.clone());
        }
    });
    g
}

fn tape_record_start() -> Option<Vec<String>> {
    TAPE.with(|t| t.borrow_mut().record.replace(Vec::new()))
}

fn tape_record_take(outer: Option<Vec<String>>) -> Vec<String> {
    TAPE.with(|t| {
        let mut t = t.borrow_mut();
        let mine = t.record.take().unwrap_or_default();
        // an enclosing recording (a nested run) keeps what this one generated too
        t.record = outer.map(|mut o| {
            o.extend(mine.iter().cloned());
            o
        });
        mine
    })
}

fn tape_play(guids: &[String]) {
    TAPE.with(|t| t.borrow_mut().play = guids.iter().cloned().collect());
}

fn tape_play_clear() {
    TAPE.with(|t| t.borrow_mut().play.clear());
}

// --- identity -------------------------------------------------------------------------------

/// What the journal's base file was at open or last save.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Identity {
    kind: String,
    size: u64,
    mtime_ms: u64,
    files: u64,
    hash: String,
}

/// FNV-1a, 64 bits: stable across runs and platforms (file names and fingerprints).
fn fnv(bytes: &[u8], mut h: u64) -> u64 {
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

const FNV_START: u64 = 0xcbf2_9ce4_8422_2325;

fn mtime_ms(m: &std::fs::Metadata) -> u64 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn file_identity(path: &Path, kind: &str) -> Result<Identity, String> {
    let m = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Identity {
        kind: kind.into(),
        size: m.len(),
        mtime_ms: mtime_ms(&m),
        files: 1,
        hash: String::new(),
    })
}

/// Every file under `dir` but `.pbi` folders: (path relative to `root`, size, mtime).
fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<(String, u64, u64)>) -> Result<(), String> {
    if depth > MAX_DEPTH || out.len() >= MAX_WALK {
        return Ok(());
    }
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for e in entries.flatten() {
        if out.len() >= MAX_WALK {
            break;
        }
        let Ok(ft) = e.file_type() else { continue };
        let p = e.path();
        if ft.is_dir() {
            if e.file_name() == ".pbi" {
                continue;
            }
            walk(root, &p, depth.saturating_add(1), out)?;
        } else if ft.is_file()
            && let Ok(m) = e.metadata()
        {
            let rel = p
                .strip_prefix(root)
                .unwrap_or(&p)
                .to_string_lossy()
                .replace('\\', "/");
            out.push((rel, m.len(), mtime_ms(&m)));
        }
    }
    Ok(())
}

fn project_identity(project: &bicraft_pbip::Project) -> Result<Identity, String> {
    let mut files = Vec::new();
    for dir in [&project.semantic_model, &project.report].into_iter().flatten() {
        walk(&project.root, dir, 0, &mut files)?;
    }
    files.sort();
    let mut h = FNV_START;
    let (mut size, mut newest) = (0u64, 0u64);
    for (rel, len, t) in &files {
        h = fnv(rel.as_bytes(), h);
        h = fnv(&len.to_le_bytes(), h);
        h = fnv(&t.to_le_bytes(), h);
        size = size.saturating_add(*len);
        newest = newest.max(*t);
    }
    Ok(Identity {
        kind: "project".into(),
        size,
        mtime_ms: newest,
        files: u64::try_from(files.len()).unwrap_or(u64::MAX),
        hash: format!("{h:016x}"),
    })
}

/// The key path of an open file (one journal name per file, however it was opened): the
/// absolute file path; for a project, its semantic model folder (else its report or root).
fn key_path(source: &Source) -> PathBuf {
    let p = match source {
        Source::Package { path, .. } | Source::Bim { path } => path.clone(),
        Source::Project { project, .. } => project
            .semantic_model
            .clone()
            .or_else(|| project.report.clone())
            .unwrap_or_else(|| project.root.clone()),
    };
    std::path::absolute(&p).unwrap_or(p)
}

fn display_path(source: &Source) -> PathBuf {
    let p = crate::commands::source_path(source).to_path_buf();
    std::path::absolute(&p).unwrap_or(p)
}

fn identity(source: &Source) -> Result<Identity, String> {
    match source {
        Source::Package { path, .. } => file_identity(path, "package"),
        Source::Bim { path } => file_identity(path, "bim"),
        Source::Project { project, .. } => project_identity(project),
    }
}

/// The journal name prefix of a key path.
fn key_of(path: &Path) -> String {
    let s = path.to_string_lossy();
    let s = if cfg!(windows) {
        s.to_lowercase()
    } else {
        s.into_owned()
    };
    format!("{:016x}", fnv(s.as_bytes(), FNV_START))
}

/// The journal name prefix of a model never saved (`model.new`, an import read into a new
/// model): no file to key on, so every such journal shares it.
fn untitled_key() -> String {
    format!("{:016x}", fnv(b"bicraft:untitled", FNV_START))
}

/// The identity kind of a never-saved model's journal.
const UNTITLED: &str = "untitled";

/// The view settings kept with the edits: the visual text language, the captions' culture, the
/// as-of date and the reader's page (`view.*` commands, never written to the file).
const VIEW_COMMANDS: &[&str] = &[
    "view.set_language",
    "view.set_culture",
    "view.set_as_of",
    "view.interact",
];

fn view_json(s: &Session) -> Value {
    json!({
        "ui_language": s.ui_language.tag(),
        "culture": s.display_culture,
        "as_of": s.as_of.map(crate::render::as_of_text),
        "page": s.interactions.current_page(),
    })
}

/// A new session id (unique per process run and per base change).
fn new_sid() -> String {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let mut h = fnv(&std::process::id().to_le_bytes(), FNV_START);
    h = fnv(&n.to_le_bytes(), h);
    h = fnv(&nanos.to_le_bytes(), h);
    format!("{h:016x}")
}

/// Whether `id` is a journal name (`<16 hex>-<16 hex>`): never a path.
fn valid_id(id: &str) -> bool {
    let b = id.as_bytes();
    b.len() == 33
        && b.get(16) == Some(&b'-')
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 16 || c.is_ascii_digit() || (b'a'..=b'f').contains(c))
}

// --- session state --------------------------------------------------------------------------

/// The open file the journal is about (a never-saved model: no path, [`untitled_key`]).
#[derive(Debug, Clone)]
struct Base {
    path: PathBuf,
    key: String,
    identity: Identity,
}

impl Base {
    fn untitled(&self) -> bool {
        self.identity.kind == UNTITLED
    }
}

#[derive(Debug)]
struct Writer {
    file: File,
    path: PathBuf,
    bytes: u64,
    /// Commands journaled since the start or the last copy.
    edits: usize,
    unsynced: u32,
    last_sync: Instant,
    copy: bool,
}

impl Drop for Writer {
    /// A session that goes (a crash of the host aside) leaves its lines on disk.
    fn drop(&mut self) {
        if self.unsynced > 0 {
            let _ = self.file.sync_data();
        }
    }
}

/// One buffered step of an atomic batch.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Step {
    cmd: String,
    params: Value,
}

/// The session's journal state.
#[derive(Debug, Default)]
pub(crate) struct Recovery {
    cfg: Option<RecoveryConfig>,
    sid: String,
    base: Option<Base>,
    writer: Option<Writer>,
    seq: u64,
    /// History entries numbered up to this one predate the journal's start.
    floor: u64,
    /// The steps of the atomic batch in progress.
    tx: Option<Vec<(Step, Vec<String>)>>,
    tx_copy: bool,
    tx_base: bool,
    /// Journals of the open file left by other sessions, found at open.
    offers: Vec<Value>,
    /// The last journal write error (the snapshot shows it).
    error: Option<String>,
    /// The view settings last written to the journal (header or line).
    view: Option<Value>,
}

impl Recovery {
    fn name(&self) -> Option<String> {
        self.base.as_ref().map(|b| format!("{}-{}", b.key, self.sid))
    }
}

/// What [`before`] saw.
pub(crate) struct Pre {
    replay: Replay,
    crosses: bool,
    was_dirty: bool,
    outer: Option<Option<Vec<String>>>,
}

fn dirty(s: &Session) -> bool {
    s.dirty || s.report_dirty
}

fn untitled_base(s: &Session) -> bool {
    s.recovery.base.as_ref().is_some_and(Base::untitled)
}

/// A never-saved model has no file to replay lines onto: its first journaled edit is a copy.
fn copy_first(s: &Session) -> bool {
    untitled_base(s) && s.recovery.writer.is_none()
}

fn steps_of(p: &Value) -> usize {
    p.get("steps")
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(1)
}

/// Called by [`Session::execute`] before a command runs: `None` when nothing is journaled.
pub(crate) fn before(s: &mut Session, id: &str, params: &Value) -> Option<Pre> {
    s.recovery.cfg.as_ref()?;
    let replay = find_command(id)?.replay;
    if replay == Replay::Skip {
        // a view setting goes into a journal that exists
        return (VIEW_COMMANDS.contains(&id) && s.recovery.writer.is_some()).then(|| Pre {
            replay,
            crosses: false,
            was_dirty: dirty(s),
            outer: None,
        });
    }
    let journaled = matches!(replay, Replay::Exact | Replay::Copy) && s.recovery.base.is_some();
    let crosses = match id {
        "edit.undo" => crate::history::crosses_floor(s, true, steps_of(params), s.recovery.floor),
        "edit.redo" => crate::history::crosses_floor(s, false, steps_of(params), s.recovery.floor),
        // an image read from a file: a journal line could not replay it
        _ => crate::rich::reads_file(id, params),
    };
    Some(Pre {
        replay,
        crosses,
        was_dirty: dirty(s),
        outer: journaled.then(tape_record_start),
    })
}

/// Called by [`Session::execute`] after a command ran (and the revision moved).
pub(crate) fn after(s: &mut Session, id: &str, params: &Value, pre: Option<Pre>, ok: bool) {
    let Some(pre) = pre else { return };
    let guids = match pre.outer {
        Some(outer) => tape_record_take(outer),
        None => Vec::new(),
    };
    if !ok {
        return;
    }
    match pre.replay {
        Replay::Skip => note_view(s),
        Replay::Base => base_changed(s, id, pre.was_dirty),
        Replay::Exact | Replay::Copy => {
            if s.recovery.base.is_none() {
                return;
            }
            let copy = pre.replay == Replay::Copy || pre.crosses || copy_first(s);
            if let Some(tx) = s.recovery.tx.as_mut() {
                tx.push((
                    Step {
                        cmd: id.to_string(),
                        params: params.clone(),
                    },
                    guids,
                ));
                s.recovery.tx_copy |= copy;
                return;
            }
            if !dirty(s) {
                clean(s);
            } else if copy {
                write_copy(s);
            } else {
                let line = json!({
                    "seq": s.recovery.seq.saturating_add(1),
                    "at_ms": now_ms(),
                    "rev": s.revision,
                    "cmd": id,
                    "params": params,
                    "guids": guids,
                });
                append(s, &line, 1);
            }
        }
    }
}

/// An atomic batch starts ([`Session::batch_atomic`]).
pub(crate) fn tx_begin(s: &mut Session) {
    if s.recovery.cfg.is_some() {
        s.recovery.tx = Some(Vec::new());
        s.recovery.tx_copy = false;
        s.recovery.tx_base = false;
    }
}

/// The batch committed: one line (or a copy).
pub(crate) fn tx_commit(s: &mut Session) {
    let Some(steps) = s.recovery.tx.take() else { return };
    if s.recovery.base.is_none() || (steps.is_empty() && !s.recovery.tx_base) {
        return;
    }
    if !dirty(s) {
        clean(s);
        return;
    }
    if s.recovery.tx_copy || (s.recovery.tx_base && steps.is_empty()) || copy_first(s) {
        write_copy(s);
        return;
    }
    if steps.is_empty() {
        return;
    }
    let n = steps.len();
    let guids: Vec<&String> = steps.iter().flat_map(|(_, g)| g.iter()).collect();
    let batch: Vec<&Step> = steps.iter().map(|(st, _)| st).collect();
    let line = json!({
        "seq": s.recovery.seq.saturating_add(1),
        "at_ms": now_ms(),
        "rev": s.revision,
        "batch": batch,
        "guids": guids,
    });
    append(s, &line, n);
}

/// The batch rolled back: its steps are dropped. A batch that moved the base (opened or saved a
/// file) puts the journal on the restored file again, with a copy when it has unsaved edits.
pub(crate) fn tx_rollback(s: &mut Session) {
    if s.recovery.tx.take().is_none() || !s.recovery.tx_base {
        return;
    }
    drop_writer(s, true);
    set_base(s);
    if dirty(s) && s.recovery.base.is_some() {
        write_copy(s);
    }
}

/// A command moved the journal's base.
fn base_changed(s: &mut Session, id: &str, was_dirty: bool) {
    if s.recovery.tx.is_some() {
        s.recovery.tx_base = true;
        if let Some(tx) = s.recovery.tx.as_mut() {
            tx.clear();
        }
        s.recovery.tx_copy = false;
    }
    let was_untitled = untitled_base(s);
    match id {
        "file.save" => {
            drop_writer(s, true);
            set_base(s);
        }
        "recovery.discard" => {}
        // the edits now live in the new file (a new model's too, in its new project)
        "file.save_as" => {
            drop_writer(s, true);
            set_base(s);
        }
        "project.new" if was_untitled => {
            drop_writer(s, true);
            set_base(s);
        }
        // the old file's unsaved edits stay recoverable
        _ => {
            drop_writer(s, !was_dirty);
            set_base(s);
        }
    }
    refresh_offers(s);
}

/// Points the journal at the open file (a new session id); no file is written yet.
fn set_base(s: &mut Session) {
    s.recovery.sid = new_sid();
    s.recovery.seq = 0;
    s.recovery.floor = crate::history::entry_floor(s);
    s.recovery.view = None;
    s.recovery.base = match &s.source {
        Some(src) => identity(src).ok().map(|identity| {
            let kp = key_path(src);
            Base {
                path: display_path(src),
                key: key_of(&kp),
                identity,
            }
        }),
        // a model never saved (`model.new`, an import read into a new model)
        None if s.model.is_some() || s.report.is_some() => Some(Base {
            path: PathBuf::new(),
            key: untitled_key(),
            identity: Identity {
                kind: UNTITLED.into(),
                size: 0,
                mtime_ms: 0,
                files: 0,
                hash: String::new(),
            },
        }),
        None => None,
    };
}

/// The session is back at its saved state: the journal goes.
fn clean(s: &mut Session) {
    drop_writer(s, true);
    s.recovery.sid = new_sid();
    s.recovery.seq = 0;
    s.recovery.floor = crate::history::entry_floor(s);
}

/// Closes the journal; with `delete`, removes it and its copy.
fn drop_writer(s: &mut Session, delete: bool) {
    let w = s.recovery.writer.take();
    let name = s.recovery.name();
    if let Some(mut w) = w {
        let path = w.path.clone();
        if delete {
            // no sync for a file about to go
            w.unsynced = 0;
        }
        drop(w);
        if delete {
            let _ = std::fs::remove_file(&path);
        }
    }
    if delete && let (Some(cfg), Some(name)) = (&s.recovery.cfg, name) {
        let _ = std::fs::remove_file(cfg.dir.join(format!("{name}{COPY_EXT}")));
    }
}

fn header(s: &Session) -> Option<Value> {
    let base = s.recovery.base.as_ref()?;
    let cfg = s.recovery.cfg.as_ref()?;
    let mut h = json!({
        "bicraft_recovery": JOURNAL_VERSION,
        "path": base.path.to_string_lossy(),
        "key": base.key,
        "base": base.identity,
        "owner": cfg.owner,
        "created_ms": now_ms(),
        "pid": std::process::id(),
        "view": view_json(s),
    });
    // a new model has no file name: the listing shows the model's
    if base.untitled()
        && let (Some(o), Some(m)) = (h.as_object_mut(), s.model.as_ref())
    {
        o.insert("label".into(), Value::String(m.name.clone()));
    }
    Some(h)
}

/// A view setting changed: one line with the settings when the journal exists and they differ
/// from the last ones written.
fn note_view(s: &mut Session) {
    if s.recovery.writer.is_none() || s.recovery.tx.is_some() {
        return;
    }
    let v = view_json(s);
    if s.recovery.view.as_ref() == Some(&v) {
        return;
    }
    let line = json!({
        "seq": s.recovery.seq.saturating_add(1),
        "at_ms": now_ms(),
        "rev": s.revision,
        "view": v,
    });
    append(s, &line, 0);
    s.recovery.view = Some(v);
}

fn line_bytes(v: &Value) -> Result<Vec<u8>, String> {
    let mut b = serde_json::to_vec(v).map_err(|e| e.to_string())?;
    b.push(b'\n');
    Ok(b)
}

/// Opens (creates, locks) the session's journal with its header.
fn ensure_writer(s: &mut Session) -> Result<(), String> {
    if s.recovery.writer.is_some() {
        return Ok(());
    }
    let (Some(cfg), Some(name)) = (s.recovery.cfg.as_ref(), s.recovery.name()) else {
        return Err("no journal base".into());
    };
    std::fs::create_dir_all(&cfg.dir).map_err(|e| format!("{}: {e}", cfg.dir.display()))?;
    let path = cfg.dir.join(format!("{name}{JOURNAL_EXT}"));
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    file.try_lock()
        .map_err(|e| format!("{}: locked by another session ({e})", path.display()))?;
    file.set_len(0).map_err(|e| e.to_string())?;
    let head = line_bytes(&header(s).ok_or("no journal base")?)?;
    let mut file = file;
    file.write_all(&head).map_err(|e| e.to_string())?;
    s.recovery.view = Some(view_json(s));
    s.recovery.writer = Some(Writer {
        file,
        path,
        bytes: u64::try_from(head.len()).unwrap_or(u64::MAX),
        edits: 0,
        unsynced: 1,
        last_sync: Instant::now(),
        copy: false,
    });
    Ok(())
}

/// Appends one line for `edits` commands (or writes a copy past the cap).
fn append(s: &mut Session, line: &Value, edits: usize) {
    let result = (|| -> Result<bool, String> {
        let bytes = line_bytes(line)?;
        ensure_writer(s)?;
        let (Some(cfg), Some(w)) = (s.recovery.cfg.as_ref(), s.recovery.writer.as_mut()) else {
            return Err("no journal".into());
        };
        let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if w.bytes.saturating_add(len) > cfg.max_bytes {
            return Ok(false);
        }
        w.file.write_all(&bytes).map_err(|e| e.to_string())?;
        w.bytes = w.bytes.saturating_add(len);
        w.edits = w.edits.saturating_add(edits);
        w.unsynced = w.unsynced.saturating_add(1);
        if w.unsynced >= cfg.sync_every.max(1) || w.last_sync.elapsed() >= cfg.sync_interval {
            w.file.sync_data().map_err(|e| e.to_string())?;
            w.unsynced = 0;
            w.last_sync = Instant::now();
        }
        Ok(true)
    })();
    match result {
        Ok(true) => {
            s.recovery.seq = s.recovery.seq.saturating_add(1);
            s.recovery.error = None;
        }
        // past the cap: a copy replaces the lines
        Ok(false) => write_copy(s),
        Err(e) => s.recovery.error = Some(e),
    }
}

/// A full recovery copy of the session's model and report.
#[derive(Serialize, Deserialize)]
struct CopyState {
    bicraft_recovery_copy: u64,
    model: Option<SemanticModel>,
    report: Option<Report>,
    model_error: Option<String>,
    report_error: Option<String>,
    dirty: bool,
    report_dirty: bool,
    /// The report's resources (the report's JSON form keeps their sizes only), hex.
    resources: Vec<(String, String)>,
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len().saturating_mul(2));
    for b in bytes {
        s.push(char::from(
            DIGITS.get(usize::from(b >> 4)).copied().unwrap_or(b'0'),
        ));
        s.push(char::from(
            DIGITS.get(usize::from(b & 15)).copied().unwrap_or(b'0'),
        ));
    }
    s
}

fn unhex(s: &str) -> Result<Vec<u8>, String> {
    let digit = |c: u8| match c {
        b'0'..=b'9' => Ok(c - b'0'),
        b'a'..=b'f' => Ok(c - b'a' + 10),
        _ => Err("bad hex digit in a recovery copy".to_string()),
    };
    let b = s.as_bytes();
    if !b.len().is_multiple_of(2) {
        return Err("bad hex length in a recovery copy".into());
    }
    b.chunks(2)
        .map(|p| match p {
            [h, l] => Ok((digit(*h)? << 4) | digit(*l)?),
            _ => Err("bad hex".to_string()),
        })
        .collect()
}

/// Writes the copy (through a temporary file) and restarts the journal after it.
fn write_copy(s: &mut Session) {
    let result = (|| -> Result<(), String> {
        let (Some(cfg), Some(name)) = (s.recovery.cfg.clone(), s.recovery.name()) else {
            return Err("no journal base".into());
        };
        std::fs::create_dir_all(&cfg.dir).map_err(|e| format!("{}: {e}", cfg.dir.display()))?;
        let resources = s
            .report
            .as_ref()
            .map(|r| {
                r.resources
                    .iter()
                    .map(|x| (x.path.clone(), hex(&x.bytes)))
                    .collect()
            })
            .unwrap_or_default();
        let state = CopyState {
            bicraft_recovery_copy: JOURNAL_VERSION,
            model: s.model.clone(),
            report: s.report.clone(),
            model_error: s.model_error.clone(),
            report_error: s.report_error.clone(),
            dirty: s.dirty,
            report_dirty: s.report_dirty,
            resources,
        };
        let copy_name = format!("{name}{COPY_EXT}");
        let path = cfg.dir.join(&copy_name);
        let tmp = cfg.dir.join(format!("{copy_name}.tmp"));
        {
            let f = File::create(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
            let mut w = std::io::BufWriter::new(f);
            serde_json::to_writer(&mut w, &state).map_err(|e| e.to_string())?;
            let f = w.into_inner().map_err(|e| e.to_string())?;
            f.sync_all().map_err(|e| e.to_string())?;
        }
        std::fs::rename(&tmp, &path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("{}: {e}", path.display())
        })?;
        ensure_writer(s)?;
        let head = line_bytes(&header(s).ok_or("no journal base")?)?;
        let seq = s.recovery.seq.saturating_add(1);
        let mark = line_bytes(&json!({"seq": seq, "at_ms": now_ms(), "rev": s.revision, "copy": copy_name}))?;
        let w = s.recovery.writer.as_mut().ok_or("no journal")?;
        w.file.set_len(0).map_err(|e| e.to_string())?;
        w.file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        w.file.write_all(&head).map_err(|e| e.to_string())?;
        w.file.write_all(&mark).map_err(|e| e.to_string())?;
        w.file.sync_data().map_err(|e| e.to_string())?;
        w.bytes = u64::try_from(head.len().saturating_add(mark.len())).unwrap_or(u64::MAX);
        w.edits = 0;
        w.unsynced = 0;
        w.last_sync = Instant::now();
        w.copy = true;
        s.recovery.seq = seq;
        s.recovery.view = Some(view_json(s));
        Ok(())
    })();
    s.recovery.error = result.err();
}

// --- reading journals -----------------------------------------------------------------------

/// One step of a journal.
enum Entry {
    Cmd {
        cmd: String,
        params: Value,
        guids: Vec<String>,
    },
    Batch {
        steps: Vec<Step>,
        guids: Vec<String>,
    },
}

impl Entry {
    fn edits(&self) -> usize {
        match self {
            Entry::Cmd { .. } => 1,
            Entry::Batch { steps, .. } => steps.len(),
        }
    }
}

struct Parsed {
    header: Value,
    copy: Option<String>,
    entries: Vec<Entry>,
    last_ms: u64,
    /// The last view settings written (header or line).
    view: Option<Value>,
}

impl Parsed {
    fn untitled(&self) -> bool {
        self.header.pointer("/base/kind").and_then(Value::as_str) == Some(UNTITLED)
    }
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|g| g.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

/// Parses a journal's text. A torn last line (a crash mid-write) is ignored.
fn parse(text: &str) -> Result<Parsed, String> {
    let mut lines = text.split('\n');
    let header: Value = lines
        .next()
        .and_then(|l| serde_json::from_str(l).ok())
        .ok_or("not a recovery journal (no header)")?;
    if header.get("bicraft_recovery").and_then(Value::as_u64) != Some(JOURNAL_VERSION) {
        return Err("not a recovery journal of this version".into());
    }
    let mut p = Parsed {
        last_ms: header.get("created_ms").and_then(Value::as_u64).unwrap_or(0),
        view: header.get("view").filter(|v| v.is_object()).cloned(),
        header,
        copy: None,
        entries: Vec::new(),
    };
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            break;
        };
        if p.entries.len() >= MAX_STEPS {
            break;
        }
        if let Some(t) = v.get("at_ms").and_then(Value::as_u64) {
            p.last_ms = p.last_ms.max(t);
        }
        if let Some(view) = v.get("view").filter(|v| v.is_object()) {
            p.view = Some(view.clone());
        } else if let Some(c) = v.get("copy").and_then(Value::as_str) {
            p.copy = Some(c.to_string());
            p.entries.clear();
        } else if let Some(cmd) = v.get("cmd").and_then(Value::as_str) {
            p.entries.push(Entry::Cmd {
                cmd: cmd.to_string(),
                params: v.get("params").cloned().unwrap_or(Value::Null),
                guids: strings(v.get("guids")),
            });
        } else if let Some(b) = v.get("batch") {
            let steps: Vec<Step> = serde_json::from_value(b.clone()).map_err(|e| e.to_string())?;
            p.entries.push(Entry::Batch {
                steps,
                guids: strings(v.get("guids")),
            });
        }
    }
    Ok(p)
}

fn read_bounded(f: &mut File, cap: u64) -> Result<String, String> {
    let mut text = String::new();
    Read::take(&mut *f, cap)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    Ok(text)
}

/// Reads journal `path` unless a live session holds it.
fn read_journal(path: &Path) -> Option<Result<Parsed, String>> {
    let mut f = File::open(path).ok()?;
    if f.try_lock_shared().is_err() {
        return None;
    }
    Some(read_bounded(&mut f, MAX_READ_BYTES).and_then(|t| parse(&t)))
}

/// What a listing says about a journal.
struct Found {
    id: String,
    parsed: Parsed,
}

/// Journals of the folder (not held by a live session), of key `key` when given, of the
/// config's owner. Journals without edits or copy are deleted on the way.
fn scan(cfg: &RecoveryConfig, key: Option<&str>, skip: Option<&str>) -> Vec<Found> {
    let Ok(dir) = std::fs::read_dir(&cfg.dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in dir.flatten().take(MAX_LISTED.saturating_mul(4)) {
        if out.len() >= MAX_LISTED {
            break;
        }
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(id) = name.strip_suffix(JOURNAL_EXT) else {
            continue;
        };
        if !valid_id(id) || Some(id) == skip || key.is_some_and(|k| !id.starts_with(k)) {
            continue;
        }
        let path = e.path();
        let Some(parsed) = read_journal(&path) else {
            continue;
        };
        let Ok(parsed) = parsed else { continue };
        let owner = parsed.header.get("owner").and_then(Value::as_str);
        if owner != cfg.owner.as_deref() {
            continue;
        }
        if parsed.entries.is_empty() && parsed.copy.is_none() {
            // nothing to recover
            let _ = std::fs::remove_file(&path);
            continue;
        }
        out.push(Found {
            id: id.to_string(),
            parsed,
        });
    }
    out.sort_by(|a, b| b.parsed.last_ms.cmp(&a.parsed.last_ms).then(a.id.cmp(&b.id)));
    out
}

/// The current identity of the file a journal names (startup listings).
fn identity_of_path(path: &Path, kind: &str) -> Option<Identity> {
    match kind {
        "project" => bicraft_pbip::locate(path)
            .ok()
            .and_then(|p| project_identity(&p).ok()),
        k => file_identity(path, k).ok(),
    }
}

/// A listing row. `current`: the open file's identity when the journal is for it; `None`
/// reads the named file's identity now.
fn offer_json(s_root: Option<&Path>, f: &Found, current: Option<&Identity>) -> Value {
    let h = &f.parsed.header;
    let path = h.get("path").and_then(Value::as_str).unwrap_or("");
    let base: Option<Identity> = h.get("base").and_then(|b| serde_json::from_value(b.clone()).ok());
    let now = match current {
        Some(c) => Some(c.clone()),
        None => base
            .as_ref()
            .and_then(|b| identity_of_path(Path::new(path), &b.kind)),
    };
    let untitled = f.parsed.untitled();
    let status = match (&base, &now) {
        _ if untitled => UNTITLED,
        (_, None) => "missing",
        (Some(b), Some(n)) if b == n => "same",
        _ => "changed",
    };
    let copy = f.parsed.copy.is_some();
    let edits: usize = f.parsed.entries.iter().map(Entry::edits).sum();
    let recoverable = status == "same" || (copy && status != "missing");
    let why = match status {
        "same" => None,
        UNTITLED if copy => None,
        UNTITLED => Some("a model never saved, without its recovery copy: nothing to replay its edits onto"),
        "changed" if copy => Some("the file changed since: the recovery copy replaces its model and report"),
        "changed" => Some(
            "the file changed since these edits were made and there is no recovery copy: they cannot be replayed safely",
        ),
        _ => Some("the file is gone"),
    };
    let shown = match s_root {
        Some(root) => Path::new(path)
            .strip_prefix(root)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default(),
        None => path.to_string(),
    };
    let label = match h.get("label").and_then(Value::as_str) {
        Some(l) if untitled => l.to_string(),
        _ => Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
    };
    json!({
        "id": f.id,
        "path": shown,
        "label": label,
        "edits": edits,
        "copy": copy,
        "at_ms": f.parsed.last_ms,
        "created_ms": h.get("created_ms").and_then(Value::as_u64).unwrap_or(0),
        "base": status,
        "recoverable": recoverable,
        "why": why,
    })
}

fn server_root(s: &Session) -> Option<PathBuf> {
    s.server_config().map(|c| c.root().to_path_buf())
}

/// Whether a server lists journal `f`: its file is in the workspace, or it is a model never
/// saved (the folder is the workspace's already).
fn in_root(root: Option<&Path>, f: &Found) -> bool {
    root.is_none_or(|r| {
        f.parsed.untitled()
            || f.parsed
                .header
                .get("path")
                .and_then(Value::as_str)
                .is_some_and(|p| Path::new(p).starts_with(r))
    })
}

/// The offers for the open file, read again (none for a new model: those are listed with
/// `recovery.list {all}` and recovered by id).
fn refresh_offers(s: &mut Session) {
    if s.recovery.cfg.is_none() || s.recovery.base.is_none() || untitled_base(s) {
        s.recovery.offers.clear();
        return;
    }
    let (Some(cfg), Some(base)) = (&s.recovery.cfg, &s.recovery.base) else {
        return;
    };
    let own = s.recovery.name();
    let root = server_root(s);
    let offers = scan(cfg, Some(&base.key), own.as_deref())
        .iter()
        .map(|f| offer_json(root.as_deref(), f, Some(&base.identity)))
        .collect();
    s.recovery.offers = offers;
}

/// Every recoverable journal of `cfg`'s folder and owner (the desktop's start screen): the
/// rows of `recovery.list {all: true}`.
pub fn recovery_offers(cfg: &RecoveryConfig) -> Vec<Value> {
    scan(cfg, None, None)
        .iter()
        .map(|f| offer_json(None, f, None))
        .collect()
}

// --- the commands ---------------------------------------------------------------------------

fn edits_now(s: &Session) -> (usize, bool) {
    s.recovery
        .writer
        .as_ref()
        .map_or((0, false), |w| (w.edits, w.copy))
}

/// The snapshot's `recovery` object (`null` when auto-recovery is off).
pub(crate) fn snapshot_json(s: &Session) -> Value {
    if s.recovery.cfg.is_none() {
        return Value::Null;
    }
    let (edits, copy) = edits_now(s);
    json!({
        "enabled": true,
        "journaled": s.recovery.base.is_some(),
        "edits": edits,
        "copy": copy,
        "error": s.recovery.error,
        "offers": s.recovery.offers,
    })
}

/// `recovery.list {all?}`.
pub(crate) fn recovery_list(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &["all"])?;
    let all = opt_bool(p, "all")?.unwrap_or(false);
    let Some(cfg) = &s.recovery.cfg else {
        return Ok(json!({"enabled": false, "edits": 0, "offers": []}));
    };
    let mut out = snapshot_json(s);
    if all {
        let root = server_root(s);
        let own = s.recovery.name();
        let rows: Vec<Value> = scan(cfg, None, own.as_deref())
            .iter()
            // a server lists the files of its workspace only
            .filter(|f| in_root(root.as_deref(), f))
            .map(|f| {
                let current = s
                    .recovery
                    .base
                    .as_ref()
                    .filter(|b| !b.untitled() && f.id.starts_with(&b.key))
                    .map(|b| &b.identity);
                offer_json(root.as_deref(), f, current)
            })
            .collect();
        if let Some(o) = out.as_object_mut() {
            o.insert("offers".into(), Value::Array(rows));
        }
    }
    Ok(out)
}

fn remove_journal(cfg: &RecoveryConfig, id: &str) -> bool {
    let j = cfg.dir.join(format!("{id}{JOURNAL_EXT}"));
    let removed = std::fs::remove_file(&j).is_ok();
    let _ = std::fs::remove_file(cfg.dir.join(format!("{id}{COPY_EXT}")));
    removed
}

/// `recovery.discard {id?, all?, current?}`.
pub(crate) fn recovery_discard(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["id", "all", "current"])?;
    let id = opt_str(p, "id")?;
    let all = opt_bool(p, "all")?.unwrap_or(false);
    let current = opt_bool(p, "current")?.unwrap_or(false);
    let Some(cfg) = s.recovery.cfg.clone() else {
        return Err("auto-recovery is off for this session".into());
    };
    if let Some(id) = id
        && !valid_id(id)
    {
        return Err("parameter `id` must be a journal id from recovery.list".into());
    }
    let own = s.recovery.name();
    let mut gone = 0usize;
    if current && s.recovery.writer.is_some() {
        drop_writer(s, true);
        clean(s);
        gone = gone.saturating_add(1);
    }
    let targets: Vec<String> = match (id, all) {
        (Some(id), _) => vec![id.to_string()],
        (None, true) => {
            let root = server_root(s);
            scan(&cfg, None, own.as_deref())
                .into_iter()
                .filter(|f| in_root(root.as_deref(), f))
                .map(|f| f.id)
                .collect()
        }
        (None, false) if current => Vec::new(),
        (None, false) => match &s.recovery.base {
            // a new model has no file of its own: other new models' journals are not its
            Some(b) if !b.untitled() => scan(&cfg, Some(&b.key), own.as_deref())
                .into_iter()
                .map(|f| f.id)
                .collect(),
            _ => return Err("no open file: pass `id` or `all`".into()),
        },
    };
    for t in &targets {
        if Some(t.as_str()) == own.as_deref() {
            continue;
        }
        // a live session's journal stays
        let j = cfg.dir.join(format!("{t}{JOURNAL_EXT}"));
        if let Ok(f) = File::open(&j) {
            if f.try_lock().is_err() {
                continue;
            }
            drop(f);
        }
        if remove_journal(&cfg, t) {
            gone = gone.saturating_add(1);
        }
    }
    refresh_offers(s);
    Ok(json!({"discarded": gone, "offers": s.recovery.offers}))
}

/// `recovery.apply {id?}`: replays a journal of the open file. Runs outside the command's own
/// history step ([`Session::execute`] calls it directly): every replayed command is its own
/// step, journaled again in this session's journal.
pub(crate) fn recovery_apply(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["id"])?;
    let Some(cfg) = s.recovery.cfg.clone() else {
        return Err("auto-recovery is off for this session".into());
    };
    if let Some(id) = opt_str(p, "id")?
        && valid_id(id)
        && id.starts_with(&untitled_key())
    {
        return apply_untitled(s, &cfg, id);
    }
    let Some(base) = s.recovery.base.clone() else {
        return Err("open the file whose edits to recover first".into());
    };
    let id = match opt_str(p, "id")? {
        Some(id) if valid_id(id) => id.to_string(),
        Some(_) => return Err("parameter `id` must be a journal id from recovery.list".into()),
        None => {
            refresh_offers(s);
            s.recovery
                .offers
                .iter()
                .find(|o| o.get("recoverable").and_then(Value::as_bool) == Some(true))
                .and_then(|o| o.get("id").and_then(Value::as_str))
                .map(str::to_string)
                .ok_or("no recoverable edits for the open file")?
        }
    };
    if !id.starts_with(&base.key) {
        return Err("that journal belongs to another file: open it first".into());
    }
    if Some(&id) == s.recovery.name().as_ref() {
        return Err("that is this session's own journal".into());
    }
    if dirty(s) {
        return Err(
            "the open file has unsaved changes: save them or reopen the file before recovering".into(),
        );
    }
    let jpath = cfg.dir.join(format!("{id}{JOURNAL_EXT}"));
    let mut f = File::open(&jpath).map_err(|e| format!("journal {id}: {e}"))?;
    f.try_lock()
        .map_err(|_| "these edits are open in another session".to_string())?;
    let parsed = parse(&read_bounded(&mut f, MAX_READ_BYTES)?)?;
    if parsed.header.get("owner").and_then(Value::as_str) != cfg.owner.as_deref() {
        return Err("that journal is not yours".into());
    }
    let recorded: Option<Identity> = parsed
        .header
        .get("base")
        .and_then(|b| serde_json::from_value(b.clone()).ok());
    let same = recorded.as_ref() == Some(&base.identity);
    let copy_path = match &parsed.copy {
        Some(c) if *c == format!("{id}{COPY_EXT}") => Some(cfg.dir.join(c)),
        Some(_) => return Err("the journal names a recovery copy that is not its own".into()),
        None => None,
    };
    if !same && copy_path.is_none() {
        return Err(
            "the file changed since these edits were made and there is no recovery copy: they cannot be replayed safely (discard them)"
                .into(),
        );
    }
    if parsed.untitled() {
        return Err("that journal belongs to another file: open it first".into());
    }
    let mut out = replay(s, &parsed, copy_path.as_deref())?;
    drop(f);
    remove_journal(&cfg, &id);
    refresh_offers(s);
    if let Some(o) = out.as_object_mut() {
        o.insert("id".into(), json!(id));
        o.insert("base".into(), json!(if same { "same" } else { "changed" }));
    }
    Ok(out)
}

/// `recovery.apply {id}` of a model never saved: the session starts a new model (as
/// `model.new`), the recovery copy replaces it, then the lines after the copy replay. The
/// session must have no unsaved edits; it needs no open file.
fn apply_untitled(s: &mut Session, cfg: &RecoveryConfig, id: &str) -> Result<Value, String> {
    if Some(id) == s.recovery.name().as_deref() {
        return Err("that is this session's own journal".into());
    }
    if dirty(s) {
        return Err(
            "the open file has unsaved changes: save them or reopen the file before recovering".into(),
        );
    }
    let jpath = cfg.dir.join(format!("{id}{JOURNAL_EXT}"));
    let mut f = File::open(&jpath).map_err(|e| format!("journal {id}: {e}"))?;
    f.try_lock()
        .map_err(|_| "these edits are open in another session".to_string())?;
    let parsed = parse(&read_bounded(&mut f, MAX_READ_BYTES)?)?;
    if parsed.header.get("owner").and_then(Value::as_str) != cfg.owner.as_deref() {
        return Err("that journal is not yours".into());
    }
    if !parsed.untitled() {
        return Err("that journal is not a new model's".into());
    }
    let copy_path = match &parsed.copy {
        Some(c) if *c == format!("{id}{COPY_EXT}") => cfg.dir.join(c),
        Some(_) => return Err("the journal names a recovery copy that is not its own".into()),
        None => {
            return Err(
                "a model never saved, without its recovery copy: nothing to replay its edits onto".into(),
            );
        }
    };
    // read before the session changes: a bad copy leaves it as it was
    let state = read_copy(&copy_path)?;
    let name = state
        .model
        .as_ref()
        .map(|m| m.name.clone())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "Model".into());
    s.execute("model.new", &json!({"name": name}))
        .or_else(|_| s.execute("model.new", &json!({"name": "Model"})))?;
    let mut out = replay_from(s, &parsed, Some(state))?;
    drop(f);
    remove_journal(cfg, id);
    refresh_offers(s);
    if let Some(o) = out.as_object_mut() {
        o.insert("id".into(), json!(id));
        o.insert("base".into(), json!(UNTITLED));
    }
    Ok(out)
}

/// Replays a parsed journal: its recovery copy (`copy`), its lines, then its view settings.
fn replay(s: &mut Session, parsed: &Parsed, copy: Option<&Path>) -> Result<Value, String> {
    let state = copy.map(read_copy).transpose()?;
    replay_from(s, parsed, state)
}

fn replay_from(s: &mut Session, parsed: &Parsed, state: Option<CopyState>) -> Result<Value, String> {
    let total: usize = parsed.entries.iter().map(Entry::edits).sum();
    let mut used_copy = false;
    if let Some(state) = state {
        overlay(s, state);
        used_copy = true;
    }
    let mut applied = 0usize;
    let mut failed = Value::Null;
    for (i, e) in parsed.entries.iter().enumerate() {
        let r = match e {
            Entry::Cmd { cmd, params, guids } => replayable(cmd, params).and_then(|()| {
                tape_play(guids);
                let r = s.execute(cmd, params).map(|_| ());
                tape_play_clear();
                r
            }),
            Entry::Batch { steps, guids } => steps
                .iter()
                .try_for_each(|st| replayable(&st.cmd, &st.params))
                .and_then(|()| {
                    tape_play(guids);
                    let list: Vec<(String, Value)> = steps
                        .iter()
                        .map(|st| (st.cmd.clone(), st.params.clone()))
                        .collect();
                    let r = s
                        .batch_atomic(&list)
                        .map(|_| ())
                        .map_err(|(k, e, _)| format!("batch step {k}: {e}"));
                    tape_play_clear();
                    r
                }),
        };
        match r {
            Ok(()) => applied = applied.saturating_add(e.edits()),
            Err(err) => {
                let cmd = match e {
                    Entry::Cmd { cmd, .. } => cmd.clone(),
                    Entry::Batch { .. } => "batch".into(),
                };
                failed = json!({"index": i, "command": cmd, "error": err});
                break;
            }
        }
    }
    let view = parsed
        .view
        .as_ref()
        .map_or_else(|| json!({"applied": 0, "errors": []}), |v| apply_view(s, v));
    Ok(json!({
        "applied": applied,
        "total": total,
        "complete": failed.is_null(),
        "failed": failed,
        "copy": used_copy,
        "view": view,
        "dirty": s.dirty,
        "report_dirty": s.report_dirty,
    }))
}

/// Puts the journal's view settings back through the `view.*` commands (each that differs):
/// `{applied, errors}`.
fn apply_view(s: &mut Session, v: &Value) -> Value {
    let now = view_json(s);
    let mut steps: Vec<(&str, Value)> = Vec::new();
    let differs = |k: &str| v.get(k).is_some_and(|x| now.get(k) != Some(x));
    if differs("ui_language")
        && let Some(lang) = v.get("ui_language").and_then(Value::as_str)
    {
        steps.push(("view.set_language", json!({"ui_language": lang})));
    }
    if differs("culture") {
        let c = v.get("culture").and_then(Value::as_str);
        steps.push(("view.set_culture", json!({"culture": c})));
    }
    if differs("as_of") {
        let a = v.get("as_of").and_then(Value::as_str);
        steps.push(("view.set_as_of", json!({"as_of": a})));
    }
    if differs("page")
        && let Some(page) = v.get("page").and_then(Value::as_str)
    {
        steps.push(("view.interact", json!({"action": "page", "page": page})));
    }
    let mut applied = 0usize;
    let mut errors = Vec::new();
    for (cmd, params) in steps {
        match s.execute(cmd, &params) {
            Ok(_) => applied = applied.saturating_add(1),
            Err(e) => errors.push(json!({"command": cmd, "error": e})),
        }
    }
    json!({"applied": applied, "errors": errors})
}

/// Only commands journaled as lines replay (never a file, network or recovery command, even
/// from a tampered journal).
fn replayable(cmd: &str, params: &Value) -> Result<(), String> {
    if crate::rich::reads_file(cmd, params) {
        return Err(format!("`{cmd}` reading a file is not replayed from a journal"));
    }
    match find_command(cmd) {
        Some(c) if c.replay == Replay::Exact => Ok(()),
        Some(_) => Err(format!("`{cmd}` is not replayed from a journal")),
        None => Err(format!("unknown command `{cmd}`")),
    }
}

fn read_copy(path: &Path) -> Result<CopyState, String> {
    let f = File::open(path).map_err(|e| format!("the recovery copy: {e}"))?;
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    if len > MAX_COPY_BYTES {
        return Err("the recovery copy is too large".into());
    }
    let state: CopyState = serde_json::from_reader(std::io::BufReader::new(f))
        .map_err(|e| format!("the recovery copy cannot be read: {e}"))?;
    if state.bicraft_recovery_copy != JOURNAL_VERSION {
        return Err("the recovery copy is of another version".into());
    }
    Ok(state)
}

/// Puts a recovery copy into the session as one undo step, then copies it into this session's
/// journal.
fn overlay(s: &mut Session, mut state: CopyState) {
    if let Some(r) = state.report.as_mut() {
        for res in &mut r.resources {
            if let Some((_, h)) = state.resources.iter().find(|(p, _)| *p == res.path) {
                res.bytes = unhex(h).unwrap_or_default();
            }
        }
    }
    let before = crate::history::before(s, "recovery.apply");
    s.model = state.model;
    s.report = state.report;
    s.model_error = state.model_error;
    s.report_error = state.report_error;
    s.dirty = state.dirty || !state.report_dirty;
    s.report_dirty = state.report_dirty;
    crate::history::after(s, "recovery.apply", before, true);
    s.revision = s.revision.saturating_add(1);
    s.preview.revised();
    write_copy(s);
}

impl Session {
    /// Turns auto-recovery on (or off with `None`) for this session; a file already open gets
    /// its journal base now. Off by default (the CLI, tests and private sessions never journal).
    pub fn set_recovery(&mut self, cfg: Option<RecoveryConfig>) {
        drop_writer(self, !dirty(self));
        self.recovery.cfg = cfg;
        if self.recovery.cfg.is_some() {
            set_base(self);
        } else {
            self.recovery.base = None;
        }
        refresh_offers(self);
    }

    pub fn recovery_config(&self) -> Option<&RecoveryConfig> {
        self.recovery.cfg.as_ref()
    }

    /// This session's journal file, once it has one.
    pub fn recovery_journal(&self) -> Option<PathBuf> {
        self.recovery.writer.as_ref().map(|w| w.path.clone())
    }

    /// The last journal write error, if any (the snapshot's `recovery.error`).
    pub fn recovery_error(&self) -> Option<&str> {
        self.recovery.error.as_deref()
    }

    /// A clean close: the user chose to drop the unsaved edits, so the journal goes. (Dropping
    /// the session without this keeps it for recovery.)
    pub fn recovery_close(&mut self) {
        drop_writer(self, true);
        self.recovery.base = None;
    }

    /// Syncs the journal to disk now (a host about to stop).
    pub fn recovery_flush(&mut self) {
        if let Some(w) = self.recovery.writer.as_mut() {
            let _ = w.file.sync_data();
            w.unsynced = 0;
        }
    }
}
