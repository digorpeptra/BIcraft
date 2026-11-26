//! The Report view's Filters pane, as in Power BI Desktop (docs/design.md §12): three sections,
//! **Filters on this visual** (the selected visual's cards, plus one card per bound field that
//! has none yet), **Filters on this page** and **Filters on all pages**. Each card shows its
//! field and its condition ("is Wheels", "is greater than 10", "Top 3 by Total"); expanded, it
//! edits the condition:
//!
//! - **Basic filtering**: a search box and the column's values (from the engine,
//!   `column.values`) as checkboxes, with Select all; each click applies at once;
//! - **Advanced filtering**: one or two conditions (is, is not, is less than…, contains, starts
//!   with, is blank…) joined by And / Or, applied with Apply filter;
//! - **Top N** (visual filters only, as in Desktop): Top or Bottom, the count and the field it
//!   ranks by (typed, or dragged from the Data pane), applied with Apply filter (Desktop has
//!   no Top N on page or report filters);
//! - **Relative date filtering** and **Relative time filtering** (date columns): is in the last
//!   / this / next, the count, the period (days to calendar years; minutes, hours) and Include
//!   today, applied with Apply filter (`relative`).
//!
//! Each card's header has Desktop's lock and eye: locked, a reader cannot change the card;
//! hidden, a reader does not see it (`*.filter {field, locked | hidden}`, the condition kept). A
//! reader's pane leaves hidden cards out and shows the lock of locked ones.
//!
//! The eraser clears a card's condition and keeps the card; × removes a card added by hand. A
//! field dragged from the Data pane onto a section adds an empty card there. Every edit is an
//! engine command (`visual.filter`, `page.filter`, `report.filter`, the `*.clear_filters`), so
//! the page re-renders with the filters applied from the next snapshot; a reader, or a command
//! the session cannot run, leaves the pane read-only with the reason as tooltip.

use std::collections::{BTreeMap, BTreeSet};

use egui::{Align2, CornerRadius, DragAndDrop, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use serde_json::{Value, json};

use crate::App;
use crate::actions::Call;
use crate::app::{Then, clip};
use crate::i18n::Lang;
use crate::icons::{self, Icon};
use crate::panes::{Drag, Field, column_text, measure_text};
use crate::ribbon;
use crate::theme::{self, Tokens};
use crate::widgets;

/// The commands the pane runs (the coverage test reads it).
pub const COMMANDS: &[&str] = &[
    "visual.filter",
    "visual.clear_filters",
    "page.filter",
    "page.clear_filters",
    "report.filter",
    "report.clear_filters",
    "column.values",
    "report.settings",
    "filter.order",
];

/// Whether the report shows the Filters pane to readers (`report.settings.filter_pane_visible`
/// in the snapshot; a report without the setting shows it).
pub fn visible_to_readers(app: &App) -> bool {
    app.loaded
        .as_ref()
        .and_then(|l| l.snap.pointer("/report/settings/filter_pane_visible"))
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

/// Whether the window is a reader's and the report hides the Filters pane from readers: then the
/// pane and its rail icon are not shown.
pub fn hidden_from_reader(app: &App) -> bool {
    app.access.as_deref() == Some("reader") && !visible_to_readers(app)
}

/// Most values a basic card draws.
const MAX_SHOWN: usize = 500;
/// Most cards drawn per section.
const MAX_CARDS: usize = 100;
/// Longest label drawn (characters).
const MAX_LABEL: usize = 48;

/// Which section a card is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Scope {
    Visual,
    Page,
    Report,
}

impl Scope {
    fn prefix(self) -> &'static str {
        match self {
            Scope::Visual => "visual",
            Scope::Page => "page",
            Scope::Report => "report",
        }
    }

    fn filter_cmd(self) -> &'static str {
        match self {
            Scope::Visual => "visual.filter",
            Scope::Page => "page.filter",
            Scope::Report => "report.filter",
        }
    }

    fn clear_cmd(self) -> &'static str {
        match self {
            Scope::Visual => "visual.clear_filters",
            Scope::Page => "page.clear_filters",
            Scope::Report => "report.clear_filters",
        }
    }

    pub fn title(self, l: Lang) -> &'static str {
        match self {
            Scope::Visual => l.t("Filters on this visual", "Filtres sur ce visuel"),
            Scope::Page => l.t("Filters on this page", "Filtres sur cette page"),
            Scope::Report => l.t("Filters on all pages", "Filtres sur toutes les pages"),
        }
    }
}

/// The filter type a card edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Basic,
    Advanced,
    TopN,
    /// Relative date filtering (date columns).
    RelativeDate,
    /// Relative time filtering (date columns).
    RelativeTime,
}

/// What an advanced or top N card is being edited to (applied with Apply filter).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Draft {
    pub mode: Mode,
    pub op1: String,
    pub value1: String,
    pub op2: String,
    pub value2: String,
    pub or: bool,
    pub bottom: bool,
    pub count: String,
    pub by: String,
    /// Relative filters: `last`, `next` or `this`.
    pub rel_range: String,
    /// Relative filters: how many periods.
    pub rel_count: String,
    /// Relative filters: the period (`days`, `calendar_months`, `hours`…).
    pub rel_unit: String,
    /// Relative date filters: today included.
    pub rel_today: bool,
}

/// The pane's view state (pure UI: never sent anywhere).
#[derive(Debug, Default)]
pub struct FiltersUi {
    /// The pane's search text: filters the cards by field, as Desktop's Visualizations-pane
    /// search does.
    pub find: String,
    /// Cards expanded (their keys).
    pub open: BTreeSet<String>,
    /// Each basic card's search text.
    pub search: BTreeMap<String, String>,
    /// Each card's edit in progress.
    pub drafts: BTreeMap<String, Draft>,
    /// Values of columns (by field text): the snapshot revision they were read at, the values,
    /// whether the list was cut.
    pub values: BTreeMap<String, (u64, Vec<Value>, bool)>,
    /// Columns whose values were asked for (field text, revision).
    pub asked: BTreeSet<(String, u64)>,
}

/// One filter card, from the snapshot (`filters[]`) or a bound field without one.
#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    /// The field as commands write it (`Sales[Category]`, `[Total]`, `Sum(Sales[Amount])`).
    pub field: String,
    /// `column`, `measure`, `aggregation`, `other`.
    pub kind: String,
    pub table: Option<String>,
    /// The column's or measure's name.
    pub name: Option<String>,
    /// `basic`, `advanced`, `topn`, `none`, `other`.
    pub ty: String,
    pub condition: Value,
    /// Made by Desktop for a bound field (`howCreated: Auto`), or a bound field with no card:
    /// it cannot be removed, only cleared.
    pub auto: bool,
    /// Hidden from readers (`isHiddenInViewMode`, the eye).
    pub hidden: bool,
    /// Readers cannot change it (`isLockedInViewMode`, the lock).
    pub locked: bool,
    /// A date column (relative date and time filtering offered), from the snapshot's model.
    pub date: bool,
    /// The card is in the level's `filterConfig` (a bound field without a card is not): it can
    /// be dragged to a new place.
    pub own: bool,
}

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_string)
}

fn arr(v: Option<&Value>) -> &[Value] {
    v.and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default()
}

/// The cards of a snapshot `filters` list.
pub fn cards_of(list: Option<&Value>) -> Vec<Card> {
    arr(list)
        .iter()
        .take(MAX_CARDS)
        .filter_map(|c| {
            let field = s(c, "field")?;
            let kind = s(c, "kind").unwrap_or_else(|| "other".into());
            let name = if kind == "measure" {
                s(c, "measure")
            } else {
                s(c, "column")
            };
            Some(Card {
                field,
                kind,
                table: s(c, "table"),
                name,
                ty: s(c, "type").unwrap_or_else(|| "none".into()),
                condition: c.get("condition").cloned().unwrap_or(Value::Null),
                auto: s(c, "how_created").as_deref() == Some("Auto"),
                hidden: c.get("hidden").and_then(Value::as_bool).unwrap_or(false),
                locked: c.get("locked").and_then(Value::as_bool).unwrap_or(false),
                date: false,
                own: true,
            })
        })
        .collect()
}

/// A visual's cards: its own, then one per bound field without a card (Desktop lists every
/// field of the visual there).
pub fn visual_cards(v: &Value) -> Vec<Card> {
    let mut out = cards_of(v.get("filters"));
    for f in arr(v.get("fields")).iter().take(MAX_CARDS) {
        let (Some(text), Some(kind)) = (s(f, "text"), s(f, "kind")) else {
            continue;
        };
        if !matches!(kind.as_str(), "column" | "measure" | "aggregation") {
            continue;
        }
        if out.iter().any(|c| c.field.eq_ignore_ascii_case(&text)) {
            continue;
        }
        out.push(Card {
            field: text,
            kind,
            table: s(f, "table"),
            name: s(f, "name"),
            ty: "none".into(),
            condition: Value::Null,
            auto: true,
            hidden: false,
            locked: false,
            date: false,
            own: false,
        });
    }
    // the pane's order: the engine's for the visual's own cards, the bound fields without a card
    // sorted in by name unless the order is custom (they then come last)
    match v.get("filter_sort").and_then(Value::as_str) {
        Some("custom") => {}
        Some("descending") => out.sort_by_key(|c| std::cmp::Reverse(sort_name(c))),
        _ => out.sort_by_key(sort_name),
    }
    out
}

/// The name a card is sorted by (its column's or measure's, as the engine sorts).
fn sort_name(c: &Card) -> String {
    c.name.as_deref().unwrap_or(&c.field).to_lowercase()
}

/// A filter card dragged to a new place in its section (`filter.order`).
#[derive(Debug, Clone, PartialEq)]
pub struct CardDrag {
    pub scope: Scope,
    pub field: String,
}

/// Marks the cards on date columns of the snapshot's model (relative filtering is offered there).
pub fn mark_dates(cards: &mut [Card], snap: &Value) {
    let tables = arr(snap.pointer("/model/tables"));
    for c in cards.iter_mut().filter(|c| c.kind == "column") {
        let (Some(t), Some(n)) = (&c.table, &c.name) else {
            continue;
        };
        c.date = tables
            .iter()
            .filter(|x| s(x, "name").is_some_and(|x| x.eq_ignore_ascii_case(t)))
            .flat_map(|x| arr(x.get("columns")))
            .any(|col| {
                s(col, "name").is_some_and(|x| x.eq_ignore_ascii_case(n))
                    && s(col, "data_type").is_some_and(|d| d.eq_ignore_ascii_case("dateTime"))
            });
    }
}

/// The relative ranges, as `relative.range` names them, with their words (EN, FR).
pub const REL_RANGES: &[(&str, &str, &str)] = &[
    ("last", "is in the last", "est dans les derniers"),
    ("this", "is in this", "est dans la période en cours"),
    ("next", "is in the next", "est dans les prochains"),
];

/// The relative date periods, as `relative.unit` names them, with their words (EN, FR).
pub const REL_DATE_UNITS: &[(&str, &str, &str)] = &[
    ("days", "Days", "Jours"),
    ("weeks", "Weeks", "Semaines"),
    ("calendar_weeks", "Weeks (calendar)", "Semaines (calendaires)"),
    ("months", "Months", "Mois"),
    ("calendar_months", "Months (calendar)", "Mois (calendaires)"),
    ("years", "Years", "Années"),
    ("calendar_years", "Years (calendar)", "Années (calendaires)"),
];

/// The relative time periods.
pub const REL_TIME_UNITS: &[(&str, &str, &str)] =
    &[("minutes", "Minutes", "Minutes"), ("hours", "Hours", "Heures")];

fn rel_words(list: &[(&str, &'static str, &'static str)], code: &str, l: Lang) -> String {
    list.iter()
        .find(|x| x.0 == code)
        .map_or_else(|| code.to_string(), |x| l.t(x.1, x.2).to_string())
}

/// A relative condition in words: `is in the last 3 days`, `is in this month`, `est dans les 2
/// dernières semaines calendaires`.
pub fn relative_words(rel: &Value, l: Lang) -> String {
    let range = rel.get("range").and_then(Value::as_str).unwrap_or("last");
    let n = rel.get("count").and_then(Value::as_u64).unwrap_or(1);
    let unit = rel.get("unit").and_then(Value::as_str).unwrap_or("days");
    let today = rel.get("include_today").and_then(Value::as_bool).unwrap_or(true);
    let calendar = unit.starts_with("calendar_");
    let base = unit.trim_start_matches("calendar_");
    // (EN singular, EN plural, FR singular, FR plural, FR feminine)
    let (en1, en, fr1, fr, fem) = match base {
        "weeks" => ("week", "weeks", "semaine", "semaines", true),
        "months" => ("month", "months", "mois", "mois", false),
        "years" => ("year", "years", "année", "années", true),
        "minutes" => ("minute", "minutes", "minute", "minutes", true),
        "hours" => ("hour", "hours", "heure", "heures", true),
        _ => ("day", "days", "jour", "jours", false),
    };
    let mut t = match l {
        Lang::En => {
            let cal = if calendar { "calendar " } else { "" };
            match range {
                "this" => format!("is in this {en1}"),
                r => format!(
                    "is in the {} {n} {cal}{}",
                    if r == "next" { "next" } else { "last" },
                    if n == 1 { en1 } else { en }
                ),
            }
        }
        _ => {
            let cal = match (calendar, n == 1) {
                (false, _) => "",
                (true, true) => " calendaire",
                (true, false) => " calendaires",
            };
            let next = range == "next";
            match (range, n == 1) {
                ("this", _) => {
                    let det = if fem { "cette" } else { "ce" };
                    format!("est dans {det} {fr1}")
                }
                (_, true) => {
                    let (det, adj) = match (fem, next) {
                        (true, true) => ("la", "prochaine"),
                        (true, false) => ("la", "dernière"),
                        (false, true) => ("le", "prochain"),
                        (false, false) => ("le", "dernier"),
                    };
                    format!("est dans {det} {adj} {fr1}{cal}")
                }
                (_, false) => {
                    let adj = match (fem, next) {
                        (true, true) => "prochaines",
                        (true, false) => "dernières",
                        (false, true) => "prochains",
                        (false, false) => "derniers",
                    };
                    format!("est dans les {n} {adj} {fr}{cal}")
                }
            }
        }
    };
    if !today && range != "this" && !matches!(base, "minutes" | "hours") {
        t.push_str(l.t(" (not including today)", " (sans aujourd'hui)"));
    }
    t
}

/// A card's title: the field's name (an aggregation as Desktop names it).
pub fn card_label(c: &Card, l: Lang) -> String {
    let name = c.name.clone().unwrap_or_else(|| c.field.clone());
    if c.kind == "aggregation"
        && let Some(f) = c.field.split('(').next().filter(|f| !f.is_empty())
    {
        let what = match f.to_ascii_lowercase().as_str() {
            "sum" => l.t("Sum of", "Somme de"),
            "avg" => l.t("Average of", "Moyenne de"),
            "min" => l.t("Min of", "Min de"),
            "max" => l.t("Max of", "Max de"),
            "count" => l.t("Count of", "Nombre de"),
            _ => return format!("{f} {name}"),
        };
        return format!("{what} {name}");
    }
    name
}

/// A value as a card writes it (`(Blank)` for null).
pub fn value_text(v: &Value, l: Lang) -> String {
    match v {
        Value::Null => l.t("(Blank)", "(Vide)").to_string(),
        Value::String(s) => s.strip_suffix("T00:00:00").unwrap_or(s).to_string(),
        other => other.to_string(),
    }
}

/// The advanced operators, as `visual.filter` names them, with Desktop's words (EN, FR) and
/// whether they need a value.
pub const OPS: &[(&str, &str, &str, bool)] = &[
    ("<", "is less than", "est inférieur à", true),
    ("<=", "is less than or equal to", "est inférieur ou égal à", true),
    (">", "is greater than", "est supérieur à", true),
    (
        ">=",
        "is greater than or equal to",
        "est supérieur ou égal à",
        true,
    ),
    ("=", "is", "est", true),
    ("<>", "is not", "n'est pas", true),
    ("contains", "contains", "contient", true),
    ("not_contains", "does not contain", "ne contient pas", true),
    ("starts_with", "starts with", "commence par", true),
    (
        "not_starts_with",
        "does not start with",
        "ne commence pas par",
        true,
    ),
    ("blank", "is blank", "est vide", false),
    ("not_blank", "is not blank", "n'est pas vide", false),
];

fn op_words(op: &str, l: Lang) -> String {
    OPS.iter()
        .find(|o| o.0.eq_ignore_ascii_case(op))
        .map_or_else(|| op.to_string(), |o| l.t(o.1, o.2).to_string())
}

/// One advanced condition in Desktop's words (`is greater than 10`, `is blank`).
fn condition_words(op: &str, value: &Value, l: Lang) -> String {
    match (op, value) {
        ("=", Value::Null) => op_words("blank", l),
        ("<>", Value::Null) => op_words("not_blank", l),
        _ => format!("{} {}", op_words(op, l), value_text(value, l)),
    }
}

/// A card's condition in plain words: `is (All)`, `is Wheels, Toys`, `is not Toys`,
/// `is greater than 10 and is less than 20`, `Top 3 by Total`.
pub fn summary(c: &Card, l: Lang) -> String {
    let all = || l.t("is (All)", "est (Tout)").to_string();
    let list = |key: &str| -> Vec<String> {
        arr(c.condition.get(key))
            .iter()
            .take(6)
            .map(|v| value_text(v, l))
            .collect()
    };
    let more = |key: &str| arr(c.condition.get(key)).len() > 6;
    match c.ty.as_str() {
        "basic" if c.condition.get("not_in").is_some() => {
            let mut t = format!("{} {}", l.t("is not", "n'est pas"), list("not_in").join(", "));
            if more("not_in") {
                t.push_str(", …");
            }
            t
        }
        "basic" => {
            let mut t = format!("{} {}", l.t("is", "est"), list("in").join(", "));
            if more("in") {
                t.push_str(", …");
            }
            t
        }
        "advanced" => {
            if let Some(op) = c.condition.get("op").and_then(Value::as_str) {
                return condition_words(op, c.condition.get("value").unwrap_or(&Value::Null), l);
            }
            let or = c.condition.get("logic").and_then(Value::as_str) == Some("or");
            let parts: Vec<String> = arr(c.condition.get("conditions"))
                .iter()
                .take(2)
                .map(|x| {
                    condition_words(
                        x.get("op").and_then(Value::as_str).unwrap_or("="),
                        x.get("value").unwrap_or(&Value::Null),
                        l,
                    )
                })
                .collect();
            parts.join(if or {
                l.t(" or ", " ou ")
            } else {
                l.t(" and ", " et ")
            })
        }
        "topn" => {
            let n = c.condition.get("top").and_then(Value::as_u64).unwrap_or(0);
            let bottom = c.condition.get("direction").and_then(Value::as_str) == Some("bottom");
            let by = c
                .condition
                .get("by")
                .and_then(Value::as_str)
                .map(|b| b.trim_start_matches('[').trim_end_matches(']').to_string())
                .unwrap_or_default();
            format!(
                "{} {n} {} {by}",
                if bottom {
                    l.t("Bottom", "Bas")
                } else {
                    l.t("Top", "Haut")
                },
                l.t("by", "par")
            )
        }
        "relative" => relative_words(c.condition.get("relative").unwrap_or(&Value::Null), l),
        "other" => l
            .t(
                "(a condition BIcraft keeps as written)",
                "(une condition conservée telle quelle)",
            )
            .into(),
        _ => all(),
    }
}

/// Whether two values are the same filter value (`100` = `100.0`; a date = its midnight).
pub fn same_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::String(x), Value::String(y)) => {
            x == y || x.strip_suffix("T00:00:00") == Some(y) || y.strip_suffix("T00:00:00") == Some(x)
        }
        _ => a == b,
    }
}

/// Whether value `v` is shown checked in a basic card.
pub fn checked(c: &Card, v: &Value) -> bool {
    if c.ty != "basic" {
        return true;
    }
    if let Some(not_in) = c.condition.get("not_in") {
        return !arr(Some(not_in)).iter().any(|x| same_value(x, v));
    }
    arr(c.condition.get("in")).iter().any(|x| same_value(x, v))
}

/// The edit a click on value `v` of a basic card makes: `Some(Ok(params))` for a `*.filter`
/// condition (`in` or `not_in`), `Some(Err(()))` to clear the card (every value selected again),
/// `None` for no change.
pub fn toggle_value(c: &Card, all: &[Value], v: &Value) -> Option<Result<Value, ()>> {
    let on = checked(c, v);
    if c.ty == "basic"
        && let Some(not_in) = c.condition.get("not_in")
    {
        let mut out: Vec<Value> = arr(Some(not_in)).to_vec();
        if on {
            out.push(v.clone());
        } else {
            out.retain(|x| !same_value(x, v));
        }
        return Some(if out.is_empty() {
            Err(())
        } else {
            Ok(json!({"not_in": out}))
        });
    }
    if c.ty != "basic" {
        // everything shown: unchecking one value excludes it
        return Some(Ok(json!({"not_in": [v.clone()]})));
    }
    let mut out: Vec<Value> = arr(c.condition.get("in")).to_vec();
    if on {
        out.retain(|x| !same_value(x, v));
    } else {
        out.push(v.clone());
    }
    if out.is_empty() {
        return Some(Err(()));
    }
    // every value picked: the same as no filter
    if !all.is_empty() && all.iter().all(|a| out.iter().any(|x| same_value(x, a))) {
        return Some(Err(()));
    }
    Some(Ok(json!({"in": out})))
}

/// A typed value of an advanced condition: a number for numbers, else the text.
fn typed(text: &str, numeric: bool) -> Value {
    let t = text.trim();
    if numeric
        && let Ok(x) = t.replace(',', ".").parse::<f64>()
        && x.is_finite()
    {
        return serde_json::Number::from_f64(x).map_or(Value::Null, Value::Number);
    }
    json!(t)
}

/// The condition of a draft as `*.filter` parameters; `Err` with the reason when it is not
/// complete.
pub fn draft_params(d: &Draft, numeric: bool, l: Lang) -> Result<Value, String> {
    match d.mode {
        Mode::Basic => Err(String::new()),
        Mode::Advanced => {
            let one = |op: &str, value: &str| -> Option<Value> {
                match op {
                    "" => None,
                    "blank" => Some(json!({"op": "=", "value": null})),
                    "not_blank" => Some(json!({"op": "<>", "value": null})),
                    _ if value.trim().is_empty() => None,
                    _ => Some(json!({"op": op, "value": typed(value, numeric)})),
                }
            };
            let a = one(&d.op1, &d.value1).ok_or_else(|| {
                l.t(
                    "Pick a condition and type its value.",
                    "Choisissez une condition et sa valeur.",
                )
                .to_string()
            })?;
            match one(&d.op2, &d.value2) {
                None => Ok(a),
                Some(b) => Ok(json!({"conditions": [a, b], "logic": if d.or { "or" } else { "and" }})),
            }
        }
        Mode::TopN => {
            let n: u64 = d.count.trim().parse().ok().filter(|n| *n >= 1).ok_or_else(|| {
                l.t("Type how many items to show.", "Indiquez le nombre d'éléments.")
                    .to_string()
            })?;
            if d.by.trim().is_empty() {
                return Err(l
                    .t(
                        "Add the field to rank by (By value).",
                        "Ajoutez le champ du classement (Par valeur).",
                    )
                    .to_string());
            }
            Ok(json!({"top": n, "by": d.by.trim(), "direction": if d.bottom { "bottom" } else { "top" }}))
        }
        Mode::RelativeDate | Mode::RelativeTime => {
            let this = d.rel_range == "this";
            let n: u64 = if this {
                1
            } else {
                d.rel_count
                    .trim()
                    .parse()
                    .ok()
                    .filter(|n| (1..=10_000).contains(n))
                    .ok_or_else(|| {
                        l.t(
                            "Type how many periods (1 to 10,000).",
                            "Indiquez le nombre de périodes (1 à 10 000).",
                        )
                        .to_string()
                    })?
            };
            let units = if d.mode == Mode::RelativeTime {
                REL_TIME_UNITS
            } else {
                REL_DATE_UNITS
            };
            if !units.iter().any(|u| u.0 == d.rel_unit) {
                return Err(l.t("Pick a period.", "Choisissez une période.").to_string());
            }
            let mut rel = json!({"range": d.rel_range, "count": n, "unit": d.rel_unit});
            if d.mode == Mode::RelativeDate && !this {
                rel["include_today"] = json!(d.rel_today);
            }
            Ok(json!({"relative": rel}))
        }
    }
}

/// The draft a card starts from: its own condition.
pub fn draft_of(c: &Card) -> Draft {
    let mut d = Draft {
        mode: match c.ty.as_str() {
            "advanced" => Mode::Advanced,
            "topn" => Mode::TopN,
            _ if c.kind != "column" => Mode::Advanced,
            _ => Mode::Basic,
        },
        op1: if c.kind == "column" {
            "contains".into()
        } else {
            ">".into()
        },
        count: "10".into(),
        rel_range: "last".into(),
        rel_count: "1".into(),
        rel_unit: "days".into(),
        rel_today: true,
        ..Draft::default()
    };
    let read = |x: &Value| -> (String, String) {
        let op = x.get("op").and_then(Value::as_str).unwrap_or("=");
        let v = x.get("value").unwrap_or(&Value::Null);
        match (op, v) {
            ("=", Value::Null) => ("blank".into(), String::new()),
            ("<>", Value::Null) => ("not_blank".into(), String::new()),
            (_, Value::String(s)) => (op.into(), s.clone()),
            (_, other) => (op.into(), other.to_string()),
        }
    };
    if c.ty == "advanced" {
        if c.condition.get("op").is_some() {
            (d.op1, d.value1) = read(&c.condition);
        } else {
            let list = arr(c.condition.get("conditions"));
            if let Some(a) = list.first() {
                (d.op1, d.value1) = read(a);
            }
            if let Some(b) = list.get(1) {
                (d.op2, d.value2) = read(b);
            }
            d.or = c.condition.get("logic").and_then(Value::as_str) == Some("or");
        }
    }
    if c.ty == "relative"
        && let Some(rel) = c.condition.get("relative")
    {
        d.rel_range = s(rel, "range").unwrap_or_else(|| "last".into());
        d.rel_count = rel.get("count").and_then(Value::as_u64).unwrap_or(1).to_string();
        d.rel_unit = s(rel, "unit").unwrap_or_else(|| "days".into());
        d.rel_today = rel.get("include_today").and_then(Value::as_bool).unwrap_or(true);
        d.mode = if REL_TIME_UNITS.iter().any(|u| u.0 == d.rel_unit) {
            Mode::RelativeTime
        } else {
            Mode::RelativeDate
        };
    }
    if c.ty == "topn" {
        d.count = c
            .condition
            .get("top")
            .and_then(Value::as_u64)
            .map_or_else(|| "10".into(), |n| n.to_string());
        d.bottom = c.condition.get("direction").and_then(Value::as_str) == Some("bottom");
        d.by = s(&c.condition, "by").unwrap_or_default();
    }
    d
}

/// Where a section's commands go: `{page?, visual?}`.
#[derive(Debug, Clone)]
struct Target {
    scope: Scope,
    page: Option<String>,
    visual: Option<String>,
    /// The section's sort mode, from the snapshot (`filter_sort`).
    sort: String,
}

impl Target {
    /// `filter.order`'s parameters for this section.
    fn order_params(&self) -> Value {
        let mut p = json!({"level": self.scope.prefix()});
        if let Some(pg) = &self.page {
            p["page"] = json!(pg);
        }
        if let Some(v) = &self.visual {
            p["visual"] = json!(v);
        }
        p
    }

    fn params(&self, field: &str) -> Value {
        let mut p = json!({"field": field});
        if let Some(pg) = &self.page {
            p["page"] = json!(pg);
        }
        if let Some(v) = &self.visual {
            p["visual"] = json!(v);
        }
        p
    }

    fn key(&self, field: &str) -> String {
        format!(
            "{}:{}:{}:{field}",
            self.scope.prefix(),
            self.page.as_deref().unwrap_or(""),
            self.visual.as_deref().unwrap_or("")
        )
    }
}

/// A level's sort mode in the snapshot (`ascending` when absent).
fn sort_of(v: &Value) -> String {
    s(v, "filter_sort").unwrap_or_else(|| "ascending".into())
}

/// What a row asks for after the frame.
enum Act {
    Run(&'static str, Value),
    Values(String, String, String),
}

/// Runs command `id` as the pane's action (refused with the reason in the status bar).
fn exec(app: &mut App, id: &'static str, params: Value) {
    if let Err(why) = ribbon::state(app, id) {
        app.status = format!("{} {why}", app.lang.t("Unavailable:", "Indisponible :"));
        return;
    }
    app.run(Call { id, params }, Then::Nothing);
}

/// Keeps the answer of a `column.values` call ([`Then::FilterValues`]).
pub fn values_answer(app: &mut App, field: &str, revision: u64, r: Result<Value, String>) {
    let (values, cut) = match r {
        Ok(v) => (
            arr(v.get("values")).iter().take(10_000).cloned().collect(),
            v.get("truncated").and_then(Value::as_bool).unwrap_or(false),
        ),
        Err(e) => {
            app.status = clip(&e, 300);
            (Vec::new(), false)
        }
    };
    app.filters
        .values
        .insert(field.to_string(), (revision, values, cut));
}

/// The pane (the leftmost of the three right panes).
pub fn pane(app: &mut App, ui: &mut Ui) {
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    // the page's Filters pane format (page.format_filter_pane, the theme's defaults)
    let style = crate::filter_pane_ui::active_style(app);
    let pane_fg = style.pane.foreground.unwrap_or(t.text);
    let family = style.pane.family.clone();
    let mut collapse = false;
    let mut eye: Option<bool> = None;
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(l.t("Filters", "Filtres"))
                .font(crate::filter_pane_ui::font(
                    family.as_deref(),
                    style.pane.title_size.unwrap_or(14.0),
                    true,
                ))
                .color(pane_fg),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            collapse = widgets::icon_toggle_tinted(
                ui,
                Icon::ChevronRight,
                l.t("Collapse the Filters pane", "Réduire le volet Filtres"),
                true,
                false,
                style.pane.foreground,
            )
            .clicked();
            // Desktop's eye next to Filters: whether readers see the pane (a report setting)
            if app.access.as_deref() != Some("reader") {
                let shown = visible_to_readers(app);
                let can = ribbon::state(app, "report.settings").is_ok();
                let (icon, tip) = if shown {
                    (
                        Icon::Eye,
                        l.t(
                            "Hide the Filters pane from readers",
                            "Masquer le volet Filtres aux lecteurs",
                        ),
                    )
                } else {
                    (
                        Icon::EyeOff,
                        l.t(
                            "Show the Filters pane to readers",
                            "Afficher le volet Filtres aux lecteurs",
                        ),
                    )
                };
                if widgets::icon_toggle_tinted(ui, icon, tip, can, !shown, style.pane.foreground).clicked() {
                    eye = Some(!shown);
                }
            }
        });
    });
    if let Some(visible) = eye {
        app.run(
            crate::actions::report_settings(json!({"filter_pane_visible": visible})),
            crate::app::Then::Nothing,
        );
    }
    if collapse {
        app.prefs.panes.filters = false;
        app.save_prefs();
        return;
    }
    ui.add_space(4.0);
    // Desktop's Filters pane has a search box under its title, over the cards
    let mut find = egui::TextEdit::singleline(&mut app.filters.find)
        .hint_text(
            egui::RichText::new(l.t("Search", "Rechercher")).color(
                style
                    .pane
                    .foreground
                    .map_or(t.text_faint, |c| c.gamma_multiply(0.7)),
            ),
        )
        .font(crate::filter_pane_ui::font(
            family.as_deref(),
            style.pane.search_size.unwrap_or(13.0),
            false,
        ))
        .desired_width(ui.available_width());
    if let Some(c) = style.pane.input_box {
        find = find.background_color(c);
    }
    if let Some(c) = style.pane.foreground {
        find = find.text_color(c);
    }
    let name = l.t("Search the filters", "Rechercher dans les filtres");
    ui.add(find)
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, name));
    ui.add_space(4.0);
    let Some(snap) = app.loaded.as_ref().map(|x| x.snap.clone()) else {
        return;
    };
    let revision = snap.get("revision").and_then(Value::as_u64).unwrap_or(0);
    let page = snap
        .pointer(&format!("/report/pages/{}", app.active_page()))
        .cloned()
        .unwrap_or(Value::Null);
    let page_name = s(&page, "name");
    let sel = app
        .cmd
        .visual
        .clone()
        .filter(|v| page_name.as_deref() == Some(v.page.as_str()));
    let visual = sel.as_ref().and_then(|sel| {
        arr(page.get("visuals"))
            .iter()
            .find(|v| s(v, "name").as_deref() == Some(sel.visual.as_str()))
            .cloned()
    });
    let mut sections: Vec<(Target, Vec<Card>)> = Vec::new();
    if let (Some(sel), Some(v)) = (&sel, &visual)
        && v.get("type").is_some_and(Value::is_string)
    {
        sections.push((
            Target {
                scope: Scope::Visual,
                page: Some(sel.page.clone()),
                visual: Some(sel.visual.clone()),
                sort: sort_of(v),
            },
            visual_cards(v),
        ));
    }
    if let Some(pg) = &page_name {
        sections.push((
            Target {
                scope: Scope::Page,
                page: Some(pg.clone()),
                visual: None,
                sort: sort_of(&page),
            },
            cards_of(page.get("filters")),
        ));
    }
    sections.push((
        Target {
            scope: Scope::Report,
            page: None,
            visual: None,
            sort: snap
                .get("report")
                .map_or_else(|| "ascending".to_string(), sort_of),
        },
        cards_of(snap.pointer("/report/filters")),
    ));
    let states: Vec<(&'static str, Result<(), String>)> =
        COMMANDS.iter().map(|id| (*id, ribbon::state(app, id))).collect();
    let state = |id: &str| {
        states
            .iter()
            .find(|(x, _)| *x == id)
            .map_or_else(|| Err("not available".to_string()), |(_, s)| s.clone())
    };
    let payload = DragAndDrop::payload::<Drag>(ui.ctx()).map(|d| (*d).clone());
    // the snapshot's model says which columns are dates; a reader does not see hidden cards
    let reader = app.access.as_deref() == Some("reader");
    for (_, cards) in &mut sections {
        mark_dates(cards, &snap);
        if reader {
            cards.retain(|c| !c.hidden);
        }
    }
    let mut acts: Vec<Act> = Vec::new();
    let ui_state = &mut app.filters;
    let q = ui_state.find.trim().to_lowercase();
    egui::ScrollArea::vertical()
        .id_salt("bicraft-filters-pane")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (target, cards) in &sections {
                // the search keeps the cards whose field matches it (a dragged field still adds
                // one: the slot below the list stays)
                let shown: Vec<Card> = if q.is_empty() {
                    cards.clone()
                } else {
                    cards
                        .iter()
                        .filter(|c| {
                            c.field.to_lowercase().contains(&q)
                                || c.name.as_deref().is_some_and(|n| n.to_lowercase().contains(&q))
                        })
                        .cloned()
                        .collect()
                };
                let edit = state(target.scope.filter_cmd());
                let clear = state(target.scope.clear_cmd());
                let order = state("filter.order");
                let cx = SectionCx {
                    target,
                    edit: &edit,
                    clear: &clear,
                    lang: l,
                    revision,
                    payload: payload.as_ref(),
                    reader,
                    style: &style,
                    order: &order,
                    sort: &target.sort,
                };
                section(ui, &cx, &shown, ui_state, &mut acts);
                ui.add_space(10.0);
            }
        });
    for a in acts {
        match a {
            Act::Run(id, p) => exec(app, id, p),
            Act::Values(field, table, column) => {
                if ribbon::state(app, "column.values").is_ok() {
                    app.filters.asked.insert((field.clone(), revision));
                    app.run(
                        Call {
                            id: "column.values",
                            params: json!({"table": table, "column": column}),
                        },
                        Then::FilterValues(field, revision),
                    );
                }
            }
        }
    }
}

struct SectionCx<'a> {
    target: &'a Target,
    edit: &'a Result<(), String>,
    clear: &'a Result<(), String>,
    lang: Lang,
    revision: u64,
    payload: Option<&'a Drag>,
    /// A reader's pane: no lock or eye toggles.
    reader: bool,
    /// The page's Filters pane format.
    style: &'a crate::filter_pane_ui::PaneStyle,
    /// Whether `filter.order` can run.
    order: &'a Result<(), String>,
    /// The section's sort mode (`ascending`, `descending`, `custom`).
    sort: &'a str,
}

fn section(
    ui: &mut Ui,
    cx: &SectionCx<'_>,
    cards: &[Card],
    st: &mut crate::filters_pane::FiltersUi,
    acts: &mut Vec<Act>,
) {
    let t = Tokens::get(ui.ctx());
    let l = cx.lang;
    let title = cx.target.scope.title(l);
    let top = ui.cursor().min.y;
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
    let ps = &cx.style.pane;
    let title_rect = ui.painter().text(
        pos2(r.min.x, r.center().y),
        Align2::LEFT_CENTER,
        title,
        crate::filter_pane_ui::font(ps.family.as_deref(), ps.header_size.unwrap_or(12.0), true),
        ps.foreground.unwrap_or(t.text_dim),
    );
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, title));
    // the section's sort order (filterSortOrder): A to Z, Z to A or custom (cards dragged)
    if !cx.reader && cx.order.is_ok() && cards.iter().any(|c| c.own) {
        let words = |m: &str| match m {
            "descending" => l.t("Z to A", "De Z à A"),
            "custom" => l.t("Custom", "Personnalisé"),
            _ => l.t("A to Z", "De A à Z"),
        };
        let name = format!("{title} {}", l.t("sort order", "ordre de tri"));
        let mut picked = None;
        // right of the title, never over it (a long French title leaves a narrower list)
        let width = (r.max.x - title_rect.max.x - 8.0).clamp(56.0, 104.0);
        let sr = Rect::from_min_max(pos2(r.max.x - width, r.min.y - 1.0), pos2(r.max.x, r.max.y + 1.0));
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(sr)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
            |ui| {
                let cr = egui::ComboBox::from_id_salt(("bicraft-filter-sort", cx.target.scope))
                    .selected_text(egui::RichText::new(words(cx.sort)).small())
                    .width(width - 8.0)
                    .show_ui(ui, |ui| {
                        for m in ["ascending", "descending", "custom"] {
                            if ui.selectable_label(cx.sort == m, words(m)).clicked() && cx.sort != m {
                                picked = Some(m);
                            }
                        }
                    });
                cr.response
                    .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, &name));
            },
        );
        if let Some(m) = picked {
            let mut p = cx.target.order_params();
            p["sort"] = json!(m);
            acts.push(Act::Run("filter.order", p));
        }
    }
    let mut placed: Vec<(&Card, Rect)> = Vec::with_capacity(cards.len());
    for c in cards {
        let rect = card_ui(ui, cx, c, st, acts);
        placed.push((c, rect));
        ui.add_space(4.0);
    }
    drop_card(ui, cx, &placed, acts);
    // the drop slot: a field dragged here adds an empty card
    let (slot, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
    let field = match cx.payload {
        Some(Drag::Field(f)) => Some(f),
        _ => None,
    };
    let can_take = cx.edit.is_ok() && field.is_some_and(|f| cx.target.scope == Scope::Visual || !f.measure);
    let area = Rect::from_min_max(pos2(slot.min.x, top), slot.max);
    let hovered = can_take && ui.rect_contains_pointer(area);
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
    let slot_text = l.t("Add data fields here", "Ajoutez des champs ici");
    ui.painter().text(
        pos2(slot.min.x + 8.0, slot.center().y),
        Align2::LEFT_CENTER,
        slot_text,
        theme::font(11.0),
        t.text_faint,
    );
    let label = format!("{slot_text} ({title})");
    let sr = ui.interact(
        slot,
        ui.id().with(("bicraft-filter-slot", cx.target.scope)),
        Sense::hover(),
    );
    sr.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &label));
    if can_take {
        ui.painter().rect_stroke(
            area.expand(2.0),
            CornerRadius::same(5),
            Stroke::new(
                if hovered { 2.0 } else { 1.0 },
                if hovered { t.ring } else { t.accent_soft },
            ),
            StrokeKind::Outside,
        );
    }
    if hovered
        && ui.input(|i| i.pointer.any_released())
        && let Some(Drag::Field(f)) = DragAndDrop::take_payload::<Drag>(ui.ctx()).map(|d| (*d).clone())
    {
        acts.push(Act::Run(cx.target.scope.filter_cmd(), cx.target.params(&f.text)));
        st.open.insert(cx.target.key(&f.text));
    }
    if let Err(why) = cx.edit {
        let _ = sr.on_hover_text(format!("{} {why}", l.t("Read-only:", "Lecture seule :")));
    }
}

/// A card dragged over its own section: a line where it would go, and on release
/// `filter.order {field, to}` (the custom order), `to` counted among the section's own cards.
fn drop_card(ui: &mut Ui, cx: &SectionCx<'_>, placed: &[(&Card, Rect)], acts: &mut Vec<Act>) {
    let Some(d) = DragAndDrop::payload::<CardDrag>(ui.ctx()).map(|d| (*d).clone()) else {
        return;
    };
    if d.scope != cx.target.scope {
        return;
    }
    let Some(pos) = ui.ctx().pointer_interact_pos() else {
        return;
    };
    let own: Vec<Rect> = placed.iter().filter(|(c, _)| c.own).map(|(_, r)| *r).collect();
    let Some(from) = placed
        .iter()
        .filter(|(c, _)| c.own)
        .position(|(c, _)| c.field == d.field)
    else {
        return;
    };
    let area = own
        .iter()
        .fold(Rect::NOTHING, |a, r| a.union(*r))
        .expand2(vec2(0.0, 12.0));
    if !area.contains(pos) {
        return;
    }
    let ins = own.iter().filter(|r| r.center().y < pos.y).count();
    let y = match own.get(ins) {
        Some(r) => r.min.y - 2.0,
        None => own.last().map_or(area.max.y, |r| r.max.y + 2.0),
    };
    let t = Tokens::get(ui.ctx());
    ui.painter().hline(area.x_range(), y, Stroke::new(2.0, t.ring));
    if ui.input(|i| i.pointer.any_released()) {
        let to = if from < ins { ins.saturating_sub(1) } else { ins };
        if to != from {
            let mut p = cx.target.order_params();
            p["field"] = json!(d.field);
            p["to"] = json!(to);
            acts.push(Act::Run("filter.order", p));
        }
        let _ = DragAndDrop::take_payload::<CardDrag>(ui.ctx());
    }
}

/// One card; returns its rectangle.
fn card_ui(ui: &mut Ui, cx: &SectionCx<'_>, c: &Card, st: &mut FiltersUi, acts: &mut Vec<Act>) -> Rect {
    let t = Tokens::get(ui.ctx());
    let l = cx.lang;
    let key = cx.target.key(&c.field);
    let open = st.open.contains(&key);
    let label = card_label(c, l);
    let sum = summary(c, l);
    // Desktop's card states: Applied (a condition) or Available
    let cs = if c.ty != "none" {
        &cx.style.applied
    } else {
        &cx.style.available
    };
    let fg = cs.foreground.unwrap_or(t.text);
    let fg_dim = cs.foreground.unwrap_or(t.text_dim);
    let size = cs.text_size.unwrap_or(12.0);
    let stroke = match cs.border {
        Some(false) => Stroke::NONE,
        _ => Stroke::new(1.0, cs.border_color.unwrap_or(t.border)),
    };
    let frame = egui::Frame::NONE
        .fill(cs.background.unwrap_or(t.field_bg))
        .stroke(stroke)
        .corner_radius(CornerRadius::same(4))
        .inner_margin(egui::Margin::same(6));
    // a card of the level's filterConfig is dragged to a new place in its section (an author's)
    let draggable = c.own && !cx.reader && cx.order.is_ok();
    frame
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            // header: chevron, name and condition; the eraser and ×
            let (head, resp) = ui.allocate_exact_size(
                vec2(ui.available_width(), 34.0),
                if draggable {
                    Sense::click_and_drag()
                } else {
                    Sense::click()
                },
            );
            if draggable && resp.drag_started() {
                DragAndDrop::set_payload(
                    ui.ctx(),
                    CardDrag {
                        scope: cx.target.scope,
                        field: c.field.clone(),
                    },
                );
            }
            if draggable && resp.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            }
            let p = ui.painter();
            let ch = pos2(head.min.x + 6.0, head.min.y + 9.0);
            if open {
                widgets::chevron(p, ch, 3.5, fg);
            } else {
                p.line_segment(
                    [pos2(ch.x - 2.0, ch.y - 4.0), pos2(ch.x + 2.0, ch.y)],
                    Stroke::new(1.2, fg),
                );
                p.line_segment(
                    [pos2(ch.x + 2.0, ch.y), pos2(ch.x - 2.0, ch.y + 4.0)],
                    Stroke::new(1.2, fg),
                );
            }
            let text_max = head.max.x - 94.0;
            let clip_rect = Rect::from_min_max(head.min, pos2(text_max, head.max.y));
            p.with_clip_rect(clip_rect).text(
                pos2(head.min.x + 16.0, head.min.y + 9.0),
                Align2::LEFT_CENTER,
                clip(&label, MAX_LABEL),
                crate::filter_pane_ui::font(cs.family.as_deref(), size, true),
                fg,
            );
            // the icons sit on the title's row: the condition below takes the card's width
            p.with_clip_rect(head).text(
                pos2(head.min.x + 16.0, head.min.y + 25.0),
                Align2::LEFT_CENTER,
                clip(&sum, 80),
                crate::filter_pane_ui::font(cs.family.as_deref(), (size - 1.0).max(6.0), false),
                fg_dim,
            );
            let full = format!("{label} {sum}");
            resp.widget_info(|| {
                egui::WidgetInfo::selected(egui::WidgetType::CollapsingHeader, true, open, &full)
            });
            if resp.clicked() {
                if open {
                    st.open.remove(&key);
                } else {
                    st.open.insert(key.clone());
                }
            }
            let has_cond = c.ty != "none";
            // the eraser: clear the condition, keep the card
            let er = Rect::from_center_size(pos2(head.max.x - 34.0, head.min.y + 10.0), vec2(18.0, 18.0));
            let clear_tip = format!("{} {label}", l.t("Clear filter on", "Effacer le filtre sur"));
            let can_clear = cx.clear.is_ok() && has_cond;
            let err = ui.interact(
                er,
                ui.id().with(("clear", &key)),
                if can_clear { Sense::click() } else { Sense::hover() },
            );
            if err.hovered() && can_clear {
                ui.painter().rect_filled(er, CornerRadius::same(4), t.pressed);
            }
            icons::paint(ui.painter(), er.shrink(3.0), Icon::Undo, fg_dim, !can_clear);
            err.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, can_clear, &clear_tip));
            let err = err.on_hover_text(&clear_tip);
            if err.clicked() && can_clear {
                let mut p = cx.target.params(&c.field);
                p["keep_cards"] = json!(true);
                acts.push(Act::Run(cx.target.scope.clear_cmd(), p));
            }
            lock_and_eye(ui, cx, c, head, &key, &label, acts);
            // ×: remove a card added by hand
            if !c.auto && cx.clear.is_ok() {
                let xr = Rect::from_center_size(pos2(head.max.x - 12.0, head.min.y + 10.0), vec2(18.0, 18.0));
                let tip = format!("{} {label}", l.t("Remove filter on", "Supprimer le filtre sur"));
                let x = ui.interact(xr, ui.id().with(("remove", &key)), Sense::click());
                if x.hovered() {
                    ui.painter().rect_filled(xr, CornerRadius::same(4), t.pressed);
                }
                icons::paint(ui.painter(), xr.shrink(4.0), Icon::Close, fg_dim, false);
                x.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &tip));
                if x.on_hover_text(&tip).clicked() {
                    acts.push(Act::Run(cx.target.scope.clear_cmd(), cx.target.params(&c.field)));
                    st.open.remove(&key);
                }
            }
            if open {
                card_body(ui, cx, c, &key, st, acts);
            }
        })
        .response
        .rect
}

/// The lock and the eye of a card's header, as Desktop draws them on each card while editing:
/// a click locks the card for readers (or unlocks it), or hides it from them (or shows it), with
/// `*.filter {field, locked | hidden}`. A reader sees the lock of a locked card, not a toggle.
#[allow(clippy::too_many_arguments)]
fn lock_and_eye(
    ui: &mut Ui,
    cx: &SectionCx<'_>,
    c: &Card,
    head: Rect,
    key: &str,
    label: &str,
    acts: &mut Vec<Act>,
) {
    let t = Tokens::get(ui.ctx());
    let l = cx.lang;
    let card_fg = if c.ty != "none" {
        cx.style.applied.foreground
    } else {
        cx.style.available.foreground
    };
    let lock_rect = Rect::from_center_size(pos2(head.max.x - 56.0, head.min.y + 10.0), vec2(18.0, 18.0));
    let eye_rect = Rect::from_center_size(pos2(head.max.x - 78.0, head.min.y + 10.0), vec2(18.0, 18.0));
    if cx.reader {
        if c.locked {
            let r = ui.interact(lock_rect, ui.id().with(("locked", key)), Sense::hover());
            icons::paint(
                ui.painter(),
                lock_rect.shrink(3.0),
                Icon::Lock,
                card_fg.unwrap_or(t.text_dim),
                false,
            );
            let tip = format!("{} {label}", l.t("Locked filter:", "Filtre verrouillé :"));
            r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &tip));
            let _ = r.on_hover_text(&tip);
        }
        return;
    }
    let enabled = cx.edit.is_ok();
    let toggles: [(Rect, &str, bool, Icon, String, &str); 2] = [
        (
            lock_rect,
            "lock",
            c.locked,
            Icon::Lock,
            if c.locked {
                format!(
                    "{} {label}",
                    l.t("Unlock filter on", "Déverrouiller le filtre sur")
                )
            } else {
                format!("{} {label}", l.t("Lock filter on", "Verrouiller le filtre sur"))
            },
            "locked",
        ),
        (
            eye_rect,
            "eye",
            c.hidden,
            if c.hidden { Icon::EyeOff } else { Icon::Eye },
            if c.hidden {
                format!(
                    "{} {label} {}",
                    l.t("Show filter on", "Afficher le filtre sur"),
                    l.t("to readers", "aux lecteurs")
                )
            } else {
                format!(
                    "{} {label} {}",
                    l.t("Hide filter on", "Masquer le filtre sur"),
                    l.t("from readers", "aux lecteurs")
                )
            },
            "hidden",
        ),
    ];
    for (rect, what, on, icon, tip, param) in toggles {
        let r = ui.interact(
            rect,
            ui.id().with((what, key)),
            if enabled { Sense::click() } else { Sense::hover() },
        );
        if on {
            ui.painter().rect_filled(rect, CornerRadius::same(4), t.pressed);
        } else if r.hovered() && enabled {
            ui.painter().rect_filled(rect, CornerRadius::same(4), t.hover);
        }
        // off: the faint outline icon; on: the full icon on a pressed background (the card's
        // font and icon colour when the report sets one)
        let color = match card_fg {
            Some(c) if on => c,
            Some(c) => c.gamma_multiply(0.7),
            None if on => t.text,
            None => t.text_faint,
        };
        icons::paint(ui.painter(), rect.shrink(3.0), icon, color, !enabled);
        r.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, on, &tip));
        let r = match cx.edit {
            Err(why) => r.on_hover_text(format!("{} {why}", l.t("Read-only:", "Lecture seule :"))),
            Ok(()) => r.on_hover_text(&tip),
        };
        if r.clicked() && enabled {
            let mut p = cx.target.params(&c.field);
            p[param] = json!(!on);
            acts.push(Act::Run(cx.target.scope.filter_cmd(), p));
        }
    }
}

fn card_body(ui: &mut Ui, cx: &SectionCx<'_>, c: &Card, key: &str, st: &mut FiltersUi, acts: &mut Vec<Act>) {
    let l = cx.lang;
    let t = Tokens::get(ui.ctx());
    let enabled = cx.edit.is_ok();
    let column = c.kind == "column";
    let mut d = st.drafts.get(key).cloned().unwrap_or_else(|| draft_of(c));
    // the filter type
    let modes: Vec<(Mode, &str)> = {
        let mut m = Vec::new();
        if column {
            m.push((Mode::Basic, l.t("Basic filtering", "Filtrage de base")));
        }
        m.push((Mode::Advanced, l.t("Advanced filtering", "Filtrage avancé")));
        if column && cx.target.scope == Scope::Visual {
            m.push((Mode::TopN, l.t("Top N", "N premiers")));
        }
        if column && c.date {
            m.push((
                Mode::RelativeDate,
                l.t("Relative date filtering", "Filtrage de date relative"),
            ));
            m.push((
                Mode::RelativeTime,
                l.t("Relative time filtering", "Filtrage d'heure relative"),
            ));
        }
        m
    };
    if !modes.iter().any(|(m, _)| *m == d.mode) {
        d.mode = modes.first().map_or(Mode::Advanced, |m| m.0);
    }
    ui.add_space(2.0);
    ui.label(
        egui::RichText::new(l.t("Filter type", "Type de filtre"))
            .size(11.0)
            .color(t.text_dim),
    );
    let shown = modes.iter().find(|(m, _)| *m == d.mode).map_or("", |m| m.1);
    ui.add_enabled_ui(enabled, |ui| {
        let r = egui::ComboBox::from_id_salt(("bicraft-filter-type", key))
            .selected_text(shown)
            .width((ui.available_width() - 4.0).max(60.0))
            .show_ui(ui, |ui| {
                for (m, text) in &modes {
                    if ui.selectable_label(d.mode == *m, *text).clicked() {
                        d.mode = *m;
                    }
                }
            });
        let name = format!(
            "{} {}",
            l.t("Filter type of", "Type de filtre de"),
            card_label(c, l)
        );
        r.response
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, enabled, &name));
    });
    ui.add_space(4.0);
    match d.mode {
        Mode::Basic => basic_ui(ui, cx, c, key, st, acts),
        Mode::Advanced => advanced_ui(ui, cx, c, &mut d, acts),
        Mode::TopN => top_n_ui(ui, cx, c, &mut d, acts),
        Mode::RelativeDate | Mode::RelativeTime => relative_ui(ui, cx, c, &mut d, acts),
    }
    st.drafts.insert(key.to_string(), d);
}

fn basic_ui(ui: &mut Ui, cx: &SectionCx<'_>, c: &Card, key: &str, st: &mut FiltersUi, acts: &mut Vec<Act>) {
    let l = cx.lang;
    let t = Tokens::get(ui.ctx());
    let enabled = cx.edit.is_ok();
    let (Some(table), Some(column)) = (c.table.clone(), c.name.clone()) else {
        return;
    };
    // the values: asked once per revision, the last ones shown meanwhile
    let fresh = st.values.get(&c.field).is_some_and(|(r, _, _)| *r == cx.revision);
    if !fresh && !st.asked.contains(&(c.field.clone(), cx.revision)) {
        acts.push(Act::Values(c.field.clone(), table, column));
    }
    let search = st.search.entry(key.to_string()).or_default();
    let cs = if c.ty != "none" {
        &cx.style.applied
    } else {
        &cx.style.available
    };
    let mut edit = egui::TextEdit::singleline(search)
        .hint_text(l.t("Search", "Rechercher"))
        .desired_width(ui.available_width());
    if let Some(bg) = cs.input_box {
        edit = edit.background_color(bg);
    }
    let r = ui.add(edit);
    let name = format!(
        "{} {}",
        l.t("Search the values of", "Rechercher les valeurs de"),
        card_label(c, l)
    );
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &name));
    let q = search.trim().to_lowercase();
    let Some((_, values, cut)) = st.values.get(&c.field) else {
        ui.label(egui::RichText::new(l.t("Reading the values…", "Lecture des valeurs…")).color(t.text_faint));
        return;
    };
    let all_on = values.iter().all(|v| checked(c, v));
    let mut clicked: Option<Option<Value>> = None;
    ui.add_enabled_ui(enabled, |ui| {
        let mut on = all_on;
        let r = ui.checkbox(&mut on, l.t("Select all", "Sélectionner tout"));
        if r.clicked() && !all_on {
            clicked = Some(None);
        }
        let mut n = 0usize;
        for v in values {
            let text = value_text(v, l);
            if !q.is_empty() && !text.to_lowercase().contains(&q) {
                continue;
            }
            n += 1;
            if n > MAX_SHOWN {
                ui.label(egui::RichText::new("…").color(t.text_faint));
                break;
            }
            let mut on = checked(c, v);
            if ui.checkbox(&mut on, clip(&text, MAX_LABEL)).clicked() {
                clicked = Some(Some(v.clone()));
            }
        }
        if values.is_empty() {
            ui.label(egui::RichText::new(l.t("No values.", "Aucune valeur.")).color(t.text_faint));
        }
    });
    if *cut {
        ui.label(
            egui::RichText::new(l.t(
                "Only the first values are listed.",
                "Seules les premières valeurs sont listées.",
            ))
            .size(11.0)
            .color(t.text_faint),
        );
    }
    let clear = || {
        let mut p = cx.target.params(&c.field);
        p["keep_cards"] = json!(true);
        Act::Run(cx.target.scope.clear_cmd(), p)
    };
    match clicked {
        None => {}
        Some(None) => {
            if c.ty != "none" {
                acts.push(clear());
            }
        }
        Some(Some(v)) => match toggle_value(c, values, &v) {
            None => {}
            Some(Err(())) => {
                if c.ty != "none" {
                    acts.push(clear());
                }
            }
            Some(Ok(cond)) => {
                let mut p = cx.target.params(&c.field);
                if let (Some(o), Some(x)) = (p.as_object_mut(), cond.as_object()) {
                    o.extend(x.clone());
                }
                acts.push(Act::Run(cx.target.scope.filter_cmd(), p));
            }
        },
    }
}

fn op_combo(ui: &mut Ui, id: (&str, &str, u8), op: &mut String, l: Lang, numeric: bool, label: &str) {
    let shown = if op.is_empty() {
        String::new()
    } else {
        op_words(op, l)
    };
    let r = egui::ComboBox::from_id_salt(id)
        .selected_text(shown)
        .width((ui.available_width() - 4.0).max(60.0))
        .show_ui(ui, |ui| {
            if id.2 == 2
                && ui
                    .selectable_label(op.is_empty(), l.t("(none)", "(aucune)"))
                    .clicked()
            {
                op.clear();
            }
            for (code, en, fr, _) in OPS {
                let text_op = matches!(
                    *code,
                    "contains" | "not_contains" | "starts_with" | "not_starts_with"
                );
                if numeric && text_op {
                    continue;
                }
                if ui.selectable_label(op == code, l.t(en, fr)).clicked() {
                    *op = (*code).to_string();
                }
            }
        });
    r.response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, label));
}

fn needs_value(op: &str) -> bool {
    !op.is_empty() && OPS.iter().find(|o| o.0 == op).is_none_or(|o| o.3)
}

fn advanced_ui(ui: &mut Ui, cx: &SectionCx<'_>, c: &Card, d: &mut Draft, acts: &mut Vec<Act>) {
    let l = cx.lang;
    let t = Tokens::get(ui.ctx());
    let enabled = cx.edit.is_ok();
    let numeric = c.kind != "column" || c.condition.get("value").is_some_and(Value::is_number);
    let name = card_label(c, l);
    ui.add_enabled_ui(enabled, |ui| {
        ui.label(
            egui::RichText::new(l.t(
                "Show items when the value",
                "Afficher les éléments dont la valeur",
            ))
            .size(11.0)
            .color(t.text_dim),
        );
        let key = c.field.as_str();
        op_combo(
            ui,
            ("bicraft-op", key, 1),
            &mut d.op1,
            l,
            numeric && c.kind != "column",
            &format!("{} 1 {name}", l.t("Condition", "Condition")),
        );
        if needs_value(&d.op1) {
            let r = ui.add(egui::TextEdit::singleline(&mut d.value1).desired_width(ui.available_width()));
            let lbl = format!("{} 1 {name}", l.t("Value", "Valeur"));
            r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &lbl));
        }
        ui.horizontal(|ui| {
            ui.radio_value(&mut d.or, false, l.t("And", "Et"));
            ui.radio_value(&mut d.or, true, l.t("Or", "Ou"));
        });
        op_combo(
            ui,
            ("bicraft-op", key, 2),
            &mut d.op2,
            l,
            numeric && c.kind != "column",
            &format!("{} 2 {name}", l.t("Condition", "Condition")),
        );
        if needs_value(&d.op2) {
            let r = ui.add(egui::TextEdit::singleline(&mut d.value2).desired_width(ui.available_width()));
            let lbl = format!("{} 2 {name}", l.t("Value", "Valeur"));
            r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &lbl));
        }
        apply_row(ui, cx, c, d, numeric, acts);
    });
}

fn top_n_ui(ui: &mut Ui, cx: &SectionCx<'_>, c: &Card, d: &mut Draft, acts: &mut Vec<Act>) {
    let l = cx.lang;
    let t = Tokens::get(ui.ctx());
    let enabled = cx.edit.is_ok();
    let name = card_label(c, l);
    ui.add_enabled_ui(enabled, |ui| {
        ui.label(
            egui::RichText::new(l.t("Show items", "Afficher les éléments"))
                .size(11.0)
                .color(t.text_dim),
        );
        ui.horizontal(|ui| {
            ui.radio_value(&mut d.bottom, false, l.t("Top", "Haut"));
            ui.radio_value(&mut d.bottom, true, l.t("Bottom", "Bas"));
            let r = ui.add(egui::TextEdit::singleline(&mut d.count).desired_width(48.0));
            let lbl = format!("{} {name}", l.t("Number of items of", "Nombre d'éléments de"));
            r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &lbl));
        });
        ui.label(
            egui::RichText::new(l.t("By value", "Par valeur"))
                .size(11.0)
                .color(t.text_dim),
        );
        let r = ui.add(
            egui::TextEdit::singleline(&mut d.by)
                .hint_text(l.t("Add data fields here", "Ajoutez des champs ici"))
                .desired_width(ui.available_width()),
        );
        let lbl = format!("{} {name}", l.t("By value of", "Par valeur de"));
        r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &lbl));
        // a measure or a number column dragged here
        if let Some(Drag::Field(f)) = cx.payload {
            let target = r.rect.expand(4.0);
            let over = ui.rect_contains_pointer(target);
            ui.painter().rect_stroke(
                target,
                CornerRadius::same(4),
                Stroke::new(
                    if over { 2.0 } else { 1.0 },
                    if over { t.ring } else { t.accent_soft },
                ),
                StrokeKind::Outside,
            );
            if over
                && ui.input(|i| i.pointer.any_released())
                && DragAndDrop::take_payload::<Drag>(ui.ctx()).is_some()
            {
                d.by = by_text(f);
            }
        }
        apply_row(ui, cx, c, d, true, acts);
    });
}

/// A combo box over `(code, en, fr)` choices, labelled `label` for the accessibility tree.
fn choice_combo(
    ui: &mut Ui,
    id: (&str, &str),
    list: &[(&str, &'static str, &'static str)],
    value: &mut String,
    l: Lang,
    label: &str,
) {
    let r = egui::ComboBox::from_id_salt(id)
        .selected_text(rel_words(list, value, l))
        .width((ui.available_width() - 4.0).max(60.0))
        .show_ui(ui, |ui| {
            for (code, en, fr) in list {
                if ui.selectable_label(value == code, l.t(en, fr)).clicked() {
                    *value = (*code).to_string();
                }
            }
        });
    r.response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, label));
}

/// Relative date and relative time filtering: is in the last / this / next, N, the period,
/// Include today (dates), applied with Apply filter.
fn relative_ui(ui: &mut Ui, cx: &SectionCx<'_>, c: &Card, d: &mut Draft, acts: &mut Vec<Act>) {
    let l = cx.lang;
    let t = Tokens::get(ui.ctx());
    let enabled = cx.edit.is_ok();
    let name = card_label(c, l);
    let time = d.mode == Mode::RelativeTime;
    let units = if time { REL_TIME_UNITS } else { REL_DATE_UNITS };
    if !units.iter().any(|u| u.0 == d.rel_unit) {
        d.rel_unit = units.first().map_or("days", |u| u.0).to_string();
    }
    if !REL_RANGES.iter().any(|r| r.0 == d.rel_range) {
        d.rel_range = "last".into();
    }
    ui.add_enabled_ui(enabled, |ui| {
        ui.label(
            egui::RichText::new(l.t(
                "Show items when the value",
                "Afficher les éléments dont la valeur",
            ))
            .size(11.0)
            .color(t.text_dim),
        );
        let key = c.field.as_str();
        choice_combo(
            ui,
            ("bicraft-rel-range", key),
            REL_RANGES,
            &mut d.rel_range,
            l,
            &format!("{} {name}", l.t("Range of", "Plage de")),
        );
        ui.horizontal(|ui| {
            if d.rel_range != "this" {
                let r = ui.add(egui::TextEdit::singleline(&mut d.rel_count).desired_width(48.0));
                let lbl = format!("{} {name}", l.t("Number of periods of", "Nombre de périodes de"));
                r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &lbl));
            }
            choice_combo(
                ui,
                ("bicraft-rel-unit", key),
                units,
                &mut d.rel_unit,
                l,
                &format!("{} {name}", l.t("Period of", "Période de")),
            );
        });
        if !time && d.rel_range != "this" {
            let r = ui.checkbox(&mut d.rel_today, l.t("Include today", "Inclure aujourd'hui"));
            let lbl = format!("{} ({name})", l.t("Include today", "Inclure aujourd'hui"));
            r.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, d.rel_today, &lbl));
        }
        apply_row(ui, cx, c, d, false, acts);
    });
}

/// How a field dropped on Top N's By value is ranked: a measure as is, a column summed (counted
/// when it is not a number).
pub fn by_text(f: &Field) -> String {
    if f.measure {
        measure_text(&f.name)
    } else if f.numeric {
        format!("Sum({})", column_text(&f.table, &f.name))
    } else {
        format!("Count({})", column_text(&f.table, &f.name))
    }
}

fn apply_row(ui: &mut Ui, cx: &SectionCx<'_>, c: &Card, d: &Draft, numeric: bool, acts: &mut Vec<Act>) {
    let l = cx.lang;
    let ready = draft_params(d, numeric, l);
    let label = format!(
        "{} {}",
        l.t("Apply filter on", "Appliquer le filtre sur"),
        card_label(c, l)
    );
    // the pane's checkbox and Apply colour (page.format_filter_pane), its text in the more
    // legible of black and white
    let button = match cx.style.pane.checkbox {
        Some(fill) => {
            let white = egui::Color32::WHITE;
            let black = egui::Color32::BLACK;
            let ink = if theme::contrast(fill, white) >= theme::contrast(fill, black) {
                white
            } else {
                black
            };
            egui::Button::new(egui::RichText::new(l.t("Apply filter", "Appliquer le filtre")).color(ink))
                .fill(fill)
        }
        None => egui::Button::new(l.t("Apply filter", "Appliquer le filtre")),
    };
    let b = ui.add_enabled(ready.is_ok(), button);
    b.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ready.is_ok(), &label));
    let b = match &ready {
        Err(why) if !why.is_empty() => b.on_disabled_hover_text(why),
        _ => b,
    };
    if b.clicked()
        && let Ok(cond) = ready
    {
        let mut p = cx.target.params(&c.field);
        if let (Some(o), Some(x)) = (p.as_object_mut(), cond.as_object()) {
            o.extend(x.clone());
        }
        acts.push(Act::Run(cx.target.scope.filter_cmd(), p));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(ty: &str, condition: Value) -> Card {
        Card {
            field: "Sales[Category]".into(),
            kind: "column".into(),
            table: Some("Sales".into()),
            name: Some("Category".into()),
            ty: ty.into(),
            condition,
            auto: false,
            hidden: false,
            locked: false,
            date: false,
            own: true,
        }
    }

    #[test]
    fn summaries_use_desktops_words() {
        let l = Lang::En;
        assert_eq!(summary(&card("none", Value::Null), l), "is (All)");
        assert_eq!(
            summary(&card("basic", json!({"in": ["Wheels", "Toys"]})), l),
            "is Wheels, Toys"
        );
        assert_eq!(
            summary(&card("basic", json!({"not_in": [null]})), l),
            "is not (Blank)"
        );
        assert_eq!(
            summary(
                &card(
                    "advanced",
                    json!({"conditions": [{"op": ">", "value": 10}, {"op": "<", "value": 20}], "logic": "and"})
                ),
                l
            ),
            "is greater than 10 and is less than 20"
        );
        assert_eq!(
            summary(&card("advanced", json!({"op": "=", "value": null})), l),
            "is blank"
        );
        assert_eq!(
            summary(
                &card("topn", json!({"top": 3, "by": "[Total]", "direction": "top"})),
                l
            ),
            "Top 3 by Total"
        );
        assert_eq!(
            summary(&card("basic", json!({"in": ["2024-01-31T00:00:00"]})), l),
            "is 2024-01-31"
        );
    }

    #[test]
    fn a_click_on_a_value_is_the_right_condition() {
        let all = vec![json!("Games"), json!("Toys"), json!("Wheels")];
        let none = card("none", Value::Null);
        // everything shown: unchecking one excludes it
        assert_eq!(
            toggle_value(&none, &all, &json!("Toys")),
            Some(Ok(json!({"not_in": ["Toys"]})))
        );
        // a list: add, remove; empty or complete is no filter
        let some = card("basic", json!({"in": ["Toys"]}));
        assert_eq!(
            toggle_value(&some, &all, &json!("Games")),
            Some(Ok(json!({"in": ["Toys", "Games"]})))
        );
        assert_eq!(toggle_value(&some, &all, &json!("Toys")), Some(Err(())));
        let two = card("basic", json!({"in": ["Toys", "Games"]}));
        assert_eq!(toggle_value(&two, &all, &json!("Wheels")), Some(Err(())));
        // excluded values: checking one back
        let not = card("basic", json!({"not_in": ["Toys", "Games"]}));
        assert_eq!(
            toggle_value(&not, &all, &json!("Toys")),
            Some(Ok(json!({"not_in": ["Games"]})))
        );
        let one = card("basic", json!({"not_in": ["Toys"]}));
        assert_eq!(toggle_value(&one, &all, &json!("Toys")), Some(Err(())));
        assert!(checked(&one, &json!("Games")) && !checked(&one, &json!("Toys")));
        // numbers and dates as the engine spells them
        assert!(same_value(&json!(100), &json!(100.0)));
        assert!(same_value(&json!("2024-01-31T00:00:00"), &json!("2024-01-31")));
    }

    #[test]
    fn drafts_become_parameters() {
        let l = Lang::En;
        let c = card(
            "advanced",
            json!({"conditions": [{"op": "contains", "value": "o"}, {"op": "=", "value": null}], "logic": "or"}),
        );
        let d = draft_of(&c);
        assert_eq!(d.mode, Mode::Advanced);
        assert_eq!(
            (d.op1.as_str(), d.value1.as_str(), d.op2.as_str(), d.or),
            ("contains", "o", "blank", true)
        );
        assert_eq!(
            draft_params(&d, false, l).unwrap(),
            json!({"conditions": [{"op": "contains", "value": "o"}, {"op": "=", "value": null}], "logic": "or"})
        );
        let mut n = Draft {
            mode: Mode::Advanced,
            op1: ">".into(),
            value1: "12,5".into(),
            ..Draft::default()
        };
        assert_eq!(
            draft_params(&n, true, l).unwrap(),
            json!({"op": ">", "value": 12.5})
        );
        n.value1.clear();
        assert!(draft_params(&n, true, l).is_err());
        let top = Draft {
            mode: Mode::TopN,
            count: "3".into(),
            by: "[Total]".into(),
            bottom: true,
            ..Draft::default()
        };
        assert_eq!(
            draft_params(&top, true, l).unwrap(),
            json!({"top": 3, "by": "[Total]", "direction": "bottom"})
        );
        assert!(
            draft_params(
                &Draft {
                    by: String::new(),
                    ..top
                },
                true,
                l
            )
            .is_err()
        );
        let back = draft_of(&card(
            "topn",
            json!({"top": 5, "by": "[Total]", "direction": "top"}),
        ));
        assert_eq!(
            (back.mode, back.count.as_str(), back.by.as_str(), back.bottom),
            (Mode::TopN, "5", "[Total]", false)
        );
    }

    #[test]
    fn a_visual_lists_its_bound_fields_as_cards() {
        let v = json!({"fields": [
            {"role": "Category", "kind": "column", "table": "Sales", "name": "Category", "text": "Sales[Category]"},
            {"role": "Y", "kind": "measure", "table": "Sales", "name": "Total", "text": "[Total]"},
            {"role": "Rows", "kind": "hierarchy_level", "text": "x"}],
            "filters": [{"field": "Sales[Category]", "kind": "column", "table": "Sales", "column": "Category",
                         "type": "basic", "condition": {"in": ["Toys"]}, "how_created": "User"}]});
        let cards = visual_cards(&v);
        assert_eq!(cards.len(), 2);
        assert_eq!(cards[0].ty, "basic");
        assert!(!cards[0].auto);
        assert_eq!((cards[1].field.as_str(), cards[1].auto), ("[Total]", true));
        // hostile input: nothing breaks
        for bad in [json!(null), json!({"fields": 3, "filters": [1, {"field": 2}]})] {
            let _ = visual_cards(&bad);
        }
    }
}
