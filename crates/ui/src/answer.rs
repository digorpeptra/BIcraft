//! A command's answer drawn for reading (docs/webui.md, "Command answers"): lists of records as
//! a grid (sortable columns, copy as TSV, only the visible rows drawn, so 100 000 rows scroll as
//! fast as 100), objects as a key/value tree, and a JSON toggle that shows the raw answer. The
//! parameters form's inline answer and the ribbon's answer window both use it, in both shells.
//!
//! The answer is read once into an [`Answer`] (cells as text, numbers kept for sorting); each
//! frame only draws it.

use egui::{CornerRadius, FontId, Label, Rect, RichText, Sense, Ui, Vec2, pos2, vec2};
use serde_json::{Map, Value};

use crate::i18n::Lang;
use crate::theme::{self, Tokens};

/// Most rows a grid keeps (the JSON view and its copy keep everything).
pub const MAX_ROWS: usize = 200_000;
/// Most columns a grid shows.
const MAX_COLS: usize = 64;
/// Most entries an object or a list shows.
const MAX_ENTRIES: usize = 2_000;
/// Longest text of one cell or value.
const MAX_CELL: usize = 2_000;
/// Deepest nesting read; deeper values are shown as compact JSON.
const MAX_DEPTH: usize = 24;
/// Most characters of the JSON view drawn (Copy JSON copies all of it).
const MAX_JSON: usize = 200_000;
/// Height of a grid row.
pub const ROW_H: f32 = 22.0;
/// A list of scalars at most this long is written on one line.
const INLINE_LIST: usize = 12;

/// One cell of a grid.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub text: String,
    /// Its number, for sorting.
    pub num: Option<f64>,
    /// The cell was `null`.
    pub null: bool,
}

/// A list of records as a grid.
#[derive(Debug, Clone)]
pub struct Grid {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Cell>>,
    /// Rows the answer had beyond [`MAX_ROWS`].
    pub rows_left_out: usize,
    /// Columns beyond [`MAX_COLS`].
    pub columns_left_out: usize,
    /// The column sorted on and whether ascending.
    pub sort: Option<(usize, bool)>,
    /// The rows in display order (indexes into `rows`).
    pub order: Vec<usize>,
    widths: Option<Vec<f32>>,
}

impl Grid {
    fn new(
        columns: Vec<String>,
        rows: Vec<Vec<Cell>>,
        rows_left_out: usize,
        columns_left_out: usize,
    ) -> Self {
        let order = (0..rows.len()).collect();
        Grid {
            columns,
            rows,
            rows_left_out,
            columns_left_out,
            sort: None,
            order,
            widths: None,
        }
    }

    /// Sorts on column `col`: ascending the first time, then descending, then back to the
    /// answer's own order.
    pub fn click(&mut self, col: usize) {
        self.sort = match self.sort {
            Some((c, true)) if c == col => Some((col, false)),
            Some((c, false)) if c == col => None,
            _ => Some((col, true)),
        };
        self.apply_sort();
    }

    fn apply_sort(&mut self) {
        let mut order: Vec<usize> = (0..self.rows.len()).collect();
        if let Some((col, asc)) = self.sort {
            let rows = &self.rows;
            order.sort_by(|a, b| {
                let ca = rows.get(*a).and_then(|r| r.get(col));
                let cb = rows.get(*b).and_then(|r| r.get(col));
                let null = |c: Option<&Cell>| c.is_none_or(|c| c.null);
                match (null(ca), null(cb)) {
                    // empty cells last either way
                    (true, true) => std::cmp::Ordering::Equal,
                    (true, false) => std::cmp::Ordering::Greater,
                    (false, true) => std::cmp::Ordering::Less,
                    _ if asc => compare(ca, cb),
                    _ => compare(ca, cb).reverse(),
                }
            });
        }
        self.order = order;
    }

    /// The grid as tab-separated text in display order, its header first (tabs and line breaks
    /// inside a cell become spaces).
    pub fn tsv(&self) -> String {
        let clean = |s: &str| s.replace(['\t', '\r', '\n'], " ");
        let mut out = self
            .columns
            .iter()
            .map(|c| clean(c))
            .collect::<Vec<_>>()
            .join("\t");
        for i in &self.order {
            let Some(row) = self.rows.get(*i) else { continue };
            out.push('\n');
            let cells: Vec<String> = row.iter().map(|c| clean(&c.text)).collect();
            out.push_str(&cells.join("\t"));
        }
        out
    }

    /// The text of the cell at display row `r`, column `c`.
    pub fn cell(&self, r: usize, c: usize) -> Option<&str> {
        let i = *self.order.get(r)?;
        self.rows.get(i)?.get(c).map(|x| x.text.as_str())
    }
}

/// Nulls last, numbers before text, numbers by value, text without regard to case.
fn compare(a: Option<&Cell>, b: Option<&Cell>) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;
    match (a, b) {
        (None, None) => Equal,
        (None, Some(_)) => Greater,
        (Some(_), None) => Less,
        (Some(a), Some(b)) => match (a.null, b.null) {
            (true, true) => Equal,
            (true, false) => Greater,
            (false, true) => Less,
            _ => match (a.num, b.num) {
                (Some(x), Some(y)) => x.total_cmp(&y),
                (Some(_), None) => Less,
                (None, Some(_)) => Greater,
                (None, None) => a
                    .text
                    .to_lowercase()
                    .cmp(&b.text.to_lowercase())
                    .then_with(|| a.text.cmp(&b.text)),
            },
        },
    }
}

/// How a scalar is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Text,
    Number,
    Flag,
    Null,
}

/// A part of the answer.
#[derive(Debug, Clone)]
pub enum Node {
    Value(String, Tone),
    /// A grid (index into [`Answer::grids`]).
    Grid(usize),
    /// Keys and their values; the count of keys left out.
    Object(Vec<(String, Node)>, usize),
    /// Items of a list that is not a list of records; the count left out.
    List(Vec<Node>, usize),
}

/// A command's answer, read for display.
#[derive(Debug, Clone)]
pub struct Answer {
    pub root: Node,
    pub grids: Vec<Grid>,
    /// The JSON toggle.
    pub json: bool,
    json_text: Option<String>,
}

impl Answer {
    pub fn new(v: &Value) -> Self {
        let mut grids = Vec::new();
        let root = node(v, 0, &mut grids);
        Answer {
            root,
            grids,
            json: false,
            json_text: None,
        }
    }

    /// The first grid (the answer's main table, if it has one).
    pub fn grid(&self) -> Option<&Grid> {
        self.grids.first()
    }

    pub fn grid_mut(&mut self) -> Option<&mut Grid> {
        self.grids.first_mut()
    }

    /// Nothing to show beyond `✓ Done` (`null`, an empty object or string).
    pub fn is_empty(&self) -> bool {
        match &self.root {
            Node::Value(s, Tone::Null) => s.is_empty() || s == "null",
            Node::Value(s, _) => s.is_empty(),
            Node::Object(o, n) => o.is_empty() && *n == 0,
            _ => false,
        }
    }
}

fn scalar(v: &Value) -> Option<(String, Tone)> {
    Some(match v {
        Value::Null => ("null".to_string(), Tone::Null),
        Value::Bool(b) => (b.to_string(), Tone::Flag),
        Value::Number(n) => (n.to_string(), Tone::Number),
        Value::String(s) => (crate::app::clip(s, MAX_CELL), Tone::Text),
        _ => return None,
    })
}

fn compact(v: &Value) -> String {
    crate::app::clip(&serde_json::to_string(v).unwrap_or_default(), 200)
}

fn cell(v: Option<&Value>) -> Cell {
    match v {
        None => Cell {
            text: String::new(),
            num: None,
            null: true,
        },
        Some(Value::Null) => Cell {
            text: String::new(),
            num: None,
            null: true,
        },
        Some(Value::Number(n)) => Cell {
            text: n.to_string(),
            num: n.as_f64().filter(|x| x.is_finite()),
            null: false,
        },
        Some(Value::String(s)) => Cell {
            text: crate::app::clip(s, MAX_CELL),
            num: None,
            null: false,
        },
        Some(Value::Bool(b)) => Cell {
            text: b.to_string(),
            num: None,
            null: false,
        },
        Some(other) => Cell {
            text: compact(other),
            num: None,
            null: false,
        },
    }
}

/// A grid of records (objects): the columns in the order they first appear, or `names` first.
fn records(items: &[Value], names: &[String]) -> Grid {
    let mut columns: Vec<String> = names.iter().take(MAX_COLS).cloned().collect();
    let mut left_cols = names.len().saturating_sub(MAX_COLS);
    for it in items.iter().take(MAX_ROWS) {
        if let Some(o) = it.as_object() {
            for k in o.keys() {
                if !columns.iter().any(|c| c == k) {
                    if columns.len() < MAX_COLS {
                        columns.push(k.clone());
                    } else {
                        left_cols = left_cols.saturating_add(1);
                    }
                }
            }
        }
        // the column list stops growing after a few thousand rows (a uniform answer)
        if left_cols > 0 || columns.len() >= MAX_COLS {
            break;
        }
    }
    let rows: Vec<Vec<Cell>> = items
        .iter()
        .take(MAX_ROWS)
        .map(|it| match it {
            Value::Object(o) => columns.iter().map(|c| cell(o.get(c))).collect(),
            Value::Array(a) => (0..columns.len()).map(|i| cell(a.get(i))).collect(),
            other => {
                let mut r = vec![cell(Some(other))];
                r.resize(columns.len(), cell(None));
                r
            }
        })
        .collect();
    Grid::new(columns, rows, items.len().saturating_sub(MAX_ROWS), left_cols)
}

/// The column names of a `columns` list: strings, or objects with a `name`.
fn column_names(v: Option<&Value>) -> Option<Vec<String>> {
    let a = v?.as_array()?;
    a.iter()
        .map(|c| match c {
            Value::String(s) => Some(s.clone()),
            Value::Object(o) => o.get("name").and_then(Value::as_str).map(str::to_string),
            _ => None,
        })
        .collect()
}

fn node(v: &Value, depth: usize, grids: &mut Vec<Grid>) -> Node {
    if let Some((s, tone)) = scalar(v) {
        return Node::Value(s, tone);
    }
    if depth >= MAX_DEPTH {
        return Node::Value(compact(v), Tone::Text);
    }
    match v {
        Value::Array(a) if a.is_empty() => Node::Value("[]".into(), Tone::Null),
        // a list of tables (`dax.evaluate`'s results): each its own grid
        Value::Array(a) if a.iter().all(is_tabular) => Node::List(
            a.iter()
                .take(MAX_ENTRIES)
                .map(|x| node(x, depth + 1, grids))
                .collect(),
            a.len().saturating_sub(MAX_ENTRIES),
        ),
        Value::Array(a) if a.iter().all(|x| x.is_object() || x.is_null()) => {
            grids.push(records(a, &[]));
            Node::Grid(grids.len() - 1)
        }
        Value::Array(a) if a.iter().all(Value::is_array) => {
            let width = a
                .iter()
                .take(MAX_ROWS)
                .filter_map(Value::as_array)
                .map(Vec::len)
                .max()
                .unwrap_or(0);
            let names: Vec<String> = (1..=width).map(|i| i.to_string()).collect();
            grids.push(records(a, &names));
            Node::Grid(grids.len() - 1)
        }
        Value::Array(a) if a.iter().all(|x| scalar(x).is_some()) => {
            if a.len() <= INLINE_LIST {
                let parts: Vec<String> = a.iter().filter_map(scalar).map(|(s, _)| s).collect();
                Node::Value(crate::app::clip(&parts.join(", "), MAX_CELL), Tone::Text)
            } else {
                let rows: Vec<Vec<Cell>> = a.iter().take(MAX_ROWS).map(|x| vec![cell(Some(x))]).collect();
                grids.push(Grid::new(
                    vec!["value".into()],
                    rows,
                    a.len().saturating_sub(MAX_ROWS),
                    0,
                ));
                Node::Grid(grids.len() - 1)
            }
        }
        Value::Array(a) => Node::List(
            a.iter()
                .take(MAX_ENTRIES)
                .map(|x| node(x, depth + 1, grids))
                .collect(),
            a.len().saturating_sub(MAX_ENTRIES),
        ),
        Value::Object(o) => object(o, depth, grids),
        _ => Node::Value(compact(v), Tone::Text),
    }
}

/// `{columns: [names], rows: [...]}`.
fn is_tabular(v: &Value) -> bool {
    v.get("rows").is_some_and(Value::is_array) && column_names(v.get("columns")).is_some()
}

fn object(o: &Map<String, Value>, depth: usize, grids: &mut Vec<Grid>) -> Node {
    // `{columns: [names], rows: [...]}`: one grid, in the columns' order
    let names = column_names(o.get("columns"));
    let tabular = names.is_some() && o.get("rows").is_some_and(Value::is_array);
    let mut out = Vec::new();
    for (k, v) in o.iter().take(MAX_ENTRIES) {
        if tabular && k == "columns" {
            continue;
        }
        if tabular
            && k == "rows"
            && let (Some(rows), Some(names)) = (v.as_array(), names.as_ref())
        {
            grids.push(records(rows, names));
            out.push((k.clone(), Node::Grid(grids.len() - 1)));
            continue;
        }
        out.push((k.clone(), node(v, depth + 1, grids)));
    }
    Node::Object(out, o.len().saturating_sub(MAX_ENTRIES))
}

// --- drawing --------------------------------------------------------------------------------

/// Draws `a` (the answer `v` read once), at most `max_h` high, its egui ids under `salt`: the
/// JSON toggle and Copy JSON, then the grid, the tree or the JSON text.
pub fn show(ui: &mut Ui, v: &Value, a: &mut Answer, salt: egui::Id, max_h: f32, l: Lang) {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.toggle_value(&mut a.json, "JSON").on_hover_text(l.t(
            "Show the answer as it came (JSON)",
            "Afficher la réponse telle quelle (JSON)",
        ));
        if ui.button(l.t("Copy JSON", "Copier le JSON")).clicked() {
            ui.ctx()
                .copy_text(serde_json::to_string_pretty(v).unwrap_or_default());
        }
        if !a.json
            && let Node::Grid(_) = a.root
            && let Some(g) = a.grids.first()
        {
            summary(ui, g, &t, l);
        }
    });
    if a.json {
        let text = a.json_text.get_or_insert_with(|| {
            crate::app::clip(&serde_json::to_string_pretty(v).unwrap_or_default(), MAX_JSON)
        });
        egui::ScrollArea::both()
            .id_salt(salt.with("answer-json"))
            .max_height(max_h)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut text.as_str())
                        .code_editor()
                        .desired_width(f32::INFINITY),
                );
            });
        return;
    }
    let Answer { root, grids, .. } = a;
    match root {
        Node::Grid(i) => {
            if let Some(g) = grids.get_mut(*i) {
                grid(ui, g, salt.with(*i), max_h, &t, l);
            }
        }
        Node::Value(s, tone) => {
            egui::ScrollArea::vertical()
                .id_salt(salt.with("answer-text"))
                .max_height(max_h)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.add(Label::new(value_text(s, *tone, &t).font(FontId::monospace(12.0))).wrap());
                });
        }
        root => {
            egui::ScrollArea::vertical()
                .id_salt(salt.with("answer-tree"))
                .max_height(max_h)
                .auto_shrink([false, true])
                .show(ui, |ui| tree(ui, root, grids, salt, 0, &t, l));
        }
    }
}

fn value_text(s: &str, tone: Tone, t: &Tokens) -> RichText {
    let color = match tone {
        Tone::Text => t.text,
        Tone::Number | Tone::Flag => t.accent_text,
        Tone::Null => t.text_faint,
    };
    RichText::new(s).color(color)
}

/// `n` with its thousands grouped (`100,000`, `100 000` in French).
pub fn count(n: usize, l: Lang) -> String {
    let digits = n.to_string();
    let sep = match l {
        Lang::En => ',',
        Lang::Fr => ' ',
    };
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(sep);
        }
        out.push(ch);
    }
    out
}

fn summary(ui: &mut Ui, g: &Grid, t: &Tokens, l: Lang) {
    let mut s = format!(
        "{} {} · {} {}",
        count(g.rows.len(), l),
        l.t("rows", "lignes"),
        g.columns.len(),
        l.t("columns", "colonnes")
    );
    if g.rows_left_out > 0 {
        s.push_str(&format!(
            " · {} {}",
            count(g.rows_left_out, l),
            l.t("more rows in the JSON", "lignes de plus dans le JSON")
        ));
    }
    ui.label(RichText::new(s).small().color(t.text_dim));
}

/// An object's keys and values, one row each; grids, objects and lists fold.
#[allow(clippy::too_many_arguments)]
fn tree(ui: &mut Ui, n: &Node, grids: &mut [Grid], salt: egui::Id, depth: usize, t: &Tokens, l: Lang) {
    let entries: Vec<(String, &Node)> = match n {
        Node::Object(o, _) => o.iter().map(|(k, v)| (k.clone(), v)).collect(),
        Node::List(items, _) => items
            .iter()
            .enumerate()
            .map(|(i, v)| (i.to_string(), v))
            .collect(),
        _ => Vec::new(),
    };
    let left = match n {
        Node::Object(_, k) | Node::List(_, k) => *k,
        _ => 0,
    };
    for (k, v) in entries {
        match v {
            Node::Value(s, tone) => {
                ui.horizontal_top(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(170.0, ROW_H - 4.0), Sense::hover());
                    ui.put(
                        r,
                        Label::new(RichText::new(&k).color(t.text_dim).font(theme::font(12.0))).truncate(),
                    );
                    ui.add(Label::new(value_text(s, *tone, t).font(theme::font(12.0))).wrap());
                });
            }
            Node::Grid(i) => {
                let rows = grids.get(*i).map_or(0, |g| g.rows.len());
                let head = format!("{k} · {} {}", count(rows, l), l.t("rows", "lignes"));
                egui::CollapsingHeader::new(RichText::new(head).color(t.text))
                    .id_salt(salt.with(("answer-node", depth, &k)))
                    .default_open(depth < 4)
                    .show(ui, |ui| {
                        if let Some(g) = grids.get_mut(*i) {
                            grid(ui, g, salt.with(*i), 260.0, t, l);
                        }
                    });
            }
            Node::Object(o, _) => {
                egui::CollapsingHeader::new(RichText::new(&k).color(t.text))
                    .id_salt(salt.with(("answer-node", depth, &k)))
                    .default_open(depth < 2 && o.len() <= 24)
                    .show(ui, |ui| tree(ui, v, grids, salt, depth + 1, t, l));
            }
            Node::List(items, _) => {
                let head = format!("{k} · {} {}", items.len(), l.t("items", "éléments"));
                egui::CollapsingHeader::new(RichText::new(head).color(t.text))
                    .id_salt(salt.with(("answer-node", depth, &k)))
                    .default_open(depth < 2 && items.len() <= 24)
                    .show(ui, |ui| tree(ui, v, grids, salt, depth + 1, t, l));
            }
        }
    }
    if left > 0 {
        ui.label(
            RichText::new(format!("… {left} {}", l.t("more", "de plus")))
                .small()
                .color(t.text_faint),
        );
    }
}

/// The columns' widths: the header's and the first rows' text, between 48 and 360 px.
fn widths(ui: &Ui, g: &Grid) -> Vec<f32> {
    let font = theme::font(12.0);
    let bold = theme::bold(12.0);
    let w = |s: &str, f: &FontId| {
        ui.fonts_mut(|x| {
            x.layout_no_wrap(crate::app::clip(s, 60), f.clone(), egui::Color32::WHITE)
                .size()
                .x
        })
    };
    g.columns
        .iter()
        .enumerate()
        .map(|(c, name)| {
            let mut m = w(name, &bold) + 22.0;
            for r in g.rows.iter().take(200) {
                if let Some(x) = r.get(c) {
                    m = m.max(w(&x.text, &font) + 12.0);
                }
            }
            m.clamp(48.0, 360.0)
        })
        .collect()
}

/// A grid: the header (a click sorts), the visible rows only, Copy as TSV.
pub fn grid(ui: &mut Ui, g: &mut Grid, salt: egui::Id, max_h: f32, t: &Tokens, l: Lang) {
    if g.columns.is_empty() {
        ui.label(RichText::new(l.t("(no columns)", "(aucune colonne)")).color(t.text_faint));
        return;
    }
    ui.horizontal(|ui| {
        if ui
            .button(l.t("Copy as TSV", "Copier en TSV"))
            .on_hover_text(l.t(
                "Copy the rows in this order, tab-separated (paste into a spreadsheet)",
                "Copier les lignes dans cet ordre, séparées par des tabulations (à coller dans un tableur)",
            ))
            .clicked()
        {
            ui.ctx().copy_text(g.tsv());
        }
        if g.columns_left_out > 0 {
            ui.label(
                RichText::new(format!(
                    "{} {}",
                    g.columns_left_out,
                    l.t("more columns in the JSON", "colonnes de plus dans le JSON")
                ))
                .small()
                .color(t.text_dim),
            );
        }
    });
    if g.widths.is_none() {
        g.widths = Some(widths(ui, g));
    }
    let widths = g.widths.clone().unwrap_or_default();
    let total: f32 = widths.iter().sum();
    let mut clicked = None;
    egui::ScrollArea::horizontal()
        .id_salt(salt.with("grid-h"))
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            ui.set_min_width(total);
            // the header
            let (head, _) = ui.allocate_exact_size(vec2(total, ROW_H + 2.0), Sense::hover());
            ui.painter().rect_filled(head, CornerRadius::same(2), t.chrome);
            let mut x = head.min.x;
            for (c, name) in g.columns.iter().enumerate() {
                let w = widths.get(c).copied().unwrap_or(80.0);
                let r = Rect::from_min_size(pos2(x, head.min.y), vec2(w, head.height()));
                let resp = ui.interact(r, salt.with(("grid-head", c)), Sense::click());
                let sorted = g.sort.filter(|(sc, _)| *sc == c).map(|(_, asc)| asc);
                let what = match sorted {
                    Some(true) => l.t("sorted ascending", "tri croissant"),
                    Some(false) => l.t("sorted descending", "tri décroissant"),
                    None => l.t("click to sort", "cliquer pour trier"),
                };
                resp.widget_info(|| {
                    egui::WidgetInfo::selected(egui::Role::Button, true, sorted.is_some(), name)
                });
                if resp.hovered() {
                    ui.painter().rect_filled(r, CornerRadius::ZERO, t.hover);
                }
                let resp = resp.on_hover_text(format!("{name}\n{what}"));
                if resp.clicked() {
                    clicked = Some(c);
                }
                let text_r = Rect::from_min_max(r.min + vec2(6.0, 0.0), r.max - vec2(16.0, 0.0));
                let galley =
                    ui.painter()
                        .layout_no_wrap(crate::app::clip(name, 60), theme::bold(12.0), t.text);
                ui.painter().with_clip_rect(text_r).galley(
                    pos2(text_r.min.x, text_r.center().y - galley.size().y / 2.0),
                    galley,
                    t.text,
                );
                if let Some(asc) = sorted {
                    triangle(ui, pos2(r.max.x - 9.0, r.center().y), asc, t.accent_text);
                }
                x += w;
                ui.painter()
                    .vline(x - 0.5, head.y_range(), egui::Stroke::new(1.0, t.separator));
            }
            // the rows: only the visible ones are laid out
            let n = g.order.len();
            let body_h = (max_h - ROW_H - 2.0).max(ROW_H * 3.0);
            egui::ScrollArea::vertical()
                .id_salt(salt.with("grid-v"))
                .max_height(body_h)
                .auto_shrink([false, true])
                .show_rows(ui, ROW_H, n, |ui, range| {
                    ui.spacing_mut().item_spacing = Vec2::ZERO;
                    for r in range {
                        let (row, _) = ui.allocate_exact_size(vec2(total, ROW_H), Sense::hover());
                        if !r.is_multiple_of(2) {
                            ui.painter().rect_filled(row, CornerRadius::ZERO, t.canvas);
                        }
                        let Some(cells) = g.order.get(r).and_then(|i| g.rows.get(*i)) else {
                            continue;
                        };
                        let mut x = row.min.x;
                        for (c, w) in widths.iter().enumerate() {
                            let cr = Rect::from_min_size(pos2(x + 6.0, row.min.y), vec2(w - 10.0, ROW_H));
                            x += w;
                            let Some(cell) = cells.get(c) else { continue };
                            if cell.text.is_empty() {
                                continue;
                            }
                            let color = if cell.num.is_some() { t.accent_text } else { t.text };
                            let lab =
                                Label::new(RichText::new(&cell.text).font(theme::font(12.0)).color(color))
                                    .truncate();
                            if cell.num.is_some() {
                                ui.put(cr, lab.halign(egui::Align::Max));
                            } else {
                                ui.put(cr, lab);
                            }
                        }
                    }
                });
        });
    if let Some(c) = clicked {
        g.click(c);
    }
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
        egui::Stroke::NONE,
    ));
}

#[cfg(test)]
#[path = "answer_tests.rs"]
mod tests;
