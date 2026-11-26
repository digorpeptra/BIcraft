//! The DAX formula bar under the ribbon, as Power BI Desktop has it (docs/design.md §13,
//! docs/desktop.md "Formula bar"): it appears when a measure, a calculated column or a
//! calculated table is selected (the Data pane, a Build pane well, the model diagram), shows
//! `Name = <DAX>` in a monospace editor with the DAX colours of the theme, one line or expanded
//! (the chevron), and commits through the engine's commands: `measure.set` (expression and
//! rename in one step), `column.set`, `partition.set` (a calculated table: the expression, which
//! re-derives its columns, and the rename in one step), `measure.add` for a new measure.
//! Enter or ✓ commits, Shift+Enter or Alt+Enter adds a line, Esc or ✗ cancels; a click away
//! from the bar, or another object picked, commits too, as in Desktop. Invalid DAX is
//! committed with `force` and its error stays under the bar ("Saved with an error"). Format DAX
//! lays the expression out with `dax.format {style: formula}` (BIcraft's style after SQLBI's
//! public conventions); the list shows the selected function's description (`dax.functions`).
//! Autocomplete lists the engine's DAX functions (`dax.functions`), the model's tables, columns
//! and measures and the keywords; the signature of the call being typed shows under the cursor;
//! `dax.validate` checks the text a moment after typing stops (through the source, so local,
//! remote and the browser behave alike), the error underlined with a wavy line and its message
//! under the bar. The bracket pair at the cursor is highlighted; Ctrl+H (or the bar's button)
//! opens find and replace over the text, pure view state: the commit stays the command's.
//! Readers see the expression read-only.
//!
//! The panes and the diagram hand their selection over with [`pick`] (egui's temporary memory),
//! so they need no access to the bar's state.

use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;

use egui::text::{CCursor, CCursorRange, LayoutJob, TextFormat};
use egui::{
    Color32, Event, FontId, Galley, Id, Key, KeyboardShortcut, Modifiers, Rect, RichText, Stroke, Ui, vec2,
};
use serde_json::{Value, json};

use crate::App;
use crate::actions::Call;
use crate::app::{Tab as View, Then};
use crate::dax_text::{self, Class, Func, Names, Suggestion, Want};
use crate::i18n::Lang;
use crate::icons::{self, Icon};
use crate::model_view::Sel;
use crate::ribbon;
use crate::theme::Tokens;

/// What the bar edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Measure,
    /// A calculated column.
    Column,
    /// A calculated table.
    Table,
}

/// The object on the bar. For a table, `table` and `name` are both its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub kind: Kind,
    pub table: String,
    pub name: String,
    /// A new measure: committed with `measure.add`.
    pub new: bool,
}

/// What an answer of the bar's commands is for ([`Then::Formula`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// The full expression (`measure.get`, `column.get`, `table.get`) of generation `.0`.
    Load(u64),
    /// `dax.validate` of edit `.0`.
    Validate(u64),
    /// A commit command of generation `.0`.
    Commit(u64),
    /// `dax.functions`.
    Functions,
    /// `dax.format` (Format DAX) of edit `.0`.
    Format(u64),
}

/// One diagnostic, its span in bytes of the bar's text.
#[derive(Debug, Clone, PartialEq)]
pub struct Diag {
    pub message: String,
    pub span: Range<usize>,
    pub line: u64,
    pub column: u64,
    pub error: bool,
}

/// The autocomplete list.
#[derive(Debug, Clone, PartialEq)]
pub struct Completion {
    pub items: Vec<Suggestion>,
    pub selected: usize,
    /// The bytes of the text a suggestion replaces.
    pub replace: Range<usize>,
}

/// The bar's state (one for the active document; reset when another document shows).
#[derive(Default)]
pub struct FormulaBar {
    pub target: Option<Target>,
    /// `Name = expression`, as edited.
    pub text: String,
    /// The text as loaded (or as committed): the bar is dirty when `text` differs.
    pub original: String,
    /// Waiting for the full expression.
    pub loading: bool,
    /// The bar shows several lines (the chevron).
    pub expanded: bool,
    /// The diagnostics of the last check of the text.
    pub diagnostics: Vec<Diag>,
    /// The last commit's refusal, shown under the bar.
    pub error: Option<String>,
    /// The autocomplete list, when open.
    pub completion: Option<Completion>,
    /// The measure's format string and display folder (from its `measure.get`).
    pub format: Option<String>,
    generation: u64,
    /// The snapshot revision the text was loaded at (`None`: to load).
    loaded_rev: Option<Option<u64>>,
    focus: bool,
    /// Where the cursor goes on the next frame (a character index).
    pub cursor_to: Option<usize>,
    edits: u64,
    edited_at: f64,
    /// (edit, byte where the expression starts) of the check in flight or done.
    checked: Option<(u64, usize)>,
    committing: usize,
    commit_name: Option<String>,
    /// The error a commit in flight saved the DAX with (shown when the commit ends).
    saved_error: Option<String>,
    functions: Option<Arc<(Vec<Func>, Vec<String>)>>,
    functions_asked: bool,
    last_sel: Option<Option<Sel>>,
    doc: Option<(usize, String)>,
    galley: Option<(u64, LayoutJob)>,
    /// The time the last frame took to lay the text out (tests of large expressions).
    pub layout_ms: f64,
    /// Find and replace over the text, when open (view state: the text changes as typing does).
    pub find: Option<Find>,
    /// (edit, cursor byte) and the bracket pair there, worked out once per change.
    brackets: Option<((u64, usize), Option<Pair>)>,
}

/// A bracket and its partner (byte ranges of the text).
type Pair = (Range<usize>, Range<usize>);

/// The find and replace row of the bar.
#[derive(Clone, Debug, Default, PartialEq, Hash)]
pub struct Find {
    pub needle: String,
    pub replacement: String,
    /// Letter case must match (else ASCII letters match in either case, as DAX names do).
    pub case: bool,
    /// The match the next Replace replaces (an index into the matches).
    pub current: usize,
    /// The find field takes the keyboard on the next frame.
    focus: bool,
}

impl Find {
    /// The matches in `text`.
    pub fn matches(&self, text: &str) -> Vec<Range<usize>> {
        dax_text::find_all(text, &self.needle, self.case)
    }
}

/// The find field's id (tests).
pub fn find_id() -> Id {
    Id::unique("bicraft-formula-find")
}

fn replace_id() -> Id {
    Id::unique("bicraft-formula-replace")
}

impl FormulaBar {
    /// The text differs from what the model has.
    pub fn dirty(&self) -> bool {
        self.target.as_ref().is_some_and(|t| t.new) || self.text != self.original
    }

    fn clear(&mut self) {
        let functions = self.functions.take();
        let asked = self.functions_asked;
        let expanded = self.expanded;
        let last_sel = self.last_sel.take();
        let doc = self.doc.take();
        let generation = self.generation.wrapping_add(1);
        *self = FormulaBar::default();
        self.functions = functions;
        self.functions_asked = asked;
        self.expanded = expanded;
        self.last_sel = last_sel;
        self.doc = doc;
        self.generation = generation;
    }

    fn set_target(&mut self, t: Option<Target>, focus: bool) {
        if t == self.target {
            self.focus |= focus;
            return;
        }
        self.clear();
        self.target = t;
        self.focus = focus;
    }

    /// The laid-out text's sections: (text, background, underline colour) (tests).
    #[cfg(test)]
    pub(crate) fn sections(&self) -> Vec<(String, Color32, Color32)> {
        let Some((_, job)) = &self.galley else {
            return Vec::new();
        };
        job.sections
            .iter()
            .filter_map(|s| {
                let text = job.text.get(s.byte_range.start.0..s.byte_range.end.0)?;
                Some((text.to_string(), s.format.background, s.format.underline.color))
            })
            .collect()
    }

    /// The text's expression part: the byte where it starts (after the first `=`).
    fn expr_at(&self) -> Option<usize> {
        dax_text::split_definition(&self.text).map(|x| x.1)
    }
}

/// The editor's id (tests, the app's Esc).
pub fn editor_id() -> Id {
    Id::unique("bicraft-formula-editor")
}

/// The editor (or its find row) has the keyboard: Esc and Enter are the bar's.
pub fn has_focus(ctx: &egui::Context) -> bool {
    ctx.memory(|m| m.has_focus(editor_id()) || m.has_focus(find_id()) || m.has_focus(replace_id()))
}

/// What a pane or the diagram picked.
#[derive(Debug, Clone, PartialEq)]
struct Pick {
    kind: Kind,
    table: Option<String>,
    name: String,
    focus: bool,
}

fn pick_id() -> Id {
    Id::unique("bicraft-formula-pick")
}

fn shared_id() -> Id {
    Id::unique("bicraft-formula-shared")
}

/// What the menus need to know: the snapshot and the chrome's language.
#[derive(Clone)]
struct Shared {
    snap: Arc<Value>,
    lang: Lang,
}

/// Puts a measure (`Kind::Measure`), a column or a table on the bar: the Data pane's click, a
/// well item, a double click in the model diagram, a menu's Edit. A column or a table that is
/// not calculated closes the bar (it has no formula). `focus` puts the cursor in the editor.
pub fn pick(ctx: &egui::Context, kind: Kind, table: Option<&str>, name: &str, focus: bool) {
    let p = Pick {
        kind,
        table: table.map(str::to_string),
        name: name.to_string(),
        focus,
    };
    ctx.data_mut(|d| d.insert_temp(pick_id(), p));
}

/// The right-click menu's "Edit measure" (or "Edit formula" of a calculated column or table):
/// shown only for an object with a formula; a click puts it on the bar, focused.
pub fn menu_entry(ui: &mut Ui, kind: Kind, table: &str, name: &str) {
    let Some(sh) = ui.ctx().data(|d| d.get_temp::<Shared>(shared_id())) else {
        return;
    };
    if resolve(&sh.snap, kind, Some(table), name).is_none() {
        return;
    }
    let l = sh.lang;
    let label = match kind {
        Kind::Measure => l.t("Edit measure", "Modifier la mesure"),
        Kind::Column | Kind::Table => l.t("Edit formula", "Modifier la formule"),
    };
    if ui.button(label).clicked() {
        pick(ui.ctx(), kind, Some(table), name, true);
        ui.close();
    }
}

fn arr(v: Option<&Value>) -> &[Value] {
    v.and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

fn text<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str)
}

/// The target `kind`/`table`/`name` names in the snapshot, if it has a formula.
fn resolve(snap: &Value, kind: Kind, table: Option<&str>, name: &str) -> Option<Target> {
    let tables = arr(snap.pointer("/model/tables"));
    let same = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
    match kind {
        Kind::Measure => tables.iter().find_map(|t| {
            let tn = text(t, "name")?;
            if table.is_some_and(|x| !same(x, tn)) {
                return None;
            }
            let m = arr(t.get("measures"))
                .iter()
                .find(|m| text(m, "name").is_some_and(|n| same(n, name)))?;
            Some(Target {
                kind,
                table: tn.to_string(),
                name: text(m, "name").unwrap_or(name).to_string(),
                new: false,
            })
        }),
        Kind::Column => {
            let t = tables
                .iter()
                .find(|t| text(t, "name").is_some_and(|n| table.is_some_and(|x| same(x, n))))?;
            let c = arr(t.get("columns"))
                .iter()
                .find(|c| text(c, "name").is_some_and(|n| same(n, name)))?;
            (c.get("calculated") == Some(&Value::Bool(true))).then(|| Target {
                kind,
                table: text(t, "name").unwrap_or_default().to_string(),
                name: text(c, "name").unwrap_or(name).to_string(),
                new: false,
            })
        }
        Kind::Table => {
            let t = tables
                .iter()
                .find(|t| text(t, "name").is_some_and(|n| same(n, name)))?;
            (t.get("calculated") == Some(&Value::Bool(true))).then(|| {
                let n = text(t, "name").unwrap_or(name).to_string();
                Target {
                    kind,
                    table: n.clone(),
                    name: n,
                    new: false,
                }
            })
        }
    }
}

/// The model's names for the suggestions.
pub(crate) fn names(snap: &Value) -> Names {
    let list = |t: &Value, k: &str| -> Vec<String> {
        arr(t.get(k))
            .iter()
            .filter_map(|x| text(x, "name").map(str::to_string))
            .collect()
    };
    Names {
        tables: arr(snap.pointer("/model/tables"))
            .iter()
            .filter_map(|t| {
                Some((
                    text(t, "name")?.to_string(),
                    list(t, "columns"),
                    list(t, "measures"),
                ))
            })
            .collect(),
    }
}

/// The command a commit of `t` runs (its state tells whether the bar is editable).
fn edit_command(t: &Target) -> &'static str {
    match (t.kind, t.new) {
        (Kind::Measure, true) => "measure.add",
        (Kind::Measure, false) => "measure.set",
        (Kind::Column, true) => "column.add",
        (Kind::Column, false) => "column.set",
        (Kind::Table, _) => "partition.set",
    }
}

/// A new measure on the bar, `Measure = ` focused, in `table` (else the selected or first
/// table): Home > New measure and the Data pane's New measure come here; ✓ runs `measure.add`.
pub fn new_measure(app: &mut App, table: Option<String>) {
    let Some(snap) = app.loaded.as_ref().map(|l| l.snap.clone()) else {
        return;
    };
    let tables = arr(snap.pointer("/model/tables"));
    let selected = match &app.model_view.sel {
        Some(Sel::Table(t)) | Some(Sel::Column { table: t, .. } | Sel::Measure { table: t, .. })
            if app.tab == View::Model =>
        {
            Some(t.clone())
        }
        _ => None,
    };
    let table = table
        .or(selected)
        .or_else(|| app.formula.target.as_ref().map(|t| t.table.clone()))
        .or_else(|| tables.iter().find_map(|t| text(t, "name").map(str::to_string)));
    let Some(table) = table else {
        app.status = app
            .lang
            .t(
                "The model has no table for a measure.",
                "Le modèle n'a pas de table pour une mesure.",
            )
            .to_string();
        return;
    };
    let taken: Vec<String> = tables
        .iter()
        .flat_map(|t| arr(t.get("measures")))
        .filter_map(|m| text(m, "name").map(str::to_lowercase))
        .collect();
    let base = app.lang.t("Measure", "Mesure");
    let mut name = base.to_string();
    let mut n = 1u32;
    while taken.contains(&name.to_lowercase()) {
        n = n.saturating_add(1);
        name = format!("{base} {n}");
    }
    let bar = &mut app.formula;
    bar.set_target(
        Some(Target {
            kind: Kind::Measure,
            table,
            name: name.clone(),
            new: true,
        }),
        true,
    );
    bar.text = format!("{name} = ");
    bar.original = String::new();
    bar.loaded_rev = Some(revision(&snap));
    bar.cursor_to = Some(bar.text.chars().count());
    bar.edits = bar.edits.wrapping_add(1);
}

/// A new calculated column on the bar, `Column = ` focused, in `table` (else the bar's table or
/// the first one): the Data view's New column comes here; ✓ runs `column.add`, whose data type
/// is the expression's.
pub fn new_column(app: &mut App, table: Option<String>) {
    let Some(snap) = app.loaded.as_ref().map(|l| l.snap.clone()) else {
        return;
    };
    let tables = arr(snap.pointer("/model/tables"));
    let table = table
        .or_else(|| app.formula.target.as_ref().map(|t| t.table.clone()))
        .or_else(|| tables.iter().find_map(|t| text(t, "name").map(str::to_string)));
    let Some(table) = table else {
        app.status = app
            .lang
            .t(
                "The model has no table for a column.",
                "Le modèle n'a pas de table pour une colonne.",
            )
            .to_string();
        return;
    };
    let taken: Vec<String> = tables
        .iter()
        .filter(|t| text(t, "name").is_some_and(|n| n.eq_ignore_ascii_case(&table)))
        .flat_map(|t| arr(t.get("columns")))
        .filter_map(|c| text(c, "name").map(str::to_lowercase))
        .collect();
    let base = app.lang.t("Column", "Colonne");
    let mut name = base.to_string();
    let mut n = 1u32;
    while taken.contains(&name.to_lowercase()) {
        n = n.saturating_add(1);
        name = format!("{base} {n}");
    }
    let bar = &mut app.formula;
    bar.set_target(
        Some(Target {
            kind: Kind::Column,
            table,
            name: name.clone(),
            new: true,
        }),
        true,
    );
    bar.text = format!("{name} = ");
    bar.original = String::new();
    bar.loaded_rev = Some(revision(&snap));
    bar.cursor_to = Some(bar.text.chars().count());
    bar.edits = bar.edits.wrapping_add(1);
}

fn revision(snap: &Value) -> Option<u64> {
    snap.get("revision").and_then(Value::as_u64)
}

/// Follows the selection: a new document resets the bar, a pick puts its object on it, the
/// model diagram's selection does too.
fn follow(app: &mut App, ctx: &egui::Context) {
    let doc = (
        app.active_doc(),
        app.origin.as_ref().map(|o| o.label()).unwrap_or_default(),
    );
    if app.formula.doc.as_ref() != Some(&doc) {
        app.formula.clear();
        app.formula.target = None;
        app.formula.doc = Some(doc);
        app.formula.last_sel = None;
    }
    let Some(snap) = app.loaded.as_ref().map(|l| l.snap.clone()) else {
        return;
    };
    ctx.data_mut(|d| {
        d.insert_temp(
            shared_id(),
            Shared {
                snap: snap.clone(),
                lang: app.lang,
            },
        )
    });
    let picked = ctx.data_mut(|d| {
        let p = d.get_temp::<Pick>(pick_id());
        d.remove::<Pick>(pick_id());
        p
    });
    if let Some(p) = picked {
        let t = resolve(&snap, p.kind, p.table.as_deref(), &p.name);
        // another object picked while the bar is edited: the edit is committed first, as a
        // click away from Desktop's formula bar commits it
        if t.is_some() && t != app.formula.target && app.formula.dirty() {
            commit(app);
        }
        if t.is_some() || !app.formula.dirty() {
            app.formula.set_target(t, p.focus);
        }
    }
    if app.tab == View::Model {
        let sel = app.model_view.sel.clone();
        if app.formula.last_sel.as_ref() != Some(&sel) {
            let first = app.formula.last_sel.is_none();
            app.formula.last_sel = Some(sel.clone());
            let t = match &sel {
                Some(Sel::Measure { table, name }) => resolve(&snap, Kind::Measure, Some(table), name),
                Some(Sel::Column { table, name }) => resolve(&snap, Kind::Column, Some(table), name),
                Some(Sel::Table(name)) => resolve(&snap, Kind::Table, None, name),
                _ => None,
            };
            // entering the view with nothing selected keeps what the panes put on the bar; an
            // edit is committed before another object takes the bar
            if t.is_some() && t != app.formula.target && app.formula.dirty() {
                // sent unless the focus loss already sent it
                commit(app);
                app.formula.set_target(t, false);
            } else if (t.is_some() || !first) && !app.formula.dirty() {
                app.formula.set_target(t, false);
            }
        }
    }
    // deleted or renamed elsewhere: the bar closes
    if let Some(t) = app.formula.target.clone()
        && !t.new
        && app.formula.committing == 0
        && !app.formula.dirty()
        && resolve(&snap, t.kind, Some(&t.table), &t.name).is_none()
    {
        app.formula.set_target(None, false);
    }
}

/// Loads the target's full expression when the model changed (and the bar is not being edited).
fn load_if_needed(app: &mut App) {
    let rev = app.loaded.as_ref().and_then(|l| revision(&l.snap));
    let bar = &app.formula;
    let Some(t) = bar.target.clone() else { return };
    if t.new || bar.dirty() || bar.committing > 0 || bar.loaded_rev == Some(rev) {
        return;
    }
    let (id, params): (&'static str, Value) = match t.kind {
        Kind::Measure => ("measure.get", json!({"name": t.name, "table": t.table})),
        Kind::Column => ("column.get", json!({"table": t.table, "name": t.name})),
        Kind::Table => ("table.get", json!({"name": t.name})),
    };
    let generation = app.formula.generation;
    app.formula.loaded_rev = Some(rev);
    if app.run(Call { id, params }, Then::Formula(Step::Load(generation))) {
        app.formula.loading = true;
    }
}

/// The answer of one of the bar's commands.
pub fn answer(app: &mut App, step: Step, r: Result<Value, String>) {
    let lang = app.lang;
    let bar = &mut app.formula;
    match step {
        Step::Functions => {
            let v = r.unwrap_or_default();
            let funcs = arr(v.get("functions"))
                .iter()
                .filter_map(|f| {
                    Some(Func {
                        name: text(f, "name")?.to_string(),
                        signature: text(f, "signature").unwrap_or_default().to_string(),
                        params: arr(f.get("params"))
                            .iter()
                            .map(|p| {
                                (
                                    text(p, "name").unwrap_or_default().to_string(),
                                    p.get("optional").and_then(Value::as_bool).unwrap_or(false),
                                    p.get("repeating").and_then(Value::as_bool).unwrap_or(false),
                                )
                            })
                            .collect(),
                        description: (
                            text(f, "description").unwrap_or_default().to_string(),
                            text(f, "description_fr").unwrap_or_default().to_string(),
                        ),
                    })
                })
                .take(5000)
                .collect();
            let keywords = arr(v.get("keywords"))
                .iter()
                .filter_map(|k| k.as_str().map(str::to_string))
                .take(100)
                .collect();
            bar.functions = Some(Arc::new((funcs, keywords)));
        }
        Step::Load(g) if g == bar.generation => {
            bar.loading = false;
            let Some(t) = bar.target.clone() else { return };
            match r {
                Ok(v) => {
                    let expression = match t.kind {
                        Kind::Measure | Kind::Column => {
                            text(&v, "expression").unwrap_or_default().to_string()
                        }
                        Kind::Table => arr(v.get("partitions"))
                            .iter()
                            .find(|p| text(p, "source_type") == Some("calculated"))
                            .and_then(|p| text(p, "expression"))
                            .unwrap_or_default()
                            .to_string(),
                    };
                    bar.format = text(&v, "format_string").map(str::to_string);
                    bar.text = dax_text::definition(&t.name, &expression);
                    bar.original = bar.text.clone();
                    bar.galley = None;
                    bar.diagnostics.clear();
                    bar.edits = bar.edits.wrapping_add(1);
                    // the loaded text is checked at once
                    bar.edited_at = f64::NEG_INFINITY;
                    bar.checked = None;
                }
                Err(e) => {
                    bar.error = Some(crate::app::clip(&e, 400));
                }
            }
        }
        Step::Validate(edit) => {
            let Some((at_edit, at)) = bar.checked else { return };
            if edit != bar.edits || at_edit != edit {
                return;
            }
            let Ok(v) = r else { return };
            bar.diagnostics = arr(v.get("diagnostics"))
                .iter()
                .take(50)
                .map(|d| {
                    let n = |p: &str| d.pointer(p).and_then(Value::as_u64).unwrap_or(0);
                    let start = usize::try_from(n("/span/start")).unwrap_or(0).saturating_add(at);
                    let end = usize::try_from(n("/span/end")).unwrap_or(0).saturating_add(at);
                    Diag {
                        message: text(d, "message").unwrap_or_default().to_string(),
                        span: start..end.max(start),
                        line: n("/line"),
                        column: n("/column"),
                        error: text(d, "severity") != Some("warning"),
                    }
                })
                .collect();
            bar.galley = None;
        }
        Step::Commit(g) if g == bar.generation => {
            bar.committing = bar.committing.saturating_sub(1);
            match r {
                Ok(v) => {
                    // invalid DAX is kept with its error, as Desktop keeps it: the error stays
                    // under the bar
                    if let Some(m) = saved_error(&v) {
                        bar.saved_error = Some(m);
                    }
                    if bar.committing > 0 {
                        return;
                    }
                    if let (Some(t), Some(name)) = (bar.target.as_mut(), bar.commit_name.take()) {
                        if t.kind == Kind::Table {
                            t.table = name.clone();
                        }
                        t.name = name;
                        t.new = false;
                    }
                    bar.original = bar.text.clone();
                    bar.loaded_rev = None;
                    bar.error = bar.saved_error.take().map(|m| {
                        format!(
                            "{} {m}",
                            lang.t("Saved with an error:", "Enregistré avec une erreur :")
                        )
                    });
                }
                Err(e) => {
                    bar.committing = 0;
                    bar.error = Some(crate::app::clip(&e, 600));
                    bar.focus = true;
                }
            }
        }
        Step::Format(edit) => {
            if edit != bar.edits {
                return;
            }
            match r {
                Ok(v) => {
                    let Some(formatted) = text(&v, "query") else {
                        return;
                    };
                    let Some((name, _)) = dax_text::split_definition(&bar.text) else {
                        return;
                    };
                    let next = dax_text::definition(&name, formatted);
                    if next != bar.text {
                        if next.contains('\n') {
                            bar.expanded = true;
                        }
                        bar.text = next;
                        bar.edits = bar.edits.wrapping_add(1);
                        bar.edited_at = f64::NEG_INFINITY;
                        bar.checked = None;
                        bar.galley = None;
                        bar.completion = None;
                    }
                    bar.error = None;
                }
                Err(e) => bar.error = Some(crate::app::clip(&e, 400)),
            }
        }
        _ => {}
    }
}

/// The message of the first error a commit kept (`force`: the DAX is saved with it).
fn saved_error(v: &Value) -> Option<String> {
    arr(v.get("diagnostics"))
        .iter()
        .filter(|o| o.get("valid").and_then(Value::as_bool) == Some(false))
        .flat_map(|o| arr(o.get("diagnostics")))
        .find(|d| text(d, "severity") == Some("error"))
        .and_then(|d| text(d, "message"))
        .map(|m| crate::app::clip(m, 300))
}

/// Format DAX: the expression laid out in BIcraft's formula style (`dax.format`, after the
/// public SQLBI conventions); the text changes on the bar, the commit stays with the user.
pub fn format_dax(app: &mut App) -> bool {
    let bar = &app.formula;
    if bar.target.is_none() || bar.loading {
        return false;
    }
    let Some((_, at)) = dax_text::split_definition(&bar.text) else {
        return false;
    };
    let expression = bar.text.get(at..).unwrap_or("").trim().to_string();
    if expression.is_empty() {
        return false;
    }
    let edit = bar.edits;
    app.run(
        Call {
            id: "dax.format",
            params: json!({"query": expression, "style": "formula"}),
        },
        Then::Formula(Step::Format(edit)),
    )
}

/// Commits the text: the engine commands of the target's kind. False when nothing was sent.
pub fn commit(app: &mut App) -> bool {
    let l = app.lang;
    let Some(t) = app.formula.target.clone() else {
        return false;
    };
    if ribbon::state(app, edit_command(&t)).is_err() || app.formula.committing > 0 {
        return false;
    }
    if !app.formula.dirty() {
        app.formula.completion = None;
        return false;
    }
    let text = app.formula.text.clone();
    let Some((name, at)) = dax_text::split_definition(&text) else {
        app.formula.error = Some(
            l.t(
                "Write the name, then = and the formula.",
                "Écrivez le nom, puis = et la formule.",
            )
            .to_string(),
        );
        return false;
    };
    let expression = text.get(at..).unwrap_or("").trim().to_string();
    if name.is_empty() || expression.is_empty() {
        app.formula.error = Some(
            l.t(
                "The name and the formula must not be empty.",
                "Le nom et la formule ne doivent pas être vides.",
            )
            .to_string(),
        );
        return false;
    }
    let old = dax_text::split_definition(&app.formula.original);
    let old_expression = old
        .as_ref()
        .and_then(|(_, a)| app.formula.original.get(*a..))
        .map(|e| e.trim().to_string());
    let renamed = name != t.name;
    let changed = old_expression.as_deref() != Some(expression.as_str());
    let mut calls: Vec<Call> = Vec::new();
    match (t.kind, t.new) {
        (Kind::Measure, true) => calls.push(Call {
            id: "measure.add",
            params: json!({"table": t.table, "name": name, "expression": expression, "force": true}),
        }),
        (Kind::Measure, false) => {
            let mut p = json!({"name": t.name, "table": t.table, "expression": expression, "force": true});
            if renamed {
                p["new_name"] = json!(name);
                p["update_report"] = json!(true);
            }
            calls.push(Call {
                id: "measure.set",
                params: p,
            });
        }
        // a new calculated column: its data type is the expression's
        (Kind::Column, true) => calls.push(Call {
            id: "column.add",
            params: json!({"table": t.table, "name": name, "expression": expression, "force": true}),
        }),
        (Kind::Column, _) => {
            let mut p = json!({"table": t.table, "name": t.name, "expression": expression, "force": true});
            if renamed {
                p["new_name"] = json!(name);
                p["update_report"] = json!(true);
            }
            calls.push(Call {
                id: "column.set",
                params: p,
            });
        }
        // the new expression re-derives the columns; with the rename, one command and one undo
        // step
        (Kind::Table, _) if changed || renamed => {
            let mut p =
                json!({"table": t.name, "expression": expression, "kind": "calculated", "force": true});
            if renamed {
                p["new_name"] = json!(name);
                p["update_report"] = json!(true);
            }
            calls.push(Call {
                id: "partition.set",
                params: p,
            });
        }
        (Kind::Table, _) => {}
    }
    if calls.is_empty() {
        app.formula.original = app.formula.text.clone();
        return false;
    }
    let g = app.formula.generation;
    app.formula.commit_name = Some(name);
    app.formula.error = None;
    app.formula.saved_error = None;
    app.formula.completion = None;
    let mut sent = 0;
    for c in calls {
        if app.run(c, Then::Formula(Step::Commit(g))) {
            sent += 1;
        }
    }
    app.formula.committing = sent;
    sent > 0
}

/// Cancels: an edited text goes back to the model's; an unedited one (or a new measure) closes
/// the bar.
pub fn cancel(app: &mut App) {
    let bar = &mut app.formula;
    bar.completion = None;
    if bar.target.as_ref().is_some_and(|t| t.new) || !bar.dirty() {
        bar.set_target(None, false);
        if app.tab == View::Model {
            // the diagram's selection stays; the bar follows it again on its next change
            bar.last_sel = Some(app.model_view.sel.clone());
        }
        return;
    }
    bar.text = bar.original.clone();
    bar.edits = bar.edits.wrapping_add(1);
    bar.edited_at = f64::NEG_INFINITY;
    bar.checked = None;
    bar.error = None;
    bar.galley = None;
}

/// Asks for the function list once (an older server without `dax.functions` just offers none).
pub(crate) fn ask_functions(app: &mut App) {
    if app.formula.functions_asked {
        return;
    }
    app.formula.functions_asked = true;
    app.run(
        Call {
            id: "dax.functions",
            params: json!({}),
        },
        Then::Formula(Step::Functions),
    );
}

/// The engine's DAX functions and keywords for other DAX editors (asked once, empty until the
/// answer).
pub(crate) fn functions(app: &mut App) -> Arc<(Vec<Func>, Vec<String>)> {
    ask_functions(app);
    app.formula
        .functions
        .clone()
        .unwrap_or_else(|| Arc::new((Vec::new(), Vec::new())))
}

/// How long the text stays unchanged before it is checked.
const CHECK_AFTER: f64 = 0.35;

/// Checks the text a moment after the last keystroke (`dax.validate`, through the source).
fn check_if_due(app: &mut App, now: f64, ctx: &egui::Context) {
    let bar = &app.formula;
    let Some(t) = bar.target.clone() else { return };
    if bar.loading || bar.checked.is_some_and(|(e, _)| e == bar.edits) {
        return;
    }
    let wait = CHECK_AFTER - (now - bar.edited_at);
    if wait > 0.0 {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait.min(1.0)));
        return;
    }
    let Some(at) = bar.expr_at() else {
        app.formula.checked = Some((bar.edits, 0));
        app.formula.diagnostics.clear();
        return;
    };
    let expression = bar.text.get(at..).unwrap_or("").to_string();
    let edit = bar.edits;
    let mut p = json!({"expression": expression});
    match t.kind {
        Kind::Measure => {
            p["table"] = json!(t.table);
            p["kind"] = json!("measure");
        }
        Kind::Column => {
            p["table"] = json!(t.table);
            p["kind"] = json!("calculated_column");
        }
        Kind::Table => p["kind"] = json!("calculated_table"),
    }
    app.formula.checked = Some((edit, at));
    app.run(
        Call {
            id: "dax.validate",
            params: p,
        },
        Then::Formula(Step::Validate(edit)),
    );
}

/// The colour of a class in theme `t`.
fn colour(t: &Tokens, c: Class) -> Color32 {
    match c {
        Class::Keyword => t.syn_keyword,
        Class::Function => t.syn_function,
        Class::String => t.syn_string,
        Class::Number => t.syn_number,
        Class::Table | Class::Column => t.syn_ref,
        Class::Comment => t.syn_comment,
        Class::Plain | Class::Operator => t.text,
    }
}

/// The editor's font.
pub fn mono() -> FontId {
    FontId::monospace(13.0)
}

/// The text laid out with the DAX colours and the first error underlined.
pub(crate) fn job(text: &str, t: &Tokens, underline: Option<Range<usize>>, wrap: f32) -> LayoutJob {
    job_marked(text, t, underline, wrap, &[])
}

/// A span of the text drawn on a background (a find match, the bracket pair at the cursor),
/// with a line under it so colour is not the only sign.
#[derive(Clone, Debug, Hash, PartialEq)]
pub(crate) struct Mark {
    pub range: Range<usize>,
    pub background: Color32,
    pub line: Color32,
}

/// [`job`] with `marks` (a later mark wins where two overlap).
pub(crate) fn job_marked(
    text: &str,
    t: &Tokens,
    underline: Option<Range<usize>>,
    wrap: f32,
    marks: &[Mark],
) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap;
    let font = mono();
    let fmt = |c: Class, under: bool, mark: Option<usize>| {
        let m = mark.and_then(|i| marks.get(i));
        TextFormat {
            font_id: font.clone(),
            color: colour(t, c),
            background: m.map_or(Color32::TRANSPARENT, |m| m.background),
            underline: if under {
                Stroke::new(1.5, t.err)
            } else if let Some(m) = m {
                Stroke::new(1.0, m.line)
            } else {
                Stroke::NONE
            },
            ..Default::default()
        }
    };
    type Run = (usize, usize, Class, bool, Option<usize>);
    // runs of one colour, underline and mark become one section (fewer sections lay out faster)
    let mut run: Option<Run> = None;
    let flush = |job: &mut LayoutJob, run: Option<Run>| {
        if let Some((a, b, c, under, mark)) = run
            && let Some(s) = text.get(a..b)
        {
            job.append(s, 0.0, fmt(c, under, mark));
        }
    };
    let bounds: Vec<usize> = underline
        .iter()
        .chain(marks.iter().map(|m| &m.range))
        .flat_map(|r| [r.start, r.end])
        .collect();
    for (r, c) in dax_text::pieces(text) {
        // operators, names and white space share the text colour
        let c = if c == Class::Operator { Class::Plain } else { c };
        let mut cuts = vec![r.start, r.end];
        for &b in &bounds {
            if b > r.start && b < r.end && text.is_char_boundary(b) {
                cuts.push(b);
            }
        }
        cuts.sort_unstable();
        for w in cuts.windows(2) {
            let (a, b) = (w[0], w[1]);
            if a >= b {
                continue;
            }
            let under = underline.as_ref().is_some_and(|u| a >= u.start && b <= u.end);
            let mark = marks.iter().rposition(|m| a >= m.range.start && b <= m.range.end);
            match &mut run {
                Some((_, end, rc, ru, rm)) if *end == a && *rc == c && *ru == under && *rm == mark => {
                    *end = b
                }
                _ => {
                    flush(&mut job, run.take());
                    run = Some((a, b, c, under, mark));
                }
            }
        }
    }
    flush(&mut job, run.take());
    if text.is_empty() {
        job.append("", 0.0, fmt(Class::Plain, false, None));
    }
    job
}

/// A wavy line from `x0` to `x1` around `y`: the error underline's points.
pub(crate) fn wave(x0: f32, x1: f32, y: f32) -> Vec<egui::Pos2> {
    const STEP: f32 = 2.0;
    const AMP: f32 = 1.0;
    let mut out = Vec::new();
    if !(x0.is_finite() && x1.is_finite()) || x1 <= x0 {
        return out;
    }
    let mut x = x0;
    let mut up = true;
    // a few thousand points at most: the bar is never wider than a screen
    while x < x1 && out.len() < 8192 {
        out.push(egui::pos2(x, if up { y - AMP } else { y + AMP }));
        x += STEP;
        up = !up;
    }
    out.push(egui::pos2(x1, if up { y - AMP } else { y + AMP }));
    out
}

/// Most characters of an error span drawn with the wavy line.
const MAX_WAVY_CHARS: usize = 4096;

/// Draws the wavy line under the characters `range` (bytes of `text`) of `galley`, laid out at
/// `pos`, one stretch per row.
fn paint_wave(
    painter: &egui::Painter,
    galley: &Galley,
    pos: egui::Pos2,
    text: &str,
    range: &Range<usize>,
    colour: Color32,
) {
    let a = dax_text::char_of_byte(text, range.start);
    let b = dax_text::char_of_byte(text, range.end).min(a.saturating_add(MAX_WAVY_CHARS));
    let mut stretch: Option<(f32, f32, f32)> = None;
    let draw = |s: Option<(f32, f32, f32)>| {
        if let Some((x0, x1, y)) = s {
            let pts = wave(pos.x + x0, pos.x + x1, pos.y + y - 1.5);
            if pts.len() > 1 {
                painter.add(egui::Shape::line(pts, Stroke::new(1.0, colour)));
            }
        }
    };
    for i in a..b {
        let p = galley.pos_from_cursor(CCursor::new(i));
        let q = galley.pos_from_cursor(CCursor::new(i + 1));
        // a line break: the next character starts another row
        if (p.min.y - q.min.y).abs() > 0.5 {
            continue;
        }
        match &mut stretch {
            Some((_, x1, y)) if (*y - p.max.y).abs() < 0.5 && (*x1 - p.min.x).abs() < 0.5 => *x1 = q.min.x,
            _ => {
                draw(stretch.take());
                stretch = Some((p.min.x, q.min.x, p.max.y));
            }
        }
    }
    draw(stretch.take());
}

/// The span to underline: the first error's, widened to one character when empty.
pub(crate) fn underline_of(text: &str, d: &[Diag]) -> Option<Range<usize>> {
    let e = d.iter().find(|d| d.error)?;
    let mut s = e.span.start.min(text.len());
    let mut end = e.span.end.min(text.len());
    while s > 0 && !text.is_char_boundary(s) {
        s -= 1;
    }
    while end < text.len() && !text.is_char_boundary(end) {
        end += 1;
    }
    if end <= s {
        // an empty span: the character there, else the one before
        match text.get(s..).and_then(|r| r.chars().next()) {
            Some(c) => end = s + c.len_utf8(),
            None => {
                let c = text.get(..s)?.chars().next_back()?;
                s -= c.len_utf8();
                end = s + c.len_utf8();
            }
        }
    }
    Some(s..end)
}

/// Takes the pressed `key` events whose modifiers `ok` accepts out of this frame's input.
pub(crate) fn take_key(ui: &Ui, key: Key, ok: impl Fn(Modifiers) -> bool) -> bool {
    ui.input_mut(|i| {
        let before = i.events.len();
        i.events.retain(|e| {
            !matches!(e, Event::Key { key: k, pressed: true, modifiers, .. } if *k == key && ok(*modifiers))
        });
        i.events.len() != before
    })
}

/// The bracket pair at the editor's cursor (none while text is selected or the editor is not
/// focused), worked out once per edit and cursor move, and only when a bracket is next to the
/// cursor (a keystroke elsewhere lexes nothing more).
fn bracket_pair(ctx: &egui::Context, bar: &mut FormulaBar, focused: bool) -> Option<Pair> {
    if !focused {
        return None;
    }
    let range = egui::text_edit::TextEditState::load(ctx, editor_id())?
        .cursor
        .char_range()?;
    if range.primary.index != range.secondary.index {
        return None;
    }
    let cb = dax_text::byte_of_char(&bar.text, range.primary.index.0);
    let key = (bar.edits, cb);
    if let Some((k, p)) = &bar.brackets
        && *k == key
    {
        return p.clone();
    }
    let bytes = bar.text.as_bytes();
    let near = |i: Option<usize>| {
        i.and_then(|i| bytes.get(i))
            .is_some_and(|c| b"(){}[]".contains(c))
    };
    let p = if near(Some(cb)) || near(cb.checked_sub(1)) {
        dax_text::matching_bracket(&bar.text, cb)
    } else {
        None
    };
    bar.brackets = Some((key, p.clone()));
    p
}

/// Opens the find row with the selected text (one line of it) to find, its field focused.
pub fn open_find(ctx: &egui::Context, bar: &mut FormulaBar) {
    let selected = egui::text_edit::TextEditState::load(ctx, editor_id())
        .and_then(|s| s.cursor.char_range())
        .and_then(|r| {
            let (a, b) = (r.primary.index.0, r.secondary.index.0);
            let (a, b) = (a.min(b), a.max(b));
            let (a, b) = (
                dax_text::byte_of_char(&bar.text, a),
                dax_text::byte_of_char(&bar.text, b),
            );
            bar.text.get(a..b).map(str::to_string)
        })
        .filter(|s| !s.is_empty() && !s.contains('\n'));
    let f = bar.find.get_or_insert_with(Find::default);
    if let Some(s) = selected {
        f.needle = s;
        f.current = 0;
    }
    f.focus = true;
}

/// Moves to the next (or the previous) match: the editor's cursor goes after it.
fn find_step(bar: &mut FormulaBar, forward: bool) {
    let Some(f) = bar.find.as_mut() else {
        return;
    };
    let n = f.matches(&bar.text).len();
    if n == 0 {
        return;
    }
    f.current = if forward {
        (f.current + 1) % n
    } else {
        (f.current + n - 1) % n
    };
    if let Some(r) = f.matches(&bar.text).get(f.current) {
        bar.cursor_to = Some(dax_text::char_of_byte(&bar.text, r.end));
    }
}

/// Replaces the current match (`all`: every match), as typing would: the bar is dirty, the
/// commit stays the user's.
fn find_replace(app: &mut App, ctx: &egui::Context, all: bool) {
    let bar = &mut app.formula;
    let Some(f) = bar.find.as_mut() else {
        return;
    };
    let found = f.matches(&bar.text);
    let picked: Vec<Range<usize>> = if all {
        found
    } else {
        found.get(f.current).cloned().into_iter().collect()
    };
    if picked.is_empty() {
        return;
    }
    // from the end, so the earlier ranges stay where they are
    for r in picked.iter().rev() {
        if r.end <= bar.text.len() && bar.text.is_char_boundary(r.start) && bar.text.is_char_boundary(r.end) {
            bar.text.replace_range(r.clone(), &f.replacement);
        }
    }
    let n = f.matches(&bar.text).len();
    f.current = if n == 0 { 0 } else { f.current.min(n - 1) };
    bar.edits = bar.edits.wrapping_add(1);
    bar.edited_at = ctx.input(|i| i.time);
    bar.error = None;
    let l = app.lang;
    app.status = if all {
        format!("{} {}", picked.len(), l.t("replaced", "remplacé(s)"))
    } else {
        l.t("Replaced", "Remplacé").to_string()
    };
}

/// A focused find-row field keeps Esc (egui would drop the focus first), so Esc closes the row
/// and gives the keyboard back to the editor.
fn keep_escape(ctx: &egui::Context, r: &egui::Response) {
    if r.has_focus() {
        ctx.memory_mut(|m| {
            m.set_focus_lock_filter(
                r.id,
                egui::EventFilter {
                    escape: true,
                    ..Default::default()
                },
            )
        });
    }
}

/// The find and replace row: the find and replace fields, the match count, previous and next,
/// Replace, Replace all, Match case, Close. Enter finds the next match, Shift+Enter the
/// previous, Esc closes the row and gives the keyboard back to the editor.
fn find_ui(app: &mut App, ui: &mut Ui, read_only: bool, t: &Tokens) {
    let l = app.lang;
    let ctx = ui.ctx().clone();
    let in_find = ctx.memory(|m| m.has_focus(find_id()) || m.has_focus(replace_id()));
    let mut close = false;
    let (mut next, mut prev, mut one, mut all) = (false, false, false, false);
    if in_find {
        if take_key(ui, Key::Escape, |_| true) {
            close = true;
        }
        if take_key(ui, Key::Enter, |m| m.shift) {
            prev = true;
        }
        if take_key(ui, Key::Enter, |m| m.is_none()) {
            next = true;
        }
    }
    let text = app.formula.text.clone();
    let Some(f) = app.formula.find.as_mut() else {
        return;
    };
    let n = f.matches(&text).len();
    if f.current >= n {
        f.current = 0;
    }
    ui.horizontal(|ui| {
        ui.add_space(56.0);
        let label = ui.label(RichText::new(l.t("Find", "Rechercher")).size(12.0).color(t.text));
        let r = ui.add(
            egui::TextEdit::singleline(&mut f.needle)
                .id(find_id())
                .desired_width(180.0)
                .font(mono()),
        );
        let r = r.labelled_by(label.id);
        keep_escape(&ctx, &r);
        if f.focus {
            r.request_focus();
            f.focus = false;
        }
        if r.changed() {
            f.current = 0;
        }
        let count = if f.needle.is_empty() {
            String::new()
        } else if n == 0 {
            l.t("No match", "Aucune correspondance").to_string()
        } else {
            format!("{} {} {n}", f.current + 1, l.t("of", "sur"))
        };
        ui.label(RichText::new(count).size(12.0).color(t.text_dim));
        prev |= icon_button(
            ui,
            Icon::ChevronUp,
            l.t(
                "Previous match (Shift+Enter)",
                "Correspondance précédente (Maj+Entrée)",
            ),
            n > 0,
        )
        .clicked();
        next |= icon_button(
            ui,
            Icon::ChevronDown,
            l.t("Next match (Enter)", "Correspondance suivante (Entrée)"),
            n > 0,
        )
        .clicked();
        ui.checkbox(&mut f.case, l.t("Match case", "Respecter la casse"));
        ui.add_space(8.0);
        let label = ui.label(
            RichText::new(l.t("Replace with", "Remplacer par"))
                .size(12.0)
                .color(t.text),
        );
        let r = ui
            .add_enabled(
                !read_only,
                egui::TextEdit::singleline(&mut f.replacement)
                    .id(replace_id())
                    .desired_width(160.0)
                    .font(mono()),
            )
            .labelled_by(label.id);
        keep_escape(&ctx, &r);
        one = ui
            .add_enabled(
                !read_only && n > 0,
                egui::Button::new(l.t("Replace", "Remplacer")),
            )
            .clicked();
        all = ui
            .add_enabled(
                !read_only && n > 0,
                egui::Button::new(l.t("Replace all", "Tout remplacer")),
            )
            .clicked();
        close |= icon_button(
            ui,
            Icon::Close,
            l.t("Close find (Esc)", "Fermer la recherche (Échap)"),
            true,
        )
        .clicked();
    });
    if prev || next {
        find_step(&mut app.formula, next && !prev);
    }
    if (one || all) && !read_only {
        find_replace(app, &ctx, all);
    }
    if close {
        app.formula.find = None;
        app.formula.focus = !read_only;
    }
}

/// The bar, under the ribbon (nothing when no object with a formula is selected).
pub fn show(app: &mut App, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    follow(app, &ctx);
    if app.formula.target.is_none() {
        return;
    }
    ask_functions(app);
    load_if_needed(app);
    let now = ctx.input(|i| i.time);
    check_if_due(app, now, &ctx);
    let t = Tokens::of(app.prefs.theme);
    let lines = app.formula.text.lines().count().max(1);
    let row = ctx.fonts_mut(|f| f.row_height(&mono()));
    let shown = if app.formula.expanded {
        lines.clamp(3, 14)
    } else {
        1
    };
    let note = !app.formula.diagnostics.is_empty() || app.formula.error.is_some();
    let editor_h = row * shown as f32 + 8.0;
    let find = if app.formula.find.is_some() { 26.0 } else { 0.0 };
    let h = editor_h + 10.0 + if note { 20.0 } else { 0.0 } + find;
    let r = egui::Panel::top("bicraft-formula-bar")
        .exact_size(h)
        .frame(
            egui::Frame::NONE
                .fill(t.chrome)
                .inner_margin(egui::Margin::symmetric(8, 5)),
        )
        .show(ui, |ui| bar_ui(app, ui, editor_h, &t));
    crate::keyboard::section(app, ui, crate::keyboard::Section::Formula, r.response.rect);
}

fn bar_ui(app: &mut App, ui: &mut Ui, editor_h: f32, t: &Tokens) {
    let l = app.lang;
    let Some(target) = app.formula.target.clone() else {
        return;
    };
    let editable = ribbon::state(app, edit_command(&target));
    let read_only = editable.is_err() || app.formula.loading;
    let dirty = app.formula.dirty();
    let mut act_commit = false;
    let mut act_cancel = false;
    let mut open_props = false;
    let mut act_format = false;
    let mut act_find = false;
    // the bar's own area: a click there (its buttons) does not commit on focus loss
    let bar_rect = ui.max_rect().expand(6.0);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let cancel_tip = l.t("Cancel (Esc)", "Annuler (Échap)");
        if icon_button(ui, Icon::Close, cancel_tip, true).clicked() {
            act_cancel = true;
        }
        let commit_tip = l.t("Commit (Enter)", "Valider (Entrée)");
        if icon_button(ui, Icon::Check, commit_tip, !read_only && dirty).clicked() {
            act_commit = true;
        }
        // the right side: home table and format, the properties, the chevron
        let side = 256.0;
        let w = (ui.available_width() - side).max(120.0);
        let (c, p) = editor(app, ui, w, editor_h, read_only, t, bar_rect);
        act_commit |= c;
        act_cancel |= p;
        ui.add_space(4.0);
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                icons::paint(
                    ui.painter(),
                    Rect::from_min_size(ui.cursor().min + vec2(0.0, 3.0), vec2(14.0, 14.0)),
                    Icon::Table,
                    t.text_dim,
                    false,
                );
                ui.add_space(16.0);
                let what = match target.kind {
                    Kind::Measure => app.formula.format.as_deref().map_or_else(
                        || l.t("No format", "Sans format").to_string(),
                        |f| format!("{} {}", l.t("Format", "Format"), crate::app::clip(f, 24)),
                    ),
                    Kind::Column => l.t("Calculated column", "Colonne calculée").to_string(),
                    Kind::Table => l.t("Calculated table", "Table calculée").to_string(),
                };
                let home = format!("{} · {what}", crate::app::clip(&target.table, 28));
                ui.label(RichText::new(home).size(12.0).color(t.text_dim))
                    .on_hover_text(l.t(
                        "Home table and format (the Measure tab changes them)",
                        "Table d'accueil et format (l'onglet Mesure les modifie)",
                    ));
                if icon_button(
                    ui,
                    Icon::Format,
                    l.t("Format DAX", "Mettre en forme le DAX"),
                    !read_only,
                )
                .clicked()
                {
                    act_format = true;
                }
                if icon_button(
                    ui,
                    Icon::Search,
                    l.t("Find and replace (Ctrl+H)", "Rechercher et remplacer (Ctrl+H)"),
                    true,
                )
                .clicked()
                {
                    act_find = true;
                }
                if target.kind == Kind::Measure
                    && !target.new
                    && icon_button(
                        ui,
                        Icon::Settings,
                        l.t("Measure properties", "Propriétés de la mesure"),
                        editable.is_ok(),
                    )
                    .clicked()
                {
                    open_props = true;
                }
                let (icon, tip) = if app.formula.expanded {
                    (
                        Icon::ChevronUp,
                        l.t("Collapse the formula bar", "Réduire la barre de formule"),
                    )
                } else {
                    (
                        Icon::ChevronDown,
                        l.t("Expand the formula bar", "Développer la barre de formule"),
                    )
                };
                if icon_button(ui, icon, tip, true).clicked() {
                    app.formula.expanded = !app.formula.expanded;
                }
            });
            if let Err(why) = &editable {
                ui.label(
                    RichText::new(format!("{} {why}", l.t("Read-only:", "Lecture seule :")))
                        .size(11.0)
                        .color(t.text_dim),
                );
            }
        });
    });
    if app.formula.find.is_some() {
        find_ui(app, ui, read_only, t);
    }
    // the message under the bar: the commit's refusal, else the first diagnostic
    let msg = app.formula.error.clone().or_else(|| {
        let d = app
            .formula
            .diagnostics
            .iter()
            .find(|d| d.error)
            .or(app.formula.diagnostics.first())?;
        Some(format!(
            "{} ({} {}, {} {})",
            d.message,
            l.t("line", "ligne"),
            d.line,
            l.t("column", "colonne"),
            d.column
        ))
    });
    if let Some(m) = msg {
        ui.horizontal(|ui| {
            ui.add_space(56.0);
            let r = ui.cursor().min;
            icons::paint(
                ui.painter(),
                Rect::from_min_size(r + vec2(0.0, 2.0), vec2(14.0, 14.0)),
                Icon::Warning,
                t.err_text,
                false,
            );
            ui.add_space(18.0);
            ui.label(
                RichText::new(crate::app::clip(&m, 300))
                    .size(12.0)
                    .color(t.err_text),
            );
        });
    }
    if act_cancel {
        cancel(app);
        ui.ctx().memory_mut(|m| m.surrender_focus(editor_id()));
    } else if act_commit && (commit(app) || app.formula.error.is_none()) {
        ui.ctx().memory_mut(|m| m.surrender_focus(editor_id()));
    }
    if act_format {
        format_dax(app);
    }
    if act_find {
        if app.formula.find.is_some() {
            app.formula.find = None;
        } else {
            open_find(ui.ctx(), &mut app.formula);
        }
    }
    if open_props {
        crate::form::open(
            app,
            "measure.set",
            json!({"name": target.name, "table": target.table}),
        );
    }
}

fn icon_button(ui: &mut Ui, icon: Icon, tip: &str, enabled: bool) -> egui::Response {
    crate::widgets::icon_toggle(ui, icon, tip, enabled, false)
}

/// The editor: keys first (the list, Enter, Esc), then the text, then the list and the
/// signature. Returns (commit, cancel).
fn editor(
    app: &mut App,
    ui: &mut Ui,
    w: f32,
    h: f32,
    read_only: bool,
    t: &Tokens,
    bar_rect: Rect,
) -> (bool, bool) {
    let id = editor_id();
    let ctx = ui.ctx().clone();
    let l = app.lang;
    let focused = ctx.memory(|m| m.has_focus(id));
    let mut commit = false;
    let mut cancel = false;
    let mut accept: Option<usize> = None;
    let mut explicit = false;
    if focused {
        // Alt+Enter adds a line as Shift+Enter does
        ui.input_mut(|i| {
            for e in &mut i.events {
                if let Event::Key {
                    key: Key::Enter,
                    modifiers,
                    ..
                } = e
                    && modifiers.alt
                    && !modifiers.command
                {
                    modifiers.alt = false;
                    modifiers.shift = true;
                }
            }
        });
        let plain = |m: Modifiers| m.is_none();
        if let Some(c) = app.formula.completion.as_mut() {
            let n = c.items.len().max(1);
            if take_key(ui, Key::ArrowDown, plain) {
                c.selected = (c.selected + 1) % n;
            }
            if take_key(ui, Key::ArrowUp, plain) {
                c.selected = (c.selected + n - 1) % n;
            }
            if take_key(ui, Key::Enter, plain) || take_key(ui, Key::Tab, plain) {
                accept = Some(c.selected);
            }
            if take_key(ui, Key::Escape, |_| true) {
                app.formula.completion = None;
            }
        } else {
            if take_key(ui, Key::Enter, plain) {
                commit = true;
            }
            if take_key(ui, Key::Escape, |_| true) {
                cancel = true;
            }
        }
        if take_key(ui, Key::Space, |m| m.command && !m.shift && !m.alt) {
            explicit = true;
        }
        if take_key(ui, Key::H, |m| m.command && !m.shift && !m.alt) {
            open_find(&ctx, &mut app.formula);
        }
    }
    let theme_key = t.dark;
    let underline = underline_of(&app.formula.text, &app.formula.diagnostics);
    let bar = &mut app.formula;
    let pair = bracket_pair(&ctx, bar, focused);
    let find = bar.find.as_ref().filter(|f| !f.needle.is_empty()).cloned();
    let marks_of = |s: &str, t: &Tokens| -> Vec<Mark> {
        let mut marks = Vec::new();
        if let Some(f) = &find {
            for (i, r) in f.matches(s).into_iter().enumerate() {
                let current = i == f.current;
                marks.push(Mark {
                    range: r,
                    background: if current { t.text_selection } else { t.accent_soft },
                    line: if current { t.text } else { t.text_dim },
                });
            }
        }
        if let Some((a, b)) = &pair {
            for r in [a, b] {
                marks.push(Mark {
                    range: r.clone(),
                    background: t.selected,
                    line: t.text,
                });
            }
        }
        marks
    };
    // a click elsewhere takes the focus away in its own frame: focus on the next one
    let clicking = ctx.input(|i| i.pointer.any_pressed() || i.pointer.any_released());
    if bar.focus && !read_only {
        if clicking {
            ctx.request_repaint();
        } else {
            ctx.memory_mut(|m| m.request_focus(id));
            bar.focus = false;
        }
    }
    let cache = &mut bar.galley;
    let layout_ms = &mut bar.layout_ms;
    let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| -> Arc<Galley> {
        let s = buf.as_str();
        let mut hs = std::collections::hash_map::DefaultHasher::new();
        s.hash(&mut hs);
        wrap.to_bits().hash(&mut hs);
        theme_key.hash(&mut hs);
        find.hash(&mut hs);
        pair.hash(&mut hs);
        let key = hs.finish();
        // the colours are worked out once per change; the job goes to egui every frame, which
        // keeps its layout of each line cached (an edit lays out the lines it changed only)
        let t0 = web_time_now();
        if cache.as_ref().is_none_or(|(k, _)| *k != key) {
            let t = Tokens::get(ui.ctx());
            // the error's wavy line is painted over the text ([`paint_wave`])
            *cache = Some((key, job_marked(s, &t, None, wrap, &marks_of(s, &t))));
        }
        let laid = cache.as_ref().map(|(_, j)| j.clone()).unwrap_or_default();
        let g = ui.fonts_mut(|f| f.layout_job(laid));
        *layout_ms = (web_time_now() - t0) * 1000.0;
        g
    };
    let out = egui::Frame::NONE
        .fill(t.field_bg)
        .stroke(Stroke::new(1.0, if focused { t.ring } else { t.border }))
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::symmetric(4, 2))
        .show(ui, |ui| {
            ui.set_width(w);
            egui::ScrollArea::vertical()
                .id_salt("bicraft-formula-scroll")
                .max_height(h - 4.0)
                .min_scrolled_height(h - 4.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    let hint = l.t("Formula", "Formule");
                    let out = if read_only {
                        let mut ro = bar.text.as_str();
                        text_edit(&mut ro, &mut layouter, id, hint).show(ui)
                    } else {
                        text_edit(&mut bar.text, &mut layouter, id, hint).show(ui)
                    };
                    if let Some(u) = &underline {
                        paint_wave(ui.painter(), &out.galley, out.galley_pos, &bar.text, u, t.err);
                    }
                    out
                })
                .inner
        })
        .inner;
    // Esc, Tab and the arrows stay with the editor (the bar handles Esc itself)
    if out.response.response.has_focus() {
        ctx.memory_mut(|m| {
            m.set_focus_lock_filter(
                id,
                egui::EventFilter {
                    tab: true,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                },
            )
        });
    }
    let changed = out.response.response.changed();
    // a click elsewhere commits the edit, as Desktop does; not a click on the bar itself (its
    // buttons act on their own) nor on the autocomplete list
    if out.response.response.lost_focus() && !read_only && bar.dirty() {
        let at = ctx.input(|i| i.pointer.interact_pos());
        let on_list = ctx
            .memory(|m| m.area_rect(Id::unique("bicraft-formula-list")))
            .zip(at)
            .is_some_and(|(r, p)| bar.completion.is_some() && r.contains(p));
        let on_bar = at.is_some_and(|p| bar_rect.contains(p));
        // the keyboard went to the find row (Ctrl+H): still editing
        let to_find = bar.find.is_some()
            && ctx
                .memory(|m| m.focused())
                .is_some_and(|f| f == find_id() || f == replace_id());
        if !on_list && !on_bar && !to_find {
            commit = true;
        }
    }
    let cursor_char = out.cursor_range.map(|r| r.primary.index.0);
    // the cursor a suggestion or a new measure asked for
    if let Some(c) = bar.cursor_to.take() {
        let mut st = out.state.clone();
        st.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(c))));
        st.store(&ctx, id);
    }
    if changed {
        bar.edits = bar.edits.wrapping_add(1);
        bar.edited_at = ctx.input(|i| i.time);
        bar.error = None;
    }
    let cursor_byte = cursor_char.map(|c| dax_text::byte_of_char(&bar.text, c));
    // the name part (before the first =) gets no suggestions
    let in_name = |cb: usize| bar.expr_at().is_none_or(|at| cb < at);
    if (changed || explicit)
        && let Some(cb) = cursor_byte
    {
        let cx = if in_name(cb) {
            None
        } else {
            dax_text::context(&bar.text, cb, explicit)
        };
        bar.completion = cx.and_then(|cx| {
            let snap = app.loaded.as_ref().map(|x| x.snap.clone())?;
            let target = bar.target.as_ref()?;
            let empty = Arc::new((Vec::new(), Vec::new()));
            let f = bar.functions.clone().unwrap_or(empty);
            let items = dax_text::suggestions(
                &cx.want,
                &names(&snap),
                &f.0,
                &f.1,
                Some(&target.table),
                target.kind == Kind::Column,
            );
            // a word typed in full that is the only suggestion needs no list
            let only_itself = matches!(&cx.want, Want::Word(w) if items.len() == 1
                && items.first().is_some_and(|s| s.label.eq_ignore_ascii_case(w)));
            (!items.is_empty() && !only_itself).then_some(Completion {
                items,
                selected: 0,
                replace: cx.replace,
            })
        });
    } else if !focused && !out.response.response.has_focus() {
        bar.completion = None;
    }
    // the list under the cursor
    let anchor = cursor_char.map(|c| {
        out.galley_pos
            + out
                .galley
                .pos_from_cursor(CCursor::new(c))
                .left_bottom()
                .to_vec2()
    });
    if let (Some(c), Some(pos)) = (bar.completion.clone(), anchor) {
        let known = bar.functions.clone();
        let funcs = known.as_ref().map_or(&[][..], |f| f.0.as_slice());
        if let Some(i) = list_ui(&ctx, Id::unique("bicraft-formula-list"), &c, pos, t, l, funcs) {
            accept = Some(i);
        }
    } else if let (Some(pos), Some(cb), true) = (anchor, cursor_byte, out.response.response.has_focus()) {
        signature_ui(&ctx, bar, cb, pos, t);
    }
    if let Some(i) = accept
        && let Some(c) = bar.completion.take()
        && let Some(s) = c.items.get(i)
        && c.replace.end <= bar.text.len()
        && bar.text.is_char_boundary(c.replace.start)
        && bar.text.is_char_boundary(c.replace.end)
    {
        bar.text.replace_range(c.replace.clone(), &s.insert);
        let at = dax_text::char_of_byte(&bar.text, c.replace.start) + s.insert.chars().count();
        bar.cursor_to = Some(at);
        bar.edits = bar.edits.wrapping_add(1);
        bar.edited_at = ctx.input(|i| i.time);
        bar.focus = true;
    }
    (commit, cancel)
}

/// Seconds, for the layout timing (no clock in tests of wasm builds: zero).
fn web_time_now() -> f64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::sync::OnceLock;
        static START: OnceLock<std::time::Instant> = OnceLock::new();
        START.get_or_init(std::time::Instant::now).elapsed().as_secs_f64()
    }
    #[cfg(target_arch = "wasm32")]
    {
        0.0
    }
}

/// The autocomplete list at `pos`; the suggestion clicked.
pub(crate) fn list_ui(
    ctx: &egui::Context,
    id: Id,
    c: &Completion,
    pos: egui::Pos2,
    t: &Tokens,
    l: Lang,
    funcs: &[Func],
) -> Option<usize> {
    let detail = c
        .items
        .get(c.selected)
        .filter(|s| s.kind == dax_text::Kind::Function)
        .and_then(|s| funcs.iter().find(|f| f.name.eq_ignore_ascii_case(&s.label)))
        .map(|f| function_detail(f, l));
    let rows: Vec<(&str, &str)> = c
        .items
        .iter()
        .map(|s| {
            let kind = match s.kind {
                dax_text::Kind::Function => l.t("function", "fonction"),
                dax_text::Kind::Keyword => l.t("keyword", "mot clé"),
                dax_text::Kind::Table => l.t("table", "table"),
                dax_text::Kind::Column => l.t("column", "colonne"),
                dax_text::Kind::Measure => l.t("measure", "mesure"),
            };
            (s.label.as_str(), kind)
        })
        .collect();
    rows_ui(ctx, id, &rows, c.selected, pos, t, detail.as_deref())
}

/// A function's signature and BIcraft's description of it in `l` (the description alone when
/// the signature is unknown).
pub(crate) fn function_detail(f: &Func, l: Lang) -> String {
    let d = match l {
        Lang::En => &f.description.0,
        Lang::Fr if f.description.1.is_empty() => &f.description.0,
        Lang::Fr => &f.description.1,
    };
    match (f.signature.is_empty(), d.is_empty()) {
        (true, _) => d.clone(),
        (false, true) => f.signature.clone(),
        (false, false) => format!("{}\n{d}", f.signature),
    }
}

/// An autocomplete list of (label, kind) rows at `pos`, row `selected` highlighted; the row
/// clicked.
pub(crate) fn rows_ui(
    ctx: &egui::Context,
    id: Id,
    rows: &[(&str, &str)],
    selected: usize,
    pos: egui::Pos2,
    t: &Tokens,
    detail: Option<&str>,
) -> Option<usize> {
    let mut picked = None;
    egui::Area::new(id)
        .order(egui::Order::Foreground)
        .fixed_pos(pos + vec2(0.0, 2.0))
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(260.0);
                egui::ScrollArea::vertical()
                    .id_salt(id.with("scroll"))
                    .max_height(220.0)
                    .show(ui, |ui| {
                        for (i, (label, kind)) in rows.iter().enumerate() {
                            let r = ui.horizontal(|ui| {
                                let r = ui.selectable_label(
                                    i == selected,
                                    RichText::new(crate::app::clip(label, 80)).font(mono()),
                                );
                                ui.label(RichText::new(*kind).size(11.0).color(t.text_dim));
                                r
                            });
                            if i == selected {
                                r.inner.scroll_to_me(None);
                            }
                            if r.inner.clicked() {
                                picked = Some(i);
                            }
                        }
                    });
                // the selected function's signature and description
                if let Some(d) = detail.filter(|d| !d.is_empty()) {
                    ui.separator();
                    ui.set_max_width(360.0);
                    let r = ui.add(
                        egui::Label::new(RichText::new(crate::app::clip(d, 400)).size(11.5).color(t.text))
                            .wrap(),
                    );
                    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, d));
                }
            });
        });
    picked
}

/// The signature of the call at the cursor, the argument being typed underlined.
fn signature_ui(ctx: &egui::Context, bar: &FormulaBar, cursor: usize, pos: egui::Pos2, t: &Tokens) {
    let Some((name, arg)) = dax_text::call_at(&bar.text, cursor) else {
        return;
    };
    let Some(funcs) = bar.functions.as_ref() else {
        return;
    };
    let Some(f) = funcs.0.iter().find(|f| f.name == name) else {
        return;
    };
    let active = dax_text::param_index(f, arg);
    let mut job = LayoutJob::default();
    let font = mono();
    let plain = TextFormat {
        font_id: font.clone(),
        color: t.text,
        ..Default::default()
    };
    let on = TextFormat {
        font_id: font,
        color: t.contextual,
        underline: Stroke::new(1.0, t.contextual),
        ..Default::default()
    };
    job.append(&f.name, 0.0, plain.clone());
    job.append("(", 0.0, plain.clone());
    for (i, (p, optional, repeating)) in f.params.iter().enumerate() {
        let sep = if i == 0 { "" } else { ", " };
        let fmt = if Some(i) == active {
            on.clone()
        } else {
            plain.clone()
        };
        if *optional {
            job.append("[", 0.0, plain.clone());
            job.append(sep, 0.0, plain.clone());
            job.append(p, 0.0, fmt);
            job.append("]", 0.0, plain.clone());
        } else {
            job.append(sep, 0.0, plain.clone());
            job.append(p, 0.0, fmt);
        }
        if *repeating && !f.params.get(i + 1).is_some_and(|q| q.2) {
            job.append(", …", 0.0, plain.clone());
        }
    }
    job.append(")", 0.0, plain);
    egui::Area::new(Id::unique("bicraft-formula-signature"))
        .order(egui::Order::Tooltip)
        .fixed_pos(pos + vec2(0.0, 4.0))
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                let r = ui.label(job);
                r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &f.signature));
            });
        });
}

/// The bar's text editor: monospace, Shift+Enter for a new line (Enter commits), Tab typed.
fn text_edit<'t>(
    text: &'t mut dyn egui::TextBuffer,
    layouter: &'t mut dyn FnMut(&Ui, &dyn egui::TextBuffer, f32) -> Arc<Galley>,
    id: Id,
    hint: &str,
) -> egui::TextEdit<'t> {
    egui::TextEdit::multiline(text)
        .id(id)
        .font(mono())
        .code_editor()
        .frame(egui::Frame::NONE)
        .desired_width(f32::INFINITY)
        .desired_rows(1)
        .return_key(Some(KeyboardShortcut::new(Modifiers::SHIFT, Key::Enter)))
        .hint_text(hint.to_string())
        .layouter(layouter)
}
