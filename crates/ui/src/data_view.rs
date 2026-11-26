//! The Data view (Power BI Desktop's table view; docs/design.md §5 and §12, docs/desktop.md "Data
//! view"): a table's rows in a virtualized grid next to Report and Model, the same in both shells.
//!
//! - The **Data** pane at the right lists the model's tables (a click shows its rows; its columns
//!   under it select a column) and its Power Query queries (a click previews the query).
//! - The **grid** pages the rows from the engine (`table.rows`, [`PAGE`] rows a request, a few
//!   requests in flight, the pages near the view kept): a table of millions of rows scrolls
//!   without being read whole. Each header shows the column's type and name; a click selects the
//!   column (the Table tab's Column tools appear in the ribbon, a calculated column goes on the
//!   formula bar), its arrow opens the sort and filter menu (Sort ascending or descending, the
//!   column's values to keep, from `column.values`; the Text, Number or Date filters and their
//!   Custom filter window, `data_filter.rs`): view state only, as in Desktop. Cells are
//!   selected with a click (Shift extends), Ctrl+C or the menu copies them as tab-separated text.
//! - The same grid shows a **query preview** (`query.preview`: Home › Queries, the pane's query
//!   entries) and **See records** of a data point (`visual.records`, the visual's right-click
//!   menu), read once, with a Close button back to the table.
//!
//! The row count is in the status bar. Nothing here changes the model: the Column tools run
//! `column.set`, New column and New measure open the formula bar.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use egui::{Align2, CornerRadius, Id, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use serde_json::{Value, json};

use crate::App;
use crate::actions::Call;
use crate::app::{Then, clip};
use crate::i18n::Lang;
use crate::icons::{self, Icon};
use crate::ribbon;
use crate::theme::{self, Tokens};
use crate::widgets;

/// Rows a request asks for.
pub const PAGE: usize = 200;
/// Requests in flight at once.
const MAX_INFLIGHT: usize = 3;
/// Pages kept around the view.
const MAX_PAGES: usize = 48;
/// Height of a row and of the header.
pub const ROW_H: f32 = 22.0;
const HEAD_H: f32 = 28.0;
/// Width of the Data pane.
const PANE_W: f32 = 240.0;
/// Most cells copied at once.
const MAX_COPY: usize = 100_000;
/// Most values the filter menu lists.
const MAX_VALUES: usize = 1_000;
/// Longest text drawn in a cell (characters).
const MAX_CELL: usize = 200;

/// The commands the Data view runs (the coverage test reads it).
pub const COMMANDS: &[&str] = &[
    "table.rows",
    "query.preview",
    "visual.records",
    "column.values",
    "column.set",
    "column.add",
    "measure.add",
];

/// What an answer of the view's commands is for ([`Then::Data`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// A page of rows from `offset`, for generation `gen` of the view.
    Rows { generation: u64, offset: usize },
    /// The values of a column for the filter menu.
    Values { generation: u64, column: String },
    /// A query preview or a See records answer.
    Static { generation: u64, records: bool },
}

/// One column of the grid.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Col {
    pub name: String,
    pub data_type: String,
    pub calculated: bool,
    pub format_string: Option<String>,
    pub summarize_by: Option<String>,
    pub data_category: Option<String>,
    pub sort_by_column: Option<String>,
    pub available: bool,
    pub reason: Option<String>,
}

impl Col {
    fn of(v: &Value) -> Col {
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        Col {
            name: s("name").unwrap_or_default(),
            data_type: s("data_type").unwrap_or_default(),
            calculated: s("kind").as_deref() == Some("calculated"),
            format_string: s("format_string"),
            summarize_by: s("summarize_by"),
            data_category: s("data_category"),
            sort_by_column: s("sort_by_column"),
            available: v.get("available").and_then(Value::as_bool).unwrap_or(true),
            reason: s("reason"),
        }
    }
}

/// A grid read once: a query preview or See records.
#[derive(Debug, Clone, Default)]
pub struct Static {
    pub title: String,
    pub columns: Vec<Col>,
    pub rows: Vec<Vec<Value>>,
    pub truncated: bool,
    pub note: String,
}

/// The filter and sort menu of a column.
#[derive(Debug, Clone)]
pub struct Menu {
    pub column: String,
    pub at: egui::Pos2,
    /// The values kept checked (JSON text of each).
    pub checked: BTreeSet<String>,
    pub search: String,
    /// The menu was opened this frame (its click does not close it).
    pub fresh: bool,
}

/// The values of the filter menu's column: (generation, column, values or error, cut).
pub type Values = (u64, String, Result<Vec<Value>, String>, bool);

/// A table of the Data pane: its name, whether hidden, its columns (name, calculated).
type PaneTable = (String, bool, Vec<(String, bool)>);

/// The Data view's state (pure view state: never sent anywhere but as parameters).
#[derive(Debug, Default)]
pub struct DataView {
    pub table: Option<String>,
    /// The column selected (the Column tools).
    pub column: Option<String>,
    pub sort: Option<(String, bool)>,
    /// Per column, its filter (the values kept, or the custom filter's tests).
    pub filters: BTreeMap<String, crate::data_filter::ColFilter>,
    /// The Custom filter window (text, number and date filters).
    pub custom: Option<crate::data_filter::Custom>,
    pub columns: Vec<Col>,
    /// Pages of rows by their first row.
    pub pages: BTreeMap<usize, Vec<Vec<Value>>>,
    pub pending: BTreeSet<usize>,
    pub row_count: Option<usize>,
    pub total_rows: Option<usize>,
    /// Why the table has no rows to show.
    pub reason: Option<String>,
    pub error: Option<String>,
    /// Bumped when the table, the sort, the filter or the model changes: older answers are dropped.
    pub generation: u64,
    /// The snapshot revision the pages were read at.
    pub rev: Option<u64>,
    /// Selected cells: anchor and focus (row, column).
    pub sel: Option<((usize, usize), (usize, usize))>,
    /// The Data pane's search text.
    pub search: String,
    /// A query preview or See records shown instead of the table.
    pub stat: Option<Static>,
    /// A query preview or See records being read (its title).
    pub stat_pending: Option<String>,
    pub menu: Option<Menu>,
    /// The values of the filter menu's column: (generation, column, values or error, cut).
    pub values: Option<Values>,
    /// The Column tools' name being typed: (column, text).
    pub name_edit: Option<(String, String)>,
    /// The visible rows the last frame drew.
    pub visible: std::ops::Range<usize>,
    widths: HashMap<String, f32>,
}

impl DataView {
    /// Starts over on `table` (or the same table with a new sort or filter).
    fn reset(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.pages.clear();
        self.pending.clear();
        self.row_count = None;
        self.total_rows = None;
        self.reason = None;
        self.error = None;
        self.sel = None;
    }

    /// The cell at display row `r`, column `c`.
    pub fn cell(&self, r: usize, c: usize) -> Option<&Value> {
        if let Some(s) = &self.stat {
            return s.rows.get(r)?.get(c);
        }
        let start = r / PAGE * PAGE;
        self.pages.get(&start)?.get(r - start)?.get(c)
    }

    /// The grid's columns.
    pub fn cols(&self) -> &[Col] {
        match &self.stat {
            Some(s) => &s.columns,
            None => &self.columns,
        }
    }

    /// The grid's row count (`None` until the first page is read).
    pub fn rows(&self) -> Option<usize> {
        match &self.stat {
            Some(s) => Some(s.rows.len()),
            None => self.row_count,
        }
    }

    /// The selected cells as tab-separated text, the header first; rows not read yet are left
    /// out (their count is the second value).
    pub fn tsv(&self) -> (String, usize) {
        let Some(((r0, c0), (r1, c1))) = self.sel else {
            return (String::new(), 0);
        };
        let (ra, rb) = (r0.min(r1), r0.max(r1));
        let (ca, cb) = (c0.min(c1), c0.max(c1));
        let cols = self.cols();
        let clean = |s: &str| s.replace(['\t', '\r', '\n'], " ");
        let head: Vec<String> = (ca..=cb)
            .filter_map(|c| cols.get(c).map(|x| clean(&x.name)))
            .collect();
        let mut out = head.join("\t");
        let mut missing = 0usize;
        let width = cb - ca + 1;
        let last = rb.min(ra.saturating_add(MAX_COPY / width.max(1)));
        for r in ra..=last {
            if self.cell(r, ca).is_none() && self.cell(r, cb).is_none() {
                missing += 1;
                continue;
            }
            out.push('\n');
            let cells: Vec<String> = (ca..=cb)
                .map(|c| clean(&text(self.cell(r, c).unwrap_or(&Value::Null))))
                .collect();
            out.push_str(&cells.join("\t"));
        }
        (out, missing)
    }
}

/// A cell's text: blank, numbers as the engine gives them, `True`/`False`, text and ISO dates.
pub fn text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::String(s) => clip(s, MAX_CELL),
        // a whole number written as a float (`20.0`) reads as the whole number
        Value::Number(n) => match n.as_f64() {
            Some(x) if n.is_f64() && x.fract() == 0.0 && x.abs() < 1e15 => format!("{}", x as i64),
            _ => n.to_string(),
        },
        other => clip(&other.to_string(), MAX_CELL),
    }
}

fn arr(v: Option<&Value>) -> &[Value] {
    v.and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

/// The model's tables in the snapshot: (name, hidden, columns).
fn tables(app: &App) -> Vec<PaneTable> {
    let Some(l) = &app.loaded else { return Vec::new() };
    arr(l.snap.pointer("/model/tables"))
        .iter()
        .filter_map(|t| {
            let name = t.get("name")?.as_str()?.to_string();
            let hidden = t.get("hidden").and_then(Value::as_bool).unwrap_or(false);
            let cols = arr(t.get("columns"))
                .iter()
                .filter_map(|c| {
                    Some((
                        c.get("name")?.as_str()?.to_string(),
                        c.get("calculated").and_then(Value::as_bool).unwrap_or(false),
                    ))
                })
                .collect();
            Some((name, hidden, cols))
        })
        .collect()
}

/// The model's Power Query queries (shared expressions) in the snapshot.
fn queries(app: &App) -> Vec<String> {
    let Some(l) = &app.loaded else { return Vec::new() };
    arr(l.snap.pointer("/model/expressions"))
        .iter()
        .filter_map(|e| e.as_str().map(str::to_string))
        .collect()
}

fn revision(app: &App) -> Option<u64> {
    app.loaded.as_ref()?.snap.get("revision")?.as_u64()
}

/// Shows `table` in the grid (the Data pane's click, `table.rows` from the palette).
pub fn open_table(app: &mut App, table: &str) {
    let dv = &mut app.cmd.data;
    if dv.table.as_deref() != Some(table) || dv.stat.is_some() {
        dv.table = Some(table.to_string());
        dv.column = None;
        dv.sort = None;
        dv.filters.clear();
        dv.columns.clear();
        dv.stat = None;
        dv.stat_pending = None;
        dv.menu = None;
        dv.custom = None;
        dv.reset();
    }
    if app.tab != crate::app::Tab::Data {
        app.show_tab(crate::app::Tab::Data);
    }
}

/// Selects `column` of the table shown: the Column tools, and the formula bar for a calculated
/// column.
pub fn select_column(app: &mut App, ctx: &egui::Context, column: &str) {
    let Some(table) = app.cmd.data.table.clone() else {
        return;
    };
    app.cmd.data.column = Some(column.to_string());
    app.cmd.data.name_edit = None;
    crate::formula::pick(ctx, crate::formula::Kind::Column, Some(&table), column, false);
    app.ribbon_tab = crate::registry::Tab::Table;
}

/// The parameters of a page request.
fn rows_params(dv: &DataView, table: &str, offset: usize) -> Value {
    let mut p = json!({"table": table, "offset": offset, "limit": PAGE});
    if let Some((c, d)) = &dv.sort {
        p["sort"] = json!({"column": c, "descending": d});
    }
    if !dv.filters.is_empty() {
        p["filter"] = Value::Array(dv.filters.iter().flat_map(|(c, f)| f.conditions(c)).collect());
    }
    p
}

/// Asks for the pages of `range` not read yet (and the first page when nothing is), keeps the
/// pages near it. A new snapshot revision reads the table again.
pub fn ensure(app: &mut App, range: std::ops::Range<usize>) {
    if app.cmd.data.stat.is_some() || app.cmd.data.stat_pending.is_some() {
        return;
    }
    let rev = revision(app);
    // the table: the one picked, else the model's first
    if app.cmd.data.table.is_none() {
        match tables(app)
            .into_iter()
            .find(|t| !t.1)
            .or_else(|| tables(app).into_iter().next())
        {
            Some((name, _, _)) => app.cmd.data.table = Some(name),
            None => return,
        }
    }
    let Some(table) = app.cmd.data.table.clone() else {
        return;
    };
    // a table removed or renamed elsewhere: the first one again
    if !tables(app).iter().any(|t| t.0 == table) {
        app.cmd.data.table = None;
        app.cmd.data.column = None;
        app.cmd.data.reset();
        return;
    }
    if app.cmd.data.rev != rev {
        app.cmd.data.rev = rev;
        app.cmd.data.reset();
    }
    let dv = &app.cmd.data;
    let mut want: Vec<usize> = Vec::new();
    match dv.row_count {
        None => want.push(0),
        Some(n) if n > 0 => {
            let a = range.start.min(n.saturating_sub(1)) / PAGE * PAGE;
            let b = range.end.min(n).saturating_sub(1) / PAGE * PAGE;
            let mut o = a;
            while o <= b {
                want.push(o);
                o += PAGE;
            }
        }
        _ => {}
    }
    let want: Vec<usize> = want
        .into_iter()
        .filter(|o| !dv.pages.contains_key(o) && !dv.pending.contains(o))
        .collect();
    let room = MAX_INFLIGHT.saturating_sub(dv.pending.len());
    let generation = dv.generation;
    for offset in want.into_iter().take(room) {
        let params = rows_params(&app.cmd.data, &table, offset);
        if app.run(
            Call {
                id: "table.rows",
                params,
            },
            Then::Data(Step::Rows { generation, offset }),
        ) {
            app.cmd.data.pending.insert(offset);
        }
    }
    // the pages far from the view go
    let dv = &mut app.cmd.data;
    while dv.pages.len() > MAX_PAGES {
        let mid = range.start;
        let far = dv.pages.keys().copied().max_by_key(|o| o.abs_diff(mid));
        match far {
            Some(o) => {
                dv.pages.remove(&o);
            }
            None => break,
        }
    }
}

/// The answer of one of the view's commands.
pub fn answer(app: &mut App, step: Step, r: Result<Value, String>) {
    let l = app.lang;
    let dv = &mut app.cmd.data;
    match step {
        Step::Rows { generation, offset } => {
            if generation != dv.generation {
                return;
            }
            dv.pending.remove(&offset);
            match r {
                Ok(v) => {
                    if v.get("available").and_then(Value::as_bool) == Some(false) {
                        dv.reason = v.get("reason").and_then(Value::as_str).map(str::to_string);
                    }
                    dv.columns = arr(v.get("columns")).iter().map(Col::of).collect();
                    dv.row_count = v
                        .get("row_count")
                        .and_then(Value::as_u64)
                        .and_then(|n| usize::try_from(n).ok());
                    dv.total_rows = v
                        .get("total_rows")
                        .and_then(Value::as_u64)
                        .and_then(|n| usize::try_from(n).ok());
                    let rows: Vec<Vec<Value>> = arr(v.get("rows"))
                        .iter()
                        .map(|r| r.as_array().cloned().unwrap_or_default())
                        .collect();
                    dv.pages.insert(offset, rows);
                }
                Err(e) => {
                    dv.error = Some(clip(&e, 600));
                    if dv.row_count.is_none() {
                        dv.row_count = Some(0);
                    }
                }
            }
        }
        Step::Values { generation, column } => {
            if generation != dv.generation {
                return;
            }
            let (vals, cut) = match r {
                Ok(v) => (
                    Ok(arr(v.get("values")).iter().take(MAX_VALUES).cloned().collect()),
                    v.get("truncated").and_then(Value::as_bool).unwrap_or(false),
                ),
                Err(e) => (Err(clip(&e, 300)), false),
            };
            dv.values = Some((generation, column, vals, cut));
        }
        Step::Static { generation, records } => {
            if generation != dv.generation {
                return;
            }
            let title = dv.stat_pending.take().unwrap_or_default();
            match r {
                Ok(v) => {
                    let columns = arr(v.get("columns")).iter().map(Col::of).collect();
                    let rows = arr(v.get("rows"))
                        .iter()
                        .map(|r| r.as_array().cloned().unwrap_or_default())
                        .collect();
                    let truncated = v.get("truncated").and_then(Value::as_bool).unwrap_or(false);
                    let note = if records {
                        let point: Vec<String> = arr(v.get("point"))
                            .iter()
                            .map(|p| {
                                format!(
                                    "{}[{}] = {}",
                                    p.get("table").and_then(Value::as_str).unwrap_or(""),
                                    p.get("column").and_then(Value::as_str).unwrap_or(""),
                                    text(p.get("value").unwrap_or(&Value::Null))
                                )
                            })
                            .collect();
                        let what = if point.is_empty() {
                            l.t("every row", "toutes les lignes").to_string()
                        } else {
                            point.join(", ")
                        };
                        let applied = if arr(v.get("filters")).is_empty() {
                            l.t("no other filter applies", "aucun autre filtre ne s'applique")
                        } else {
                            l.t(
                                "with the report, page and visual filters and the slicers",
                                "avec les filtres du rapport, de la page et du visuel et les segments",
                            )
                        };
                        let skipped = arr(v.get("not_applied")).len();
                        let mut note = format!("{what} · {applied}");
                        if skipped > 0 {
                            note.push_str(&format!(
                                " · {skipped} {}",
                                l.t(
                                    "filter(s) not applied to the rows",
                                    "filtre(s) non appliqué(s) aux lignes"
                                )
                            ));
                        }
                        note
                    } else {
                        String::new()
                    };
                    let title = match (records, v.get("table").and_then(Value::as_str)) {
                        (true, Some(t)) => format!("{title} · {t}"),
                        _ => title,
                    };
                    dv.stat = Some(Static {
                        title,
                        columns,
                        rows,
                        truncated,
                        note,
                    });
                    dv.sel = None;
                    dv.error = None;
                }
                Err(e) => {
                    dv.stat = None;
                    dv.error = Some(clip(&e, 600));
                }
            }
        }
    }
}

/// Runs a query preview (`query.preview {name | expression}`) into the grid.
pub fn preview(app: &mut App, params: Value) {
    let l = app.lang;
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_else(|| l.t("expression", "expression"))
        .to_string();
    start_static(
        app,
        format!("{} · {name}", l.t("Query preview", "Aperçu de la requête")),
        "query.preview",
        params,
        false,
    );
}

/// Runs See records (`visual.records {page, visual, point}`) into the grid.
pub fn records(app: &mut App, params: Value) {
    let l = app.lang;
    start_static(
        app,
        l.t("Records", "Enregistrements").to_string(),
        "visual.records",
        params,
        true,
    );
}

fn start_static(app: &mut App, title: String, id: &'static str, params: Value, records: bool) {
    let dv = &mut app.cmd.data;
    dv.generation = dv.generation.wrapping_add(1);
    dv.stat = None;
    dv.menu = None;
    dv.sel = None;
    dv.error = None;
    dv.stat_pending = Some(title);
    let generation = dv.generation;
    if app.tab != crate::app::Tab::Data {
        app.show_tab(crate::app::Tab::Data);
    }
    if !app.run(
        Call { id, params },
        Then::Data(Step::Static { generation, records }),
    ) {
        app.cmd.data.stat_pending = None;
    }
}

/// Closes a preview or See records: the table again.
pub fn close_static(app: &mut App) {
    let dv = &mut app.cmd.data;
    dv.stat = None;
    dv.stat_pending = None;
    dv.sel = None;
    dv.reset();
}

/// The buttons of the view's commands (`ribbon::invoke`): true when handled here.
pub fn invoke(app: &mut App, id: &str, params: &Value) -> bool {
    match id {
        "table.rows" => {
            let table = params.get("table").and_then(Value::as_str).map(str::to_string);
            match table {
                Some(t) => open_table(app, &t),
                None if app.tab != crate::app::Tab::Data => app.show_tab(crate::app::Tab::Data),
                None => {}
            }
            true
        }
        "query.preview" => {
            if params.get("name").is_some() || params.get("expression").is_some() {
                preview(app, params.clone());
            } else {
                // Home > Preview query: the Data view's query list picks one
                if app.tab != crate::app::Tab::Data {
                    app.show_tab(crate::app::Tab::Data);
                }
                app.status = app
                    .lang
                    .t(
                        "Pick a query or a table in the Data pane to preview it.",
                        "Choisissez une requête ou une table dans le volet Données pour l'afficher.",
                    )
                    .to_string();
            }
            true
        }
        "visual.records" => {
            let mut p = params.clone();
            if p.get("point").is_none()
                && let (Some(o), Some(pt)) = (p.as_object_mut(), app.cmd.point.clone())
            {
                o.insert("point".into(), pt);
                if let Some(pts) = app.cmd.points.clone() {
                    o.insert("points".into(), pts);
                }
            }
            records(app, p);
            true
        }
        _ => false,
    }
}

/// The status bar's text in the Data view: the table and its rows.
pub fn status(app: &App) -> Option<String> {
    let l = app.lang;
    let dv = &app.cmd.data;
    let rows = |n: usize| rows_text(n, l);
    if let Some(s) = &dv.stat {
        let mut t = format!("{} · {}", s.title, rows(s.rows.len()));
        if s.truncated {
            t.push_str(&format!(" ({})", l.t("first rows", "premières lignes")));
        }
        return Some(t);
    }
    let table = dv.table.as_deref()?;
    let n = dv.row_count?;
    let mut t = format!("{} {table} · {}", l.t("Table:", "Table :"), rows(n));
    if let Some(total) = dv.total_rows.filter(|t| *t != n) {
        t.push_str(&format!(
            " {} {}",
            l.t("of", "sur"),
            crate::answer::count(total, l)
        ));
    }
    if let Some(c) = &dv.column {
        t.push_str(&format!(" · {} {c}", l.t("column", "colonne")));
    }
    Some(t)
}

// --- drawing --------------------------------------------------------------------------------

/// The type's tag in a header (and the Data pane): its short text, or `None` for the calendar.
fn type_tag(c: &Col) -> Option<&'static str> {
    Some(match c.data_type.as_str() {
        "string" => "Aa",
        "int64" => "123",
        "double" => "1.2",
        "decimal" => "$",
        "boolean" => "T/F",
        "dateTime" => return None,
        _ => "?",
    })
}

fn type_name(c: &Col, l: Lang) -> &'static str {
    match c.data_type.as_str() {
        "string" => l.t("Text", "Texte"),
        "int64" => l.t("Whole number", "Nombre entier"),
        "double" => l.t("Decimal number", "Nombre décimal"),
        "decimal" => l.t("Fixed decimal number", "Nombre décimal fixe"),
        "boolean" => l.t("True/false", "Vrai/faux"),
        "dateTime" => l.t("Date/time", "Date/heure"),
        _ => l.t("Other", "Autre"),
    }
}

fn default_width(c: &Col) -> f32 {
    match c.data_type.as_str() {
        "string" => 150.0,
        "dateTime" => 130.0,
        "boolean" => 80.0,
        _ => 110.0,
    }
}

/// What a frame of the grid asks the app to do.
enum Act {
    SelectColumn(usize),
    Menu(usize, egui::Pos2),
    Cell(usize, usize, bool),
    Copy,
}

/// The view: the grid at the left, the Data pane at the right.
pub fn show(app: &mut App, ui: &mut Ui) {
    // below what the view's panel already holds (the error bar), not over it
    let rect = ui.available_rect_before_wrap();
    let pane = Rect::from_min_max(pos2(rect.max.x - PANE_W, rect.min.y), rect.max);
    let main = Rect::from_min_max(rect.min, pos2(pane.min.x, rect.max.y));
    let mut pui = ui.new_child(egui::UiBuilder::new().max_rect(pane.shrink(8.0)));
    pane_ui(app, &mut pui, pane);
    let mut gui = ui.new_child(egui::UiBuilder::new().max_rect(main.shrink2(vec2(10.0, 8.0))));
    grid_ui(app, &mut gui);
    menu_ui(app, ui.ctx());
    crate::data_filter::window(app, ui.ctx());
}

/// The Data pane: tables (and the selected table's columns), then queries.
fn pane_ui(app: &mut App, ui: &mut Ui, rect: Rect) {
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    ui.painter().rect_filled(rect, CornerRadius::ZERO, t.chrome);
    ui.painter()
        .vline(rect.min.x, rect.y_range(), Stroke::new(1.0, t.separator));
    ui.label(
        RichText::new(l.t("Data", "Données"))
            .font(theme::bold(16.0))
            .color(t.text),
    );
    let r = ui.add(
        egui::TextEdit::singleline(&mut app.cmd.data.search)
            .hint_text(l.t("Search", "Rechercher"))
            .desired_width(ui.available_width()),
    );
    r.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::TextEdit,
            true,
            l.t("Search the tables", "Rechercher les tables"),
        )
    });
    let q = app.cmd.data.search.trim().to_lowercase();
    let list = tables(app);
    let qs = queries(app);
    let ctx = ui.ctx().clone();
    let mut pick_table: Option<String> = None;
    let mut pick_column: Option<(String, String)> = None;
    let mut preview_of: Option<String> = None;
    let shown = app.cmd.data.table.clone();
    let shown_col = app.cmd.data.column.clone();
    let stat = app.cmd.data.stat.is_some();
    egui::ScrollArea::vertical()
        .id_salt("bicraft-data-pane")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(4.0);
            ui.label(RichText::new(l.t("Tables", "Tables")).small().color(t.text_dim));
            for (name, hidden, cols) in &list {
                let hit = q.is_empty()
                    || name.to_lowercase().contains(&q)
                    || cols.iter().any(|c| c.0.to_lowercase().contains(&q));
                if !hit {
                    continue;
                }
                let on = shown.as_deref() == Some(name.as_str()) && !stat;
                let r = row(ui, Icon::Table, name, on, *hidden, 0.0, &t);
                r.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, on, name));
                if r.clicked() {
                    pick_table = Some(name.clone());
                }
                crate::keyboard::context_menu(&r, |ui| {
                    if ui.button(l.t("Preview query", "Aperçu de la requête")).clicked() {
                        preview_of = Some(name.clone());
                        ui.close();
                    }
                });
                if on {
                    for (c, calc) in cols {
                        if !q.is_empty()
                            && !c.to_lowercase().contains(&q)
                            && !name.to_lowercase().contains(&q)
                        {
                            continue;
                        }
                        let con = shown_col.as_deref() == Some(c.as_str());
                        let icon = if *calc { Icon::Calculator } else { Icon::Column };
                        let r = row(ui, icon, c, con, false, 16.0, &t);
                        let label = format!("{} {name}[{c}]", l.t("Column", "Colonne"));
                        r.widget_info(|| {
                            egui::WidgetInfo::selected(egui::WidgetType::Button, true, con, &label)
                        });
                        if r.clicked() {
                            pick_column = Some((name.clone(), c.clone()));
                        }
                    }
                }
            }
            if !qs.is_empty() {
                ui.add_space(8.0);
                ui.label(
                    RichText::new(l.t("Queries", "Requêtes"))
                        .small()
                        .color(t.text_dim),
                );
                for name in &qs {
                    if !q.is_empty() && !name.to_lowercase().contains(&q) {
                        continue;
                    }
                    let r = row(ui, Icon::List, name, false, false, 0.0, &t);
                    let label = format!("{} {name}", l.t("Preview query", "Aperçu de la requête"));
                    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
                    if r.on_hover_text(l.t(
                        "Preview the query's first rows",
                        "Afficher les premières lignes de la requête",
                    ))
                    .clicked()
                    {
                        preview_of = Some(name.clone());
                    }
                }
            }
        });
    if let Some(name) = pick_table {
        open_table(app, &name);
    }
    if let Some((table, column)) = pick_column {
        open_table(app, &table);
        select_column(app, &ctx, &column);
    }
    if let Some(name) = preview_of {
        if ribbon::state(app, "query.preview").is_ok() {
            preview(app, json!({"name": name}));
        } else if let Err(why) = ribbon::state(app, "query.preview") {
            app.status = why;
        }
    }
}

/// A row of the Data pane: an icon and a name.
fn row(ui: &mut Ui, icon: Icon, name: &str, on: bool, dim: bool, indent: f32, t: &Tokens) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
    if on {
        ui.painter().rect_filled(r, CornerRadius::same(4), t.selected);
    } else if resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(4), t.hover);
    }
    let ir = Rect::from_center_size(pos2(r.min.x + indent + 10.0, r.center().y), vec2(14.0, 14.0));
    icons::paint(ui.painter(), ir, icon, t.text_dim, false);
    let col = if dim { t.text_faint } else { t.text };
    let tr = Rect::from_min_max(pos2(ir.max.x + 6.0, r.min.y), r.max);
    let g = ui
        .painter()
        .layout_no_wrap(clip(name, 60), theme::font(12.5), col);
    ui.painter()
        .with_clip_rect(tr)
        .galley(pos2(tr.min.x, r.center().y - g.size().y / 2.0), g, col);
    resp
}

/// The title row over the grid and the grid.
fn grid_ui(app: &mut App, ui: &mut Ui) {
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    let mut close = false;
    let mut clear = false;
    let mut copy = false;
    ui.horizontal(|ui| {
        let dv = &app.cmd.data;
        let title = match (&dv.stat, &dv.stat_pending, &dv.table) {
            (Some(s), _, _) => s.title.clone(),
            (None, Some(p), _) => p.clone(),
            (None, None, Some(t)) => t.clone(),
            _ => l.t("Data", "Données").to_string(),
        };
        ui.label(
            RichText::new(clip(&title, 80))
                .font(theme::bold(15.0))
                .color(t.text),
        );
        if let Some(n) = dv.rows() {
            ui.label(RichText::new(rows_text(n, l)).small().color(t.text_dim));
        }
        if dv.stat.is_some() || dv.stat_pending.is_some() {
            if ui
                .button(l.t("Close", "Fermer"))
                .on_hover_text(l.t("Back to the table", "Retour à la table"))
                .clicked()
            {
                close = true;
            }
        } else if (!dv.filters.is_empty() || dv.sort.is_some())
            && ui
                .button(l.t("Clear sort and filters", "Effacer le tri et les filtres"))
                .clicked()
        {
            clear = true;
        }
        if dv.sel.is_some() && ui.button(l.t("Copy", "Copier")).on_hover_text("Ctrl+C").clicked() {
            copy = true;
        }
    });
    if let Some(s) = &app.cmd.data.stat
        && !s.note.is_empty()
    {
        ui.label(RichText::new(clip(&s.note, 300)).small().color(t.text_dim));
    }
    if close {
        close_static(app);
    }
    if clear {
        let dv = &mut app.cmd.data;
        dv.sort = None;
        dv.filters.clear();
        dv.reset();
    }
    if copy {
        copy_selection(app, ui.ctx());
    }
    if let Some(e) = app.cmd.data.error.clone()
        && widgets::error_bar(ui, &e, l.t("Close", "Fermer"))
    {
        app.cmd.data.error = None;
    }
    if app.cmd.data.stat_pending.is_some() {
        ui.spinner();
        return;
    }
    if let Some(r) = app.cmd.data.reason.clone()
        && app.cmd.data.stat.is_none()
    {
        ui.label(
            RichText::new(format!(
                "{} {}",
                l.t("No data to show:", "Aucune donnée à afficher :"),
                clip(&r, 400)
            ))
            .color(t.text_dim),
        );
    }
    if app.cmd.data.table.is_none() && app.cmd.data.stat.is_none() {
        // the first table is picked by the next `ensure`
        ensure(app, 0..1);
        if app.cmd.data.table.is_none() {
            crate::app::empty(
                ui,
                l.t("No table", "Aucune table"),
                l.t(
                    "The model has no table to show.",
                    "Le modèle n'a aucune table à afficher.",
                ),
            );
            return;
        }
    }
    let acts = grid(app, ui, &t, l);
    let ctx = ui.ctx().clone();
    for a in acts {
        match a {
            Act::SelectColumn(c) => {
                if let Some(name) = app.cmd.data.cols().get(c).map(|c| c.name.clone()) {
                    if app.cmd.data.stat.is_none() {
                        select_column(app, &ctx, &name);
                    }
                    let n = app.cmd.data.rows().unwrap_or(0);
                    app.cmd.data.sel = (n > 0).then_some(((0, c), (n - 1, c)));
                }
            }
            Act::Menu(c, at) => {
                if let Some(name) = app.cmd.data.cols().get(c).map(|c| c.name.clone()) {
                    open_menu(app, &name, at);
                }
            }
            Act::Cell(r, c, extend) => {
                let dv = &mut app.cmd.data;
                dv.sel = match (extend, dv.sel) {
                    (true, Some((a, _))) => Some((a, (r, c))),
                    _ => Some(((r, c), (r, c))),
                };
            }
            Act::Copy => copy_selection(app, &ctx),
        }
    }
    let visible = app.cmd.data.visible.clone();
    ensure(app, visible);
}

fn copy_selection(app: &mut App, ctx: &egui::Context) {
    let (text, missing) = app.cmd.data.tsv();
    if text.is_empty() {
        return;
    }
    ctx.copy_text(text);
    let l = app.lang;
    app.status = if missing > 0 {
        format!(
            "{} ({} {})",
            l.t("Copied", "Copié"),
            crate::answer::count(missing, l),
            l.t("rows not read yet left out", "lignes pas encore lues omises")
        )
    } else {
        l.t(
            "Copied as tab-separated text",
            "Copié en texte séparé par des tabulations",
        )
        .to_string()
    };
}

/// The grid: the header (type, name, sort and filter marks, the menu arrow), only the visible
/// rows drawn; returns what was clicked.
fn grid(app: &mut App, ui: &mut Ui, t: &Tokens, l: Lang) -> Vec<Act> {
    let mut acts = Vec::new();
    let dv = &mut app.cmd.data;
    let cols: Vec<Col> = dv.cols().to_vec();
    let n = dv.rows().unwrap_or(0);
    let stat = dv.stat.is_some();
    let widths: Vec<f32> = cols
        .iter()
        .map(|c| {
            dv.widths
                .get(&c.name)
                .copied()
                .unwrap_or_else(|| default_width(c))
        })
        .collect();
    let total: f32 = widths.iter().sum::<f32>().max(1.0);
    let salt = Id::new("bicraft-data-grid");
    let selected_col = dv.column.clone();
    let sort = dv.sort.clone();
    let filtered: BTreeSet<String> = dv.filters.keys().cloned().collect();
    let sel = dv.sel;
    let in_sel = |r: usize, c: usize| {
        sel.is_some_and(|((r0, c0), (r1, c1))| {
            (r0.min(r1)..=r0.max(r1)).contains(&r) && (c0.min(c1)..=c0.max(c1)).contains(&c)
        })
    };
    let mut visible = 0..0;
    let mut resized: Option<(String, f32)> = None;
    if cols.is_empty() {
        if dv.row_count.is_none() || !dv.pending.is_empty() {
            ui.spinner();
        }
        return acts;
    }
    let body_h = (ui.available_height() - HEAD_H).max(ROW_H * 3.0);
    egui::ScrollArea::horizontal()
        .id_salt(salt.with("h"))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
            ui.set_min_width(total);
            let (head, _) = ui.allocate_exact_size(vec2(total, HEAD_H), Sense::hover());
            ui.painter().rect_filled(head, CornerRadius::same(2), t.chrome);
            let mut x = head.min.x;
            for (c, col) in cols.iter().enumerate() {
                let w = widths.get(c).copied().unwrap_or(100.0);
                let r = Rect::from_min_size(pos2(x, head.min.y), vec2(w, HEAD_H));
                let arrow = Rect::from_min_max(pos2(r.max.x - 20.0, r.min.y), r.max);
                let name_r = Rect::from_min_max(r.min, pos2(arrow.min.x, r.max.y));
                let on = selected_col.as_deref() == Some(col.name.as_str()) && !stat;
                let resp = ui.interact(name_r, salt.with(("head", c)), Sense::click());
                let sorted = sort.as_ref().filter(|(s, _)| *s == col.name).map(|(_, d)| !*d);
                let filt = filtered.contains(&col.name);
                let label = col.name.clone();
                resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, on, &label));
                if on {
                    ui.painter().rect_filled(r, CornerRadius::ZERO, t.selected);
                } else if resp.hovered() {
                    ui.painter().rect_filled(name_r, CornerRadius::ZERO, t.hover);
                }
                let mut tip = format!("{}\n{}", col.name, type_name(col, l));
                if col.calculated {
                    tip.push_str(&format!("\n{}", l.t("Calculated column", "Colonne calculée")));
                }
                if let Some(why) = &col.reason {
                    tip.push_str(&format!("\n{}", clip(why, 200)));
                }
                if resp.on_hover_text(tip).clicked() {
                    acts.push(Act::SelectColumn(c));
                }
                // the type: a calculated column's calculator, the calendar, or a short tag
                let tag_r = Rect::from_min_size(pos2(r.min.x + 5.0, r.center().y - 8.0), vec2(22.0, 16.0));
                if col.calculated {
                    icons::paint(
                        ui.painter(),
                        tag_r.shrink2(vec2(3.0, 0.0)),
                        Icon::Calculator,
                        t.text_dim,
                        false,
                    );
                } else if let Some(tag) = type_tag(col) {
                    ui.painter().text(
                        pos2(tag_r.min.x, tag_r.center().y),
                        Align2::LEFT_CENTER,
                        tag,
                        theme::font(10.5),
                        t.text_dim,
                    );
                } else {
                    icons::paint(
                        ui.painter(),
                        tag_r.shrink2(vec2(3.0, 0.0)),
                        Icon::Calendar,
                        t.text_dim,
                        false,
                    );
                }
                let tr =
                    Rect::from_min_max(pos2(tag_r.max.x + 2.0, r.min.y), pos2(arrow.min.x - 2.0, r.max.y));
                let color = if col.available { t.text } else { t.text_faint };
                let g = ui
                    .painter()
                    .layout_no_wrap(clip(&col.name, 60), theme::bold(12.0), color);
                ui.painter().with_clip_rect(tr).galley(
                    pos2(tr.min.x, r.center().y - g.size().y / 2.0),
                    g,
                    color,
                );
                // the menu's arrow, with the sort and the filter marks
                if !stat {
                    let ar = ui.interact(arrow, salt.with(("menu", c)), Sense::click());
                    let mname = format!("{} {}", l.t("Sort and filter", "Trier et filtrer"), col.name);
                    ar.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &mname));
                    if ar.hovered() {
                        ui.painter().rect_filled(arrow, CornerRadius::same(3), t.hover);
                    }
                    // full-strength text when sorted or filtered (`accent_text` is for an accent
                    // fill and vanished on the dark header)
                    let ink = if filt || sorted.is_some() {
                        t.text
                    } else {
                        t.text_dim
                    };
                    match sorted {
                        Some(up) => triangle(ui, arrow.center(), up, ink),
                        None if filt => {
                            icons::paint(ui.painter(), arrow.shrink(5.0), Icon::Filter, ink, false);
                        }
                        None => widgets::chevron(ui.painter(), arrow.center(), 3.5, ink),
                    }
                    if ar.clicked() {
                        acts.push(Act::Menu(c, pos2(arrow.min.x, arrow.max.y)));
                    }
                }
                // a drag on the right edge resizes the column
                let edge = Rect::from_min_max(pos2(r.max.x - 3.0, r.min.y), pos2(r.max.x + 3.0, r.max.y));
                let er = ui.interact(edge, salt.with(("edge", c)), Sense::drag());
                if er.hovered() || er.dragged() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeColumn);
                }
                if er.dragged() {
                    let nw = (w + er.drag_delta().x).clamp(48.0, 800.0);
                    resized = Some((col.name.clone(), nw));
                }
                x += w;
                ui.painter()
                    .vline(x - 0.5, head.y_range(), Stroke::new(1.0, t.separator));
            }
            // the rows sit on the report surface, as Desktop's white grid
            let body_w = total.max(ui.clip_rect().max.x - head.min.x);
            let body = Rect::from_min_size(head.left_bottom(), vec2(body_w, body_h));
            ui.painter().rect_filled(body, CornerRadius::ZERO, t.card);
            let line = Stroke::new(1.0, t.separator);
            egui::ScrollArea::vertical()
                .id_salt(salt.with("v"))
                .max_height(body_h)
                .auto_shrink([false, false])
                .show_rows(ui, ROW_H, n, |ui, range| {
                    ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                    visible = range.clone();
                    for r in range {
                        let (row_r, resp) = ui.allocate_exact_size(vec2(total, ROW_H), Sense::click());
                        let col_at = |px: f32| -> usize {
                            let mut x = row_r.min.x;
                            for (i, w) in widths.iter().enumerate() {
                                x += w;
                                if px < x {
                                    return i;
                                }
                            }
                            widths.len().saturating_sub(1)
                        };
                        if resp.clicked()
                            && let Some(p) = resp.interact_pointer_pos()
                        {
                            let shift = ui.input(|i| i.modifiers.shift);
                            acts.push(Act::Cell(r, col_at(p.x), shift));
                        }
                        if resp.secondary_clicked()
                            && let Some(p) = resp.interact_pointer_pos()
                            && !in_sel(r, col_at(p.x))
                        {
                            acts.push(Act::Cell(r, col_at(p.x), false));
                        }
                        crate::keyboard::context_menu(&resp, |ui| {
                            if ui.button(l.t("Copy", "Copier")).clicked() {
                                acts.push(Act::Copy);
                                ui.close();
                            }
                        });
                        let mut x = row_r.min.x;
                        for (c, w) in widths.iter().enumerate() {
                            let cr = Rect::from_min_size(pos2(x, row_r.min.y), vec2(*w, ROW_H));
                            x += w;
                            if in_sel(r, c) {
                                ui.painter().rect_filled(cr, CornerRadius::ZERO, t.accent_soft);
                            }
                            let v = app.cmd.data.cell(r, c);
                            let Some(v) = v else {
                                if c == 0 {
                                    ui.painter().text(
                                        pos2(cr.min.x + 6.0, cr.center().y),
                                        Align2::LEFT_CENTER,
                                        "…",
                                        theme::font(12.0),
                                        t.text_faint,
                                    );
                                }
                                continue;
                            };
                            let s = text(v);
                            if s.is_empty() {
                                continue;
                            }
                            let num = v.is_number();
                            let color = t.text;
                            let inner = cr.shrink2(vec2(6.0, 0.0));
                            let g = ui.painter().layout_no_wrap(s, theme::font(12.0), color);
                            let at = if num {
                                pos2(inner.max.x - g.size().x, cr.center().y - g.size().y / 2.0)
                            } else {
                                pos2(inner.min.x, cr.center().y - g.size().y / 2.0)
                            };
                            ui.painter().with_clip_rect(inner).galley(at, g, color);
                        }
                        // the row's and the columns' lines
                        ui.painter().hline(row_r.x_range(), row_r.max.y - 0.5, line);
                        let mut x = row_r.min.x;
                        for w in &widths {
                            x += w;
                            ui.painter().vline(x - 0.5, row_r.y_range(), line);
                        }
                        // the cells of this row for the accessibility tree: the first column's text
                        let first = app.cmd.data.cell(r, 0).map(text).unwrap_or_default();
                        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, &first));
                    }
                });
        });
    // Ctrl+C on the grid (no text being typed)
    let copy = ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy)));
    if copy && !ui.ctx().egui_wants_keyboard_input() && app.cmd.data.sel.is_some() {
        acts.push(Act::Copy);
    }
    let dv = &mut app.cmd.data;
    if let Some((name, w)) = resized {
        dv.widths.insert(name, w);
    }
    dv.visible = visible;
    acts
}

/// The sort mark: a small triangle pointing up (ascending) or down.
fn triangle(ui: &Ui, c: egui::Pos2, up: bool, color: egui::Color32) {
    let (a, b) = if up { (-3.0, 3.0) } else { (3.0, -3.0) };
    ui.painter().add(egui::Shape::convex_polygon(
        vec![
            pos2(c.x - 4.0, c.y + b),
            pos2(c.x + 4.0, c.y + b),
            pos2(c.x, c.y + a),
        ],
        color,
        Stroke::NONE,
    ));
}

/// Opens the sort and filter menu of `column` (and asks for its values).
pub fn open_menu(app: &mut App, column: &str, at: egui::Pos2) {
    let Some(table) = app.cmd.data.table.clone() else {
        return;
    };
    let checked = app
        .cmd
        .data
        .filters
        .get(column)
        .and_then(|f| match f {
            crate::data_filter::ColFilter::Values(v) => Some(v.iter().map(|x| x.to_string()).collect()),
            crate::data_filter::ColFilter::Custom { .. } => None,
        })
        .unwrap_or_default();
    app.cmd.data.menu = Some(Menu {
        column: column.to_string(),
        at,
        checked,
        search: String::new(),
        fresh: true,
    });
    let generation = app.cmd.data.generation;
    let fresh = app
        .cmd
        .data
        .values
        .as_ref()
        .is_some_and(|(g, c, _, _)| *g == generation && c == column);
    if !fresh {
        app.cmd.data.values = None;
        app.run(
            Call {
                id: "column.values",
                params: json!({"table": table, "column": column, "limit": MAX_VALUES}),
            },
            Then::Data(Step::Values {
                generation,
                column: column.to_string(),
            }),
        );
    }
}

/// Sorts the view on `column` (`None`: no sort).
pub fn set_sort(app: &mut App, sort: Option<(String, bool)>) {
    let dv = &mut app.cmd.data;
    if dv.sort != sort {
        dv.sort = sort;
        dv.reset();
    }
}

/// Filters the view on `column` (`None`: no filter on it).
pub fn set_filter(app: &mut App, column: &str, values: Option<crate::data_filter::ColFilter>) {
    let dv = &mut app.cmd.data;
    let before = dv.filters.get(column).cloned();
    match values {
        Some(v) => {
            dv.filters.insert(column.to_string(), v);
        }
        None => {
            dv.filters.remove(column);
        }
    }
    if dv.filters.get(column) != before.as_ref() {
        dv.reset();
    }
}

/// The column's menu: sort, then the values to keep.
fn menu_ui(app: &mut App, ctx: &egui::Context) {
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    let Some(mut m) = app.cmd.data.menu.take() else {
        return;
    };
    let values = app
        .cmd
        .data
        .values
        .as_ref()
        .filter(|(_, c, _, _)| *c == m.column)
        .map(|(_, _, v, cut)| (v.clone(), *cut));
    let mut close = false;
    let mut sort: Option<Option<(String, bool)>> = None;
    let mut apply: Option<Option<crate::data_filter::ColFilter>> = None;
    let mut custom: Option<crate::data_filter::Custom> = None;
    let col = app.cmd.data.columns.iter().find(|c| c.name == m.column).cloned();
    let current = app.cmd.data.filters.get(&m.column).cloned();
    let resp = egui::Area::new(Id::new("bicraft-data-menu"))
        .fixed_pos(m.at)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_width(240.0);
                ui.label(
                    RichText::new(clip(&m.column, 60))
                        .font(theme::bold(13.0))
                        .color(t.text),
                );
                if ui.button(l.t("Sort ascending", "Tri croissant")).clicked() {
                    sort = Some(Some((m.column.clone(), false)));
                }
                if ui.button(l.t("Sort descending", "Tri décroissant")).clicked() {
                    sort = Some(Some((m.column.clone(), true)));
                }
                if ui.button(l.t("Clear sort", "Effacer le tri")).clicked() {
                    sort = Some(None);
                }
                ui.separator();
                if ui.button(l.t("Clear filter", "Effacer le filtre")).clicked() {
                    apply = Some(None);
                }
                if let Some(c) = &col {
                    custom = crate::data_filter::menu_section(ui, c, current.as_ref(), l);
                }
                let r = ui.add(
                    egui::TextEdit::singleline(&mut m.search)
                        .hint_text(l.t("Search", "Rechercher"))
                        .desired_width(f32::INFINITY),
                );
                r.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::TextEdit,
                        true,
                        l.t("Search the values", "Rechercher les valeurs"),
                    )
                });
                match &values {
                    None => {
                        ui.spinner();
                    }
                    Some((Err(e), _)) => {
                        ui.label(RichText::new(clip(e, 200)).color(t.err_text));
                    }
                    Some((Ok(vals), cut)) => {
                        let q = m.search.trim().to_lowercase();
                        let all =
                            m.checked.is_empty() || vals.iter().all(|v| m.checked.contains(&v.to_string()));
                        let mut all_box = all;
                        if ui
                            .checkbox(&mut all_box, l.t("(Select all)", "(Tout sélectionner)"))
                            .changed()
                        {
                            m.checked.clear();
                            if !all_box {
                                // nothing kept: a value that is never there
                                m.checked.insert("__none__".into());
                            }
                        }
                        egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                            for v in vals {
                                let s = text(v);
                                let shown = if v.is_null() {
                                    l.t("(Blank)", "(Vide)").to_string()
                                } else {
                                    s
                                };
                                if !q.is_empty() && !shown.to_lowercase().contains(&q) {
                                    continue;
                                }
                                let key = v.to_string();
                                let mut on = all || m.checked.contains(&key);
                                if ui.checkbox(&mut on, clip(&shown, 60)).changed() {
                                    if all {
                                        // the first box unticked: every other value kept
                                        m.checked = vals.iter().map(Value::to_string).collect();
                                    }
                                    m.checked.remove("__none__");
                                    if on {
                                        m.checked.insert(key);
                                    } else {
                                        m.checked.remove(&key);
                                    }
                                }
                            }
                        });
                        if *cut {
                            ui.label(
                                RichText::new(l.t(
                                    "More values: search to find them",
                                    "D'autres valeurs : recherchez-les",
                                ))
                                .small()
                                .color(t.text_dim),
                            );
                        }
                        ui.horizontal(|ui| {
                            if ui.button(l.t("OK", "OK")).clicked() {
                                let every = m.checked.is_empty()
                                    || vals.iter().all(|v| m.checked.contains(&v.to_string()));
                                apply = Some(if every {
                                    None
                                } else {
                                    Some(crate::data_filter::ColFilter::Values(
                                        vals.iter()
                                            .filter(|v| m.checked.contains(&v.to_string()))
                                            .cloned()
                                            .collect(),
                                    ))
                                });
                            }
                            if ui.button(l.t("Cancel", "Annuler")).clicked() {
                                close = true;
                            }
                        });
                    }
                }
            });
        });
    let clicked_out = ctx.input(|i| i.pointer.any_pressed())
        && !m.fresh
        && ctx
            .pointer_interact_pos()
            .is_some_and(|p| !resp.response.rect.contains(p));
    m.fresh = false;
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) || clicked_out {
        close = true;
    }
    let column = m.column.clone();
    if let Some(s) = sort {
        set_sort(app, s);
        close = true;
    }
    if let Some(v) = apply {
        set_filter(app, &column, v);
        close = true;
    }
    if let Some(c) = custom {
        app.cmd.data.custom = Some(c);
        close = true;
    }
    if !close {
        app.cmd.data.menu = Some(m);
    }
}

// --- Column tools ---------------------------------------------------------------------------

const DATA_TYPES: &[(&str, &str, &str)] = &[
    ("string", "Text", "Texte"),
    ("int64", "Whole number", "Nombre entier"),
    ("double", "Decimal number", "Nombre décimal"),
    ("decimal", "Fixed decimal number", "Nombre décimal fixe"),
    ("dateTime", "Date/time", "Date/heure"),
    ("boolean", "True/false", "Vrai/faux"),
];

const FORMATS: &[(&str, &str, &str)] = &[
    ("", "General", "Général"),
    ("0", "Whole number (0)", "Nombre entier (0)"),
    ("#,0", "Thousands (#,0)", "Milliers (#,0)"),
    ("#,0.00", "Two decimals (#,0.00)", "Deux décimales (#,0.00)"),
    ("0.00%", "Percentage (0.00%)", "Pourcentage (0.00%)"),
    ("\\$#,0.00;(\\$#,0.00);\\$#,0.00", "Currency ($)", "Monétaire ($)"),
    ("yyyy-mm-dd", "Date (yyyy-mm-dd)", "Date (aaaa-mm-jj)"),
    ("dd/mm/yyyy", "Date (dd/mm/yyyy)", "Date (jj/mm/aaaa)"),
    ("General Date", "Date and time", "Date et heure"),
];

const SUMMARIZE: &[(&str, &str, &str)] = &[
    ("none", "Don't summarize", "Ne pas résumer"),
    ("sum", "Sum", "Somme"),
    ("average", "Average", "Moyenne"),
    ("min", "Minimum", "Minimum"),
    ("max", "Maximum", "Maximum"),
    ("count", "Count", "Nombre"),
    ("distinctCount", "Count (distinct)", "Nombre (distinct)"),
];

const CATEGORIES: &[(&str, &str, &str)] = &[
    ("", "Uncategorized", "Non catégorisé"),
    ("Address", "Address", "Adresse"),
    ("City", "City", "Ville"),
    ("Continent", "Continent", "Continent"),
    ("Country", "Country/Region", "Pays/Région"),
    ("County", "County", "Comté"),
    ("Latitude", "Latitude", "Latitude"),
    ("Longitude", "Longitude", "Longitude"),
    ("Place", "Place", "Lieu"),
    ("PostalCode", "Postal code", "Code postal"),
    ("StateOrProvince", "State or province", "État ou province"),
    ("WebUrl", "Web URL", "URL web"),
    ("ImageUrl", "Image URL", "URL d'image"),
    ("Barcode", "Barcode", "Code-barres"),
];

/// The label of `code` in `list` (the code itself when it is not listed).
fn label_of<'a>(list: &[(&str, &'a str, &'a str)], code: &'a str, l: Lang) -> &'a str {
    list.iter()
        .find(|(c, _, _)| *c == code)
        .map_or(code, |(_, en, fr)| match l {
            Lang::En => en,
            Lang::Fr => fr,
        })
}

/// A picker of the Column tools: its label, the current choice, the choices; returns the code
/// picked.
fn picker(
    ui: &mut Ui,
    id: &str,
    label: &str,
    current: &str,
    list: &[(&str, &str, &str)],
    enabled: bool,
    l: Lang,
) -> Option<String> {
    let mut picked = None;
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).font(theme::font(11.5)));
        ui.add_enabled_ui(enabled, |ui| {
            let r = egui::ComboBox::from_id_salt(("bicraft-col-tools", id))
                .selected_text(label_of(list, current, l))
                .width(150.0)
                .show_ui(ui, |ui| {
                    for (code, en, fr) in list {
                        let text = match l {
                            Lang::En => *en,
                            Lang::Fr => *fr,
                        };
                        if ui.selectable_label(*code == current, text).clicked() && *code != current {
                            picked = Some((*code).to_string());
                        }
                    }
                });
            r.response
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, enabled, label));
        });
    });
    picked
}

/// The selected column's tools on the Table tab (Desktop's Column tools): name, data type,
/// format, summarization, data category, sort by column (each a `column.set`), New column and
/// New measure (the formula bar).
pub fn column_tools(app: &mut App, ui: &mut Ui) {
    let l = app.lang;
    let (Some(table), Some(column)) = (app.cmd.data.table.clone(), app.cmd.data.column.clone()) else {
        return;
    };
    let Some(col) = app.cmd.data.columns.iter().find(|c| c.name == column).cloned() else {
        return;
    };
    let can = ribbon::state(app, "column.set");
    let enabled = can.is_ok();
    let mut set: Option<(&'static str, Value)> = None;
    widgets::group(ui, |ui| {
        widgets::rows(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(l.t("Name", "Nom")).font(theme::font(11.5)));
                let edit = app
                    .cmd
                    .data
                    .name_edit
                    .get_or_insert_with(|| (column.clone(), column.clone()));
                if edit.0 != column {
                    *edit = (column.clone(), column.clone());
                }
                let r = ui.add_enabled(
                    enabled,
                    egui::TextEdit::singleline(&mut edit.1).desired_width(150.0),
                );
                r.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::TextEdit,
                        enabled,
                        l.t("Column name", "Nom de la colonne"),
                    )
                });
                let done = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let new = edit.1.trim().to_string();
                if done && !new.is_empty() && new != column {
                    set = Some(("new_name", json!(new)));
                }
            });
            let dt = picker(
                ui,
                "type",
                l.t("Data type", "Type de données"),
                &col.data_type,
                DATA_TYPES,
                enabled,
                l,
            );
            if let Some(d) = dt {
                set = Some(("data_type", json!(d)));
            }
        });
    });
    widgets::group(ui, |ui| {
        widgets::rows(ui, |ui| {
            let fmt = col.format_string.clone().unwrap_or_default();
            if let Some(f) = picker(ui, "format", l.t("Format", "Format"), &fmt, FORMATS, enabled, l) {
                set = Some(("format_string", if f.is_empty() { Value::Null } else { json!(f) }));
            }
            let sum = col.summarize_by.clone().unwrap_or_else(|| "none".into());
            if let Some(s) = picker(
                ui,
                "summarize",
                l.t("Summarization", "Synthèse"),
                &sum,
                SUMMARIZE,
                enabled,
                l,
            ) {
                set = Some(("summarize_by", json!(s)));
            }
        });
    });
    widgets::group(ui, |ui| {
        widgets::rows(ui, |ui| {
            let cat = col.data_category.clone().unwrap_or_default();
            if let Some(c) = picker(
                ui,
                "category",
                l.t("Data category", "Catégorie de données"),
                &cat,
                CATEGORIES,
                enabled,
                l,
            ) {
                set = Some(("data_category", if c.is_empty() { Value::Null } else { json!(c) }));
            }
            // sort by column: the column itself or another of the table
            let others: Vec<(String, String)> = std::iter::once((String::new(), String::new()))
                .chain(
                    app.cmd
                        .data
                        .columns
                        .iter()
                        .filter(|c| c.name != column)
                        .map(|c| (c.name.clone(), c.name.clone())),
                )
                .collect();
            let own = l.t("(itself)", "(elle-même)");
            let list: Vec<(&str, &str, &str)> = others
                .iter()
                .map(|(a, b)| {
                    if a.is_empty() {
                        ("", own, own)
                    } else {
                        (a.as_str(), b.as_str(), b.as_str())
                    }
                })
                .collect();
            let cur = col.sort_by_column.clone().unwrap_or_default();
            if let Some(s) = picker(
                ui,
                "sort-by",
                l.t("Sort by column", "Trier par colonne"),
                &cur,
                &list,
                enabled,
                l,
            ) {
                set = Some((
                    "sort_by_column",
                    if s.is_empty() { Value::Null } else { json!(s) },
                ));
            }
        });
    });
    widgets::group(ui, |ui| {
        let add_m = ribbon::state(app, "measure.add");
        if widgets::big_button(
            ui,
            Icon::Measure,
            l.t("New measure", "Nouvelle mesure"),
            add_m.is_ok(),
        )
        .on_hover_text(l.t(
            "A measure, written in the formula bar",
            "Une mesure, écrite dans la barre de formule",
        ))
        .clicked()
        {
            crate::formula::new_measure(app, Some(table.clone()));
        }
    });
    if let Some((key, value)) = set {
        let mut p = json!({"table": table, "name": column});
        p[key] = value;
        if key == "new_name" {
            p["update_report"] = json!(true);
        }
        let renamed = (key == "new_name").then(|| p["new_name"].as_str().unwrap_or_default().to_string());
        if app.run(
            Call {
                id: "column.set",
                params: p,
            },
            Then::Nothing,
        ) && let Some(n) = renamed
        {
            app.cmd.data.column = Some(n.clone());
            app.cmd.data.name_edit = Some((n.clone(), n));
        }
    }
    if let Err(why) = can {
        ui.label(RichText::new(why).small());
    }
}

/// Where the hit map says a right click on the page landed: the data point's `select` action
/// point and, for a mark that stands for several (a histogram bin, a Venn set), all its `points`,
/// kept for See records.
pub fn point_at(app: &App, pos: egui::Pos2) -> Option<(Value, Option<Value>)> {
    let (_, Ok(img)) = app.shown.as_ref()? else {
        return None;
    };
    let placed = app.placed?;
    let (x, y) = (
        (pos.x - placed.rect.min.x) / placed.zoom,
        (pos.y - placed.rect.min.y) / placed.zoom,
    );
    let hit = bicraft_render::hits::hit_at(&img.hits, x, y)?;
    let a = hit.action_for(false)?;
    if a.get("action").and_then(Value::as_str) != Some("select") {
        return None;
    }
    let point = a.get("point").cloned().filter(|p| !p.is_null())?;
    Some((point, a.get("points").cloned().filter(Value::is_array)))
}

/// `1 row`, `3,000 rows` in the chrome's language.
pub fn rows_text(n: usize, l: Lang) -> String {
    let word = if n == 1 {
        l.t("row", "ligne")
    } else {
        l.t("rows", "lignes")
    };
    format!("{} {word}", crate::answer::count(n, l))
}
