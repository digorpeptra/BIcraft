//! The DAX query view (Power BI Desktop's DAX query view, as Microsoft Learn's "DAX query view"
//! page describes it): query tabs along the bottom (add, rename, duplicate, close), the DAX editor
//! with the formula bar's colours, Run (the button, Ctrl+Enter or F5) through `dax.evaluate`, the
//! results grid with its row count and the errors inline, and "Update model" lenses on each
//! `DEFINE MEASURE` (add a new measure or overwrite the model's) plus "Update model with changes"
//! for them all, through `dax.apply_define` (one undo step).
//!
//! Format query (`dax.format`) lays the tab's text out; Cancel (`dax.cancel`) stops the running
//! query; the text is checked a moment after typing stops (`dax.validate_query`), the first error
//! underlined and told under the editor, as the formula bar does.
//!
//! The tabs, their text and their last results are view state, kept per document
//! ([`crate::ribbon::DocUi`]). The tabs and their text are also the open file's: they are loaded
//! from the snapshot's `dax_queries` when a file opens, and handed back with `dax.set_queries` a
//! moment after they change and before every save, which writes them into a PBIP project.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use egui::text::{CCursor, LayoutJob};
use egui::{CornerRadius, Galley, Id, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use serde_json::{Value, json};

use crate::App;
use crate::actions::Call;
use crate::answer::{self, Grid};
use crate::app::{Tab as View, Then};
use crate::dax_text::{self, Class};
use crate::i18n::Lang;
use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::widgets;

/// Run.
pub const RUN: &str = "dax.evaluate";
/// Update model (a lens, or every DEFINE MEASURE).
pub const APPLY: &str = "dax.apply_define";
/// Format query.
pub const FORMAT: &str = "dax.format";
/// Cancel.
pub const CANCEL: &str = crate::source::CANCEL;
/// The live check.
pub const CHECK: &str = "dax.validate_query";
/// Hands the tabs to the session (saved with the file).
pub const SET: &str = "dax.set_queries";
/// How long the text stays unchanged before it is checked.
const CHECK_AFTER: f64 = 0.35;
/// How long the tabs stay unchanged before they go to the session.
const SYNC_AFTER: f64 = 1.5;
/// Most query tabs of one document.
pub const MAX_TABS: usize = 64;
/// Longest tab name, in characters.
const MAX_NAME: usize = 100;
/// Height of the tab strip and of the tool bar.
const TABS_H: f32 = 32.0;
const BAR_H: f32 = 40.0;

/// What an answer is for: the tab (by id) that ran or applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Run(u64),
    Apply(u64),
    /// `dax.validate_query` of tab `.0` at text hash `.1`.
    Check(u64, u64),
    /// `dax.format` of tab `.0` at text hash `.1`.
    Format(u64, u64),
    Cancel,
    Sync,
}

/// One EVALUATE's result.
#[derive(Debug, Clone)]
pub struct Res {
    /// `None` for an answer without columns and rows.
    pub grid: Option<Grid>,
    pub row_count: usize,
    pub truncated: bool,
}

/// The last Run of a tab.
#[derive(Debug, Clone)]
pub enum Outcome {
    Rows {
        results: Vec<Res>,
        elapsed_ms: Option<f64>,
    },
    Error(String),
}

/// The answer of the last Update model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub ok: bool,
    pub text: String,
}

/// One query tab.
#[derive(Debug, Clone)]
pub struct QueryTab {
    pub id: u64,
    pub name: String,
    pub text: String,
    /// `None` until it runs.
    pub outcome: Option<Outcome>,
    pub running: bool,
    /// The result shown, when the query has several EVALUATEs.
    pub shown: usize,
    pub note: Option<Note>,
    /// The diagnostics of the last check of the text.
    pub diagnostics: Vec<crate::formula::Diag>,
    /// The text hash checked (or being checked), and the hash and time of the last change seen.
    checked: Option<u64>,
    seen: Option<u64>,
    changed_at: f64,
}

/// A hash of a text.
fn hash_of(s: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// The view's state for one document.
#[derive(Debug, Clone, Default)]
pub struct QueryView {
    pub tabs: Vec<QueryTab>,
    pub active: usize,
    next: u64,
    /// The tab being renamed and the name typed.
    pub renaming: Option<(u64, String)>,
    /// Give the editor the focus on the next frame.
    pub focus: bool,
    /// The editor's coloured layout and the key it was built for.
    job: Option<(u64, LayoutJob)>,
    /// The snapshot's `dax_queries.generation` the tabs were loaded from.
    loaded: Option<u64>,
    /// The hash of the tabs the session has, and the hash and time of the last change seen.
    synced: Option<u64>,
    seen: Option<u64>,
    changed_at: f64,
}

impl QueryView {
    /// Makes a first tab when there is none and keeps `active` on a tab.
    pub fn ensure(&mut self, l: Lang) {
        if self.tabs.is_empty() {
            self.add(l, String::new());
        }
        self.active = self.active.min(self.tabs.len().saturating_sub(1));
    }

    /// The tab shown (a first one is made on demand).
    pub fn current(&mut self, l: Lang) -> Option<&mut QueryTab> {
        self.ensure(l);
        self.tabs.get_mut(self.active)
    }

    /// The tab shown, if any.
    pub fn active_tab(&self) -> Option<&QueryTab> {
        self.tabs.get(self.active)
    }

    /// A new tab `Query N` (the first free N) with `text`, shown; none past [`MAX_TABS`].
    pub fn add(&mut self, l: Lang, text: String) -> bool {
        if self.tabs.len() >= MAX_TABS {
            return false;
        }
        let prefix = l.t("Query", "Requête");
        let name = (1..=MAX_TABS + 1)
            .map(|n| format!("{prefix} {n}"))
            .find(|n| !self.tabs.iter().any(|t| t.name.eq_ignore_ascii_case(n)))
            .unwrap_or_else(|| prefix.to_string());
        self.next = self.next.wrapping_add(1);
        self.tabs.push(QueryTab {
            id: self.next,
            name,
            text,
            outcome: None,
            running: false,
            shown: 0,
            note: None,
            diagnostics: Vec::new(),
            checked: None,
            seen: None,
            changed_at: f64::NEG_INFINITY,
        });
        self.active = self.tabs.len() - 1;
        self.focus = true;
        true
    }

    /// Closes the tab `id`; the last one closed leaves a new empty tab.
    pub fn close(&mut self, id: u64, l: Lang) {
        let Some(i) = self.tabs.iter().position(|t| t.id == id) else {
            return;
        };
        self.tabs.remove(i);
        if self.active > i || self.active >= self.tabs.len() {
            self.active = self.active.saturating_sub(1);
        }
        if self.tabs.is_empty() {
            self.add(l, String::new());
        }
    }

    /// A copy of tab `id` after it (its text, not its results).
    pub fn duplicate(&mut self, id: u64, l: Lang) {
        let Some(text) = self.tabs.iter().find(|t| t.id == id).map(|t| t.text.clone()) else {
            return;
        };
        self.add(l, text);
    }

    /// Renames tab `id` (a blank name or one another tab has is refused).
    pub fn rename(&mut self, id: u64, name: &str) -> bool {
        let name: String = name.trim().chars().take(MAX_NAME).collect();
        if name.is_empty()
            || self
                .tabs
                .iter()
                .any(|t| t.id != id && t.name.eq_ignore_ascii_case(&name))
        {
            return false;
        }
        match self.tabs.iter_mut().find(|t| t.id == id) {
            Some(t) => {
                t.name = name;
                true
            }
            None => false,
        }
    }

    fn by_id(&mut self, id: u64) -> Option<&mut QueryTab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    /// The hash of the tabs as the session keeps them (names, texts, the tab shown).
    pub fn tabs_hash(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for t in &self.tabs {
            t.name.hash(&mut h);
            t.text.hash(&mut h);
        }
        self.active_tab().map(|t| t.name.as_str()).hash(&mut h);
        h.finish()
    }

    /// `dax.set_queries`'s parameters for the tabs.
    pub fn set_params(&self) -> Value {
        json!({
            "tabs": self.tabs.iter().map(|t| json!({"name": t.name, "text": t.text})).collect::<Vec<_>>(),
            "default_tab": self.active_tab().map(|t| t.name.clone()),
        })
    }

    /// Replaces the tabs with the file's (`dax_queries` of a snapshot): its tabs, the default one
    /// shown; an empty first tab when it has none.
    pub fn load(&mut self, dq: &Value, l: Lang) {
        self.tabs.clear();
        self.renaming = None;
        self.job = None;
        for t in dq
            .get("tabs")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .take(MAX_TABS)
        {
            let Some(name) = t.get("name").and_then(Value::as_str) else {
                continue;
            };
            let text = t.get("text").and_then(Value::as_str).unwrap_or("");
            if self.add(l, text.to_string())
                && let Some(last) = self.tabs.last_mut()
            {
                last.name = name.chars().take(MAX_NAME).collect();
            }
        }
        self.ensure(l);
        let default = dq.get("default_tab").and_then(Value::as_str);
        self.active = default
            .and_then(|d| self.tabs.iter().position(|t| t.name.eq_ignore_ascii_case(d)))
            .unwrap_or(0);
        self.loaded = dq.get("generation").and_then(Value::as_u64);
        let h = self.tabs_hash();
        self.synced = Some(h);
        self.seen = Some(h);
    }
}

/// Loads the file's tabs when the snapshot comes from a newly opened file.
pub fn adopt(app: &mut App) {
    let Some(dq) = app
        .loaded
        .as_ref()
        .and_then(|x| x.snap.get("dax_queries").cloned())
    else {
        return;
    };
    let generation = dq.get("generation").and_then(Value::as_u64);
    if generation.is_none() || app.cmd.dax_query.loaded == generation {
        return;
    }
    app.cmd.dax_query.load(&dq, app.lang);
}

/// Hands the tabs to the session when they differ from what it has (before a save, and a moment
/// after they change).
pub fn flush(app: &mut App) {
    adopt(app);
    let v = &app.cmd.dax_query;
    if v.loaded.is_none() || v.tabs.is_empty() || crate::ribbon::state(app, SET).is_err() {
        return;
    }
    let h = v.tabs_hash();
    if v.synced == Some(h) {
        return;
    }
    let params = v.set_params();
    app.cmd.dax_query.synced = Some(h);
    app.run(Call { id: SET, params }, Then::DaxQuery(Step::Sync));
}

/// The timed work of the view: the tabs to the session [`SYNC_AFTER`] after they change, the
/// shown tab's text checked [`CHECK_AFTER`] after it changes.
fn tick(app: &mut App, now: f64, ctx: &egui::Context) {
    let v = &mut app.cmd.dax_query;
    let h = v.tabs_hash();
    if v.seen != Some(h) {
        v.seen = Some(h);
        v.changed_at = now;
    }
    if v.synced != Some(h) {
        let wait = SYNC_AFTER - (now - v.changed_at);
        if wait > 0.0 {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait.min(2.0)));
        } else {
            flush(app);
        }
    }
    let check_ok = crate::ribbon::state(app, CHECK).is_ok();
    let active = app.cmd.dax_query.active;
    let Some(tab) = app.cmd.dax_query.tabs.get_mut(active) else {
        return;
    };
    let th = hash_of(&tab.text);
    if tab.seen != Some(th) {
        tab.seen = Some(th);
        tab.changed_at = now;
    }
    if tab.checked == Some(th) {
        return;
    }
    if tab.text.trim().is_empty() || !check_ok {
        tab.checked = Some(th);
        tab.diagnostics.clear();
        return;
    }
    let wait = CHECK_AFTER - (now - tab.changed_at);
    if wait > 0.0 {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait.min(1.0)));
        return;
    }
    tab.checked = Some(th);
    let (id, query) = (tab.id, tab.text.clone());
    app.run(
        Call {
            id: CHECK,
            params: json!({"query": query}),
        },
        Then::DaxQuery(Step::Check(id, th)),
    );
}

/// Format query of the tab shown.
pub fn format(app: &mut App) {
    let l = app.lang;
    let Some(tab) = app.cmd.dax_query.current(l) else {
        return;
    };
    if tab.text.trim().is_empty() {
        app.status = l
            .t("Write a DAX query first.", "Écrivez d'abord une requête DAX.")
            .to_string();
        return;
    }
    let (id, query) = (tab.id, tab.text.clone());
    let h = hash_of(&query);
    app.run(
        Call {
            id: FORMAT,
            params: json!({"query": query}),
        },
        Then::DaxQuery(Step::Format(id, h)),
    );
}

/// Cancel: stops the running query.
pub fn cancel(app: &mut App) {
    app.run(
        Call {
            id: CANCEL,
            params: json!({}),
        },
        Then::DaxQuery(Step::Cancel),
    );
}

/// A `MEASURE Table[Name] =` of the query's DEFINE block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lens {
    pub table: String,
    pub name: String,
    /// Byte offset of `MEASURE`.
    pub at: usize,
    /// The model has a measure of that name.
    pub exists: bool,
}

/// The measures `text` defines (between DEFINE and the first EVALUATE), each with whether the
/// model in `snap` has one of that name. The engine's parser decides what is applied; this only
/// places the lenses.
pub fn lenses(text: &str, snap: Option<&Value>) -> Vec<Lens> {
    let toks = dax_text::lex(text);
    let word = |i: usize, w: &str| {
        toks.get(i).is_some_and(|t| {
            t.class == Class::Keyword
                && text
                    .get(t.range.clone())
                    .is_some_and(|s| s.eq_ignore_ascii_case(w))
        })
    };
    let Some(start) = (0..toks.len()).find(|i| word(*i, "DEFINE")) else {
        return Vec::new();
    };
    let model: Vec<String> = snap
        .and_then(|s| s.pointer("/model/tables"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|t| t.get("measures").and_then(Value::as_array))
        .flatten()
        .filter_map(|m| m.get("name").and_then(Value::as_str))
        .map(str::to_lowercase)
        .collect();
    let mut out = Vec::new();
    let mut i = start + 1;
    while i < toks.len() && !word(i, "EVALUATE") {
        if word(i, "MEASURE")
            && let (Some(t), Some(c), Some(eq)) = (toks.get(i + 1), toks.get(i + 2), toks.get(i + 3))
            && t.class == Class::Table
            && c.class == Class::Column
            && text.get(eq.range.clone()) == Some("=")
        {
            let table = dax_text::unquote_table(text.get(t.range.clone()).unwrap_or(""));
            let raw = text.get(c.range.clone()).unwrap_or("");
            let name = raw
                .strip_prefix('[')
                .and_then(|r| r.strip_suffix(']'))
                .unwrap_or(raw)
                .replace("]]", "]");
            if !name.is_empty() && out.len() < 10_000 {
                let exists = model.contains(&name.to_lowercase());
                let at = toks.get(i).map_or(0, |t| t.range.start);
                out.push(Lens {
                    table,
                    name,
                    at,
                    exists,
                });
            }
            i += 4;
            continue;
        }
        i += 1;
    }
    out
}

/// What a lens says: `Update model: Add new measure` or `… Overwrite measure`.
pub fn lens_text(exists: bool, l: Lang) -> &'static str {
    if exists {
        l.t(
            "Update model: Overwrite measure",
            "Mettre à jour le modèle : remplacer la mesure",
        )
    } else {
        l.t(
            "Update model: Add new measure",
            "Mettre à jour le modèle : ajouter la mesure",
        )
    }
}

/// A lens's accessible label: its text and the measure.
pub fn lens_label(lens: &Lens, l: Lang) -> String {
    format!("{} {}", lens_text(lens.exists, l), lens.name)
}

/// The view's tool bar button for every lens.
pub fn apply_all_text(l: Lang) -> &'static str {
    l.t(
        "Update model with changes",
        "Mettre à jour le modèle avec les modifications",
    )
}

/// Run DAX query and Update model with changes, from the ribbon or the palette without a
/// query: open the view and act on its tab.
pub fn intercept(app: &mut App, id: &str, params: &Value) -> bool {
    if id == SET && params.get("tabs").is_none() {
        // the palette's Keep DAX query tabs: the view's tabs now
        app.cmd.dax_query.synced = None;
        flush(app);
        return true;
    }
    if !matches!(id, RUN | APPLY | FORMAT) || params.get("query").is_some() {
        return false;
    }
    app.show_tab(View::Query);
    adopt(app);
    match id {
        RUN => run(app),
        FORMAT => format(app),
        _ => apply(app, None),
    }
    true
}

/// Runs the tab shown (nothing for an empty query, with a word in the status bar).
pub fn run(app: &mut App) {
    let l = app.lang;
    let Some(tab) = app.cmd.dax_query.current(l) else {
        return;
    };
    if tab.text.trim().is_empty() {
        app.status = l
            .t("Write a DAX query first.", "Écrivez d'abord une requête DAX.")
            .to_string();
        return;
    }
    let (id, query) = (tab.id, tab.text.clone());
    tab.running = true;
    tab.note = None;
    let sent = app.run(
        Call {
            id: RUN,
            params: json!({"query": query}),
        },
        Then::DaxQuery(Step::Run(id)),
    );
    if !sent && let Some(t) = app.cmd.dax_query.by_id(id) {
        t.running = false;
    }
}

/// Update model for measure `only` (every DEFINE MEASURE when `None`) of the tab shown.
pub fn apply(app: &mut App, only: Option<&str>) {
    let l = app.lang;
    let snap = app.loaded.as_ref().map(|x| x.snap.clone());
    let Some(tab) = app.cmd.dax_query.current(l) else {
        return;
    };
    if lenses(&tab.text, snap.as_deref()).is_empty() {
        app.status = l
            .t(
                "The query defines no measure (DEFINE MEASURE Table[Name] = …).",
                "La requête ne définit aucune mesure (DEFINE MEASURE Table[Nom] = …).",
            )
            .to_string();
        return;
    }
    let (id, query) = (tab.id, tab.text.clone());
    tab.note = None;
    let mut params = json!({"query": query});
    if let Some(name) = only {
        params["measures"] = json!([name]);
    }
    app.run(Call { id: APPLY, params }, Then::DaxQuery(Step::Apply(id)));
}

/// The answer of a Run or an Update model, to its tab.
pub fn answer(app: &mut App, step: Step, r: Result<Value, String>) {
    let l = app.lang;
    match step {
        Step::Run(id) => {
            let Some(t) = app.cmd.dax_query.by_id(id) else {
                return;
            };
            t.running = false;
            t.shown = 0;
            let r = match r {
                Err(e) if e.contains(CANCELLED) => Err(l
                    .t("The query was cancelled.", "La requête a été annulée.")
                    .to_string()),
                r => r,
            };
            t.outcome = Some(match r {
                Ok(v) => Outcome::Rows {
                    results: v
                        .get("results")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .map(result)
                        .collect(),
                    elapsed_ms: v.get("elapsed_ms").and_then(Value::as_f64),
                },
                Err(e) => Outcome::Error(crate::app::clip(&e, 2000)),
            });
        }
        Step::Apply(id) => {
            let note = match r {
                Ok(v) => Note {
                    ok: true,
                    text: applied_text(&v, l),
                },
                Err(e) => Note {
                    ok: false,
                    text: crate::app::clip(&e, 2000),
                },
            };
            if note.ok {
                app.status = note.text.clone();
            }
            if let Some(t) = app.cmd.dax_query.by_id(id) {
                t.note = Some(note);
            }
        }
        Step::Check(id, h) => {
            let Some(t) = app.cmd.dax_query.by_id(id) else {
                return;
            };
            if hash_of(&t.text) != h {
                return;
            }
            let Ok(v) = r else { return };
            t.diagnostics = diagnostics(&v);
            app.cmd.dax_query.job = None;
        }
        Step::Format(id, h) => {
            let Some(t) = app.cmd.dax_query.by_id(id) else {
                return;
            };
            match r {
                Ok(v) => {
                    // a text typed meanwhile wins
                    if hash_of(&t.text) == h
                        && let Some(q) = v.get("query").and_then(Value::as_str)
                    {
                        if q != t.text {
                            t.text = q.to_string();
                        }
                        app.status = l.t("Query formatted.", "Requête mise en forme.").to_string();
                    }
                }
                Err(e) => {
                    t.note = Some(Note {
                        ok: false,
                        text: crate::app::clip(&e, 2000),
                    });
                }
            }
        }
        Step::Cancel => {
            if let Err(e) = r {
                app.status = crate::app::clip(&e, 400);
            }
        }
        Step::Sync => {
            if let Err(e) = r {
                // sent again after the next change
                app.cmd.dax_query.synced = None;
                app.status = crate::app::clip(&e, 400);
            }
        }
    }
}

/// The engine's text of a cancelled evaluation (`bicraft_engine::CANCELLED`; the UI does not
/// link the engine).
const CANCELLED: &str = "cancelled by the client";

/// `dax.validate_query`'s diagnostics (the first 50).
fn diagnostics(v: &Value) -> Vec<crate::formula::Diag> {
    v.get("diagnostics")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(50)
        .map(|d| {
            let n = |p: &str| d.pointer(p).and_then(Value::as_u64).unwrap_or(0);
            let start = usize::try_from(n("/span/start")).unwrap_or(0);
            let end = usize::try_from(n("/span/end")).unwrap_or(0);
            crate::formula::Diag {
                message: d
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                span: start..end.max(start),
                line: n("/line"),
                column: n("/column"),
                error: d.get("severity").and_then(Value::as_str) != Some("warning"),
            }
        })
        .collect()
}

/// The first error's bytes (underlined), when they are in `text`.
pub fn underline(diags: &[crate::formula::Diag], text: &str) -> Option<std::ops::Range<usize>> {
    let d = diags.iter().find(|d| d.error)?;
    let end = d.span.end.min(text.len());
    let start = d.span.start.min(end);
    // an empty span (at the end of the text): the character before it
    let start = if start == end {
        text.get(..end)
            .and_then(|s| s.char_indices().last())
            .map_or(start, |(i, _)| i)
    } else {
        start
    };
    (start < end && text.is_char_boundary(start) && text.is_char_boundary(end)).then_some(start..end)
}

/// One `dax.evaluate` result as a grid (`{columns, rows, row_count, truncated}`).
fn result(v: &Value) -> Res {
    let grid = answer::Answer::new(v).grids.into_iter().next();
    let rows = grid.as_ref().map_or(0, |g| g.rows.len());
    Res {
        row_count: v
            .get("row_count")
            .and_then(Value::as_u64)
            .map_or(rows, |n| usize::try_from(n).unwrap_or(usize::MAX)),
        truncated: v.get("truncated").and_then(Value::as_bool).unwrap_or(false),
        grid,
    }
}

/// `2 measures added, 1 overwritten` in the window's language.
fn applied_text(v: &Value, l: Lang) -> String {
    let n = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
    let (added, updated, unchanged) = (n("added"), n("updated"), n("unchanged"));
    let mut parts = Vec::new();
    if added > 0 {
        parts.push(match l {
            Lang::En => format!(
                "{added} {} added",
                if added == 1 { "measure" } else { "measures" }
            ),
            Lang::Fr => format!(
                "{added} {} ajoutée{}",
                if added == 1 { "mesure" } else { "mesures" },
                if added == 1 { "" } else { "s" }
            ),
        });
    }
    if updated > 0 {
        parts.push(match l {
            Lang::En => format!("{updated} overwritten"),
            Lang::Fr => format!("{updated} remplacée{}", if updated == 1 { "" } else { "s" }),
        });
    }
    if unchanged > 0 {
        parts.push(match l {
            Lang::En => format!("{unchanged} already in the model"),
            Lang::Fr => format!("{unchanged} déjà dans le modèle"),
        });
    }
    if parts.is_empty() {
        return l.t("Nothing to update.", "Rien à mettre à jour.").to_string();
    }
    let mut s = parts.join(", ");
    s.push('.');
    match l {
        Lang::En => format!("Model updated: {s}"),
        Lang::Fr => format!("Modèle mis à jour : {s}"),
    }
}

/// Ctrl+Enter and F5 run the tab shown, in the view, even while the editor has the focus.
pub fn keys(app: &mut App, ui: &Ui) -> bool {
    if app.tab != View::Query || app.form.is_some() || app.palette.is_some() || app.confirm.is_some() {
        return false;
    }
    let hit = ui.input_mut(|i| {
        i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter)
            || i.consume_key(egui::Modifiers::NONE, egui::Key::F5)
    });
    if hit {
        run(app);
    }
    hit
}

// --- drawing --------------------------------------------------------------------------------

/// What a click asked for (done after drawing).
enum Act {
    None,
    Run,
    Apply(Option<String>),
    Format,
    Cancel,
}

/// The view: the tool bar, the editor with its lenses, the results, the tab strip.
pub fn show(app: &mut App, ui: &mut Ui) {
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    let snap = app.loaded.as_ref().map(|x| x.snap.clone());
    adopt(app);
    app.cmd.dax_query.ensure(l);
    let now = ui.input(|i| i.time);
    tick(app, now, ui.ctx());
    let rect = ui.max_rect();
    let bar = Rect::from_min_size(rect.min, vec2(rect.width(), BAR_H));
    let strip = Rect::from_min_max(pos2(rect.min.x, rect.max.y - TABS_H), rect.max);
    let body = Rect::from_min_max(pos2(rect.min.x, bar.max.y), pos2(rect.max.x, strip.min.y));
    let editor_h = (body.height() * 0.45).clamp(80.0, 520.0);
    let editor = Rect::from_min_size(
        body.min + vec2(12.0, 8.0),
        vec2((body.width() - 24.0).max(40.0), editor_h),
    );
    let results = Rect::from_min_max(
        pos2(body.min.x + 12.0, editor.max.y + 10.0),
        pos2(body.max.x - 12.0, (body.max.y - 4.0).max(editor.max.y + 10.0)),
    );
    let lens_list = app
        .cmd
        .dax_query
        .active_tab()
        .map(|q| lenses(&q.text, snap.as_deref()))
        .unwrap_or_default();
    let mut act = Act::None;
    ui.painter().rect_filled(bar, CornerRadius::ZERO, t.chrome);
    ui.painter()
        .hline(bar.x_range(), bar.max.y - 0.5, Stroke::new(1.0, t.separator));
    let mut bui = ui.new_child(egui::UiBuilder::new().max_rect(bar.shrink2(vec2(12.0, 6.0))));
    tool_bar(app, &mut bui, lens_list.len(), &mut act);
    let mut eui = ui.new_child(egui::UiBuilder::new().max_rect(editor));
    editor_ui(app, &mut eui, editor, &lens_list, &mut act);
    let mut rui = ui.new_child(egui::UiBuilder::new().max_rect(results));
    results_ui(app, &mut rui, results);
    ui.painter().rect_filled(strip, CornerRadius::ZERO, t.chrome);
    ui.painter()
        .hline(strip.x_range(), strip.min.y + 0.5, Stroke::new(1.0, t.separator));
    let mut sui = ui.new_child(egui::UiBuilder::new().max_rect(strip.shrink2(vec2(8.0, 2.0))));
    tab_strip(app, &mut sui);
    match act {
        Act::None => {}
        Act::Run => run(app),
        Act::Apply(only) => apply(app, only.as_deref()),
        Act::Format => format(app),
        Act::Cancel => cancel(app),
    }
}

/// Run, Update model with changes, and the running mark.
fn tool_bar(app: &mut App, ui: &mut Ui, lenses: usize, act: &mut Act) {
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    let run_ok = crate::ribbon::state(app, RUN);
    let apply_ok = crate::ribbon::state(app, APPLY);
    let empty = app
        .cmd
        .dax_query
        .active_tab()
        .is_none_or(|q| q.text.trim().is_empty());
    let running = app.cmd.dax_query.active_tab().is_some_and(|q| q.running);
    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let why = match &run_ok {
            Err(e) => e.clone(),
            Ok(()) => l
                .t("Write a DAX query first", "Écrivez d'abord une requête DAX")
                .to_string(),
        };
        let r = widgets::primary(ui, l.t("Run", "Exécuter"), run_ok.is_ok() && !empty, &why);
        if r.on_hover_text(l.t(
            "Run the query (Ctrl+Enter, F5)",
            "Exécuter la requête (Ctrl+Entrée, F5)",
        ))
        .clicked()
        {
            *act = Act::Run;
        }
        let label = format!("{} ({lenses})", apply_all_text(l));
        let enabled = apply_ok.is_ok() && lenses > 0;
        let tip = match &apply_ok {
            Err(e) => e.clone(),
            Ok(()) if lenses == 0 => l
                .t(
                    "Define measures in the query first (DEFINE MEASURE Table[Name] = …)",
                    "Définissez d'abord des mesures dans la requête (DEFINE MEASURE Table[Nom] = …)",
                )
                .to_string(),
            Ok(()) => l
                .t(
                    "Add or overwrite every measure the query defines, in one step",
                    "Ajouter ou remplacer chaque mesure définie par la requête, en une étape",
                )
                .to_string(),
        };
        let r = widgets::small_button(ui, Icon::Measure, &label, enabled, false);
        if r.on_hover_text(tip).clicked() && enabled {
            *act = Act::Apply(None);
        }
        let format_ok = crate::ribbon::state(app, FORMAT);
        let enabled = format_ok.is_ok() && !empty;
        let r = widgets::small_button(ui, Icon::Format, l.t("Format", "Mettre en forme"), enabled, false);
        let tip = match &format_ok {
            Err(e) => e.clone(),
            Ok(()) => l
                .t(
                    "Lay the query out: one clause per line, long calls broken (comments kept)",
                    "Mettre la requête en page : une clause par ligne, les appels longs coupés (commentaires gardés)",
                )
                .to_string(),
        };
        if r.on_hover_text(tip).clicked() && enabled {
            *act = Act::Format;
        }
        if running {
            let cancel_ok = crate::ribbon::state(app, CANCEL).is_ok();
            let r = widgets::small_button(ui, Icon::Close, l.t("Cancel", "Annuler"), cancel_ok, false);
            if r
                .on_hover_text(l.t("Stop the running query", "Arrêter la requête en cours"))
                .clicked()
                && cancel_ok
            {
                *act = Act::Cancel;
            }
            ui.spinner();
            ui.label(RichText::new(l.t("Running…", "Exécution…")).color(t.text_dim));
        }
    });
}

/// The editor of the tab shown, its colours from the formula bar's, and a lens at the right end
/// of each DEFINE MEASURE line.
fn editor_ui(app: &mut App, ui: &mut Ui, rect: Rect, lens_list: &[Lens], act: &mut Act) {
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    let ctx = ui.ctx().clone();
    let apply_ok = crate::ribbon::state(app, APPLY).is_ok();
    let v = &mut app.cmd.dax_query;
    let Some(tab_id) = v.active_tab().map(|q| q.id) else {
        return;
    };
    let id = Id::new(("bicraft-dax-query-editor", tab_id));
    if v.focus {
        ctx.memory_mut(|m| m.request_focus(id));
        v.focus = false;
    }
    let focused = ctx.memory(|m| m.has_focus(id));
    let dark = t.dark;
    let QueryView {
        tabs, active, job, ..
    } = v;
    let Some(tab) = tabs.get_mut(*active) else {
        return;
    };
    // the first error of the last check, while the text is the one checked
    let under = if tab.checked == Some(hash_of(&tab.text)) {
        underline(&tab.diagnostics, &tab.text)
    } else {
        None
    };
    let checked = tab.checked;
    let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| -> Arc<Galley> {
        let s = buf.as_str();
        let under = under
            .clone()
            .filter(|u| u.end <= s.len() && checked == Some(hash_of(s)));
        let mut hs = std::collections::hash_map::DefaultHasher::new();
        s.hash(&mut hs);
        wrap.to_bits().hash(&mut hs);
        dark.hash(&mut hs);
        under.hash(&mut hs);
        let key = hs.finish();
        if job.as_ref().is_none_or(|(k, _)| *k != key) {
            *job = Some((key, crate::formula::job(s, &Tokens::get(ui.ctx()), under, wrap)));
        }
        let laid = job.as_ref().map(|(_, j)| j.clone()).unwrap_or_default();
        ui.fonts_mut(|f| f.layout_job(laid))
    };
    let inner = rect.shrink(1.0);
    let mut out = None;
    egui::Frame::NONE
        .fill(t.field_bg)
        .stroke(Stroke::new(1.0, if focused { t.ring } else { t.border }))
        .corner_radius(CornerRadius::same(4))
        .inner_margin(egui::Margin::same(6))
        .show(ui, |ui| {
            ui.set_min_size(rect.size() - vec2(14.0, 14.0));
            egui::ScrollArea::both()
                .id_salt(("bicraft-dax-query-scroll", tab_id))
                .max_height(rect.height() - 14.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let o = egui::TextEdit::multiline(&mut tab.text)
                        .id(id)
                        .font(crate::formula::mono())
                        .code_editor()
                        .frame(egui::Frame::NONE)
                        .desired_width(f32::INFINITY)
                        .desired_rows(8)
                        .hint_text(l.t(
                            "Write a DAX query: EVALUATE and a table expression, DEFINE MEASURE blocks before it",
                            "Écrivez une requête DAX : EVALUATE et une expression de table, les blocs DEFINE MEASURE avant",
                        ))
                        .layouter(&mut layouter)
                        .show(ui);
                    o.response.response.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, l.t("DAX query", "Requête DAX"))
                    });
                    out = Some((o.galley_pos, o.galley.clone()));
                });
        });
    let Some((at, galley)) = out else { return };
    let text = tab.text.clone();
    let font = theme::font(11.0);
    for lens in lens_list {
        let ci = dax_text::char_of_byte(&text, lens.at);
        let row = galley.pos_from_cursor(CCursor::new(ci));
        let y = at.y + row.center().y;
        if y < inner.min.y + 10.0 || y > inner.max.y - 10.0 {
            continue;
        }
        let label = lens_text(lens.exists, l);
        let w = ui.fonts_mut(|f| f.layout_no_wrap(label.to_string(), font.clone(), t.text).size().x) + 30.0;
        let lh = (row.height() - 1.0).clamp(12.0, 20.0);
        let r = Rect::from_min_size(pos2(inner.max.x - w - 16.0, y - lh / 2.0), vec2(w, lh));
        let sense = if apply_ok { Sense::click() } else { Sense::hover() };
        let resp = ui.interact(
            r,
            Id::new(("bicraft-dax-lens", tab_id, lens.name.as_str())),
            sense,
        );
        let p = ui.painter().with_clip_rect(inner);
        let fill = if resp.hovered() && apply_ok {
            t.hover
        } else {
            t.card
        };
        p.rect_filled(r, CornerRadius::same(7), fill);
        p.rect_stroke(
            r,
            CornerRadius::same(7),
            Stroke::new(1.0, t.border),
            egui::StrokeKind::Inside,
        );
        let ink = if apply_ok { t.accent_text } else { t.text_faint };
        icons::paint(
            &p,
            Rect::from_center_size(pos2(r.min.x + 13.0, r.center().y), vec2(12.0, 12.0)),
            Icon::Measure,
            ink,
            !apply_ok,
        );
        p.text(
            pos2(r.min.x + 23.0, r.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            font.clone(),
            ink,
        );
        let full = lens_label(lens, l);
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, apply_ok, &full));
        let tip = format!("{label} {}[{}]", lens.table, lens.name);
        if resp.on_hover_text(tip).clicked() && apply_ok {
            *act = Act::Apply(Some(lens.name.clone()));
        }
    }
}

/// The last Update model's answer, then the results of the last Run: the result picked (when
/// there are several), its row count and the grid, or the error.
fn results_ui(app: &mut App, ui: &mut Ui, rect: Rect) {
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    let v = &mut app.cmd.dax_query;
    let Some(tab) = v.tabs.get_mut(v.active) else {
        return;
    };
    let tab_id = tab.id;
    // the check's first error, unless a Run or an Update model already tells one
    let told = matches!(tab.outcome, Some(Outcome::Error(_))) || tab.note.as_ref().is_some_and(|n| !n.ok);
    if !told
        && tab.checked == Some(hash_of(&tab.text))
        && let Some(d) = tab.diagnostics.iter().find(|d| d.error)
    {
        let text = match l {
            Lang::En => format!("Line {}, column {}: {}", d.line, d.column, d.message),
            Lang::Fr => format!("Ligne {}, colonne {} : {}", d.line, d.column, d.message),
        };
        status_line(ui, &t, false, &text, l);
        ui.add_space(6.0);
    }
    if let Some(n) = &tab.note {
        status_line(ui, &t, n.ok, &n.text, l);
        ui.add_space(6.0);
    }
    let Some(outcome) = tab.outcome.as_mut() else {
        if !tab.running {
            ui.label(
                RichText::new(l.t(
                    "Run the query to see its results here (Ctrl+Enter or F5).",
                    "Exécutez la requête pour voir ses résultats ici (Ctrl+Entrée ou F5).",
                ))
                .color(t.text_dim),
            );
        }
        return;
    };
    let (results, elapsed) = match outcome {
        Outcome::Error(e) => {
            status_line(ui, &t, false, e, l);
            return;
        }
        Outcome::Rows { results, elapsed_ms } => (results, *elapsed_ms),
    };
    if results.is_empty() {
        ui.label(
            RichText::new(l.t(
                "The query returned no table.",
                "La requête n'a renvoyé aucune table.",
            ))
            .color(t.text_dim),
        );
        return;
    }
    if results.len() > 1 {
        ui.horizontal(|ui| {
            for k in 0..results.len() {
                let name = format!("{} {}", l.t("Result", "Résultat"), k + 1);
                if ui.selectable_label(tab.shown == k, name).clicked() {
                    tab.shown = k;
                }
            }
        });
    }
    let k = tab.shown.min(results.len() - 1);
    let Some(res) = results.get_mut(k) else { return };
    let shown = res.grid.as_ref().map_or(0, |g| g.rows.len());
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
        icons::paint(ui.painter(), r, Icon::Table, t.text_dim, false);
        ui.label(RichText::new(row_summary(res.row_count, res.truncated, shown, l)).color(t.text));
        if let Some(ms) = elapsed.filter(|m| m.is_finite()) {
            ui.label(RichText::new(format!("· {ms:.0} ms")).color(t.text_dim));
        }
    });
    ui.add_space(2.0);
    let max_h = (rect.max.y - ui.cursor().min.y - 32.0).max(answer::ROW_H * 4.0);
    match res.grid.as_mut() {
        Some(g) => answer::grid(ui, g, Id::new(("bicraft-dax-grid", tab_id, k)), max_h, &t, l),
        None => {
            ui.label(RichText::new(l.t("(no columns)", "(aucune colonne)")).color(t.text_faint));
        }
    }
}

/// `12 rows` (with how many are shown when the result was cut).
pub fn row_summary(rows: usize, truncated: bool, shown: usize, l: Lang) -> String {
    let n = answer::count(rows, l);
    let mut s = match (l, rows) {
        (Lang::En, 1) => "1 row".to_string(),
        (Lang::En, _) => format!("{n} rows"),
        (Lang::Fr, 0 | 1) => format!("{n} ligne"),
        (Lang::Fr, _) => format!("{n} lignes"),
    };
    if truncated {
        let k = answer::count(shown, l);
        s.push_str(&match l {
            Lang::En => format!(", the first {k} shown"),
            Lang::Fr => format!(", les {k} premières affichées"),
        });
    }
    s
}

/// A message with its icon and its word (an error is never told by colour alone).
fn status_line(ui: &mut Ui, t: &Tokens, ok: bool, text: &str, l: Lang) {
    egui::Frame::NONE
        .fill(if ok { t.card } else { t.err_soft })
        .stroke(Stroke::new(1.0, if ok { t.card_border } else { t.err_soft }))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                let (icon, ink, word, color) = if ok {
                    (Icon::Check, t.ok, l.t("Done", "Terminé"), t.text)
                } else {
                    (Icon::Warning, t.err, l.t("Error", "Erreur"), t.err_text)
                };
                let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                icons::paint(ui.painter(), r, icon, ink, false);
                ui.label(RichText::new(word).font(theme::bold(13.0)).color(color));
                ui.add(egui::Label::new(RichText::new(text).color(color)).wrap());
            });
        });
}

/// What the tab strip asked for (done after drawing).
#[derive(Default)]
struct StripAct {
    select: Option<usize>,
    close: Option<u64>,
    duplicate: Option<u64>,
    /// Start renaming, or the name being typed.
    renaming: Option<(u64, String)>,
    commit: Option<(u64, String)>,
    cancel: bool,
    add: bool,
}

/// The query tabs: a click shows one, a double click or its menu renames it, its menu duplicates
/// or closes it, × closes it, + adds one.
fn tab_strip(app: &mut App, ui: &mut Ui) {
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    let mut a = StripAct::default();
    let v = &app.cmd.dax_query;
    egui::ScrollArea::horizontal()
        .id_salt("bicraft-dax-tabs")
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (i, q) in v.tabs.iter().enumerate() {
                    if let Some((rid, name)) = &v.renaming
                        && *rid == q.id
                    {
                        let mut name = name.clone();
                        let r = ui.add(egui::TextEdit::singleline(&mut name).desired_width(140.0));
                        let r = r.on_hover_text(l.t(
                            "Enter to rename, Esc to cancel",
                            "Entrée pour renommer, Échap pour annuler",
                        ));
                        r.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::TextEdit,
                                true,
                                l.t("Query name", "Nom de la requête"),
                            )
                        });
                        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            a.cancel = true;
                        } else if r.lost_focus() {
                            a.commit = Some((q.id, name));
                        } else {
                            if !r.has_focus() {
                                r.request_focus();
                            }
                            a.renaming = Some((q.id, name));
                        }
                        continue;
                    }
                    let r = widgets::tab(ui, &crate::app::clip(&q.name, 40), i == v.active, false);
                    if r.double_clicked() {
                        a.renaming = Some((q.id, q.name.clone()));
                    } else if r.clicked() {
                        a.select = Some(i);
                    }
                    crate::keyboard::context_menu(&r, |ui| {
                        if ui.button(l.t("Rename", "Renommer")).clicked() {
                            a.renaming = Some((q.id, q.name.clone()));
                            ui.close();
                        }
                        if ui.button(l.t("Duplicate", "Dupliquer")).clicked() {
                            a.duplicate = Some(q.id);
                            ui.close();
                        }
                        if ui.button(l.t("Close", "Fermer")).clicked() {
                            a.close = Some(q.id);
                            ui.close();
                        }
                    });
                    let (cr, cresp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
                    if cresp.hovered() {
                        ui.painter().rect_filled(cr, CornerRadius::same(4), t.hover);
                    }
                    icons::paint(ui.painter(), cr.shrink(4.0), Icon::Close, t.text_dim, false);
                    let tip = match l {
                        Lang::En => format!("Close {}", q.name),
                        Lang::Fr => format!("Fermer {}", q.name),
                    };
                    cresp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &tip));
                    if cresp.on_hover_text(tip.clone()).clicked() {
                        a.close = Some(q.id);
                    }
                    ui.add_space(8.0);
                }
                let full = v.tabs.len() >= MAX_TABS;
                let sense = if full { Sense::hover() } else { Sense::click() };
                let (pr, presp) = ui.allocate_exact_size(vec2(24.0, 22.0), sense);
                if presp.hovered() && !full {
                    ui.painter().rect_filled(pr, CornerRadius::same(4), t.hover);
                }
                icons::paint(ui.painter(), pr.shrink(5.0), Icon::Plus, t.text, full);
                let tip = l.t("New query", "Nouvelle requête");
                presp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, !full, tip));
                if presp.on_hover_text(tip).clicked() && !full {
                    a.add = true;
                }
            });
        });
    let v = &mut app.cmd.dax_query;
    if let Some((id, name)) = a.commit {
        v.renaming = None;
        if !v.rename(id, &name) {
            app.status = l
                .t(
                    "A query tab needs a name that no other tab has.",
                    "Un onglet de requête a besoin d'un nom qu'aucun autre onglet n'a.",
                )
                .to_string();
        }
    } else if a.cancel {
        v.renaming = None;
    } else if a.renaming.is_some() {
        v.renaming = a.renaming;
    }
    let v = &mut app.cmd.dax_query;
    if let Some(i) = a.select {
        v.active = i;
        v.focus = true;
    }
    if let Some(id) = a.duplicate {
        v.duplicate(id, l);
    }
    if let Some(id) = a.close {
        v.close(id, l);
    }
    if a.add {
        v.add(l, String::new());
    }
}
