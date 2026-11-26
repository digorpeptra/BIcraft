//! The Report view's task panes on the right, as in Power BI Desktop (docs/design.md §5, §12):
//!
//! - **Data** (the rightmost): a search box, a toggle for hidden objects, and the model's tables
//!   with their columns, measures (the calculator glyph), hierarchies and levels, grouped by
//!   display folder. Every field can be dragged; its box, with a visual selected, adds it to the
//!   visual's default role (or removes it); its right-click menu has New measure, Rename and
//!   Hide / Unhide.
//! - **Build** (Desktop's Visualizations pane) for the visual selected on the canvas, with
//!   Desktop's two sub-tabs at its top: **Build visual**, its type (a picker over the snapshot's
//!   `report.visual_types`) and one well per data role of the type (the snapshot's `roles`), each
//!   listing its fields with the aggregation of a column, a remove button, and drag to reorder or
//!   to move to another well; **Format visual**, the formatting cards (`crate::format_pane`).
//!
//! Drag and drop: a field onto a well is `visual.bind` at the drop position; onto a visual on the
//! canvas, `visual.bind` to that visual's default role; a well's field onto a well is
//! `visual.move_field`, anywhere else `visual.unbind`. Drop targets light up while dragging.
//! Every edit is an engine command (one undo step), so the page re-renders from the next
//! snapshot; a reader, or a command the session cannot run, leaves the panes read-only with the
//! reason as tooltip. Open or collapsed and the widths are UI preferences
//! ([`crate::prefs::PanePrefs`]).

use std::collections::BTreeSet;

use egui::{Align2, CornerRadius, DragAndDrop, Id, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use serde_json::{Value, json};

use crate::App;
use crate::actions::Call;
use crate::app::{Then, clip};
use crate::i18n::Lang;
use crate::icons::{self, Icon};
use crate::keyboard;
use crate::prefs::PANE_WIDTHS;
use crate::ribbon::{self, VisualSel};
use crate::theme::{self, Tokens};
use crate::widgets;

/// Height of a row of the field list and of a well's field.
const ROW_H: f32 = 22.0;
/// Indent of each level of the field list.
const INDENT: f32 = 14.0;
/// Width of a collapsed pane.
const STRIP_W: f32 = 30.0;
/// Most fields the field list draws per table (the snapshot caps them too).
const MAX_ITEMS: usize = 2000;
/// Longest name drawn (characters).
const MAX_LABEL: usize = 60;

/// The commands the panes run (the coverage test reads it).
pub const COMMANDS: &[&str] = &[
    "visual.bind",
    "model.refresh",
    "visual.bind_hierarchy",
    "visual.rename_field",
    "visual.show_value_as",
    "visual.show_items_no_data",
    "visual.add_sparkline",
    "visual.edit_sparkline",
    "visual.add",
    "visual.unbind",
    "visual.move_field",
    "visual.set_aggregation",
    "visual.set_type",
    "measure.add",
    "table.rename",
    "table.entered_data",
    "column.rename",
    "column.group",
    "measure.quick",
    "measure.rename",
    "hierarchy.set",
    "table.set",
    "table.set_refresh_policy",
    "column.set",
    "measure.set",
    "tmdl.script",
];

/// The panes' view state (pure UI: never sent anywhere).
#[derive(Debug, Default)]
pub struct PanesUi {
    /// The Data pane's search text.
    pub search: String,
    /// Hidden tables, columns, measures and hierarchies are listed (dimmed).
    pub show_hidden: bool,
    /// The tree nodes expanded: `t:<table>`, `f:<table>/<folder>`, `h:<table>/<hierarchy>`.
    pub open: BTreeSet<String>,
    /// A well's field being renamed.
    pub renaming: Option<Rename>,
    /// The Build pane's sub-tab and its Format visual tab (`crate::format_pane`).
    pub format: crate::format_pane::FormatUi,
}

/// A model field the Data pane drags or checks.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    /// The field as commands write it: `Sales[Amount]`, `[Total]`.
    pub text: String,
    pub table: String,
    pub name: String,
    pub measure: bool,
    /// A number column (int64, double, decimal): summarized when it goes to a value well.
    pub numeric: bool,
    /// The column's `summarize_by` (`sum`, `none`…), when the model says.
    pub summarize: Option<String>,
    /// The column's data category (`City`, `Country`…), when the model says.
    pub category: Option<String>,
}

/// A field bound to a visual, dragged out of its well.
#[derive(Debug, Clone, PartialEq)]
pub struct Bound {
    pub page: String,
    pub visual: String,
    pub role: String,
    /// Its position inside the role.
    pub index: usize,
    /// Its text (`None`: a field commands cannot name, such as a hierarchy level: by position).
    pub field: Option<String>,
    pub label: String,
}

/// What is being dragged (egui's drag-and-drop payload).
#[derive(Debug, Clone, PartialEq)]
pub enum Drag {
    Field(Field),
    Bound(Bound),
    /// A whole hierarchy of the Data pane: bound as a hierarchy (`visual.bind_hierarchy`).
    Hierarchy(Hier),
}

/// A hierarchy dragged from the Data pane.
#[derive(Debug, Clone, PartialEq)]
pub struct Hier {
    pub table: String,
    pub name: String,
}

impl Drag {
    fn label(&self) -> &str {
        match self {
            Drag::Field(f) => &f.name,
            Drag::Bound(b) => &b.label,
            Drag::Hierarchy(h) => &h.name,
        }
    }
}

/// What a data role accepts (the snapshot's `roles[].kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleKind {
    Grouping,
    Measure,
    Either,
}

/// One data role of a visual: a well.
#[derive(Debug, Clone, PartialEq)]
pub struct Role {
    pub name: String,
    pub kind: RoleKind,
    pub max: Option<usize>,
}

/// One field of a well, as the snapshot lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub role: String,
    /// `column`, `measure`, `aggregation`, `other`.
    pub kind: String,
    pub table: Option<String>,
    pub name: Option<String>,
    pub function: Option<String>,
    pub text: String,
    pub label: String,
    /// Its name for this visual only (`visual.rename_field`).
    pub display_name: Option<String>,
    /// What its value is shown as (`visual.show_value_as`): `percent_of_grand_total`…
    pub show_as: Option<String>,
    /// Its role shows the items with no data (`visual.show_items_no_data`).
    pub show_all: bool,
    /// A sparkline's place among the visual's sparklines (`visual.edit_sparkline {index}`).
    pub spark: Option<usize>,
}

/// A well's field being renamed for its visual (double click, or its menu).
#[derive(Debug, Clone, PartialEq)]
pub struct Rename {
    pub visual: String,
    pub role: String,
    /// Its position inside the role.
    pub index: usize,
    pub text: String,
    /// The box has had the focus once.
    pub focused: bool,
}

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_string)
}

fn arr(v: Option<&Value>) -> &[Value] {
    v.and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default()
}

/// `Table[Column]` as `visual.bind` reads it (quoted when the table name needs it).
pub fn column_text(table: &str, column: &str) -> String {
    let plain = !table.is_empty() && table.chars().all(|c| c.is_alphanumeric() || c == '_');
    let col = column.replace(']', "]]");
    if plain {
        format!("{table}[{col}]")
    } else {
        format!("'{}'[{col}]", table.replace('\'', "''"))
    }
}

/// `[Measure]`.
pub fn measure_text(name: &str) -> String {
    format!("[{}]", name.replace(']', "]]"))
}

/// The roles of a snapshot visual: the type's (`roles`), then those its fields use that the type
/// does not list (custom visuals: only those).
pub fn roles_of(v: &Value) -> Vec<Role> {
    let mut out: Vec<Role> = arr(v.get("roles"))
        .iter()
        .take(64)
        .filter_map(|r| {
            Some(Role {
                name: s(r, "name")?,
                kind: match r.get("kind").and_then(Value::as_str) {
                    Some("grouping") => RoleKind::Grouping,
                    Some("measure") => RoleKind::Measure,
                    _ => RoleKind::Either,
                },
                max: r
                    .get("max")
                    .and_then(Value::as_u64)
                    .and_then(|m| usize::try_from(m).ok()),
            })
        })
        .collect();
    for it in items_of(v) {
        if !out.iter().any(|r| r.name.eq_ignore_ascii_case(&it.role)) && out.len() < 64 {
            out.push(Role {
                name: it.role.clone(),
                kind: RoleKind::Either,
                max: None,
            });
        }
    }
    out
}

/// The fields of a snapshot visual, in order.
pub fn items_of(v: &Value) -> Vec<Item> {
    arr(v.get("fields"))
        .iter()
        .take(500)
        .filter_map(|f| {
            let name = s(f, "name");
            let text = s(f, "text").unwrap_or_default();
            let label = s(f, "label")
                .filter(|l| !l.is_empty())
                .or_else(|| name.clone())
                .unwrap_or_else(|| text.clone());
            // a percentage is named as Desktop names it (`%GT Sum of Amount`), unless renamed
            let label = match show_as_prefix(f.get("show_as").and_then(Value::as_str)) {
                Some(p) if s(f, "display_name").is_none() => format!("{p} {label}"),
                _ => label,
            };
            Some(Item {
                role: s(f, "role")?,
                kind: s(f, "kind").unwrap_or_else(|| "other".into()),
                table: s(f, "table"),
                label,
                name,
                function: s(f, "function"),
                text,
                display_name: s(f, "display_name"),
                show_as: s(f, "show_as"),
                show_all: f.get("show_all").and_then(Value::as_bool).unwrap_or(false),
                spark: f
                    .pointer("/sparkline/index")
                    .and_then(Value::as_u64)
                    .and_then(|i| usize::try_from(i).ok()),
            })
        })
        .collect()
}

/// The aggregation `summarize_by` asks for a number column going to a value well (`None`: the
/// column itself).
fn summarized(f: &Field) -> Option<&'static str> {
    match f.summarize.as_deref().map(str::to_ascii_lowercase).as_deref() {
        Some("none") => None,
        Some("average") => Some("Avg"),
        Some("min") => Some("Min"),
        Some("max") => Some("Max"),
        Some("count") | Some("distinctcount") => Some("Count"),
        Some("sum") => Some("Sum"),
        _ if f.numeric => Some("Sum"),
        _ => None,
    }
}

/// Where a field dropped on a visual goes, as Desktop does it: a measure (or a number column,
/// summarized) to the first value role with room, a column to the first grouping role with room
/// (then, counted, to a value role). Returns (role, field text); `None` when no role has room.
/// `bound` is the visual's (role, field text) list.
pub fn default_target(roles: &[Role], bound: &[(String, String)], f: &Field) -> Option<(String, String)> {
    let count = |r: &Role| {
        bound
            .iter()
            .filter(|(x, _)| x.eq_ignore_ascii_case(&r.name))
            .count()
    };
    let room = |r: &Role| r.max.is_none_or(|m| count(r) < m);
    let fresh = |r: &Role, text: &str| {
        !bound
            .iter()
            .any(|(x, t)| x.eq_ignore_ascii_case(&r.name) && t.eq_ignore_ascii_case(text))
    };
    let agg = |function: &str| format!("{function}({})", f.text);
    let mut tries: Vec<(&Role, String)> = Vec::new();
    if f.measure {
        tries.extend(
            roles
                .iter()
                .filter(|r| r.kind != RoleKind::Grouping)
                .map(|r| (r, f.text.clone())),
        );
    } else if let Some(function) = summarized(f) {
        tries.extend(
            roles
                .iter()
                .filter(|r| r.kind != RoleKind::Grouping)
                .map(|r| (r, agg(function))),
        );
        tries.extend(
            roles
                .iter()
                .filter(|r| r.kind == RoleKind::Grouping)
                .map(|r| (r, f.text.clone())),
        );
    } else {
        tries.extend(
            roles
                .iter()
                .filter(|r| r.kind != RoleKind::Measure)
                .map(|r| (r, f.text.clone())),
        );
        tries.extend(
            roles
                .iter()
                .filter(|r| r.kind == RoleKind::Measure)
                .map(|r| (r, agg("Count"))),
        );
    }
    tries
        .into_iter()
        .find(|(r, text)| room(r) && fresh(r, text))
        .map(|(r, text)| (r.name.clone(), text))
}

/// The text a field takes in a given well (a number column in a value well is summarized).
fn text_for_role(role: &Role, f: &Field) -> String {
    if f.measure || role.kind == RoleKind::Grouping {
        return f.text.clone();
    }
    match summarized(f) {
        Some(function) => format!("{function}({})", f.text),
        None if role.kind == RoleKind::Measure => format!("Count({})", f.text),
        None => f.text.clone(),
    }
}

/// A well's caption for the visual's type, in BIcraft's words (the role name when it has none).
pub fn caption(visual_type: &str, role: &str, l: Lang) -> String {
    let bar = visual_type.to_ascii_lowercase().contains("barchart");
    let pie = matches!(visual_type, "pieChart" | "donutChart");
    let scatter = visual_type == "scatterChart";
    let chart = visual_type.to_ascii_lowercase().contains("chart") && !pie && !scatter;
    let t = match role {
        "Category" if bar => l.t("Y-axis", "Axe Y"),
        "Category" if pie => l.t("Legend", "Légende"),
        // a scatter chart's Category groups its points; X and Y are its two values
        "Category" if scatter => l.t("Details", "Détails"),
        "Category" if chart => l.t("X-axis", "Axe X"),
        "Category" => l.t("Category", "Catégorie"),
        "Group" => l.t("Category", "Catégorie"),
        "Y" if bar => l.t("X-axis", "Axe X"),
        "Y" if visual_type == "gauge" => l.t("Value", "Valeur"),
        "Y" if scatter => l.t("Y values", "Valeurs Y"),
        "Y" if chart => l.t("Y-axis", "Axe Y"),
        "X" if scatter => l.t("X values", "Valeurs X"),
        "Y2" => l.t("Secondary y-axis", "Axe Y secondaire"),
        "Series" if pie => l.t("Details", "Détails"),
        "Series" => l.t("Legend", "Légende"),
        "Rows" if visual_type == "pivotTable" => l.t("Rows", "Lignes"),
        "Rows" => l.t("Small multiples", "Multiples"),
        "Columns" => l.t("Columns", "Colonnes"),
        "Values" if visual_type == "tableEx" => l.t("Columns", "Colonnes"),
        "Values" if visual_type == "card" || visual_type == "multiRowCard" => l.t("Fields", "Champs"),
        "Values" if visual_type.to_ascii_lowercase().contains("slicer") => l.t("Field", "Champ"),
        "Values" | "Y" => l.t("Values", "Valeurs"),
        "Tooltips" => l.t("Tooltips", "Info-bulles"),
        "Details" => l.t("Details", "Détails"),
        "Size" => l.t("Size", "Taille"),
        "X" => l.t("X-axis", "Axe X"),
        "Legend" => l.t("Legend", "Légende"),
        "Target" | "TargetValue" => l.t("Target", "Cible"),
        "MinValue" => l.t("Minimum", "Minimum"),
        "MaxValue" => l.t("Maximum", "Maximum"),
        "Indicator" => l.t("Indicator", "Indicateur"),
        "TrendLine" => l.t("Trend line", "Courbe de tendance"),
        "Goal" => l.t("Goal", "Objectif"),
        "Play" => l.t("Play axis", "Axe de lecture"),
        "Breakdown" => l.t("Breakdown", "Ventilation"),
        "Analyze" => l.t("Analyze", "Analyser"),
        "ExplainBy" => l.t("Explain by", "Expliquer par"),
        "Value" => l.t("Value", "Valeur"),
        "PathID" => l.t("Path ID", "ID du chemin"),
        "PointOrder" => l.t("Point order", "Ordre des points"),
        "Label" => l.t("Label", "Étiquette"),
        "Data" => l.t("Data", "Données"),
        "parameterValues" => l.t("Parameter values", "Valeurs de paramètre"),
        _ => return role.to_string(),
    };
    t.to_string()
}

/// "Show value as" choices: (code for `visual.show_value_as`, EN, FR, matrix only).
const SHOW_VALUE_AS: &[(&str, &str, &str, bool)] = &[
    ("none", "No calculation", "Aucun calcul", false),
    (
        "percent_of_grand_total",
        "Percent of grand total",
        "Pourcentage du total général",
        false,
    ),
    (
        "percent_of_column_total",
        "Percent of column total",
        "Pourcentage du total de la colonne",
        true,
    ),
    (
        "percent_of_row_total",
        "Percent of row total",
        "Pourcentage du total de la ligne",
        true,
    ),
];

/// The prefix a well shows before a field shown as a percentage (Desktop's `%GT`, `%CT`, `%RT`).
fn show_as_prefix(code: Option<&str>) -> Option<&'static str> {
    match code? {
        "percent_of_grand_total" => Some("%GT"),
        "percent_of_column_total" => Some("%CT"),
        "percent_of_row_total" => Some("%RT"),
        _ => None,
    }
}

/// The aggregations a well's column offers: (code for `visual.set_aggregation`, EN, FR, numbers
/// only).
const AGGREGATIONS: &[(&str, &str, &str, bool)] = &[
    ("none", "Don't summarize", "Ne pas synthétiser", false),
    ("Sum", "Sum", "Somme", true),
    ("Avg", "Average", "Moyenne", true),
    ("Min", "Minimum", "Minimum", false),
    ("Max", "Maximum", "Maximum", false),
    ("Count", "Count", "Nombre", false),
    ("CountNonNull", "Count (not blank)", "Nombre (non vides)", false),
    ("Median", "Median", "Médiane", true),
    ("StandardDeviation", "Standard deviation", "Écart type", true),
    ("Variance", "Variance", "Variance", true),
];

fn aggregation_text(code: Option<&str>, l: Lang) -> &'static str {
    let code = code.unwrap_or("none");
    AGGREGATIONS
        .iter()
        .find(|a| a.0.eq_ignore_ascii_case(code))
        .map_or("", |a| l.t(a.1, a.2))
}

/// Runs command `id` on the active document, as a pane's action: refused with the reason in the
/// status bar when the session cannot run it.
pub(crate) fn exec(app: &mut App, id: &'static str, params: Value) -> bool {
    if let Err(why) = ribbon::state(app, id) {
        app.status = format!("{} {why}", app.lang.t("Unavailable:", "Indisponible :"));
        return false;
    }
    app.run(Call { id, params }, Then::Nothing)
}

/// The selected visual's snapshot entry.
fn selected(app: &App) -> Option<(VisualSel, Value)> {
    let sel = app.cmd.visual.clone()?;
    let v = visual_value(app, &sel)?;
    Some((sel, v))
}

fn visual_value(app: &App, sel: &VisualSel) -> Option<Value> {
    let snap = &app.loaded.as_ref()?.snap;
    let page = arr(snap.pointer("/report/pages"))
        .iter()
        .find(|p| p.get("name").and_then(Value::as_str) == Some(sel.page.as_str()))?;
    arr(page.get("visuals"))
        .iter()
        .find(|v| v.get("name").and_then(Value::as_str) == Some(sel.visual.as_str()))
        .cloned()
}

/// Whether model field `f` is bound to visual `v` (any role).
fn is_bound(v: &Value, f: &Field) -> Vec<(String, Option<String>, usize)> {
    let mut per_role: Vec<(String, usize)> = Vec::new();
    let mut out = Vec::new();
    for it in items_of(v) {
        let pos = match per_role
            .iter_mut()
            .find(|(r, _)| r.eq_ignore_ascii_case(&it.role))
        {
            Some((_, n)) => {
                *n += 1;
                *n - 1
            }
            None => {
                per_role.push((it.role.clone(), 1));
                0
            }
        };
        let same_name = it
            .name
            .as_deref()
            .is_some_and(|n| n.eq_ignore_ascii_case(&f.name));
        let same_table = it
            .table
            .as_deref()
            .is_some_and(|t| t.eq_ignore_ascii_case(&f.table));
        let hit = match it.kind.as_str() {
            "measure" => f.measure && same_name,
            "column" | "aggregation" => !f.measure && same_name && same_table,
            _ => false,
        };
        if hit {
            out.push((it.role.clone(), Some(it.text.clone()), pos));
        }
    }
    out
}

/// Binds `f` to visual `sel` (whose snapshot entry is `v`) in its default role; says why in the
/// status bar when no role has room.
fn bind_default(app: &mut App, sel: &VisualSel, v: &Value, f: &Field) {
    let roles = roles_of(v);
    let bound: Vec<(String, String)> = items_of(v).into_iter().map(|i| (i.role, i.text)).collect();
    match default_target(&roles, &bound, f) {
        Some((role, text)) => {
            exec(
                app,
                "visual.bind",
                json!({"page": sel.page, "visual": sel.visual, "role": role, "field": text}),
            );
        }
        None => {
            app.status = app
                .lang
                .t(
                    "This visual has no room for that field: drop it on a well.",
                    "Ce visuel n'a pas de place pour ce champ : déposez-le dans un emplacement.",
                )
                .to_string();
        }
    }
}

/// The pointer's position this frame.
fn pointer(ui: &Ui) -> Option<egui::Pos2> {
    ui.input(|i| i.pointer.hover_pos().or(i.pointer.interact_pos()))
}

fn released(ui: &Ui) -> bool {
    ui.input(|i| i.pointer.any_released())
}

/// The payload being dragged, if it is the panes'.
fn dragging(ctx: &egui::Context) -> Option<Drag> {
    DragAndDrop::payload::<Drag>(ctx).map(|d| (*d).clone())
}

/// Draws the panes (Report view): the rail of the closed ones at the right edge, then Selection,
/// Bookmarks, Sync slicers, Data, Build and Filters, from the edge inwards. Called before the
/// canvas.
pub fn show(app: &mut App, ui: &mut Ui) {
    let t = Tokens::of(app.prefs.theme);
    let frame = egui::Frame::NONE
        .fill(t.card)
        .stroke(Stroke::new(1.0, t.card_border))
        .inner_margin(egui::Margin::same(10));
    let strip = egui::Frame::NONE
        .fill(t.chrome)
        .inner_margin(egui::Margin::symmetric(3, 8));
    let range = *PANE_WIDTHS.start()..=*PANE_WIDTHS.end();
    let l = app.lang;
    // The right rail: one icon per closed pane, at the window's edge (a click opens it). One
    // rail keeps the canvas wide whatever the number of panes closed, as Desktop's right rail.
    if RAIL
        .iter()
        .any(|(pane, _)| !pane_open(app, *pane) && offered(app, *pane))
    {
        let r = egui::Panel::right("bicraft-panes-rail")
            .exact_size(STRIP_W)
            .resizable(false)
            .frame(strip)
            .show(ui, |ui| {
                for (pane, icon) in RAIL {
                    if pane_open(app, pane) || !offered(app, pane) {
                        continue;
                    }
                    let tip = rail_tip(l, pane);
                    if widgets::icon_toggle(ui, icon, tip, true, false).clicked() {
                        open_pane(app, pane);
                    }
                }
            });
        keyboard::section(app, ui, keyboard::Section::Pane, r.response.rect);
    }
    // Selection (rightmost of the panes, as in Desktop)
    if app.prefs.panes.selection {
        let r = egui::Panel::right("bicraft-selection-pane")
            .resizable(true)
            .default_size(app.prefs.panes.selection_width)
            .size_range(range.clone())
            .frame(frame)
            .show(ui, |ui| crate::selection_pane::show(app, ui));
        keep_width(app, ui, r.response.rect.width(), Pane::Selection);
        keyboard::section(app, ui, keyboard::Section::Selection, r.response.rect);
    }
    // Bookmarks (left of the Selection pane)
    if app.prefs.panes.bookmarks {
        let r = egui::Panel::right("bicraft-bookmarks-pane")
            .resizable(true)
            .default_size(app.prefs.panes.bookmarks_width)
            .size_range(range.clone())
            .frame(frame)
            .show(ui, |ui| crate::bookmarks_pane::show(app, ui));
        keep_width(app, ui, r.response.rect.width(), Pane::Bookmarks);
        keyboard::section(app, ui, keyboard::Section::Pane, r.response.rect);
    }
    // Sync slicers (left of the Bookmarks pane)
    if app.prefs.panes.sync {
        let r = egui::Panel::right("bicraft-sync-pane")
            .resizable(true)
            .default_size(app.prefs.panes.sync_width)
            .size_range(range.clone())
            .frame(frame)
            .show(ui, |ui| crate::sync_pane::show(app, ui));
        keep_width(app, ui, r.response.rect.width(), Pane::Sync);
        keyboard::section(app, ui, keyboard::Section::Pane, r.response.rect);
    }
    // Data (the Page visuals pane while the mobile layout is edited, crate::mobile)
    if app.prefs.panes.data {
        let r = egui::Panel::right("bicraft-data-pane")
            .resizable(true)
            .default_size(app.prefs.panes.data_width)
            .size_range(range.clone())
            .frame(frame)
            .show(ui, |ui| {
                if crate::mobile::editing(app) {
                    crate::mobile::pane(app, ui);
                } else {
                    data_pane(app, ui);
                }
            });
        keep_width(app, ui, r.response.rect.width(), Pane::Data);
        keyboard::section(app, ui, keyboard::Section::Pane, r.response.rect);
    }
    // Build
    if app.prefs.panes.build {
        let r = egui::Panel::right("bicraft-build-pane")
            .resizable(true)
            .default_size(app.prefs.panes.build_width)
            .size_range(range)
            .frame(frame)
            .show(ui, |ui| build_pane(app, ui));
        keep_width(app, ui, r.response.rect.width(), Pane::Build);
        keyboard::section(app, ui, keyboard::Section::Pane, r.response.rect);
    }
    // Filters (left of Build, as in Desktop)
    if app.prefs.panes.filters && offered(app, Pane::Filters) {
        let r = egui::Panel::right("bicraft-filters-pane")
            .resizable(true)
            .default_size(app.prefs.panes.filters_width)
            .size_range(*PANE_WIDTHS.start()..=*PANE_WIDTHS.end())
            .frame(crate::filter_pane_ui::frame(app, frame))
            .show(ui, |ui| crate::filters_pane::pane(app, ui));
        crate::filter_pane_ui::paint_border(app, ui, r.response.rect);
        keep_width(app, ui, r.response.rect.width(), Pane::Filters);
        keyboard::section(app, ui, keyboard::Section::Pane, r.response.rect);
    }
    // the Performance analyzer, when it is open (left of the others)
    crate::perf_pane::show(app, ui, frame);
}

/// The right rail's icons, in the order listed: each opens its pane.
const RAIL: [(Pane, Icon); 6] = [
    (Pane::Selection, Icon::Layers),
    (Pane::Bookmarks, Icon::Pin),
    (Pane::Sync, Icon::Link),
    (Pane::Data, Icon::Table),
    (Pane::Build, Icon::Chart),
    (Pane::Filters, Icon::Filter),
];

/// Whether `pane` is open.
fn pane_open(app: &App, pane: Pane) -> bool {
    match pane {
        Pane::Data => app.prefs.panes.data,
        Pane::Build => app.prefs.panes.build,
        Pane::Filters => app.prefs.panes.filters,
        Pane::Selection => app.prefs.panes.selection,
        Pane::Bookmarks => app.prefs.panes.bookmarks,
        Pane::Sync => app.prefs.panes.sync,
    }
}

/// Whether `pane` is offered at all: the report hides the Filters pane from its readers
/// (`report.settings {filter_pane_visible: false}`, the pane's eye).
fn offered(app: &App, pane: Pane) -> bool {
    !matches!(pane, Pane::Filters) || !crate::filters_pane::hidden_from_reader(app)
}

/// Opens `pane` from the rail, and keeps the change.
fn open_pane(app: &mut App, pane: Pane) {
    match pane {
        Pane::Data => app.prefs.panes.data = true,
        Pane::Build => app.prefs.panes.build = true,
        Pane::Filters => app.prefs.panes.filters = true,
        Pane::Selection => app.prefs.panes.selection = true,
        Pane::Bookmarks => app.prefs.panes.bookmarks = true,
        Pane::Sync => app.prefs.panes.sync = true,
    }
    app.save_prefs();
}

/// The rail icon's tip: the pane it opens.
fn rail_tip(l: crate::i18n::Lang, pane: Pane) -> &'static str {
    match pane {
        Pane::Selection => l.t("Open the Selection pane", "Ouvrir le volet Sélection"),
        Pane::Bookmarks => l.t("Open the Bookmarks pane", "Ouvrir le volet Signets"),
        Pane::Sync => l.t(
            "Open the Sync slicers pane",
            "Ouvrir le volet Synchroniser les segments",
        ),
        Pane::Data => l.t("Open the Data pane", "Ouvrir le volet Données"),
        Pane::Build => l.t("Open the Visualizations pane", "Ouvrir le volet Visualisations"),
        Pane::Filters => l.t("Open the Filters pane", "Ouvrir le volet Filtres"),
    }
}

/// The right panes.
#[derive(Clone, Copy)]
enum Pane {
    Data,
    Build,
    Filters,
    Selection,
    Bookmarks,
    Sync,
}

/// Keeps a pane's width in the prefs once the person stops resizing it.
fn keep_width(app: &mut App, ui: &Ui, w: f32, pane: Pane) {
    if !w.is_finite() || ui.input(|i| i.pointer.any_down()) {
        return;
    }
    let w = w.clamp(*PANE_WIDTHS.start(), *PANE_WIDTHS.end()).round();
    let kept = match pane {
        Pane::Data => &mut app.prefs.panes.data_width,
        Pane::Build => &mut app.prefs.panes.build_width,
        Pane::Filters => &mut app.prefs.panes.filters_width,
        Pane::Selection => &mut app.prefs.panes.selection_width,
        Pane::Bookmarks => &mut app.prefs.panes.bookmarks_width,
        Pane::Sync => &mut app.prefs.panes.sync_width,
    };
    if (*kept - w).abs() >= 1.0 {
        *kept = w;
        app.save_prefs();
    }
}

/// A pane's title row: the title (16 semibold) and the collapse button. True when clicked.
pub(crate) fn title_row(ui: &mut Ui, title: &str, collapse_tip: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    let mut collapse = false;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(title).font(theme::bold(14.0)).color(t.text));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            collapse = widgets::icon_toggle(ui, Icon::ChevronRight, collapse_tip, true, false).clicked();
        });
    });
    ui.add_space(4.0);
    collapse
}

// ----- Data pane -----

/// One node of a table's field tree.
enum Node {
    Folder(String, Vec<Node>),
    Field(Field, bool),
    Hierarchy {
        name: String,
        hidden: bool,
        levels: Vec<Field>,
    },
}

impl Node {
    fn sort_key(&self) -> (u8, String) {
        match self {
            Node::Folder(n, _) => (0, n.to_lowercase()),
            Node::Hierarchy { name, .. } => (1, name.to_lowercase()),
            Node::Field(f, _) => (1, f.name.to_lowercase()),
        }
    }

    fn matches(&self, q: &str) -> bool {
        match self {
            Node::Folder(n, kids) => n.to_lowercase().contains(q) || kids.iter().any(|k| k.matches(q)),
            Node::Field(f, _) => f.name.to_lowercase().contains(q),
            Node::Hierarchy { name, levels, .. } => {
                name.to_lowercase().contains(q) || levels.iter().any(|f| f.name.to_lowercase().contains(q))
            }
        }
    }
}

/// Puts `node` under display folder `folder` (`\`-separated levels) of `nodes`.
fn place(nodes: &mut Vec<Node>, folder: Option<&str>, node: Node) {
    let parts: Vec<&str> = folder
        .unwrap_or("")
        .split(['\\', '/'])
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .take(8)
        .collect();
    let mut at = nodes;
    for p in parts {
        let i = match at.iter().position(|n| matches!(n, Node::Folder(f, _) if f == p)) {
            Some(i) => i,
            None => {
                at.push(Node::Folder(p.to_string(), Vec::new()));
                at.len() - 1
            }
        };
        let Some(Node::Folder(_, kids)) = at.get_mut(i) else {
            return;
        };
        at = kids;
    }
    at.push(node);
}

fn sort_nodes(nodes: &mut [Node]) {
    nodes.sort_by_key(Node::sort_key);
    for n in nodes {
        if let Node::Folder(_, kids) = n {
            sort_nodes(kids);
        }
    }
}

/// A snapshot table's field tree (hidden objects only with `show_hidden`).
fn table_tree(t: &Value, show_hidden: bool) -> Vec<Node> {
    let table = s(t, "name").unwrap_or_default();
    let mut nodes = Vec::new();
    let hidden = |v: &Value| v.get("hidden").and_then(Value::as_bool) == Some(true);
    let numeric = |dt: Option<&str>| matches!(dt, Some("int64" | "double" | "decimal"));
    for c in arr(t.get("columns")).iter().take(MAX_ITEMS) {
        let Some(name) = s(c, "name") else { continue };
        if hidden(c) && !show_hidden {
            continue;
        }
        let f = Field {
            text: column_text(&table, &name),
            table: table.clone(),
            name,
            measure: false,
            numeric: numeric(c.get("data_type").and_then(Value::as_str)),
            summarize: s(c, "summarize_by"),
            category: s(c, "data_category"),
        };
        place(
            &mut nodes,
            c.get("display_folder").and_then(Value::as_str),
            Node::Field(f, hidden(c)),
        );
    }
    for m in arr(t.get("measures")).iter().take(MAX_ITEMS) {
        let Some(name) = s(m, "name") else { continue };
        if hidden(m) && !show_hidden {
            continue;
        }
        let f = Field {
            text: measure_text(&name),
            table: table.clone(),
            name,
            measure: true,
            numeric: true,
            summarize: None,
            category: None,
        };
        place(
            &mut nodes,
            m.get("display_folder").and_then(Value::as_str),
            Node::Field(f, hidden(m)),
        );
    }
    let col_info = |name: &str| {
        arr(t.get("columns"))
            .iter()
            .find(|c| c.get("name").and_then(Value::as_str) == Some(name))
            .cloned()
    };
    for h in arr(t.get("hierarchy_levels")).iter().take(MAX_ITEMS) {
        let Some(name) = s(h, "name") else { continue };
        if hidden(h) && !show_hidden {
            continue;
        }
        let levels = arr(h.get("levels"))
            .iter()
            .filter_map(|lv| {
                let column = s(lv, "column")?;
                let info = col_info(&column);
                Some(Field {
                    text: column_text(&table, &column),
                    table: table.clone(),
                    name: s(lv, "name").unwrap_or_else(|| column.clone()),
                    measure: false,
                    numeric: numeric(
                        info.as_ref()
                            .and_then(|c| c.get("data_type"))
                            .and_then(Value::as_str),
                    ),
                    summarize: info.as_ref().and_then(|c| s(c, "summarize_by")),
                    category: info.as_ref().and_then(|c| s(c, "data_category")),
                })
            })
            .collect();
        place(
            &mut nodes,
            h.get("display_folder").and_then(Value::as_str),
            Node::Hierarchy {
                name,
                hidden: hidden(h),
                levels,
            },
        );
    }
    sort_nodes(&mut nodes);
    nodes
}

/// The state the Data pane's rows need, computed once per frame.
struct DataCx {
    /// The selected visual and its snapshot entry.
    sel: Option<(VisualSel, Value)>,
    /// Why fields cannot be added to the selected visual (a reader, a command refused).
    bind: Result<(), String>,
    query: String,
    lang: Lang,
}

/// What a row asks for after the frame (the app is borrowed while drawing).
enum Act {
    Toggle(Field),
    Menu(&'static str, Value),
}

fn data_pane(app: &mut App, ui: &mut Ui) {
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    if title_row(
        ui,
        l.t("Data", "Données"),
        l.t("Collapse the Data pane", "Réduire le volet Données"),
    ) {
        app.prefs.panes.data = false;
        app.save_prefs();
        return;
    }
    ui.horizontal(|ui| {
        let w = (ui.available_width() - 30.0).max(40.0);
        let name = l.t("Search the fields", "Rechercher dans les champs");
        ui.add(
            egui::TextEdit::singleline(&mut app.panes.search)
                .hint_text(l.t("Search", "Rechercher"))
                .desired_width(w),
        )
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, name));
        let tip = if app.panes.show_hidden {
            l.t("Hide hidden fields", "Masquer les champs masqués")
        } else {
            l.t("Show hidden fields", "Afficher les champs masqués")
        };
        let icon = if app.panes.show_hidden {
            Icon::Eye
        } else {
            Icon::EyeOff
        };
        if widgets::icon_toggle(ui, icon, tip, true, app.panes.show_hidden).clicked() {
            app.panes.show_hidden = !app.panes.show_hidden;
        }
    });
    ui.add_space(4.0);
    let Some(snap) = app.loaded.as_ref().map(|x| x.snap.clone()) else {
        return;
    };
    let mut tables: Vec<&Value> = arr(snap.pointer("/model/tables"))
        .iter()
        .take(MAX_ITEMS)
        .collect();
    tables.sort_by_key(|x| x.get("name").and_then(Value::as_str).unwrap_or("").to_lowercase());
    if tables.is_empty() {
        ui.label(
            egui::RichText::new(l.t("The model has no tables.", "Le modèle n'a pas de table."))
                .color(t.text_dim),
        );
        return;
    }
    let cx = DataCx {
        sel: selected(app),
        bind: ribbon::state(app, "visual.bind"),
        query: app.panes.search.trim().to_lowercase(),
        lang: l,
    };
    let menu_states: Vec<(&'static str, Result<(), String>)> =
        COMMANDS.iter().map(|id| (*id, ribbon::state(app, id))).collect();
    let mut acts: Vec<Act> = Vec::new();
    let show_hidden = app.panes.show_hidden;
    let open = &mut app.panes.open;
    // tables left out because they are hidden or show no field (as in Desktop's Data pane)
    let mut left_out = 0usize;
    let mut reveal = false;
    egui::ScrollArea::vertical()
        .id_salt("bicraft-data-tree")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            for tv in tables {
                let table = s(tv, "name").unwrap_or_default();
                let hidden = tv.get("hidden").and_then(Value::as_bool) == Some(true);
                let entered = tv.get("entered_data").and_then(Value::as_bool) == Some(true);
                if hidden && !show_hidden {
                    left_out += 1;
                    continue;
                }
                let tree = table_tree(tv, show_hidden);
                if tree.is_empty() && !show_hidden {
                    // every column and measure hidden: nothing to unroll
                    left_out += 1;
                    continue;
                }
                let q = &cx.query;
                if !q.is_empty()
                    && !table.to_lowercase().contains(q.as_str())
                    && !tree.iter().any(|n| n.matches(q))
                {
                    continue;
                }
                let key = format!("t:{table}");
                let expanded = open.contains(&key) || !q.is_empty();
                let r = tree_row(ui, 0, Icon::Table, &table, expanded, hidden, true, false);
                if r.clicked() || keyboard::tree_toggle(ui, &r, expanded) {
                    toggle(open, &key);
                }
                crate::keyboard::context_menu(&r, |ui| {
                    table_menu(ui, &table, hidden, entered, &menu_states, cx.lang, &mut acts);
                });
                if expanded {
                    nodes_ui(ui, &tree, 1, &table, &cx, open, &menu_states, &mut acts);
                }
            }
            if left_out > 0 && cx.query.is_empty() {
                ui.add_space(8.0);
                let text = format!(
                    "{} ({left_out})",
                    cx.lang.t("Show hidden tables", "Afficher les tables masquées")
                );
                reveal = ui
                    .add(egui::Button::new(egui::RichText::new(text).color(t.text_dim)).frame(false))
                    .clicked();
            }
        });
    if reveal {
        app.panes.show_hidden = true;
    }
    for a in acts {
        match a {
            Act::Toggle(f) => toggle_field(app, &f),
            Act::Menu(id, params) => menu_act(app, id, params),
        }
    }
}

fn toggle(open: &mut BTreeSet<String>, key: &str) {
    if !open.remove(key) {
        open.insert(key.to_string());
    }
}

/// A table, folder or hierarchy row: chevron, icon, name. Click toggles it.
#[allow(clippy::too_many_arguments)]
fn tree_row(
    ui: &mut Ui,
    depth: usize,
    icon: Icon,
    name: &str,
    open: bool,
    hidden: bool,
    strong: bool,
    drag: bool,
) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let sense = if drag {
        Sense::click_and_drag()
    } else {
        Sense::click()
    };
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), sense);
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(4), t.hover);
    }
    let x = rect.min.x + depth as f32 * INDENT;
    let c = pos2(x + 7.0, rect.center().y);
    let ink = if hidden { t.text_faint } else { t.text };
    let p = ui.painter();
    if open {
        widgets::chevron(p, c, 3.5, ink);
    } else {
        p.line_segment(
            [pos2(c.x - 2.0, c.y - 4.0), pos2(c.x + 2.0, c.y)],
            Stroke::new(1.2, ink),
        );
        p.line_segment(
            [pos2(c.x + 2.0, c.y), pos2(c.x - 2.0, c.y + 4.0)],
            Stroke::new(1.2, ink),
        );
    }
    let ir = Rect::from_center_size(pos2(x + 24.0, rect.center().y), vec2(16.0, 16.0));
    icons::paint(p, ir, icon, ink, hidden);
    let font = if strong {
        theme::bold(13.0)
    } else {
        theme::font(13.0)
    };
    p.text(
        pos2(x + 36.0, rect.center().y),
        Align2::LEFT_CENTER,
        clip(name, MAX_LABEL),
        font,
        ink,
    );
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::CollapsingHeader, true, open, name));
    resp
}

#[allow(clippy::too_many_arguments)]
fn nodes_ui(
    ui: &mut Ui,
    nodes: &[Node],
    depth: usize,
    table: &str,
    cx: &DataCx,
    open: &mut BTreeSet<String>,
    states: &[(&'static str, Result<(), String>)],
    acts: &mut Vec<Act>,
) {
    let q = cx.query.as_str();
    for n in nodes {
        if !q.is_empty() && !n.matches(q) {
            continue;
        }
        match n {
            Node::Folder(name, kids) => {
                let key = format!("f:{table}/{depth}/{name}");
                let expanded = open.contains(&key) || !q.is_empty();
                let r = tree_row(ui, depth, Icon::Folder, name, expanded, false, false, false);
                if r.clicked() || keyboard::tree_toggle(ui, &r, expanded) {
                    toggle(open, &key);
                }
                if expanded {
                    nodes_ui(ui, kids, depth + 1, table, cx, open, states, acts);
                }
            }
            Node::Field(f, hidden) => field_row(ui, depth, f, *hidden, cx, states, acts),
            Node::Hierarchy { name, hidden, levels } => {
                let key = format!("h:{table}/{name}");
                let expanded = open.contains(&key) || !q.is_empty();
                let r = tree_row(
                    ui,
                    depth,
                    Icon::Hierarchy,
                    name,
                    expanded,
                    *hidden,
                    false,
                    cx.bind.is_ok(),
                );
                if r.clicked() || keyboard::tree_toggle(ui, &r, expanded) {
                    toggle(open, &key);
                }
                if r.drag_started() && cx.bind.is_ok() {
                    DragAndDrop::set_payload(
                        ui.ctx(),
                        Drag::Hierarchy(Hier {
                            table: table.to_string(),
                            name: name.clone(),
                        }),
                    );
                }
                crate::keyboard::context_menu(&r, |ui| {
                    let p = json!({"table": table, "name": name});
                    let mut hp = p.clone();
                    hp["hidden"] = json!(!*hidden);
                    menu_rows(
                        ui,
                        cx.lang,
                        states,
                        acts,
                        &[
                            ("measure.add", json!({"table": table}), None),
                            ("hierarchy.set", p, Some(cx.lang.t("Rename", "Renommer"))),
                            ("hierarchy.set", hp, Some(hide_text(*hidden, cx.lang))),
                        ],
                    );
                });
                if expanded {
                    for f in levels {
                        field_row(ui, depth + 1, f, *hidden, cx, states, acts);
                    }
                }
            }
        }
    }
}

fn hide_text(hidden: bool, l: Lang) -> &'static str {
    if hidden {
        l.t("Unhide", "Afficher")
    } else {
        l.t("Hide", "Masquer")
    }
}

/// A field's row: its box (bound to the selected visual), its icon and name; dragged, it carries
/// the field; right-clicked, its menu.
fn field_row(
    ui: &mut Ui,
    depth: usize,
    f: &Field,
    hidden: bool,
    cx: &DataCx,
    states: &[(&'static str, Result<(), String>)],
    acts: &mut Vec<Act>,
) {
    let t = Tokens::get(ui.ctx());
    // with nothing selected, the box makes a new visual (Desktop)
    let can = cx.bind.is_ok();
    let checked = cx.sel.as_ref().is_some_and(|(_, v)| !is_bound(v, f).is_empty());
    let id = Id::new(("bicraft-field", &f.table, &f.name, f.measure, depth));
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::hover());
    let sense = if cx.bind.is_ok() {
        Sense::click_and_drag()
    } else {
        Sense::click()
    };
    let resp = ui.interact(rect, id, sense);
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(4), t.hover);
    }
    let x = rect.min.x + depth as f32 * INDENT;
    let ink = if hidden { t.text_faint } else { t.text };
    let p = ui.painter();
    // the box
    let bx = Rect::from_center_size(pos2(x + 8.0, rect.center().y), vec2(13.0, 13.0));
    let box_ink = if can { t.text_dim } else { t.text_faint };
    p.rect_stroke(
        bx,
        CornerRadius::same(2),
        Stroke::new(1.0, box_ink),
        StrokeKind::Inside,
    );
    if checked {
        p.rect_filled(bx.shrink(1.0), CornerRadius::same(2), t.accent);
        icons::paint(p, bx.shrink(1.5), Icon::Check, t.accent_text, false);
    }
    let icon = if f.measure { Icon::Calculator } else { Icon::Column };
    let ir = Rect::from_center_size(pos2(x + 26.0, rect.center().y), vec2(16.0, 16.0));
    icons::paint(p, ir, icon, ink, hidden);
    p.text(
        pos2(x + 38.0, rect.center().y),
        Align2::LEFT_CENTER,
        clip(&f.name, MAX_LABEL),
        theme::font(13.0),
        ink,
    );
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, can, checked, &f.name));
    if resp.drag_started() && cx.bind.is_ok() {
        DragAndDrop::set_payload(ui.ctx(), Drag::Field(f.clone()));
    }
    let resp = match (&cx.bind, cx.sel.is_some()) {
        (Err(why), _) => resp.on_hover_text(format!("{} {why}", cx.lang.t("Read-only:", "Lecture seule :"))),
        (Ok(()), false) => resp.on_hover_text(cx.lang.t(
            "Check it to make a new visual, or drag it onto the page, a visual or a well.",
            "Cochez-le pour créer un visuel, ou glissez-le sur la page, un visuel ou un emplacement.",
        )),
        _ => resp.on_hover_text(&f.text),
    };
    let kind = if f.measure {
        crate::formula::Kind::Measure
    } else {
        crate::formula::Kind::Column
    };
    if resp.clicked() {
        crate::formula::pick(ui.ctx(), kind, Some(&f.table), &f.name, false);
    }
    if resp.clicked() && can {
        acts.push(Act::Toggle(f.clone()));
    }
    crate::keyboard::context_menu(&resp, |ui| {
        crate::formula::menu_entry(ui, kind, &f.table, &f.name);
        let base = json!({"table": f.table, "name": f.name});
        let (rename, set) = if f.measure {
            ("measure.rename", "measure.set")
        } else {
            ("column.rename", "column.set")
        };
        let mut rp = base.clone();
        rp["update_report"] = json!(true);
        let mut hp = base;
        hp["hidden"] = json!(!hidden);
        menu_rows(
            ui,
            cx.lang,
            states,
            acts,
            &[
                ("measure.add", json!({"table": f.table}), None),
                (
                    crate::quick_dialog::QUICK,
                    json!({"base": f.text}),
                    Some(cx.lang.t("New quick measure", "Nouvelle mesure rapide")),
                ),
                (rename, rp, Some(cx.lang.t("Rename", "Renommer"))),
                (set, hp, Some(hide_text(hidden, cx.lang))),
                (
                    crate::tmdl_view::SCRIPT,
                    json!({"objects": [{
                        "kind": if f.measure { "measure" } else { "column" },
                        "table": f.table,
                        "name": f.name,
                    }]}),
                    None,
                ),
            ],
        );
        // New group (Edit groups on a group column): the Groups dialog (`group_dialog.rs`)
        if !f.measure {
            menu_rows(
                ui,
                cx.lang,
                states,
                acts,
                &[(
                    crate::group_dialog::GROUP,
                    json!({"table": f.table, "column": f.name}),
                    Some(cx.lang.t("New group", "Nouveau groupe")),
                )],
            );
        }
    });
}

/// A table's right-click menu.
fn table_menu(
    ui: &mut Ui,
    table: &str,
    hidden: bool,
    entered: bool,
    states: &[(&'static str, Result<(), String>)],
    l: Lang,
    acts: &mut Vec<Act>,
) {
    crate::formula::menu_entry(ui, crate::formula::Kind::Table, table, table);
    // a table typed in (Enter data): Edit data opens its grid, as in Desktop
    if entered {
        menu_rows(
            ui,
            l,
            states,
            acts,
            &[("table.entered_data", json!({"name": table}), None)],
        );
    }
    menu_rows(
        ui,
        l,
        states,
        acts,
        &[
            (
                "model.refresh",
                json!({"tables": [table]}),
                Some(crate::menus::refresh_text(l)),
            ),
            ("measure.add", json!({"table": table}), None),
            (
                crate::quick_dialog::QUICK,
                json!({"table": table}),
                Some(l.t("New quick measure", "Nouvelle mesure rapide")),
            ),
            (
                "table.rename",
                json!({"name": table, "update_report": true}),
                Some(l.t("Rename", "Renommer")),
            ),
            (
                "table.set",
                json!({"name": table, "hidden": !hidden}),
                Some(hide_text(hidden, l)),
            ),
            (
                "table.set_refresh_policy",
                json!({"table": table}),
                Some(l.t("Incremental refresh", "Actualisation incrémentielle")),
            ),
            (
                crate::tmdl_view::SCRIPT,
                json!({"objects": [{"kind": "table", "name": table}]}),
                None,
            ),
        ],
    );
}

/// Menu items (command, its parameters, a label other than the registry's); a click queues it.
fn menu_rows(
    ui: &mut Ui,
    l: Lang,
    states: &[(&'static str, Result<(), String>)],
    acts: &mut Vec<Act>,
    items: &[(&'static str, Value, Option<&'static str>)],
) {
    ui.set_min_width(170.0);
    for (id, params, label) in items {
        let Some(c) = crate::registry::find(id) else {
            continue;
        };
        let st = states
            .iter()
            .find(|(x, _)| x == id)
            .map_or_else(|| Err("not available".to_string()), |(_, s)| s.clone());
        let text = label.unwrap_or_else(|| c.text(l));
        if crate::menus::item(ui, text, &st, l) {
            acts.push(Act::Menu(c.id, params.clone()));
        }
    }
}

/// A Data pane menu's action: Hide / Unhide run at once, the others as their buttons do (the
/// form asks for the new name or the measure's expression).
fn menu_act(app: &mut App, id: &'static str, params: Value) {
    if params.get("hidden").is_some() {
        exec(app, id, params);
    } else {
        ribbon::invoke(app, id, params);
    }
}

/// The box of field `f` with the selected visual: bound → unbound from every role it is in, else
/// bound to the visual's default role.
fn toggle_field(app: &mut App, f: &Field) {
    let Some((sel, v)) = selected(app) else {
        new_visual(app, f, None);
        return;
    };
    let hits = is_bound(&v, f);
    if hits.is_empty() {
        bind_default(app, &sel, &v, f);
        return;
    }
    // the last first: positions of earlier ones do not move
    for (role, text, index) in hits.into_iter().rev() {
        let mut p = json!({"page": sel.page, "visual": sel.visual, "role": role});
        match text.filter(|t| !t.is_empty() && t != "(other field)") {
            Some(t) => p["field"] = json!(t),
            None => p["index"] = json!(index),
        }
        exec(app, "visual.unbind", p);
    }
}

// ----- Build pane -----

fn build_pane(app: &mut App, ui: &mut Ui) {
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    if title_row(
        ui,
        l.t("Visualizations", "Visualisations"),
        l.t(
            "Collapse the Visualizations pane",
            "Réduire le volet Visualisations",
        ),
    ) {
        app.prefs.panes.build = false;
        app.save_prefs();
        return;
    }
    // Desktop's Build visual and Format visual sub-tabs, under the active view's name
    let tab = crate::format_pane::tabs(app, ui);
    let Some((sel, v)) = selected(app) else {
        let (en, fr) = match tab {
            crate::format_pane::Tab::Build => (
                "Select a visual on the page to edit its fields.",
                "Sélectionnez un visuel de la page pour modifier ses champs.",
            ),
            crate::format_pane::Tab::Format => (
                "Select a visual on the page to format it.",
                "Sélectionnez un visuel de la page pour le mettre en forme.",
            ),
            crate::format_pane::Tab::Analytics => (
                "Select a chart on the page to add analytics to it.",
                "Sélectionnez un graphique de la page pour lui ajouter des analyses.",
            ),
        };
        // nothing selected: Desktop's Format pane formats the page (its Filters pane cards)
        if tab == crate::format_pane::Tab::Format && app.loaded.is_some() {
            crate::filter_pane_ui::page_cards(app, ui);
            return;
        }
        // Desktop's gallery is there whatever is selected: a click makes a new visual
        if tab == crate::format_pane::Tab::Build {
            gallery(app, ui, None, "");
            ui.add_space(6.0);
        }
        ui.label(egui::RichText::new(l.t(en, fr)).color(t.text_dim));
        return;
    };
    if tab == crate::format_pane::Tab::Format {
        crate::format_pane::pane(app, ui, &sel, &v);
        return;
    }
    if tab == crate::format_pane::Tab::Analytics {
        crate::analytics_pane::pane(app, ui, &sel, &v);
        return;
    }
    let Some(vtype) = s(&v, "type") else {
        ui.label(
            egui::RichText::new(l.t("A group has no fields.", "Un groupe n'a pas de champ."))
                .color(t.text_dim),
        );
        return;
    };
    gallery(app, ui, Some(&sel), &vtype);
    ui.add_space(6.0);
    let edit = ribbon::state(app, "visual.bind");
    let agg_state = ribbon::state(app, "visual.set_aggregation");
    let roles = roles_of(&v);
    let items = items_of(&v);
    let model = app.loaded.as_ref().map(|x| x.snap.clone());
    let mut renaming = app.panes.renaming.take();
    let payload = dragging(ui.ctx());
    let mut act: Option<(&'static str, Value)> = None;
    egui::ScrollArea::vertical()
        .id_salt("bicraft-build-wells")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if roles.is_empty() {
                ui.label(
                    egui::RichText::new(
                        l.t("This visual takes no fields.", "Ce visuel ne prend pas de champ."),
                    )
                    .color(t.text_dim),
                );
            }
            for role in &roles {
                let in_role: Vec<&Item> = items
                    .iter()
                    .filter(|i| i.role.eq_ignore_ascii_case(&role.name))
                    .collect();
                let cap = caption(&vtype, &role.name, l);
                let well = Well {
                    sel: &sel,
                    role,
                    cap: &cap,
                    items: &in_role,
                    edit: &edit,
                    agg: &agg_state,
                    payload: payload.as_ref(),
                    model: model.as_deref(),
                    vtype: &vtype,
                    lang: l,
                };
                if let Some(a) = well_ui(ui, &well, &mut renaming) {
                    act = Some(a);
                }
                ui.add_space(6.0);
            }
        });
    app.panes.renaming = renaming;
    match act {
        // the sparkline dialog is the command's form
        Some((id @ ("visual.add_sparkline" | "visual.edit_sparkline"), p)) => crate::form::open(app, id, p),
        Some((id, p)) => {
            exec(app, id, p);
        }
        None => {}
    }
}

/// The common visual types, in the picker's order, with BIcraft's names (EN, FR).
const COMMON_TYPES: &[(&str, &str, &str)] = &[
    ("tableEx", "Table", "Tableau"),
    ("pivotTable", "Matrix", "Matrice"),
    ("card", "Card", "Carte"),
    ("multiRowCard", "Multi-row card", "Carte multiligne"),
    ("slicer", "Slicer", "Sélecteur"),
    (
        "clusteredColumnChart",
        "Clustered column chart",
        "Colonnes groupées",
    ),
    ("columnChart", "Stacked column chart", "Colonnes empilées"),
    ("clusteredBarChart", "Clustered bar chart", "Barres groupées"),
    ("barChart", "Stacked bar chart", "Barres empilées"),
    ("lineChart", "Line chart", "Courbes"),
    ("areaChart", "Area chart", "Aires"),
    (
        "lineClusteredColumnComboChart",
        "Line and clustered column chart",
        "Courbes et colonnes groupées",
    ),
    (
        "lineStackedColumnComboChart",
        "Line and stacked column chart",
        "Courbes et colonnes empilées",
    ),
    ("pieChart", "Pie chart", "Secteurs"),
    ("donutChart", "Donut chart", "Anneau"),
    ("scatterChart", "Scatter chart", "Nuage de points"),
    ("treemap", "Treemap", "Carte proportionnelle"),
    ("funnel", "Funnel", "Entonnoir"),
    ("waterfallChart", "Waterfall chart", "Cascade"),
    ("gauge", "Gauge", "Jauge"),
    ("kpi", "KPI", "Indicateur clé"),
];

/// A visual type's name in the picker (`None`: not one of [`COMMON_TYPES`]).
pub fn type_label(name: &str, l: Lang) -> Option<&'static str> {
    COMMON_TYPES
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|(_, en, fr)| l.t(en, fr))
}

/// The built-in types that take fields, in the gallery's order: the common ones in
/// [`COMMON_TYPES`]' order, then the others by name.
fn built_in_types(app: &App, l: Lang) -> Vec<(String, String)> {
    let mut types: Vec<(String, String)> = app
        .loaded
        .as_ref()
        .map(|x| {
            arr(x.snap.pointer("/report/visual_types"))
                .iter()
                .take(200)
                .filter_map(|t| {
                    let n = s(t, "name")?;
                    let d = type_label(&n, l)
                        .map(str::to_string)
                        .or_else(|| s(t, "display_name").filter(|d| !d.is_empty()))
                        .unwrap_or_else(|| n.clone());
                    Some((n, d))
                })
                .collect()
        })
        .unwrap_or_default();
    types.sort_by_key(|(n, d)| {
        let rank = COMMON_TYPES
            .iter()
            .position(|(x, _, _)| x == n)
            .unwrap_or(COMMON_TYPES.len());
        (rank, d.to_lowercase())
    });
    types
}

/// The Build view's gallery of built-in types, as Desktop's grid of icons: fixed cells, as many
/// a row as the pane holds (five at least), the selected visual's type marked. A click switches the selected visual's type, or adds a new
/// visual of that type when nothing is selected.
fn gallery(app: &mut App, ui: &mut Ui, sel: Option<&VisualSel>, current: &str) {
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    let types = built_in_types(app, l);
    if types.is_empty() {
        return;
    }
    // BIcraft's visuals and the report's own custom visuals after the built-in ones, with their
    // icon
    let customs = crate::custom_visuals::gallery_entries(app);
    // BIcraft's visuals with a drawing of their own paint it; the others show their package's icon
    let drawn: Vec<bool> = customs
        .iter()
        .map(|c| c.bicraft && bicraft_visuals::icons::bicraft_visual_icon(&c.name).is_some())
        .collect();
    let icons: Vec<Option<egui::TextureId>> = customs
        .iter()
        .zip(&drawn)
        .map(|(c, &drawn)| {
            c.icon
                .as_deref()
                .filter(|_| !drawn)
                .and_then(|id| crate::custom_visuals::icon(app, ui.ctx(), id))
        })
        .collect();
    let state = if sel.is_some() {
        ribbon::state(app, "visual.set_type")
    } else {
        ribbon::state(app, "visual.add")
    };
    let enabled = state.is_ok();
    // fixed cells a little larger than the icon, as many a row as the pane's width holds
    let gap = 2.0;
    let cell = 30.0;
    let cols = (((ui.available_width() + gap) / (cell + gap)).floor() as usize).max(5);
    let w = cell;
    let mut picked: Option<String> = None;
    // Desktop's gallery is tall; our panes are shorter, so it scrolls and keeps room for the
    // wells under it (a third of the pane at most).
    let h = (ui.available_height() * 0.34).clamp(80.0, 400.0);
    egui::ScrollArea::vertical()
        .id_salt("bicraft-visual-gallery")
        .max_height(h)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            egui::Grid::new("bicraft-visual-gallery-grid")
                .num_columns(cols)
                .min_col_width(cell)
                .spacing(vec2(gap, gap))
                .show(ui, |ui| {
                    for (i, (n, d)) in types.iter().enumerate() {
                        let (rect, resp) = ui.allocate_exact_size(
                            vec2(w, 28.0),
                            if enabled { Sense::click() } else { Sense::hover() },
                        );
                        let on = n == current;
                        if on {
                            ui.painter().rect_filled(rect, CornerRadius::same(4), t.selected);
                            ui.painter().rect_stroke(
                                rect,
                                CornerRadius::same(4),
                                Stroke::new(1.0, t.border),
                                StrokeKind::Inside,
                            );
                        } else if resp.hovered() && enabled {
                            ui.painter().rect_filled(rect, CornerRadius::same(4), t.hover);
                        }
                        icons::paint_visual_type(
                            ui.painter(),
                            Rect::from_center_size(rect.center(), vec2(22.0, 22.0)),
                            n,
                            t.text,
                            !enabled,
                        );
                        resp.widget_info(|| {
                            egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, on, d.as_str())
                        });
                        let tip = if d == n { n.clone() } else { format!("{d}\n{n}") };
                        let resp = resp.on_hover_text(tip);
                        if enabled {
                            if resp.clicked() {
                                picked = Some(n.clone());
                            }
                        } else if let Err(why) = &state {
                            resp.on_disabled_hover_text(format!(
                                "{} {why}",
                                l.t("Unavailable:", "Indisponible :")
                            ));
                        }
                        if (i + 1) % cols == 0 {
                            ui.end_row();
                        }
                    }
                    let mut i = types.len();
                    for ((c, tex), &drawn) in customs.iter().zip(&icons).zip(&drawn) {
                        let (rect, resp) = ui.allocate_exact_size(
                            vec2(w, 28.0),
                            if enabled { Sense::click() } else { Sense::hover() },
                        );
                        let on = c.visual_type.eq_ignore_ascii_case(current);
                        if on {
                            ui.painter().rect_filled(rect, CornerRadius::same(4), t.selected);
                            ui.painter().rect_stroke(
                                rect,
                                CornerRadius::same(4),
                                Stroke::new(1.0, t.border),
                                StrokeKind::Inside,
                            );
                        } else if resp.hovered() && enabled {
                            ui.painter().rect_filled(rect, CornerRadius::same(4), t.hover);
                        }
                        let icon_rect = Rect::from_center_size(rect.center(), vec2(20.0, 20.0));
                        match tex {
                            // BIcraft's visuals: their drawing in the theme's ink, as the
                            // built-in types (their package's icon is the same drawing)
                            _ if drawn => icons::paint_visual_type(
                                ui.painter(),
                                Rect::from_center_size(rect.center(), vec2(22.0, 22.0)),
                                &c.name,
                                t.text,
                                !enabled,
                            ),
                            Some(tex) => {
                                let tint = if enabled {
                                    egui::Color32::WHITE
                                } else {
                                    egui::Color32::from_white_alpha(110)
                                };
                                ui.painter().image(
                                    *tex,
                                    icon_rect,
                                    Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                    tint,
                                );
                            }
                            // no icon in the package: the generic picture of a visual
                            None => {
                                icons::paint_visual_type(ui.painter(), icon_rect, &c.name, t.text, !enabled)
                            }
                        }
                        let label = c.label.clone();
                        resp.widget_info(|| {
                            egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, on, label.as_str())
                        });
                        let resp = resp.on_hover_text(format!(
                            "{}\n{}",
                            c.label,
                            if c.bicraft {
                                l.t("BIcraft visual", "Visuel BIcraft")
                            } else {
                                l.t("Custom visual", "Visuel personnalisé")
                            }
                        ));
                        if enabled {
                            if resp.clicked() {
                                picked = Some(c.name.clone());
                            }
                        } else if let Err(why) = &state {
                            resp.on_disabled_hover_text(format!(
                                "{} {why}",
                                l.t("Unavailable:", "Indisponible :")
                            ));
                        }
                        i += 1;
                        if i.is_multiple_of(cols) {
                            ui.end_row();
                        }
                    }
                    // Desktop's "…" cell: Import a visual from a file, Remove a custom visual
                    let (rect, resp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(rect, CornerRadius::same(4), t.hover);
                    }
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "…",
                        egui::FontId::proportional(16.0),
                        t.text,
                    );
                    let more = l.t("More visuals", "Plus de visuels");
                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, more));
                    let resp = resp.on_hover_text(more);
                    egui::Popup::menu(&resp).show(|ui| crate::custom_visuals::more_menu(app, ui));
                    i += 1;
                    if !i.is_multiple_of(cols) {
                        ui.end_row();
                    }
                });
        });
    let Some(n) = picked else { return };
    match sel {
        Some(sel) => {
            exec(
                app,
                "visual.set_type",
                json!({"page": sel.page, "visual": sel.visual, "visual_type": n}),
            );
        }
        None => add_visual(app, &n),
    }
}

/// Adds a visual of type `ty` picked in the gallery with nothing selected: at the first free spot
/// of the page shown, then selects it.
fn add_visual(app: &mut App, ty: &str) {
    if let Err(why) = ribbon::state(app, "visual.add") {
        app.status = format!("{} {why}", app.lang.t("Unavailable:", "Indisponible :"));
        return;
    }
    let Some(page) = app.loaded.as_ref().and_then(|l| {
        l.snap
            .pointer(&format!("/report/pages/{}", app.active_page()))
            .cloned()
    }) else {
        return;
    };
    let Some(name) = s(&page, "name") else { return };
    let (w, h) = NEW_SIZE;
    let (x, y) = free_spot(&page, w, h);
    let p = json!({"page": name, "visual_type": ty, "x": x.round(), "y": y.round(),
                   "width": w, "height": h});
    app.run(
        Call {
            id: "visual.add",
            params: p,
        },
        Then::SelectVisual,
    );
}

/// What one well needs to draw itself.
struct Well<'a> {
    sel: &'a VisualSel,
    role: &'a Role,
    cap: &'a str,
    items: &'a [&'a Item],
    edit: &'a Result<(), String>,
    agg: &'a Result<(), String>,
    payload: Option<&'a Drag>,
    model: Option<&'a Value>,
    /// The visual's type (a matrix offers column and row totals).
    vtype: &'a str,
    lang: Lang,
}

/// Whether column `table[name]` of the snapshot's model holds numbers.
fn numeric_column(model: Option<&Value>, table: Option<&str>, name: Option<&str>) -> bool {
    let (Some(m), Some(t), Some(n)) = (model, table, name) else {
        return false;
    };
    arr(m.pointer("/model/tables"))
        .iter()
        .find(|x| x.get("name").and_then(Value::as_str) == Some(t))
        .and_then(|x| {
            arr(x.get("columns"))
                .iter()
                .find(|c| c.get("name").and_then(Value::as_str) == Some(n))
                .and_then(|c| c.get("data_type").and_then(Value::as_str))
                .map(|d| matches!(d, "int64" | "double" | "decimal"))
        })
        .unwrap_or(false)
}

/// One well: its caption, its fields (drag, aggregation, ×) and the drop target. Returns the
/// command to run.
fn well_ui(ui: &mut Ui, w: &Well<'_>, rename: &mut Option<Rename>) -> Option<(&'static str, Value)> {
    let t = Tokens::get(ui.ctx());
    let l = w.lang;
    let mut act = None;
    let top = ui.cursor().min.y;
    // the caption: the well's name in the accessibility tree
    let (cap_rect, cap_resp) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
    ui.painter().text(
        pos2(cap_rect.min.x, cap_rect.center().y),
        Align2::LEFT_CENTER,
        w.cap,
        theme::bold(12.0),
        t.text_dim,
    );
    cap_resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, w.cap));
    let _ = cap_resp.on_hover_text(&w.role.name);
    let mut rows: Vec<Rect> = Vec::new();
    for (i, it) in w.items.iter().enumerate() {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H + 2.0), Sense::hover());
        rows.push(rect);
        let id = Id::new(("bicraft-well-item", &w.sel.visual, &w.role.name, i));
        let resp = ui.interact(
            rect,
            id,
            if w.edit.is_ok() {
                Sense::click_and_drag()
            } else {
                Sense::hover()
            },
        );
        let fill = if resp.hovered() { t.hover } else { t.field_bg };
        let body = rect.shrink2(vec2(0.0, 1.0));
        ui.painter().rect_filled(body, CornerRadius::same(4), fill);
        ui.painter().rect_stroke(
            body,
            CornerRadius::same(4),
            Stroke::new(1.0, t.border),
            StrokeKind::Inside,
        );
        let icon = if it.kind == "measure" {
            Icon::Calculator
        } else if it.kind == "sparkline" {
            Icon::Chart
        } else if it.kind == "other" || it.kind == "hierarchy_level" {
            Icon::Hierarchy
        } else {
            Icon::Column
        };
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(body.min.x + 12.0, body.center().y), vec2(14.0, 14.0)),
            icon,
            t.text,
            false,
        );
        // right side: × then the aggregation menu
        let x_rect = Rect::from_center_size(pos2(body.max.x - 12.0, body.center().y), vec2(18.0, 18.0));
        // a grouping well shows its columns as they are: no aggregation there
        let column =
            matches!(it.kind.as_str(), "column" | "aggregation") && w.role.kind != RoleKind::Grouping;
        let agg_rect = Rect::from_min_max(
            pos2(x_rect.min.x - 92.0, body.min.y + 2.0),
            pos2(x_rect.min.x - 4.0, body.max.y - 2.0),
        );
        let label_max = if column { agg_rect.min.x } else { x_rect.min.x } - 4.0;
        let label_rect = Rect::from_min_max(pos2(body.min.x + 24.0, body.min.y), pos2(label_max, body.max.y));
        let field = (!matches!(it.kind.as_str(), "other" | "hierarchy_level" | "sparkline")
            && !it.text.is_empty())
        .then(|| it.text.clone());
        if resp.clicked()
            && it.kind == "measure"
            && let Some(n) = &it.name
        {
            crate::formula::pick(
                ui.ctx(),
                crate::formula::Kind::Measure,
                it.table.as_deref(),
                n,
                false,
            );
        }
        let target = |p: &mut Value| match &field {
            Some(f) => p["field"] = json!(f),
            None => p["index"] = json!(i),
        };
        let here =
            |r: &Rename| r.visual == w.sel.visual && r.role.eq_ignore_ascii_case(&it.role) && r.index == i;
        let renaming = rename.as_ref().is_some_and(here);
        let mut cut = false;
        if renaming && let Some(r) = rename.as_mut() {
            // "Rename for this visual": Enter or leaving the box keeps the name, Escape drops it
            let te_id = id.with("rename");
            let te = ui.put(
                label_rect,
                egui::TextEdit::singleline(&mut r.text)
                    .id(te_id)
                    .font(theme::font(12.0))
                    .margin(egui::Margin::symmetric(2, 2)),
            );
            let rl = format!(
                "{} {}",
                l.t("Rename for this visual:", "Renommer pour ce visuel :"),
                it.label
            );
            te.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &rl));
            if !r.focused {
                te.request_focus();
                r.focused = true;
            }
            let escape = ui.input(|x| x.key_pressed(egui::Key::Escape));
            if escape {
                *rename = None;
            } else if te.lost_focus() {
                let name = r.text.trim().to_string();
                if name != it.label {
                    let mut p = json!({"page": w.sel.page, "visual": w.sel.visual, "role": it.role,
                                       "name": if name.is_empty() { Value::Null } else { json!(name) }});
                    target(&mut p);
                    act = Some(("visual.rename_field", p));
                }
                *rename = None;
            }
        } else {
            // cut with an ellipsis where it does not fit, shown whole on hover
            let shown = crate::pq_deps::fit_text(ui, &it.label, theme::font(12.0), label_rect.width());
            cut = shown != it.label;
            ui.painter().with_clip_rect(label_rect).text(
                pos2(label_rect.min.x, body.center().y),
                Align2::LEFT_CENTER,
                shown,
                theme::font(12.0),
                t.text,
            );
        }
        let full = format!("{} {} {}", it.label, l.t("in", "dans"), w.cap);
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, w.edit.is_ok(), &full));
        let resp = if cut { resp.on_hover_text(&it.label) } else { resp };
        let start_rename = |rename: &mut Option<Rename>| {
            *rename = Some(Rename {
                visual: w.sel.visual.clone(),
                role: it.role.clone(),
                index: i,
                text: it.label.clone(),
                focused: false,
            });
        };
        if w.edit.is_ok() && resp.double_clicked() {
            start_rename(rename);
        }
        if w.edit.is_ok() {
            let mut menu_rename = false;
            let mut menu_restore = false;
            let mut menu_show_as: Option<&'static str> = None;
            let mut menu_show_all: Option<bool> = None;
            let mut menu_spark = false;
            let value = matches!(it.kind.as_str(), "measure" | "aggregation");
            let category = matches!(it.kind.as_str(), "column" | "hierarchy_level");
            // Desktop's "Add a sparkline" on a table's or matrix's value, "Edit sparkline" on one
            let tabular = matches!(w.vtype, "tableEx" | "pivotTable");
            crate::keyboard::context_menu(&resp, |ui| {
                ui.set_min_width(170.0);
                if ui
                    .button(l.t("Rename for this visual", "Renommer pour ce visuel"))
                    .clicked()
                {
                    menu_rename = true;
                    ui.close();
                }
                if it.display_name.is_some()
                    && ui
                        .button(l.t("Restore the field's name", "Rétablir le nom du champ"))
                        .clicked()
                {
                    menu_restore = true;
                    ui.close();
                }
                // Desktop's "Show value as" (a value) and "Show items with no data" (a category)
                if value {
                    ui.separator();
                    ui.menu_button(l.t("Show value as", "Afficher la valeur comme"), |ui| {
                        ui.set_min_width(190.0);
                        for (code, en, fr, matrix) in SHOW_VALUE_AS {
                            if *matrix && w.vtype != "pivotTable" {
                                continue;
                            }
                            let on = it.show_as.as_deref().unwrap_or("none") == *code;
                            if ui.add(egui::Button::selectable(on, l.t(en, fr))).clicked() {
                                if !on {
                                    menu_show_as = Some(code);
                                }
                                ui.close();
                            }
                        }
                    });
                }
                if (value && tabular) || it.spark.is_some() {
                    ui.separator();
                    let label = if it.spark.is_some() {
                        l.t("Edit sparkline…", "Modifier le graphique sparkline…")
                    } else {
                        l.t("Add a sparkline…", "Ajouter un graphique sparkline…")
                    };
                    if ui.button(label).clicked() {
                        menu_spark = true;
                        ui.close();
                    }
                }
                if category {
                    ui.separator();
                    let mut on = it.show_all;
                    if ui
                        .checkbox(
                            &mut on,
                            l.t("Show items with no data", "Afficher les éléments sans données"),
                        )
                        .clicked()
                    {
                        menu_show_all = Some(on);
                        ui.close();
                    }
                }
            });
            if menu_spark {
                // a form: the Y axis prefilled, the X axis and the look chosen there
                act = Some(match it.spark {
                    Some(i) => (
                        "visual.edit_sparkline",
                        json!({"page": w.sel.page, "visual": w.sel.visual, "index": i}),
                    ),
                    None => (
                        "visual.add_sparkline",
                        json!({"page": w.sel.page, "visual": w.sel.visual, "y": it.text}),
                    ),
                });
            }
            if let Some(code) = menu_show_as {
                let mut p = json!({"page": w.sel.page, "visual": w.sel.visual, "role": it.role, "as": code});
                target(&mut p);
                act = Some(("visual.show_value_as", p));
            }
            if let Some(show) = menu_show_all {
                act = Some((
                    "visual.show_items_no_data",
                    json!({"page": w.sel.page, "visual": w.sel.visual, "role": it.role, "show": show}),
                ));
            }
            if menu_rename {
                start_rename(rename);
            }
            if menu_restore {
                let mut p =
                    json!({"page": w.sel.page, "visual": w.sel.visual, "role": it.role, "name": null});
                target(&mut p);
                act = Some(("visual.rename_field", p));
            }
        }
        if w.edit.is_ok() {
            if resp.drag_started() {
                DragAndDrop::set_payload(
                    ui.ctx(),
                    Drag::Bound(Bound {
                        page: w.sel.page.clone(),
                        visual: w.sel.visual.clone(),
                        role: it.role.clone(),
                        index: i,
                        field: field.clone(),
                        label: it.label.clone(),
                    }),
                );
            }
            let remove_tip = format!(
                "{} {} {} {}",
                l.t("Remove", "Retirer"),
                it.label,
                l.t("from", "de"),
                w.cap
            );
            let xr = ui
                .interact(x_rect, id.with("x"), Sense::click())
                .on_hover_text(&remove_tip);
            if xr.hovered() {
                ui.painter().rect_filled(x_rect, CornerRadius::same(4), t.pressed);
            }
            icons::paint(ui.painter(), x_rect.shrink(4.0), Icon::Close, t.text_dim, false);
            xr.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &remove_tip));
            if xr.clicked() {
                let mut p = json!({"page": w.sel.page, "visual": w.sel.visual, "role": it.role});
                target(&mut p);
                act = Some(("visual.unbind", p));
            }
        }
        if column {
            let numeric = numeric_column(w.model, it.table.as_deref(), it.name.as_deref());
            let current = aggregation_text(it.function.as_deref(), l);
            let enabled = w.agg.is_ok() && w.edit.is_ok();
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(agg_rect));
            child.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
            child.add_enabled_ui(enabled, |ui| {
                let mb = ui.menu_button(egui::RichText::new(clip(current, 18)).size(11.0), |ui| {
                    ui.set_min_width(170.0);
                    for (code, en, fr, numbers) in AGGREGATIONS {
                        if *code == "none" && w.role.kind == RoleKind::Measure {
                            continue;
                        }
                        let on = it
                            .function
                            .as_deref()
                            .unwrap_or("none")
                            .eq_ignore_ascii_case(code);
                        let ok = !*numbers || numeric;
                        if ui
                            .add_enabled(ok, egui::Button::selectable(on, l.t(en, fr)))
                            .clicked()
                            && !on
                        {
                            let mut p = json!({"page": w.sel.page, "visual": w.sel.visual, "role": it.role,
                                               "aggregation": code});
                            target(&mut p);
                            act = Some(("visual.set_aggregation", p));
                        }
                    }
                });
                let label = format!("{} {}", l.t("Summarize", "Synthétiser"), it.label);
                mb.response
                    .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, enabled, &label));
            });
        }
    }
    // the empty slot / the end of the well
    // the same height while dragging: the wells never move under the pointer
    let slot_h = if w.items.is_empty() { ROW_H } else { 2.0 };
    let (slot, _) = ui.allocate_exact_size(vec2(ui.available_width(), slot_h), Sense::hover());
    let well_rect = Rect::from_min_max(pos2(slot.min.x, top), slot.max);
    let can_take = match w.payload {
        Some(Drag::Field(f)) => w.edit.is_ok() && (!f.measure || w.role.kind != RoleKind::Grouping),
        Some(Drag::Bound(b)) => {
            w.edit.is_ok()
                && b.visual == w.sel.visual
                && !(w.role.kind == RoleKind::Grouping
                    && b.field.as_deref().is_some_and(|f| f.starts_with('[')))
        }
        Some(Drag::Hierarchy(_)) => w.edit.is_ok() && w.role.kind != RoleKind::Measure,
        None => false,
    };
    let hovered = can_take && ui.rect_contains_pointer(well_rect);
    let full = w.role.max.is_some_and(|m| w.items.len() >= m);
    let slot_text = if w.items.is_empty() {
        l.t("Add data fields here", "Ajoutez des champs ici")
    } else if full {
        ""
    } else {
        l.t("Drop to add", "Déposer pour ajouter")
    };
    if w.items.is_empty() {
        let stroke = if hovered {
            Stroke::new(1.5, t.ring)
        } else {
            Stroke::new(1.0, t.border)
        };
        ui.painter().rect_stroke(
            slot.shrink2(vec2(0.0, 1.0)),
            CornerRadius::same(4),
            stroke,
            StrokeKind::Inside,
        );
        ui.painter().text(
            pos2(slot.min.x + 8.0, slot.center().y),
            Align2::LEFT_CENTER,
            slot_text,
            theme::font(11.0),
            t.text_faint,
        );
    }
    if can_take {
        // the drop target: an outline around the well, the insertion line where it would go
        let outline = if hovered { t.ring } else { t.accent_soft };
        ui.painter().rect_stroke(
            well_rect.expand(2.0),
            CornerRadius::same(5),
            Stroke::new(if hovered { 2.0 } else { 1.0 }, outline),
            StrokeKind::Outside,
        );
    }
    if hovered && let Some(pos) = pointer(ui) {
        let slot_i = rows.iter().filter(|r| r.center().y < pos.y).count();
        let y = rows
            .get(slot_i)
            .map_or_else(|| rows.last().map_or(slot.min.y, |r| r.max.y), |r| r.min.y);
        ui.painter().line_segment(
            [pos2(well_rect.min.x, y), pos2(well_rect.max.x, y)],
            Stroke::new(2.0, t.ring),
        );
        if released(ui)
            && let Some(d) = DragAndDrop::take_payload::<Drag>(ui.ctx())
        {
            act = drop_on_well(w, &d, slot_i);
        }
    }
    act
}

/// The command a drop on well `w` at position `slot` runs.
fn drop_on_well(w: &Well<'_>, d: &Drag, slot: usize) -> Option<(&'static str, Value)> {
    match d {
        Drag::Field(f) => {
            let text = text_for_role(w.role, f);
            Some((
                "visual.bind",
                json!({"page": w.sel.page, "visual": w.sel.visual, "role": w.role.name, "field": text,
                       "index": slot.min(w.items.len())}),
            ))
        }
        Drag::Bound(b) => {
            let same = b.role.eq_ignore_ascii_case(&w.role.name);
            // the engine takes the field out first: a later slot of its own role moves up one
            let to = if same && slot > b.index { slot - 1 } else { slot };
            if same && to == b.index {
                return None;
            }
            let mut p = json!({"page": b.page, "visual": b.visual, "role": b.role,
                               "to_role": w.role.name, "to_index": to});
            match &b.field {
                Some(f) => p["field"] = json!(f),
                None => p["index"] = json!(b.index),
            }
            Some(("visual.move_field", p))
        }
        Drag::Hierarchy(h) => Some((
            "visual.bind_hierarchy",
            json!({"page": w.sel.page, "visual": w.sel.visual, "role": w.role.name, "hierarchy": h.name,
                   "table": h.table, "index": slot.min(w.items.len())}),
        )),
    }
}

// ----- the canvas and the end of the frame -----

/// The page's drop target (called with the page's response): a field dragged over a visual
/// outlines it; dropped, it is bound to that visual's default role and the visual is selected.
pub fn canvas(app: &mut App, ui: &Ui, resp: &egui::Response, rect: Rect, zoom: f32) {
    let Some(d) = dragging(ui.ctx()).filter(|d| !matches!(d, Drag::Bound(_))) else {
        return;
    };
    if ribbon::state(app, "visual.bind").is_err() || !resp.contains_pointer() {
        return;
    }
    let Some(pos) = pointer(ui) else { return };
    let (x, y) = ((pos.x - rect.min.x) / zoom, (pos.y - rect.min.y) / zoom);
    let t = Tokens::of(app.prefs.theme);
    let Some(sel) = crate::menus::visual_at(app, x, y) else {
        // an empty part of the page: a field makes a new visual there
        let Drag::Field(f) = d else { return };
        if ribbon::state(app, "visual.add").is_err() {
            return;
        }
        let (w, h) = NEW_SIZE;
        ui.painter().rect_stroke(
            Rect::from_min_size(pos, vec2(w, h) * zoom).intersect(rect),
            CornerRadius::same(2),
            Stroke::new(1.5, t.ring),
            StrokeKind::Outside,
        );
        if released(ui) && DragAndDrop::take_payload::<Drag>(ui.ctx()).is_some() {
            new_visual(app, &f, Some((x, y)));
        }
        return;
    };
    let Some(v) = visual_value(app, &sel) else { return };
    let f = match d {
        Drag::Field(f) => f,
        Drag::Hierarchy(h) => {
            let n = |k: &str| v.get(k).and_then(Value::as_f64).unwrap_or(0.0) as f32;
            let r = Rect::from_min_size(
                rect.min + vec2(n("x"), n("y")) * zoom,
                vec2(n("width"), n("height")) * zoom,
            );
            ui.painter().rect_stroke(
                r.intersect(rect),
                CornerRadius::same(2),
                Stroke::new(2.0, t.ring),
                StrokeKind::Outside,
            );
            if released(ui) && DragAndDrop::take_payload::<Drag>(ui.ctx()).is_some() {
                app.cmd.visual = Some(sel.clone());
                // the first role that takes columns
                match roles_of(&v).into_iter().find(|r| r.kind != RoleKind::Measure) {
                    Some(role) => {
                        exec(
                            app,
                            "visual.bind_hierarchy",
                            json!({"page": sel.page, "visual": sel.visual, "role": role.name,
                                   "hierarchy": h.name, "table": h.table}),
                        );
                    }
                    None => {
                        app.status = app
                            .lang
                            .t(
                                "This visual takes no hierarchy: drop it on a well.",
                                "Ce visuel ne prend pas de hiérarchie : déposez-la dans un emplacement.",
                            )
                            .to_string();
                    }
                }
            }
            return;
        }
        Drag::Bound(_) => return,
    };
    let n = |k: &str| v.get(k).and_then(Value::as_f64).unwrap_or(0.0) as f32;
    let r = Rect::from_min_size(
        rect.min + vec2(n("x"), n("y")) * zoom,
        vec2(n("width"), n("height")) * zoom,
    );
    ui.painter().rect_stroke(
        r.intersect(rect),
        CornerRadius::same(2),
        Stroke::new(2.0, t.ring),
        StrokeKind::Outside,
    );
    if released(ui) && DragAndDrop::take_payload::<Drag>(ui.ctx()).is_some() {
        app.cmd.visual = Some(sel.clone());
        bind_default(app, &sel, &v, &f);
    }
}

/// The size of a visual made from a field (report pixels).
pub const NEW_SIZE: (f32, f32) = (400.0, 300.0);

/// The visual Desktop makes from one field, with nothing selected (Microsoft Learn, "Add
/// visuals to a report"; black-box checks): a measure or a summarized number column → a
/// clustered column chart with it as its value, a geography column → a map with it as its
/// location, any other column → a table. Returns (visual type, role, field text).
pub fn new_visual_for(f: &Field) -> (&'static str, &'static str, String) {
    if f.measure {
        return ("clusteredColumnChart", "Y", f.text.clone());
    }
    if let Some(function) = summarized(f) {
        return ("clusteredColumnChart", "Y", format!("{function}({})", f.text));
    }
    let geo = f.category.as_deref().is_some_and(|c| {
        matches!(
            c.to_ascii_lowercase().as_str(),
            "city"
                | "country"
                | "stateorprovince"
                | "county"
                | "continent"
                | "postalcode"
                | "place"
                | "address"
        )
    });
    if geo {
        return ("map", "Category", f.text.clone());
    }
    ("tableEx", "Values", f.text.clone())
}

/// Where a new visual goes on page `page` (a snapshot page) when no point is given: the first
/// free spot of a 20-pixel grid, else the top-left corner.
pub fn free_spot(page: &Value, w: f32, h: f32) -> (f32, f32) {
    let n = |v: &Value, k: &str| v.get(k).and_then(Value::as_f64).unwrap_or(0.0) as f32;
    let (pw, ph) = (n(page, "width").max(w + 40.0), n(page, "height").max(h + 40.0));
    let rects: Vec<Rect> = arr(page.get("visuals"))
        .iter()
        .take(2000)
        .map(|v| Rect::from_min_size(pos2(n(v, "x"), n(v, "y")), vec2(n(v, "width"), n(v, "height"))))
        .collect();
    let mut y = 20.0;
    while y + h <= ph - 20.0 + 0.5 {
        let mut x = 20.0;
        while x + w <= pw - 20.0 + 0.5 {
            let r = Rect::from_min_size(pos2(x, y), vec2(w, h));
            if !rects.iter().any(|o| o.intersects(r)) {
                return (x, y);
            }
            x += 20.0;
        }
        y += 20.0;
    }
    (20.0, 20.0)
}

/// Makes a new visual from field `f` (Desktop's choice of type, [`new_visual_for`]) on the page
/// shown, at `at` (report pixels) or the first free spot; it is selected once made.
fn new_visual(app: &mut App, f: &Field, at: Option<(f32, f32)>) {
    if let Err(why) = ribbon::state(app, "visual.add") {
        app.status = format!("{} {why}", app.lang.t("Unavailable:", "Indisponible :"));
        return;
    }
    let Some(page) = app.loaded.as_ref().and_then(|l| {
        l.snap
            .pointer(&format!("/report/pages/{}", app.active_page()))
            .cloned()
    }) else {
        return;
    };
    let Some(name) = s(&page, "name") else { return };
    let (w, h) = NEW_SIZE;
    let (x, y) = at.unwrap_or_else(|| free_spot(&page, w, h));
    let (ty, role, text) = new_visual_for(f);
    let p = json!({"page": name, "visual_type": ty, "x": x.round(), "y": y.round(), "width": w, "height": h,
                   "fields": [{"role": role, "field": text}]});
    app.run(
        Call {
            id: "visual.add",
            params: p,
        },
        Then::SelectVisual,
    );
}

/// The end of the frame, after every drop target: a well's field released anywhere else is
/// removed (`visual.unbind`); the dragged item follows the pointer.
pub fn finish(app: &mut App, ctx: &egui::Context) {
    let Some(d) = dragging(ctx) else { return };
    if ctx.input(|i| i.pointer.any_released()) {
        DragAndDrop::clear_payload(ctx);
        if let Drag::Bound(b) = d {
            let mut p = json!({"page": b.page, "visual": b.visual, "role": b.role});
            match &b.field {
                Some(f) => p["field"] = json!(f),
                None => p["index"] = json!(b.index),
            }
            exec(app, "visual.unbind", p);
        }
        return;
    }
    let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else {
        return;
    };
    let t = Tokens::of(app.prefs.theme);
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        Id::new("bicraft-drag-chip"),
    ));
    let text = clip(d.label(), 40);
    let galley = painter.layout_no_wrap(text, theme::font(12.0), t.text);
    let r = Rect::from_min_size(pos + vec2(12.0, 10.0), galley.size() + vec2(16.0, 8.0));
    painter.rect_filled(r, CornerRadius::same(4), t.card);
    painter.rect_stroke(
        r,
        CornerRadius::same(4),
        Stroke::new(1.0, t.ring),
        StrokeKind::Inside,
    );
    painter.galley(r.min + vec2(8.0, 4.0), galley, t.text);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn role(name: &str, kind: RoleKind, max: Option<usize>) -> Role {
        Role {
            name: name.into(),
            kind,
            max,
        }
    }

    fn column(name: &str, numeric: bool, summarize: Option<&str>) -> Field {
        Field {
            text: column_text("Sales", name),
            table: "Sales".into(),
            name: name.into(),
            measure: false,
            numeric,
            summarize: summarize.map(str::to_string),
            category: None,
        }
    }

    fn measure(name: &str) -> Field {
        Field {
            text: measure_text(name),
            table: "Sales".into(),
            name: name.into(),
            measure: true,
            numeric: true,
            summarize: None,
            category: None,
        }
    }

    /// Desktop's default wells: a column chart takes a text column on its axis and a number in
    /// its values (summarized); a card takes one value; a full visual takes nothing.
    #[test]
    fn the_default_role_follows_the_field_and_the_room() {
        let chart = [
            role("Category", RoleKind::Grouping, None),
            role("Y", RoleKind::Measure, None),
            role("Series", RoleKind::Grouping, Some(1)),
            role("Tooltips", RoleKind::Measure, None),
        ];
        let none: Vec<(String, String)> = Vec::new();
        assert_eq!(
            default_target(&chart, &none, &column("Category", false, None)),
            Some(("Category".into(), "Sales[Category]".into()))
        );
        assert_eq!(
            default_target(&chart, &none, &column("Amount", true, None)),
            Some(("Y".into(), "Sum(Sales[Amount])".into()))
        );
        assert_eq!(
            default_target(&chart, &none, &column("Amount", true, Some("average"))),
            Some(("Y".into(), "Avg(Sales[Amount])".into()))
        );
        assert_eq!(
            default_target(&chart, &none, &column("Year", true, Some("none"))),
            Some(("Category".into(), "Sales[Year]".into()))
        );
        assert_eq!(
            default_target(&chart, &none, &measure("Total")),
            Some(("Y".into(), "[Total]".into()))
        );
        // already in Y: the next value role
        let bound = vec![("Y".to_string(), "[Total]".to_string())];
        assert_eq!(
            default_target(&chart, &bound, &measure("Total")),
            Some(("Tooltips".into(), "[Total]".into()))
        );
        let card = [role("Values", RoleKind::Measure, Some(1))];
        assert_eq!(
            default_target(&card, &none, &column("Category", false, None)),
            Some(("Values".into(), "Count(Sales[Category])".into()))
        );
        let full = vec![("Values".to_string(), "[Total]".to_string())];
        assert_eq!(default_target(&card, &full, &measure("Other")), None);
        let slicer = [role("Values", RoleKind::Grouping, None)];
        assert_eq!(default_target(&slicer, &none, &measure("Total")), None);
        assert_eq!(default_target(&[], &none, &measure("Total")), None);
    }

    #[test]
    fn field_texts_are_what_the_engine_reads() {
        assert_eq!(column_text("Sales", "Amount"), "Sales[Amount]");
        assert_eq!(column_text("Date Table", "Year"), "'Date Table'[Year]");
        assert_eq!(column_text("O'Brien", "a]b"), "'O''Brien'[a]]b]");
        assert_eq!(measure_text("x]y"), "[x]]y]");
    }

    #[test]
    fn display_folders_nest_and_sort() {
        let t = json!({"name": "Sales", "columns": [
            {"name": "b", "data_type": "string"},
            {"name": "a", "data_type": "double", "display_folder": "Money\\Raw"},
            {"name": "h", "data_type": "string", "hidden": true}],
            "measures": [{"name": "Total", "display_folder": "Money"}],
            "hierarchy_levels": [{"name": "H", "levels": [{"name": "L", "column": "b"}]}]});
        let tree = table_tree(&t, false);
        let names: Vec<(u8, String)> = tree.iter().map(Node::sort_key).collect();
        assert_eq!(
            names,
            vec![(0, "money".into()), (1, "b".into()), (1, "h".into())],
            "the folder first, the hidden column left out"
        );
        let Some(Node::Folder(_, kids)) = tree.first() else {
            panic!()
        };
        assert!(matches!(kids.first(), Some(Node::Folder(n, _)) if n == "Raw"));
        assert!(tree.iter().any(|n| n.matches("total")));
        assert_eq!(table_tree(&t, true).len(), 4);
        // hostile input: nothing breaks
        for bad in [
            json!(null),
            json!({"columns": 3}),
            json!({"name": 1, "hierarchy_levels": [{}]}),
        ] {
            let _ = table_tree(&bad, true);
        }
    }

    #[test]
    fn captions_name_the_wells_by_type() {
        let l = Lang::En;
        assert_eq!(caption("clusteredColumnChart", "Category", l), "X-axis");
        assert_eq!(caption("clusteredBarChart", "Category", l), "Y-axis");
        assert_eq!(caption("pieChart", "Series", l), "Details");
        assert_eq!(caption("tableEx", "Values", l), "Columns");
        assert_eq!(caption("pivotTable", "Rows", l), "Rows");
        assert_eq!(caption("lineChart", "Rows", l), "Small multiples");
        assert_eq!(caption("myVisual", "Weird", l), "Weird");
        assert_eq!(caption("card", "Values", Lang::Fr), "Champs");
    }

    #[test]
    fn a_drop_on_a_well_is_the_right_command() {
        let sel = VisualSel {
            page: "p".into(),
            visual: "v".into(),
        };
        let y = role("Y", RoleKind::Measure, None);
        let a = Item {
            role: "Y".into(),
            kind: "measure".into(),
            table: Some("Sales".into()),
            name: Some("Total".into()),
            function: None,
            text: "[Total]".into(),
            label: "Total".into(),
            display_name: None,
            show_as: None,
            show_all: false,
            spark: None,
        };
        let b = Item {
            text: "[Other]".into(),
            label: "Other".into(),
            ..a.clone()
        };
        let items = [&a, &b];
        let ok = Ok(());
        let w = Well {
            sel: &sel,
            role: &y,
            cap: "Y-axis",
            items: &items,
            edit: &ok,
            agg: &ok,
            payload: None,
            model: None,
            vtype: "clusteredColumnChart",
            lang: Lang::En,
        };
        let bound = |index: usize| {
            Drag::Bound(Bound {
                page: "p".into(),
                visual: "v".into(),
                role: "Y".into(),
                index,
                field: Some(if index == 0 { "[Total]" } else { "[Other]" }.into()),
                label: String::new(),
            })
        };
        // the first one dropped after the second: to_index 1 once it is taken out
        let (id, p) = drop_on_well(&w, &bound(0), 2).unwrap();
        assert_eq!(id, "visual.move_field");
        assert_eq!(p["to_index"], 1);
        assert_eq!(p["field"], "[Total]");
        // dropped where it is: nothing
        assert!(drop_on_well(&w, &bound(0), 0).is_none());
        assert!(drop_on_well(&w, &bound(0), 1).is_none());
        let (id, p) = drop_on_well(&w, &Drag::Field(column("Amount", true, None)), 1).unwrap();
        assert_eq!(id, "visual.bind");
        assert_eq!(p["field"], "Sum(Sales[Amount])");
        assert_eq!(p["index"], 1);
    }
}
