//! The DAX query view's editing tools (docs/gaps.md G81), pure view state like typing: they
//! change the tab's text, and a quick query runs through `dax.evaluate` as Run does.
//!
//! - **Comment** (the tool bar's button, Ctrl+/): the lines the selection touches get `// ` at
//!   their common indent, or lose their line comment (`//` or `--`) when every non-blank one has
//!   one.
//! - **Find and replace** (the tool bar's button, Ctrl+F, Ctrl+H for the replace field): a row
//!   above the editor with the match count, previous and next (Shift+Enter, Enter), Replace,
//!   Replace all, Match case and Whole word; the matches are marked in the editor, the current
//!   one selected and scrolled into view; Esc closes the row.
//! - **The Data pane** beside the editor: the model's tables with their columns and measures; a
//!   click on a column or a measure (Enter from the keyboard) inserts its reference at the
//!   editor's cursor; the right-click menu (Shift+F10) of a table, a column or a measure writes
//!   a quick query into a new tab and runs it: the table's first 100 rows, the column's distinct
//!   values, the measure's value, and through `dax.define_query` a table's measures (Define all
//!   measures) or a measure with the measures it references (Define with references). The pane
//!   also lists hierarchies (their levels insert their column) and calculation items; every
//!   field row drags into the editor, inserted where it is dropped.
//! - **The lenses' own line**: [`lift`] lays out the first blank of each DEFINE MEASURE line
//!   taller, so the view draws the Update model lens above the line's text.

use std::collections::BTreeSet;
use std::ops::Range;

use egui::text::{CCursor, CCursorRange};
use egui::{Id, Key, RichText, Ui};
use serde_json::Value;

use crate::App;
use crate::dax_text;
use crate::formula::take_key;
use crate::i18n::Lang;
use crate::icons::Icon;
use crate::theme::{self, Tokens};
use crate::widgets;

/// Most rows the Data pane lists.
const MAX_ROWS: usize = 5000;
/// Longest search text.
const MAX_SEARCH: usize = 200;

// --- text ------------------------------------------------------------------------------------

/// The byte offsets where `text`'s lines start.
fn line_starts(text: &str) -> Vec<usize> {
    let mut v = vec![0];
    v.extend(text.match_indices('\n').map(|(i, _)| i + 1));
    v
}

/// The line comment a line starts with after its indent (`//` or `--`), as (where, length with
/// the one space after it).
fn comment_at(line: &str) -> Option<(usize, usize)> {
    let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
    let rest = line.get(indent..)?;
    if !(rest.starts_with("//") || rest.starts_with("--")) {
        return None;
    }
    let space = usize::from(rest.get(2..3) == Some(" "));
    Some((indent, 2 + space))
}

/// Comments the lines `sel` (bytes) touches, `// ` at the smallest indent of their non-blank
/// ones, or uncomments them when every non-blank one is a line comment. A selection that ends
/// at the start of a line leaves that line out. Returns the new text and where `sel` lands;
/// `None` when the lines are all blank.
pub fn toggle_comment(text: &str, sel: Range<usize>) -> Option<(String, Range<usize>)> {
    let a = sel.start.min(sel.end).min(text.len());
    let b = sel.start.max(sel.end).min(text.len());
    let starts = line_starts(text);
    let line_of = |p: usize| starts.partition_point(|&s| s <= p).saturating_sub(1);
    let first = line_of(a);
    let mut last = line_of(b);
    if b > a && last > first && starts.get(last) == Some(&b) {
        last -= 1;
    }
    let line = |k: usize| -> (usize, &str) {
        let s = starts.get(k).copied().unwrap_or(text.len());
        let e = starts.get(k + 1).map_or(text.len(), |n| n - 1);
        (s, text.get(s..e).unwrap_or(""))
    };
    let lines: Vec<(usize, &str)> = (first..=last)
        .map(line)
        .filter(|(_, s)| !s.trim().is_empty())
        .collect();
    if lines.is_empty() {
        return None;
    }
    // (at, removed bytes, inserted text), in text order
    let edits: Vec<(usize, usize, &str)> = if lines.iter().all(|(_, s)| comment_at(s).is_some()) {
        lines
            .iter()
            .filter_map(|(s, l)| comment_at(l).map(|(at, n)| (s + at, n, "")))
            .collect()
    } else {
        let indent = lines
            .iter()
            .map(|(_, s)| s.len() - s.trim_start_matches([' ', '\t']).len())
            .min()
            .unwrap_or(0);
        lines.iter().map(|(s, _)| (s + indent, 0, "// ")).collect()
    };
    let mut out = String::with_capacity(text.len() + edits.len() * 3);
    let mut from = 0;
    for (at, n, ins) in &edits {
        out.push_str(text.get(from..*at)?);
        out.push_str(ins);
        from = at + n;
    }
    out.push_str(text.get(from..)?);
    // `left`: a selection's start stays before a comment inserted where it is
    let shift = |p: usize, left: bool| -> usize {
        let mut q = p;
        for (at, n, ins) in &edits {
            if *n > 0 {
                if p >= at + n {
                    q = q.saturating_sub(*n);
                } else if p > *at {
                    q = q.saturating_sub(p - at);
                }
            } else if p > *at || (p == *at && !left) {
                q += ins.len();
            }
        }
        q.min(out.len())
    };
    let (s, e) = (shift(a, a < b), shift(b, false));
    let moved = if sel.start <= sel.end { s..e } else { e..s };
    Some((out, moved))
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The matches of `needle` in `text` (see [`dax_text::find_all`]); with `whole`, only those not
/// inside a longer word (letters, digits, `_`) where the needle starts or ends with one.
pub fn find_matches(text: &str, needle: &str, case: bool, whole: bool) -> Vec<Range<usize>> {
    let all = dax_text::find_all(text, needle, case);
    if !whole {
        return all;
    }
    let starts_word = needle.chars().next().is_some_and(is_word);
    let ends_word = needle.chars().next_back().is_some_and(is_word);
    all.into_iter()
        .filter(|r| {
            let before = text.get(..r.start).and_then(|s| s.chars().next_back());
            let after = text.get(r.end..).and_then(|s| s.chars().next());
            !(starts_word && before.is_some_and(is_word)) && !(ends_word && after.is_some_and(is_word))
        })
        .collect()
}

/// `'Sales'[Amount]`.
pub fn column_ref(table: &str, column: &str) -> String {
    format!("{}{}", dax_text::quote_table(table), dax_text::bracket(column))
}

/// The quick query of a table: its first 100 rows.
pub fn top_rows(table: &str) -> String {
    format!("EVALUATE\n    TOPN(100, {})\n", dax_text::quote_table(table))
}

/// The quick query of a column: its distinct values, in order.
pub fn distinct_values(table: &str, column: &str) -> String {
    let c = column_ref(table, column);
    format!("EVALUATE\n    DISTINCT({c})\nORDER BY {c}\n")
}

/// The quick query of a measure: its value.
pub fn evaluate_measure(name: &str) -> String {
    format!(
        "EVALUATE\n    ROW(\"{}\", {})\n",
        name.replace('"', "\"\""),
        dax_text::bracket(name)
    )
}

/// What a calculation item inserts: the filter that picks it, `'Time'[Name] = "YTD"` (the group's
/// first column, the one that lists the items), or the item's name as a text without a column.
pub fn calc_item_ref(table: &str, column: Option<&str>, item: &str) -> String {
    let text = format!("\"{}\"", item.replace('"', "\"\""));
    match column {
        Some(c) => format!("{} = {text}", column_ref(table, c)),
        None => text,
    }
}

// --- the lenses' own line --------------------------------------------------------------------

/// The space the lenses take above their line, in points.
pub const LENS_GAP: f32 = 20.0;

/// The white space of the line holding byte `at` that makes room for that line's lens: its
/// first run of spaces or tabs. A section taller than the others is centred in its line and the
/// others sit at its bottom, so a tall blank leaves the room above the line's text. `None` when
/// the line has no blank (the lens then sits at the line's right end).
pub fn lift_range(text: &str, at: usize) -> Option<Range<usize>> {
    let at = at.min(text.len());
    let start = text.get(..at)?.rfind('\n').map_or(0, |i| i + 1);
    let end = text.get(start..)?.find('\n').map_or(text.len(), |i| start + i);
    let line = text.get(start..end)?;
    let a = line.find([' ', '\t'])?;
    let n = line.get(a..)?.len() - line.get(a..)?.trim_start_matches([' ', '\t']).len();
    Some(start + a..start + a + n)
}

/// Gives the bytes `ranges` of `job`'s text sections of their own, `line_height` tall.
pub fn lift(job: &mut egui::text::LayoutJob, ranges: &[Range<usize>], line_height: f32) {
    use egui::epaint::text::{ByteIndex, LayoutSection};
    if ranges.is_empty() {
        return;
    }
    let old = std::mem::take(&mut job.sections);
    for sec in old {
        let (a, b) = (sec.byte_range.start.0, sec.byte_range.end.0);
        let mut cuts = vec![a, b];
        for r in ranges {
            for p in [r.start, r.end] {
                if p > a && p < b {
                    cuts.push(p);
                }
            }
        }
        cuts.sort_unstable();
        cuts.dedup();
        for (k, w) in cuts.windows(2).enumerate() {
            let (s, e) = (w[0], w[1]);
            let mut format = sec.format.clone();
            if ranges.iter().any(|r| s >= r.start && e <= r.end) {
                format.line_height = Some(line_height);
            }
            job.sections.push(LayoutSection {
                leading_space: if k == 0 { sec.leading_space } else { 0.0 },
                byte_range: ByteIndex(s)..ByteIndex(e),
                format,
            });
        }
    }
}

/// A Data pane field dragged into the editor: the reference it inserts where it is dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldDrag(pub String);

// --- state -----------------------------------------------------------------------------------

/// The find and replace row.
#[derive(Debug, Clone, Default, PartialEq, Hash)]
pub struct Find {
    pub needle: String,
    pub replacement: String,
    /// Letter case must match.
    pub case: bool,
    /// Whole words only.
    pub whole: bool,
    /// The current match (an index into the matches).
    pub current: usize,
    /// The field that takes the keyboard on the next frame.
    focus: Option<Field>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Field {
    Find,
    Replace,
}

impl Find {
    pub fn matches(&self, text: &str) -> Vec<Range<usize>> {
        find_matches(text, &self.needle, self.case, self.whole)
    }
}

/// The Data pane's view state.
#[derive(Debug, Clone, Default)]
pub struct DataPane {
    pub hidden: bool,
    /// The tables unrolled.
    pub open: BTreeSet<String>,
    /// The hierarchies unrolled (table, hierarchy).
    pub open_hierarchies: BTreeSet<(String, String)>,
    pub search: String,
}

/// The find field's id (tests).
pub fn find_id() -> Id {
    Id::unique("bicraft-dax-query-find")
}

/// The replace field's id (tests).
pub fn replace_id() -> Id {
    Id::unique("bicraft-dax-query-replace")
}

// --- actions ---------------------------------------------------------------------------------

/// The editor's selection in tab `tab_id` (bytes of `text`), as the view last saw it
/// ([`crate::dax_query::QueryView::sel`]); the end of the text when there is none. egui drops a
/// text field's selection when it loses the keyboard, so a button or a Data pane row reads this.
fn selection(saved: Option<&(u64, Range<usize>)>, tab_id: u64, text: &str) -> Range<usize> {
    saved
        .filter(|(id, _)| *id == tab_id)
        .map_or(text.len()..text.len(), |(_, r)| {
            dax_text::byte_of_char(text, r.start)..dax_text::byte_of_char(text, r.end)
        })
}

/// `r` (bytes of `text`) in characters.
fn chars(text: &str, r: &Range<usize>) -> Range<usize> {
    dax_text::char_of_byte(text, r.start)..dax_text::char_of_byte(text, r.end)
}

/// Gives the editor `r` (characters) as its selection.
pub fn select(ctx: &egui::Context, id: Id, r: Range<usize>) {
    let mut st = egui::text_edit::TextEditState::load(ctx, id).unwrap_or_default();
    st.cursor.set_char_range(Some(CCursorRange::two(
        CCursor::new(r.start),
        CCursor::new(r.end),
    )));
    st.store(ctx, id);
}

/// Comment: comments or uncomments the shown tab's selected lines.
pub fn comment(app: &mut App) {
    let l = app.lang;
    let v = &mut app.cmd.dax_query;
    let saved = v.sel.clone();
    let Some(tab) = v.current(l) else { return };
    let id = tab.id;
    let sel = selection(saved.as_ref(), id, &tab.text);
    let Some((text, moved)) = toggle_comment(&tab.text, sel) else {
        app.status = l
            .t(
                "Put the cursor on the lines to comment first.",
                "Placez d'abord le curseur sur les lignes à commenter.",
            )
            .to_string();
        return;
    };
    let r = chars(&text, &moved);
    tab.text = text;
    v.sel = Some((id, r));
    v.focus = true;
}

/// Inserts `text` at the editor's cursor (over its selection), the cursor after it.
pub fn insert(app: &mut App, text: &str) {
    let l = app.lang;
    let v = &mut app.cmd.dax_query;
    let saved = v.sel.clone();
    let Some(tab) = v.current(l) else { return };
    let id = tab.id;
    let sel = selection(saved.as_ref(), id, &tab.text);
    let (a, b) = (sel.start.min(sel.end), sel.start.max(sel.end));
    if !(tab.text.is_char_boundary(a) && tab.text.is_char_boundary(b)) {
        return;
    }
    tab.text.replace_range(a..b, text);
    let c = dax_text::char_of_byte(&tab.text, a + text.len());
    v.sel = Some((id, c..c));
    v.focus = true;
}

/// A quick query: `text` in a new tab, run.
pub fn quick(app: &mut App, text: String) {
    let l = app.lang;
    if !app.cmd.dax_query.add(l, text) {
        app.status = l
            .t(
                "Close a query tab first: a document keeps at most 64.",
                "Fermez d'abord un onglet de requête : un document en garde 64 au plus.",
            )
            .to_string();
        return;
    }
    if crate::ribbon::state(app, crate::dax_query::RUN).is_ok() {
        crate::dax_query::run(app);
    }
}

/// A Define quick query: `dax.define_query` writes it from the model, and its answer goes to a
/// new tab, run ([`quick`]).
pub fn define(app: &mut App, params: Value) {
    if let Err(why) = crate::ribbon::state(app, crate::dax_query::DEFINE) {
        app.status = why;
        return;
    }
    app.run(
        crate::actions::Call {
            id: crate::dax_query::DEFINE,
            params,
        },
        crate::app::Then::DaxQuery(crate::dax_query::Step::Define),
    );
}

/// Opens the find row (`replace`: its replace field takes the keyboard), the selected text (one
/// line of it) to find.
pub fn open_find(app: &mut App, replace: bool) {
    let l = app.lang;
    let v = &mut app.cmd.dax_query;
    let saved = v.sel.clone();
    let picked = v.current(l).and_then(|tab| {
        let r = selection(saved.as_ref(), tab.id, &tab.text);
        tab.text
            .get(r.start.min(r.end)..r.start.max(r.end))
            .filter(|s| !s.is_empty() && !s.contains('\n'))
            .map(str::to_string)
    });
    let f = v.find.get_or_insert_with(Find::default);
    if let Some(s) = picked {
        f.needle = s;
        f.current = 0;
    }
    f.focus = Some(if replace { Field::Replace } else { Field::Find });
}

/// Moves to the next (or previous) match: it becomes the editor's selection, scrolled into view.
fn step(app: &mut App, forward: bool) {
    let v = &mut app.cmd.dax_query;
    let (Some(f), Some(tab)) = (v.find.as_mut(), v.tabs.get(v.active)) else {
        return;
    };
    let found = f.matches(&tab.text);
    let n = found.len();
    if n == 0 {
        return;
    }
    f.current = if forward {
        (f.current + 1) % n
    } else {
        (f.current + n - 1) % n
    };
    if let Some(r) = found.get(f.current) {
        let r = chars(&tab.text, r);
        v.reveal = Some(r.start);
        v.sel = Some((tab.id, r));
    }
}

/// Replaces the current match (`all`: every match), as typing would.
fn replace(app: &mut App, all: bool) {
    let l = app.lang;
    let v = &mut app.cmd.dax_query;
    let active = v.active;
    let (Some(f), Some(tab)) = (v.find.as_mut(), v.tabs.get_mut(active)) else {
        return;
    };
    let found = f.matches(&tab.text);
    let picked: Vec<Range<usize>> = if all {
        found
    } else {
        found.get(f.current).cloned().into_iter().collect()
    };
    if picked.is_empty() {
        return;
    }
    for r in picked.iter().rev() {
        if r.end <= tab.text.len() && tab.text.is_char_boundary(r.start) && tab.text.is_char_boundary(r.end) {
            tab.text.replace_range(r.clone(), &f.replacement);
        }
    }
    let n = f.matches(&tab.text).len();
    f.current = if n == 0 { 0 } else { f.current.min(n - 1) };
    let k = picked.len();
    app.status = match (l, k) {
        (Lang::En, 1) => "1 match replaced.".to_string(),
        (Lang::En, _) => format!("{k} matches replaced."),
        (Lang::Fr, 1) => "1 correspondance remplacée.".to_string(),
        (Lang::Fr, _) => format!("{k} correspondances remplacées."),
    };
}

/// The marks of the find row's matches in `text`, the current one stronger.
pub(crate) fn marks(find: Option<&Find>, text: &str, t: &Tokens) -> Vec<crate::formula::Mark> {
    let Some(f) = find.filter(|f| !f.needle.is_empty()) else {
        return Vec::new();
    };
    f.matches(text)
        .into_iter()
        .enumerate()
        .map(|(i, range)| {
            let current = i == f.current;
            crate::formula::Mark {
                range,
                background: if current { t.text_selection } else { t.accent_soft },
                line: if current { t.text } else { t.text_dim },
            }
        })
        .collect()
}

// --- drawing ---------------------------------------------------------------------------------

/// The find and replace row: Find, the count, previous and next, Match case, Whole word, Replace
/// with, Replace, Replace all, Close. Enter finds the next match, Shift+Enter the previous, Esc
/// closes the row and gives the keyboard back to the editor.
pub fn find_row(app: &mut App, ui: &mut Ui) {
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    let ctx = ui.ctx().clone();
    let in_find = ctx.memory(|m| m.has_focus(find_id()) || m.has_focus(replace_id()));
    let (mut close, mut next, mut prev, mut one, mut all) = (false, false, false, false, false);
    if in_find {
        close |= take_key(ui, Key::Escape, |_| true);
        prev |= take_key(ui, Key::Enter, |m| m.shift);
        next |= take_key(ui, Key::Enter, |m| m.is_none());
    }
    let v = &mut app.cmd.dax_query;
    let text = v.active_tab().map(|q| q.text.clone()).unwrap_or_default();
    let Some(f) = v.find.as_mut() else { return };
    let n = f.matches(&text).len();
    if f.current >= n {
        f.current = 0;
    }
    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let label = ui.label(RichText::new(l.t("Find", "Rechercher")).size(12.0).color(t.text));
        let r = ui
            .add(
                egui::TextEdit::singleline(&mut f.needle)
                    .id(find_id())
                    .desired_width(150.0)
                    .font(crate::formula::mono()),
            )
            .labelled_by(label.id);
        crate::formula::keep_escape(&ctx, &r);
        if f.focus == Some(Field::Find) {
            r.request_focus();
            f.focus = None;
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
        prev |= widgets::icon_toggle(
            ui,
            Icon::ChevronUp,
            l.t(
                "Previous match (Shift+Enter)",
                "Correspondance précédente (Maj+Entrée)",
            ),
            n > 0,
            false,
        )
        .clicked();
        next |= widgets::icon_toggle(
            ui,
            Icon::ChevronDown,
            l.t("Next match (Enter)", "Correspondance suivante (Entrée)"),
            n > 0,
            false,
        )
        .clicked();
        ui.checkbox(&mut f.case, l.t("Match case", "Respecter la casse"));
        ui.checkbox(&mut f.whole, l.t("Whole word", "Mot entier"));
        ui.add_space(6.0);
        let label = ui.label(
            RichText::new(l.t("Replace with", "Remplacer par"))
                .size(12.0)
                .color(t.text),
        );
        let r = ui
            .add(
                egui::TextEdit::singleline(&mut f.replacement)
                    .id(replace_id())
                    .desired_width(130.0)
                    .font(crate::formula::mono()),
            )
            .labelled_by(label.id);
        crate::formula::keep_escape(&ctx, &r);
        if f.focus == Some(Field::Replace) {
            r.request_focus();
            f.focus = None;
        }
        one = ui
            .add_enabled(n > 0, egui::Button::new(l.t("Replace", "Remplacer")))
            .clicked();
        all = ui
            .add_enabled(n > 0, egui::Button::new(l.t("Replace all", "Tout remplacer")))
            .clicked();
        close |= widgets::icon_toggle(
            ui,
            Icon::Close,
            l.t("Close find (Esc)", "Fermer la recherche (Échap)"),
            true,
            false,
        )
        .clicked();
    });
    if prev || next {
        step(app, next && !prev);
    }
    if one || all {
        replace(app, all);
    }
    if close {
        app.cmd.dax_query.find = None;
        app.cmd.dax_query.focus = true;
    }
}

/// What a Data pane gesture asks for (done after drawing).
#[derive(Clone)]
enum Act {
    Toggle(String),
    ToggleHierarchy(String, String),
    Insert(String),
    Quick(String),
    Define(Value),
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

fn hidden(v: &Value) -> bool {
    v.get("hidden").and_then(Value::as_bool) == Some(true)
}

fn named<'a>(v: &'a Value, k: &str) -> impl Iterator<Item = &'a Value> {
    v.get(k)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|x| !s(x, "name").is_empty())
}

/// A field row of the Data pane.
struct FieldRow<'a> {
    depth: usize,
    icon: Icon,
    name: &'a str,
    dim: bool,
    /// What a click (Enter) inserts at the editor's cursor, and a drag where it is dropped.
    reference: String,
    /// The menu's quick queries, before Insert into the query.
    menu: Vec<(&'static str, Act)>,
}

fn field_row(ui: &mut Ui, l: Lang, f: FieldRow<'_>, acts: &mut Vec<Act>) {
    let r = crate::model_explorer::row(ui, l, f.depth, f.icon, f.name, None, false, f.dim, true);
    r.dnd_set_drag_payload(FieldDrag(f.reference.clone()));
    let tip = l.t(
        "Click to insert, or drag into the query",
        "Cliquer pour insérer, ou glisser dans la requête",
    );
    let r = r.on_hover_text(format!("{tip}: {}", f.reference));
    if r.clicked() {
        acts.push(Act::Insert(f.reference.clone()));
    }
    crate::keyboard::context_menu(&r, |ui| {
        for (text, act) in &f.menu {
            if ui.button(*text).clicked() {
                acts.push(act.clone());
                ui.close();
            }
        }
        if ui
            .button(l.t("Insert into the query", "Insérer dans la requête"))
            .clicked()
        {
            acts.push(Act::Insert(f.reference.clone()));
            ui.close();
        }
    });
}

/// The Data pane: its title, a search box and the model's tables with their columns, measures,
/// hierarchies (their levels) and calculation items.
pub fn data_pane(app: &mut App, ui: &mut Ui, snap: Option<&Value>) {
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    let mut acts = Vec::new();
    let pane = &mut app.cmd.dax_query.data;
    ui.label(
        RichText::new(l.t("Data", "Données"))
            .font(theme::bold(14.0))
            .color(t.text),
    );
    ui.add_space(4.0);
    let name = l.t("Search the fields", "Rechercher des champs");
    ui.add(
        egui::TextEdit::singleline(&mut pane.search)
            .char_limit(MAX_SEARCH)
            .hint_text(l.t("Search", "Rechercher"))
            .desired_width(ui.available_width()),
    )
    .widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, name));
    ui.add_space(4.0);
    let needle = pane.search.trim().to_lowercase();
    let hit = |n: &str| needle.is_empty() || n.to_lowercase().contains(&needle);
    let tables: Vec<&Value> = snap
        .and_then(|s| s.get("model"))
        .map(|m| named(m, "tables").collect())
        .unwrap_or_default();
    if tables.is_empty() {
        ui.label(
            RichText::new(l.t("The model has no table.", "Le modèle n'a aucune table.")).color(t.text_dim),
        );
        return;
    }
    let distinct = l.t("Show distinct values", "Afficher les valeurs distinctes");
    egui::ScrollArea::vertical()
        .id_salt("bicraft-dax-query-data")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let mut rows = 0usize;
            for tbl in tables {
                let table = s(tbl, "name");
                let hierarchies: Vec<&Value> = named(tbl, "hierarchy_levels").collect();
                let items: Vec<&str> = tbl
                    .get("calculation_items")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .collect();
                let fields: Vec<(&Value, bool)> = named(tbl, "columns")
                    .map(|c| (c, false))
                    .chain(named(tbl, "measures").map(|m| (m, true)))
                    .collect();
                let all = hit(table);
                let any = fields.iter().any(|(x, _)| hit(s(x, "name")))
                    || hierarchies
                        .iter()
                        .any(|h| hit(s(h, "name")) || named(h, "levels").any(|lv| hit(s(lv, "name"))))
                    || items.iter().any(|i| hit(i));
                if !all && !any {
                    continue;
                }
                rows += 1;
                if rows > MAX_ROWS {
                    break;
                }
                let open = pane.open.contains(table) || !needle.is_empty();
                let icon = if items.is_empty() {
                    Icon::Table
                } else {
                    Icon::Calculator
                };
                let r =
                    crate::model_explorer::row(ui, l, 0, icon, table, Some(open), false, hidden(tbl), false);
                if r.clicked() || crate::keyboard::tree_toggle(ui, &r, open) {
                    acts.push(Act::Toggle(table.to_string()));
                }
                let quoted = dax_text::quote_table(table);
                let has_measures = named(tbl, "measures").next().is_some();
                crate::keyboard::context_menu(&r, |ui| {
                    if ui
                        .button(l.t("Evaluate top 100 rows", "Évaluer les 100 premières lignes"))
                        .clicked()
                    {
                        acts.push(Act::Quick(top_rows(table)));
                        ui.close();
                    }
                    if has_measures
                        && ui
                            .button(l.t("Define all measures", "Définir toutes les mesures"))
                            .clicked()
                    {
                        acts.push(Act::Define(serde_json::json!({"table": table})));
                        ui.close();
                    }
                    if ui
                        .button(l.t("Insert into the query", "Insérer dans la requête"))
                        .clicked()
                    {
                        acts.push(Act::Insert(quoted.clone()));
                        ui.close();
                    }
                });
                if !open {
                    continue;
                }
                for (x, measure) in &fields {
                    let field = s(x, "name");
                    if !all && !hit(field) {
                        continue;
                    }
                    rows += 1;
                    if rows > MAX_ROWS {
                        break;
                    }
                    let f = if *measure {
                        FieldRow {
                            depth: 1,
                            icon: Icon::Measure,
                            name: field,
                            dim: hidden(x),
                            reference: dax_text::bracket(field),
                            menu: vec![
                                (
                                    l.t("Evaluate measure", "Évaluer la mesure"),
                                    Act::Quick(evaluate_measure(field)),
                                ),
                                (
                                    l.t("Define with references", "Définir avec les références"),
                                    Act::Define(serde_json::json!({"measure": field, "references": true})),
                                ),
                            ],
                        }
                    } else {
                        FieldRow {
                            depth: 1,
                            icon: Icon::Column,
                            name: field,
                            dim: hidden(x),
                            reference: column_ref(table, field),
                            menu: vec![(distinct, Act::Quick(distinct_values(table, field)))],
                        }
                    };
                    field_row(ui, l, f, &mut acts);
                }
                // hierarchies, unrolled to their levels (each level is a column reference)
                for h in hierarchies {
                    let hname = s(h, "name");
                    let levels: Vec<&Value> = named(h, "levels").collect();
                    let whole = all || hit(hname);
                    if !whole && !levels.iter().any(|lv| hit(s(lv, "name"))) {
                        continue;
                    }
                    rows += 1;
                    if rows > MAX_ROWS {
                        break;
                    }
                    let key = (table.to_string(), hname.to_string());
                    let hopen = pane.open_hierarchies.contains(&key) || !needle.is_empty();
                    let r = crate::model_explorer::row(
                        ui,
                        l,
                        1,
                        Icon::Hierarchy,
                        hname,
                        Some(hopen),
                        false,
                        hidden(h),
                        false,
                    );
                    if r.clicked() || crate::keyboard::tree_toggle(ui, &r, hopen) {
                        acts.push(Act::ToggleHierarchy(key.0, key.1));
                    }
                    if !hopen {
                        continue;
                    }
                    for lv in levels {
                        let level = s(lv, "name");
                        if !whole && !hit(level) {
                            continue;
                        }
                        rows += 1;
                        if rows > MAX_ROWS {
                            break;
                        }
                        let column = lv.get("column").and_then(Value::as_str).unwrap_or(level);
                        let f = FieldRow {
                            depth: 2,
                            icon: Icon::Column,
                            name: level,
                            dim: hidden(h),
                            reference: column_ref(table, column),
                            menu: vec![(distinct, Act::Quick(distinct_values(table, column)))],
                        };
                        field_row(ui, l, f, &mut acts);
                    }
                }
                // a calculation group's items: each inserts the filter that picks it
                let column = named(tbl, "columns").find(|c| !hidden(c)).map(|c| s(c, "name"));
                for item in items {
                    if !all && !hit(item) {
                        continue;
                    }
                    rows += 1;
                    if rows > MAX_ROWS {
                        break;
                    }
                    let f = FieldRow {
                        depth: 1,
                        icon: Icon::Calculator,
                        name: item,
                        dim: false,
                        reference: calc_item_ref(table, column, item),
                        menu: Vec::new(),
                    };
                    field_row(ui, l, f, &mut acts);
                }
            }
        });
    for a in acts {
        match a {
            Act::Toggle(table) => {
                let open = &mut app.cmd.dax_query.data.open;
                if !open.remove(&table) {
                    open.insert(table);
                }
            }
            Act::ToggleHierarchy(table, h) => {
                let open = &mut app.cmd.dax_query.data.open_hierarchies;
                let key = (table, h);
                if !open.remove(&key) {
                    open.insert(key);
                }
            }
            Act::Insert(text) => insert(app, &text),
            Act::Quick(text) => quick(app, text),
            Act::Define(p) => define(app, p),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comment_takes_the_lines_the_selection_touches_and_keeps_it_on_them() {
        let text = "DEFINE\n\tMEASURE T[M] = 1\n\n\t\tVAR x = 2\nEVALUATE T";
        // from inside line 2 to the start of line 5: lines 2 to 4, the blank one left alone
        let a = text.find("MEAS").unwrap() + 2;
        let b = text.find("EVALUATE").unwrap();
        let (out, sel) = toggle_comment(text, a..b).unwrap();
        assert_eq!(
            out,
            "DEFINE\n\t// MEASURE T[M] = 1\n\n\t// \tVAR x = 2\nEVALUATE T"
        );
        assert_eq!(out.get(sel.clone()).map(|s| s.starts_with("ASURE")), Some(true));
        assert_eq!(sel.end, out.find("EVALUATE").unwrap());
        // and back, `--` comments too
        let (back, _) = toggle_comment(&out, sel).unwrap();
        assert_eq!(back, text);
        let (out, _) = toggle_comment("--x\n  //  y", 0..6).unwrap();
        assert_eq!(out, "x\n   y");
        // one line commented and one not: both get a comment
        let (out, _) = toggle_comment("// a\nb", 0..6).unwrap();
        assert_eq!(out, "// // a\n// b");
        assert!(toggle_comment("a\n\n  \nb", 2..4).is_none());
    }

    #[test]
    fn whole_word_keeps_matches_outside_longer_names() {
        let t = "Sales[Amount] AmountX _Amount amount [Amount]";
        assert_eq!(find_matches(t, "amount", false, false).len(), 5);
        let whole: Vec<&str> = find_matches(t, "amount", false, true)
            .into_iter()
            .filter_map(|r| t.get(r))
            .collect();
        assert_eq!(whole, ["Amount", "amount", "Amount"]);
        // a needle with brackets: its bracket edges need no word boundary
        assert_eq!(find_matches(t, "[Amount]", true, true).len(), 2);
        assert_eq!(find_matches("é1 1", "1", true, true).len(), 1);
    }

    #[test]
    fn a_lifted_line_keeps_its_text_and_drops_below_the_room_for_its_lens() {
        let text = "DEFINE\n    MEASURE T[M] = 1\nEVALUATE ROW(\"m\", [M])";
        let at = text.find("MEASURE").unwrap();
        // the line's first blank: its indent; a line without one is not lifted
        assert_eq!(lift_range(text, at), Some(7..11));
        assert_eq!(lift_range("DEFINE\nMEASURE\nX", 7), None);
        let ctx = egui::Context::default();
        let mut tops = Vec::new();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let t = Tokens::get(ui.ctx());
            let row = ui.fonts_mut(|f| f.row_height(&crate::formula::mono()));
            for lifted in [false, true] {
                let mut job = crate::formula::job_marked(text, &t, None, f32::INFINITY, &[]);
                if lifted {
                    let indent = lift_range(text, at).unwrap();
                    lift(&mut job, std::slice::from_ref(&indent), row + LENS_GAP);
                }
                assert_eq!(job.text, text);
                let g = ui.fonts_mut(|f| f.layout_job(job));
                assert_eq!(g.rows.len(), 3);
                // where MEASURE's first letter is drawn, and the last line
                let m = g.rows[1].glyphs.iter().find(|x| x.chr == 'M').map(|x| x.pos.y);
                tops.push((m.unwrap(), g.rows[2].min_y(), row));
            }
        });
        out.textures_delta.clear();
        let [(m0, last0, row), (m1, last1, _)] = tops[..] else {
            panic!("{tops:?}")
        };
        // the text sits at the bottom of the taller line: the room is above it
        assert!(m1 - m0 > LENS_GAP - 1.0, "{m0} {m1} (row {row})");
        assert!(last1 - last0 > LENS_GAP - 1.0, "{last0} {last1}");
    }
}
