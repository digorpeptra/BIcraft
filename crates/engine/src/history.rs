//! Undo/redo history and atomic batches (M1.6).
//!
//! **Snapshots, shared.** A history step keeps the session as it was before the step: the model,
//! the report, their read errors (`model_error`, `report_error`), the edited model diagram
//! (`diagram_layout`, shared text). Each part is an
//! `Arc` snapshot with an id; consecutive steps that leave a part alone share the same `Arc`, so
//! a report-only edit costs no model copy and vice versa. Around every mutating command the live
//! part is compared with the last snapshot (`PartialEq`, no allocation) and cloned only when it
//! changed. The newest snapshot (`current`) mirrors the live session, which is one extra copy of
//! the model and the report while history is enabled. Measured in release builds
//! (`history_tests::history_costs`): a 10 000-measure model costs about 1 ms per compare and
//! 3 to 5 ms per copy, so roughly 5 to 10 ms added to a model edit; a 115-visual real report adds
//! about 5 to 12 ms to a report edit. Read-only commands cost nothing.
//!
//! **Bounds.** At most [`HistoryLimits::max_steps`] undo steps (default 100) and
//! [`HistoryLimits::max_bytes`] (default 256 MiB) of snapshots, whichever comes first; the
//! oldest steps go first. Sizes are estimates (twice the serialized JSON, plus the bytes of
//! report resources), counted once per distinct snapshot.
//!
//! **What is not undone.** The open file (`source`) and the files written by `file.save`,
//! `file.save_as` and `project.new`: undo restores the in-memory model and report only. The
//! dirty flags are recomputed after an undo/redo against what was last saved or opened: each
//! snapshot id is compared with the id of the state that was clean, so undoing past a save makes
//! the session dirty again and redoing back to the saved state makes it clean. `file.open` and
//! `model.new` clear the history.
//!
//! **Atomic batches.** [`Session::batch_atomic`] runs commands as one transaction: if one fails,
//! the model, report, read errors, dirty flags, history and (when a step of the batch opened
//! another file or started a new model) the open file are put back as they were before the batch.
//! Files written by save steps stay written. A committed batch is one undo step.

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use bicraft_model::SemanticModel;
use bicraft_report::Report;
use serde::Serialize;
use serde_json::{Value, json};

use crate::params::allow;
use crate::{Session, Source, find_command, is_mutating, now_ms};

/// Default cap on undo steps.
pub const DEFAULT_MAX_STEPS: usize = 100;
/// Default cap on the estimated memory of all snapshots.
pub const DEFAULT_MAX_BYTES: usize = 256 * 1024 * 1024;
/// Command ids kept per batch step (the label still counts them all).
const MAX_STEP_COMMANDS: usize = 64;
/// Largest `steps` an undo/redo accepts.
const MAX_STEPS_PARAM: u64 = 1_000_000;

/// The history commands: they manage the history themselves, so they are not steps.
const HISTORY_COMMANDS: &[&str] = &["edit.undo", "edit.redo", "edit.history"];
/// Commands that replace the session's file or model: they clear the history.
const CLEARS: &[&str] = &["file.open", "model.new"];
/// Commands that write files (never undone).
const WRITES: &[&str] = &["file.save", "file.save_as", "project.new"];

/// Caps of the undo history. `max_steps: 0` disables it (and its snapshot cost); atomic batches
/// still roll back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct HistoryLimits {
    pub max_steps: usize,
    pub max_bytes: usize,
}

impl Default for HistoryLimits {
    fn default() -> Self {
        Self {
            max_steps: DEFAULT_MAX_STEPS,
            max_bytes: DEFAULT_MAX_BYTES,
        }
    }
}

/// One part of the session (model or report), shared between snapshots.
#[derive(Debug)]
struct Snap<T> {
    value: Option<Arc<T>>,
    /// Changes whenever the content does (unique within the session's history).
    id: u64,
    /// Estimated heap size of `value`.
    bytes: usize,
}

impl<T> Clone for Snap<T> {
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
            id: self.id,
            bytes: self.bytes,
        }
    }
}

#[derive(Debug, Clone)]
struct State {
    model: Snap<SemanticModel>,
    report: Snap<Report>,
    model_error: Option<String>,
    report_error: Option<String>,
    /// The edited model diagram (`Session::diagram_layout`), shared between steps.
    diagram: Option<Arc<str>>,
}

impl State {
    fn same(&self, other: &State) -> bool {
        self.model.id == other.model.id
            && self.report.id == other.report.id
            && self.model_error == other.model_error
            && self.report_error == other.report_error
            && self.diagram == other.diagram
    }
}

#[derive(Debug, Clone)]
struct Entry {
    /// Unique within the session, kept when the entry moves between undo and redo
    /// (`recovery.rs` tells the steps made before its journal started from the others).
    seq: u64,
    label: String,
    commands: Vec<String>,
    at_ms: u64,
    /// For an undo entry, the state before the step; for a redo entry, the state after it.
    state: State,
}

impl Entry {
    fn describe(&self) -> Value {
        json!({"label": self.label, "commands": self.commands, "at_ms": self.at_ms})
    }
}

/// An atomic batch in progress.
#[derive(Debug)]
struct Tx {
    /// The session before the batch.
    rollback: State,
    dirty: bool,
    report_dirty: bool,
    saved: (Option<u64>, Option<u64>),
    undo: VecDeque<Entry>,
    redo: Vec<Entry>,
    /// The state the batch's undo step goes back to: the state before the batch, or right after
    /// its last `file.open` / `model.new`.
    undo_base: State,
    /// The open file before the batch, once a step replaced it (`file.open`, `model.new`).
    old_source: Option<Option<Source>>,
    wrote_files: bool,
    /// Mutating commands that succeeded (after the last clear).
    commands: Vec<String>,
    command_count: usize,
    revision: u64,
}

/// The session's undo/redo history.
#[derive(Debug)]
pub(crate) struct History {
    limits: HistoryLimits,
    /// Oldest first.
    undo: VecDeque<Entry>,
    /// Next to redo last.
    redo: Vec<Entry>,
    /// The session as of the last synchronization.
    current: Option<State>,
    next_id: u64,
    /// The last [`Entry::seq`] handed out.
    entry_seq: u64,
    /// Ids of the model / report as last opened or saved (`None`: no clean state known).
    saved_model: Option<u64>,
    saved_report: Option<u64>,
    /// The edited diagram of the saved state (meaningful while `saved_model` is known).
    saved_diagram: Option<Arc<str>>,
    tx: Option<Box<Tx>>,
}

impl History {
    /// Whether `edit.undo` has a step to undo (the `enabled` check of `command_ui.rs`).
    pub(crate) fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Whether `edit.redo` has a step to redo.
    pub(crate) fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

impl Default for History {
    fn default() -> Self {
        Self {
            limits: HistoryLimits::default(),
            undo: VecDeque::new(),
            redo: Vec::new(),
            current: None,
            next_id: 1,
            entry_seq: 0,
            saved_model: None,
            saved_report: None,
            saved_diagram: None,
            tx: None,
        }
    }
}

impl History {
    fn push(&mut self, entry: Entry) {
        let mut entry = entry;
        self.entry_seq = self.entry_seq.saturating_add(1);
        entry.seq = self.entry_seq;
        self.undo.push_back(entry);
        self.redo.clear();
        self.trim();
    }

    /// Estimated bytes held by snapshots (each distinct snapshot once).
    fn bytes(&self) -> usize {
        let mut seen = HashSet::new();
        let mut total = 0usize;
        let states = self
            .undo
            .iter()
            .chain(self.redo.iter())
            .map(|e| &e.state)
            .chain(self.current.iter());
        for st in states {
            if st.model.value.is_some() && seen.insert(st.model.id) {
                total = total.saturating_add(st.model.bytes);
            }
            if st.report.value.is_some() && seen.insert(st.report.id) {
                total = total.saturating_add(st.report.bytes);
            }
        }
        total
    }

    fn trim(&mut self) {
        while self.undo.len() > self.limits.max_steps {
            self.undo.pop_front();
        }
        if self.redo.len() > self.limits.max_steps {
            let excess = self.redo.len().saturating_sub(self.limits.max_steps);
            self.redo = self.redo.split_off(excess);
        }
        while self.bytes() > self.limits.max_bytes {
            if self.undo.pop_front().is_some() {
                continue;
            }
            if self.redo.is_empty() {
                break;
            }
            self.redo.remove(0);
        }
    }

    fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

/// Writes nothing, counts bytes.
struct Counter(usize);

impl std::io::Write for Counter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0 = self.0.saturating_add(buf.len());
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn json_len<T: Serialize>(v: &T) -> usize {
    let mut c = Counter(0);
    // A failure only makes the estimate low.
    let _ = serde_json::to_writer(&mut c, v);
    c.0
}

fn model_bytes(m: &SemanticModel) -> usize {
    json_len(m).saturating_mul(2)
}

fn report_bytes(r: &Report) -> usize {
    // Resources serialize as their length only: add their bytes.
    r.resources
        .iter()
        .fold(json_len(r).saturating_mul(2), |acc, res| {
            acc.saturating_add(res.bytes.len())
        })
}

/// The snapshot of `live`: `old` when unchanged, else a fresh copy with a new id.
fn sync_part<T: Clone + PartialEq>(
    next_id: &mut u64,
    old: Option<&Snap<T>>,
    live: Option<&T>,
    size: fn(&T) -> usize,
) -> Snap<T> {
    if let Some(old) = old {
        let same = match (old.value.as_deref(), live) {
            (None, None) => true,
            (Some(a), Some(b)) => a == b,
            _ => false,
        };
        if same {
            return old.clone();
        }
    }
    let id = *next_id;
    *next_id = next_id.saturating_add(1);
    match live {
        None => Snap {
            value: None,
            id,
            bytes: 0,
        },
        Some(v) => Snap {
            value: Some(Arc::new(v.clone())),
            id,
            bytes: size(v),
        },
    }
}

/// Brings `current` up to date with the live session and returns it. Also records which state
/// is clean: a clean flag marks the current snapshot as the saved one.
fn sync(s: &mut Session) -> State {
    let h = &mut s.history;
    let old = h.current.take();
    let model = sync_part(
        &mut h.next_id,
        old.as_ref().map(|o| &o.model),
        s.model.as_ref(),
        model_bytes,
    );
    let report = sync_part(
        &mut h.next_id,
        old.as_ref().map(|o| &o.report),
        s.report.as_ref(),
        report_bytes,
    );
    // the diagram's text is shared with the last snapshot while unchanged
    let diagram = match (
        old.as_ref().and_then(|o| o.diagram.clone()),
        s.diagram_layout.as_deref(),
    ) {
        (Some(a), Some(b)) if &*a == b => Some(a),
        (_, live) => live.map(Arc::from),
    };
    let state = State {
        model,
        report,
        model_error: s.model_error.clone(),
        report_error: s.report_error.clone(),
        diagram,
    };
    if !s.dirty {
        h.saved_model = Some(state.model.id);
        h.saved_diagram = state.diagram.clone();
    } else if h.saved_model == Some(state.model.id) && h.saved_diagram == state.diagram {
        h.saved_model = None;
    }
    if !s.report_dirty {
        h.saved_report = Some(state.report.id);
    } else if h.saved_report == Some(state.report.id) {
        h.saved_report = None;
    }
    h.current = Some(state.clone());
    state
}

/// Puts `st` back into the live session (model, report, read errors).
fn restore(s: &mut Session, st: &State) {
    s.model = st.model.value.as_deref().cloned();
    s.report = st.report.value.as_deref().cloned();
    s.model_error = st.model_error.clone();
    s.report_error = st.report_error.clone();
    s.diagram_layout = st.diagram.as_deref().map(str::to_string);
    s.history.current = Some(st.clone());
}

/// Dirty flags of `st` against the saved ids.
fn recompute_flags(s: &mut Session, st: &State) {
    // a diagram edit (`diagram.move`) makes the model dirty without changing it
    s.dirty = s.history.saved_model != Some(st.model.id) || s.history.saved_diagram != st.diagram;
    s.report_dirty = s.history.saved_report != Some(st.report.id);
}

fn label_of(id: &str) -> String {
    find_command(id).map_or_else(|| id.to_string(), |c| c.label.to_string())
}

/// What [`before`] saw, for [`after`].
pub(crate) struct Before {
    state: State,
    dirty: bool,
    report_dirty: bool,
}

/// Called by [`Session::execute`] before a command runs. `None`: the command is not a step.
pub(crate) fn before(s: &mut Session, id: &str) -> Option<Before> {
    if HISTORY_COMMANDS.contains(&id) || !is_mutating(id) || find_command(id).is_none() {
        return None;
    }
    if s.history.limits.max_steps == 0 && s.history.tx.is_none() {
        // History disabled: no snapshot cost.
        return None;
    }
    let state = sync(s);
    if CLEARS.contains(&id)
        && let Some(tx) = s.history.tx.as_mut()
        && tx.old_source.is_none()
    {
        // Moved, not cloned (a package can be gigabytes): the command replaces it anyway, and
        // a rollback puts it back.
        tx.old_source = Some(s.source.take());
    }
    Some(Before {
        state,
        dirty: s.dirty,
        report_dirty: s.report_dirty,
    })
}

/// Called by [`Session::execute`] after a command ran: records the step, or, when the command
/// failed but changed the model or report anyway (a panic midway), puts them back.
pub(crate) fn after(s: &mut Session, id: &str, before: Option<Before>, ok: bool) {
    let Some(b) = before else { return };
    if !ok {
        let live = {
            // Compare without recording the failed command's flags as clean.
            let (dirty, report_dirty) = (s.dirty, s.report_dirty);
            s.dirty = b.dirty;
            s.report_dirty = b.report_dirty;
            let live = sync(s);
            s.dirty = dirty;
            s.report_dirty = report_dirty;
            live
        };
        if !live.same(&b.state) {
            restore(s, &b.state);
        }
        s.dirty = b.dirty;
        s.report_dirty = b.report_dirty;
        return;
    }
    let state = sync(s);
    let h = &mut s.history;
    if let Some(tx) = h.tx.as_mut() {
        tx.wrote_files |= WRITES.contains(&id);
        if CLEARS.contains(&id) {
            tx.undo_base = state.clone();
            tx.commands.clear();
            tx.command_count = 0;
        } else {
            tx.command_count = tx.command_count.saturating_add(1);
            if tx.commands.len() < MAX_STEP_COMMANDS {
                tx.commands.push(id.to_string());
            }
        }
    }
    if CLEARS.contains(&id) {
        h.clear();
        return;
    }
    if h.tx.is_some() || state.same(&b.state) {
        return;
    }
    h.push(Entry {
        seq: 0,
        label: label_of(id),
        commands: vec![id.to_string()],
        at_ms: now_ms(),
        state: b.state,
    });
}

fn steps_param(p: &Value) -> Result<usize, String> {
    match p.get("steps") {
        None | Some(Value::Null) => Ok(1),
        Some(v) => match v.as_u64() {
            Some(n) if (1..=MAX_STEPS_PARAM).contains(&n) => Ok(usize::try_from(n).unwrap_or(1)),
            _ => Err(format!(
                "parameter `steps` must be an integer from 1 to {MAX_STEPS_PARAM}"
            )),
        },
    }
}

fn move_steps(s: &mut Session, p: &Value, undo: bool) -> Result<Value, String> {
    allow(p, &["steps"])?;
    let steps = steps_param(p)?;
    let verb = if undo { "undo" } else { "redo" };
    if s.history.tx.is_some() {
        return Err(format!("edit.{verb} cannot run inside an atomic batch"));
    }
    let mut cur = sync(s);
    let h = &mut s.history;
    let available = if undo { h.undo.len() } else { h.redo.len() };
    if available == 0 {
        return Err(format!("nothing to {verb}"));
    }
    let mut moved = Vec::new();
    for _ in 0..steps.min(available) {
        let next = if undo { h.undo.pop_back() } else { h.redo.pop() };
        let Some(e) = next else { break };
        moved.push(e.describe());
        let back = Entry {
            seq: e.seq,
            label: e.label,
            commands: e.commands,
            at_ms: e.at_ms,
            state: cur,
        };
        if undo {
            h.redo.push(back);
        } else {
            h.undo.push_back(back);
        }
        cur = e.state;
    }
    restore(s, &cur);
    recompute_flags(s, &cur);
    let key = if undo { "undone" } else { "redone" };
    Ok(json!({
        key: moved,
        "can_undo": !s.history.undo.is_empty(),
        "can_redo": !s.history.redo.is_empty(),
        "dirty": s.dirty,
        "report_dirty": s.report_dirty,
    }))
}

/// `edit.undo {steps?}`.
pub(crate) fn edit_undo(s: &mut Session, p: &Value) -> Result<Value, String> {
    move_steps(s, p, true)
}

/// `edit.redo {steps?}`.
pub(crate) fn edit_redo(s: &mut Session, p: &Value) -> Result<Value, String> {
    move_steps(s, p, false)
}

/// `edit.history {}`.
pub(crate) fn edit_history(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &[])?;
    let h = &s.history;
    Ok(json!({
        "undo": h.undo.iter().rev().map(Entry::describe).collect::<Vec<_>>(),
        "redo": h.redo.iter().rev().map(Entry::describe).collect::<Vec<_>>(),
        "can_undo": !h.undo.is_empty(),
        "can_redo": !h.redo.is_empty(),
        "bytes": h.bytes(),
        "limits": h.limits,
    }))
}

/// The last history entry number handed out: entries up to it predate a journal started now.
pub(crate) fn entry_floor(s: &Session) -> u64 {
    s.history.entry_seq
}

/// Whether `edit.undo` (`undo`) or `edit.redo` of `steps` would move an entry numbered up to
/// `floor` (one made before the journal started, which a replay from the journal's base cannot
/// reach).
pub(crate) fn crosses_floor(s: &Session, undo: bool, steps: usize, floor: u64) -> bool {
    let h = &s.history;
    let steps = steps.max(1);
    if undo {
        h.undo.iter().rev().take(steps).any(|e| e.seq <= floor)
    } else {
        h.redo.iter().rev().take(steps).any(|e| e.seq <= floor)
    }
}

impl Session {
    /// Changes the history caps and trims the history to them.
    pub fn set_history_limits(&mut self, limits: HistoryLimits) {
        self.history.limits = limits;
        self.history.trim();
    }

    pub fn history_limits(&self) -> HistoryLimits {
        self.history.limits
    }

    /// Runs commands in order as one transaction. On the first error, the session goes back to
    /// its state before the batch (see the module docs for what that covers) and the error says
    /// which step failed; the results of the steps before it are returned. A committed batch is
    /// one undo step. Inside a batch already running atomically, this is [`Session::batch`].
    pub fn batch_atomic(
        &mut self,
        steps: &[(String, Value)],
    ) -> Result<Vec<Value>, (usize, String, Vec<Value>)> {
        if self.history.tx.is_some() {
            return self.batch(steps);
        }
        begin(self);
        crate::recovery::tx_begin(self);
        let mut out = Vec::with_capacity(steps.len());
        for (i, (id, params)) in steps.iter().enumerate() {
            match self.execute(id, params) {
                Ok(v) => out.push(v),
                Err(e) => {
                    rollback(self);
                    crate::recovery::tx_rollback(self);
                    return Err((i, e, out));
                }
            }
        }
        commit(self);
        crate::recovery::tx_commit(self);
        Ok(out)
    }
}

fn begin(s: &mut Session) {
    let state = sync(s);
    let h = &mut s.history;
    h.tx = Some(Box::new(Tx {
        rollback: state.clone(),
        dirty: s.dirty,
        report_dirty: s.report_dirty,
        saved: (h.saved_model, h.saved_report),
        undo: h.undo.clone(),
        redo: h.redo.clone(),
        undo_base: state,
        old_source: None,
        wrote_files: false,
        commands: Vec::new(),
        command_count: 0,
        revision: s.revision,
    }));
}

fn rollback(s: &mut Session) {
    let Some(tx) = s.history.tx.take() else {
        return;
    };
    let tx = *tx;
    let source_restored = tx.old_source.is_some();
    if let Some(source) = tx.old_source {
        s.source = source;
    }
    restore(s, &tx.rollback);
    s.history.undo = tx.undo;
    s.history.redo = tx.redo;
    if tx.wrote_files {
        // A save wrote some state of the batch: compare against it (or, when the batch also
        // switched files, assume the restored state differs from the disk).
        if source_restored {
            s.history.saved_model = None;
            s.history.saved_report = None;
        }
        recompute_flags(s, &tx.rollback);
    } else {
        (s.history.saved_model, s.history.saved_report) = tx.saved;
        s.dirty = tx.dirty;
        s.report_dirty = tx.report_dirty;
    }
    if s.revision != tx.revision {
        s.revision = s.revision.saturating_add(1);
        s.preview.revised();
    }
}

fn commit(s: &mut Session) {
    let Some(tx) = s.history.tx.take() else {
        return;
    };
    let tx = *tx;
    let state = sync(s);
    if state.same(&tx.undo_base) {
        return;
    }
    let label = match tx.commands.as_slice() {
        [one] if tx.command_count == 1 => label_of(one),
        _ => format!("Batch of {} commands", tx.command_count),
    };
    s.history.push(Entry {
        seq: 0,
        label,
        commands: tx.commands,
        at_ms: now_ms(),
        state: tx.undo_base,
    });
}
