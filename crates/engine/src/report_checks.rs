//! `report.validate`, the parts that read the raw PBIR JSON beyond plain field references:
//!
//! - filter containers (`filterConfig.filters[]`, and the filter states of bookmarks): the
//!   filter `type` against the shape of its conditions and the type of its field, and literals
//!   against the column they compare with;
//! - hierarchy references (`Hierarchy`, `HierarchyLevel`, `PropertyVariationSource`) in visuals,
//!   filters and bookmarks, resolved against the model's hierarchies and date variations;
//! - bookmark targets (`explorationState`: pages, visuals, groups, target visuals).
//!
//! What is an error and what is a warning follows how Power BI Desktop 2.158 treats each case
//! (`docs/formats/pbir.md`, "Report validation against Desktop"): an error is what makes Desktop
//! show an error in a visual (a filter whose query fails, a field that does not resolve); a
//! warning is what Desktop runs anyway (a filter `type` that does not match its conditions) and
//! everything in bookmarks (Desktop opens the report and keeps the bookmark).

use std::collections::HashSet;

use bicraft_model::{DataType, SemanticModel, Table};
use bicraft_report::Report;
use serde_json::{Map, Value, json};

use crate::edit::dax_column;
use crate::refs::{Loc, raw_documents_ref};

/// Deepest JSON nesting walked (serde_json already stops at 128).
const MAX_DEPTH: usize = 256;

/// One finding: (error, location, message).
pub(crate) type Issue = (bool, Value, String);

/// The filter `type` values of the public `filterConfiguration` schema.
const FILTER_TYPES: &[&str] = &[
    "Categorical",
    "Range",
    "Advanced",
    "Passthrough",
    "TopN",
    "Include",
    "Exclude",
    "RelativeDate",
    "Tuple",
    "RelativeTime",
    "VisualTopN",
];

pub(crate) fn validation_issues(model: Option<&SemanticModel>, r: &Report) -> Vec<Issue> {
    let mut out = Vec::new();
    // Filter containers of the report, its pages and visuals.
    if let Some(fc) = &r.filter_config {
        filter_list(
            model,
            fc,
            Level::Report,
            false,
            &json!({"part": "filters"}),
            &mut out,
        );
    }
    for page in &r.pages {
        if let Some(fc) = &page.filter_config {
            let at = json!({"page": page.display_name, "part": "filters"});
            filter_list(model, fc, Level::Page, false, &at, &mut out);
        }
        for v in &page.visuals {
            if let Some(fc) = &v.filter_config {
                let at = json!({"page": page.display_name, "visual": v.name, "part": "filters"});
                filter_list(model, fc, Level::Visual, false, &at, &mut out);
            }
        }
    }
    for b in &r.bookmarks {
        bookmark_issues(model, r, &b.name, &b.document, &mut out);
    }
    if let Some(m) = model {
        hierarchy_issues(m, r, &mut out);
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Filters
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Level {
    Report,
    Page,
    Visual,
}

/// What a filter's field is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldKind {
    /// A column, a column of a date variation, or a hierarchy level (its column's type).
    Column(DataType),
    Measure,
    Aggregation,
    /// Not resolved (missing object, reported elsewhere) or not a model field.
    Unknown,
}

/// `{"filters": [...]}` of a `filterConfig`.
fn filter_list(
    model: Option<&SemanticModel>,
    fc: &Value,
    level: Level,
    bookmark: bool,
    at: &Value,
    out: &mut Vec<Issue>,
) {
    let Some(list) = fc.get("filters").and_then(Value::as_array) else {
        return;
    };
    for c in list {
        check_container(model, c, level, bookmark, at, out);
    }
}

/// The filter containers of a bookmark `FiltersState` (`byName`, `byExpr`, `byType`,
/// `byTransientState`).
fn filter_state(model: Option<&SemanticModel>, fs: &Value, level: Level, at: &Value, out: &mut Vec<Issue>) {
    let Some(o) = fs.as_object() else {
        return;
    };
    for (k, v) in o {
        match (k.as_str(), v) {
            ("byName", Value::Object(m)) => {
                for c in m.values() {
                    check_container(model, c, level, true, at, out);
                }
            }
            ("byExpr" | "byType" | "byTransientState", Value::Array(list)) => {
                for c in list {
                    check_container(model, c, level, true, at, out);
                }
            }
            _ => {}
        }
    }
}

fn check_container(
    model: Option<&SemanticModel>,
    c: &Value,
    level: Level,
    bookmark: bool,
    at: &Value,
    out: &mut Vec<Issue>,
) {
    let Some(o) = c.as_object() else {
        return;
    };
    let Some(kind) = o.get("type").and_then(Value::as_str) else {
        return;
    };
    // Unknown types are schema issues.
    if !FILTER_TYPES.contains(&kind) || kind == "Passthrough" {
        return;
    }
    let mut at = at.clone();
    if let (Some(a), Some(name)) = (at.as_object_mut(), o.get("name").and_then(Value::as_str)) {
        a.insert("filter".into(), json!(name));
    }
    let mut push = |error: bool, message: String| {
        // Desktop keeps a bookmark whatever its filter state holds.
        out.push((error && !bookmark, at.clone(), message));
    };
    let field = o.get("field").or_else(|| o.get("expression"));
    let fk = match (model, field) {
        (Some(m), Some(f)) => field_kind(m, f, &[]),
        _ => FieldKind::Unknown,
    };
    let label = field.map(|f| field_label(f, &[])).unwrap_or_default();
    let filter = o.get("filter").and_then(Value::as_object);
    let scope: Vec<(String, String)> = filter
        .and_then(|f| f.get("From"))
        .map(from_scope)
        .unwrap_or_default();
    let conditions: Vec<&Value> = filter
        .and_then(|f| f.get("Where"))
        .and_then(Value::as_array)
        .map(|w| w.iter().filter_map(|x| x.get("Condition")).collect())
        .unwrap_or_default();

    // The type against the field.
    let relative = matches!(kind, "RelativeDate" | "RelativeTime");
    match fk {
        FieldKind::Column(t)
            if relative && !matches!(t, DataType::DateTime | DataType::Unknown | DataType::Variant) =>
        {
            push(
                true,
                format!(
                    "this {kind} filter needs a date column, but {label} is {}",
                    type_name(t)
                ),
            );
        }
        // Desktop 2.158 runs these (no error in the visual) but never writes them.
        FieldKind::Measure | FieldKind::Aggregation
            if matches!(
                kind,
                "Categorical" | "Include" | "Exclude" | "Tuple" | "RelativeDate" | "RelativeTime" | "TopN"
            ) =>
        {
            push(
                relative,
                format!("this {kind} filter cannot filter a measure ({label}): use an Advanced filter"),
            );
        }
        FieldKind::Measure if level != Level::Visual => {
            push(
                false,
                format!(
                    "{label} is a measure filtering {}: Desktop applies it, but its filter pane offers measures only as visual filters",
                    if level == Level::Page {
                        "a page"
                    } else {
                        "the report"
                    }
                ),
            );
        }
        _ => {}
    }

    if conditions.is_empty() {
        return;
    }
    // The type against the shape of its conditions.
    let mut shape = Shape::default();
    for c in &conditions {
        shape.scan(c, 0);
    }
    let expected = match kind {
        "TopN" if !shape.table_in => Some("an In condition over a Top N subquery"),
        "VisualTopN" if !shape.table_in && !shape.visual_top_n => {
            Some("a VisualTopN item count or a Top N subquery")
        }
        "RelativeDate" | "RelativeTime" if !shape.now => {
            Some("a date range relative to now (Now, DateAdd, DateSpan)")
        }
        "Categorical" | "Include" | "Exclude" | "Tuple"
            if shape.text || shape.ranged || shape.now || shape.table_in =>
        {
            Some("In conditions on the field's values")
        }
        "Advanced" if shape.now || shape.table_in => Some("comparisons with literal values"),
        _ => None,
    };
    // Desktop 2.158 applies the conditions as written: the visual shows no error, but the
    // filter card does not match the conditions.
    if let Some(expected) = expected {
        push(
            false,
            format!(
                "{kind} filters hold {expected}, but this one uses {}",
                shape.describe()
            ),
        );
    }

    // Text operators and literals against column types.
    let Some(m) = model else {
        return;
    };
    let mut seen = HashSet::new();
    for c in &conditions {
        literal_checks(m, c, &scope, 0, &mut |error, message| {
            if seen.insert(message.clone()) {
                push(error, message);
            }
        });
    }
}

/// What a filter's conditions are made of.
#[derive(Debug, Default)]
struct Shape {
    /// `In` with `Table` (a Top N subquery).
    table_in: bool,
    /// A `VisualTopN` expression.
    visual_top_n: bool,
    /// `In` with `Values`.
    values_in: bool,
    /// `Now`, `DateAdd` or `DateSpan`.
    now: bool,
    /// `Contains`, `StartsWith` or `EndsWith`.
    text: bool,
    /// `Comparison` other than equality, or `Between`.
    ranged: bool,
}

impl Shape {
    fn scan(&mut self, v: &Value, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        match v {
            Value::Array(items) => items.iter().for_each(|x| self.scan(x, depth.saturating_add(1))),
            Value::Object(o) => {
                for (k, x) in o {
                    match k.as_str() {
                        "In" => {
                            if x.get("Table").is_some() {
                                self.table_in = true;
                            } else {
                                self.values_in = true;
                            }
                        }
                        "Now" | "DateAdd" | "DateSpan" => self.now = true,
                        "VisualTopN" => self.visual_top_n = true,
                        "Contains" | "StartsWith" | "EndsWith" => self.text = true,
                        "Between" => self.ranged = true,
                        "Comparison"
                            if x.get("ComparisonKind")
                                .and_then(Value::as_u64)
                                .is_some_and(|k| k != 0) =>
                        {
                            self.ranged = true;
                        }
                        _ => {}
                    }
                    // Subqueries (`Table`'s source) are not conditions of this filter.
                    if k != "Subquery" {
                        self.scan(x, depth.saturating_add(1));
                    }
                }
            }
            _ => {}
        }
    }

    fn describe(&self) -> String {
        let mut parts = Vec::new();
        if self.table_in {
            parts.push("a Top N subquery");
        }
        if self.now {
            parts.push("relative dates");
        }
        if self.text {
            parts.push("text operators");
        }
        if self.ranged {
            parts.push("range comparisons");
        }
        if self.values_in && parts.is_empty() {
            parts.push("a list of values");
        }
        if parts.is_empty() {
            parts.push("equality comparisons");
        }
        parts.join(", ")
    }
}

/// The literal kinds of the semantic query language (`'text'`, `1L`, `1D`, `1M`,
/// `datetime'…'`, `true`, `null`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LitKind {
    Text,
    Number,
    Date,
    Boolean,
    Null,
}

fn literal_kind(v: &Value) -> Option<LitKind> {
    let s = v.get("Literal")?.get("Value")?.as_str()?.trim();
    if s.eq_ignore_ascii_case("null") {
        return Some(LitKind::Null);
    }
    if s == "true" || s == "false" {
        return Some(LitKind::Boolean);
    }
    let lower = s.to_ascii_lowercase();
    if lower.starts_with("datetime'") || lower.starts_with("datetimeoffset'") {
        return Some(LitKind::Date);
    }
    if s.starts_with('\'') {
        return Some(LitKind::Text);
    }
    let body = s.strip_suffix(['L', 'D', 'M'])?;
    if !body.is_empty() && body.parse::<f64>().is_ok() {
        return Some(LitKind::Number);
    }
    None
}

fn literal_fits(t: DataType, l: LitKind) -> bool {
    matches!(
        (t, l),
        (_, LitKind::Null)
            | (DataType::String, LitKind::Text)
            | (
                DataType::Int64 | DataType::Double | DataType::Decimal,
                LitKind::Number
            )
            | (DataType::DateTime, LitKind::Date)
            | (DataType::Boolean, LitKind::Boolean)
            | (DataType::Unknown | DataType::Variant | DataType::Binary, _)
    )
}

fn lit_name(l: LitKind) -> &'static str {
    match l {
        LitKind::Text => "a text",
        LitKind::Number => "a number",
        LitKind::Date => "a date",
        LitKind::Boolean => "a true/false",
        LitKind::Null => "a blank",
    }
}

/// Text operators on columns that are not text, and literals of the wrong type for the column
/// they are compared with. The callback gets `(error, message)`.
fn literal_checks(
    m: &SemanticModel,
    v: &Value,
    scope: &[(String, String)],
    depth: usize,
    f: &mut dyn FnMut(bool, String),
) {
    if depth > MAX_DEPTH {
        return;
    }
    let column_type = |e: &Value| match field_kind(m, e, scope) {
        FieldKind::Column(t) => Some(t),
        _ => None,
    };
    let check = |left: &Value, right: &Value, f: &mut dyn FnMut(bool, String)| {
        if let (Some(t), Some(l)) = (column_type(left), literal_kind(right))
            && !literal_fits(t, l)
        {
            f(
                true,
                format!(
                    "{} value is compared with {} ({}); the visual shows an error",
                    lit_name(l),
                    field_label(left, scope),
                    type_name(t)
                ),
            );
        }
    };
    match v {
        Value::Array(items) => {
            for x in items {
                literal_checks(m, x, scope, depth.saturating_add(1), f);
            }
        }
        Value::Object(o) => {
            for (k, x) in o {
                match k.as_str() {
                    "Comparison" => {
                        if let (Some(l), Some(r)) = (x.get("Left"), x.get("Right")) {
                            check(l, r, f);
                        }
                    }
                    "Between" => {
                        if let Some(e) = x.get("Expression") {
                            for b in ["LowerBound", "UpperBound"] {
                                if let Some(r) = x.get(b) {
                                    check(e, r, f);
                                }
                            }
                        }
                    }
                    "In" => {
                        if let (Some(Value::Array(exprs)), Some(Value::Array(rows))) =
                            (x.get("Expressions"), x.get("Values"))
                        {
                            for row in rows.iter().filter_map(Value::as_array) {
                                for (e, r) in exprs.iter().zip(row) {
                                    check(e, r, f);
                                }
                            }
                        }
                    }
                    "Contains" | "StartsWith" | "EndsWith" => {
                        if let Some(l) = x.get("Left")
                            && let Some(t) = column_type(l)
                            && !matches!(t, DataType::String | DataType::Unknown | DataType::Variant)
                        {
                            f(
                                true,
                                format!(
                                    "{k} applies to text, but {} is {}",
                                    field_label(l, scope),
                                    type_name(t)
                                ),
                            );
                        }
                        if let (Some(l), Some(r)) = (x.get("Left"), x.get("Right")) {
                            check(l, r, f);
                        }
                    }
                    "Subquery" => continue,
                    _ => {}
                }
                literal_checks(m, x, scope, depth.saturating_add(1), f);
            }
        }
        _ => {}
    }
}

fn type_name(t: DataType) -> &'static str {
    match t {
        DataType::String => "text",
        DataType::Int64 => "a whole number",
        DataType::Double => "a decimal number",
        DataType::Decimal => "a fixed decimal number",
        DataType::DateTime => "a date",
        DataType::Boolean => "true/false",
        DataType::Binary => "binary",
        DataType::Variant | DataType::Unknown => "of unknown type",
    }
}

/// `From` items that name a table: `(alias, entity)`.
fn from_scope(from: &Value) -> Vec<(String, String)> {
    from.as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|i| {
                    Some((
                        i.get("Name")?.as_str()?.to_string(),
                        i.get("Entity")?.as_str()?.to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The model table of a `SourceRef` (`Entity`, or a `Source` alias), `None` for report-level
/// objects (`Schema: extension`) and unknown aliases.
fn source_entity<'a>(sr: &'a Value, scope: &'a [(String, String)]) -> Option<&'a str> {
    if sr.get("Schema").and_then(Value::as_str) == Some("extension") {
        return None;
    }
    if let Some(e) = sr.get("Entity").and_then(Value::as_str) {
        return Some(e);
    }
    let alias = sr.get("Source").and_then(Value::as_str)?;
    scope
        .iter()
        .rev()
        .find(|(n, _)| n == alias)
        .map(|(_, e)| e.as_str())
}

fn field_kind(m: &SemanticModel, e: &Value, scope: &[(String, String)]) -> FieldKind {
    let Some(o) = e.as_object() else {
        return FieldKind::Unknown;
    };
    if let Some(c) = o.get("Column") {
        let Some(prop) = c.get("Property").and_then(Value::as_str) else {
            return FieldKind::Unknown;
        };
        let Some(expr) = c.get("Expression") else {
            return FieldKind::Unknown;
        };
        if let Some(sr) = expr.get("SourceRef") {
            return source_entity(sr, scope)
                .and_then(|t| m.table(t))
                .and_then(|t| t.column(prop))
                .map_or(FieldKind::Unknown, |c| FieldKind::Column(c.data_type));
        }
        if let Some(pvs) = expr.get("PropertyVariationSource") {
            return variation_table(m, pvs, scope)
                .ok()
                .flatten()
                .and_then(|t| t.column(prop))
                .map_or(FieldKind::Unknown, |c| FieldKind::Column(c.data_type));
        }
        return FieldKind::Unknown;
    }
    if o.contains_key("Measure") {
        return FieldKind::Measure;
    }
    if o.contains_key("Aggregation") {
        return FieldKind::Aggregation;
    }
    if let Some(hl) = o.get("HierarchyLevel") {
        let Some(level) = hl.get("Level").and_then(Value::as_str) else {
            return FieldKind::Unknown;
        };
        let Some((t, h)) = hl
            .get("Expression")
            .and_then(|x| x.get("Hierarchy"))
            .and_then(|h| resolve_hierarchy(m, h, scope).ok().flatten())
        else {
            return FieldKind::Unknown;
        };
        return level_column(h, level)
            .and_then(|c| t.column(c))
            .map_or(FieldKind::Unknown, |c| FieldKind::Column(c.data_type));
    }
    FieldKind::Unknown
}

/// `Table[Column]`, `[Measure]`, `Table.Hierarchy.Level` for messages.
fn field_label(e: &Value, scope: &[(String, String)]) -> String {
    let named = |x: &Value| {
        let prop = x.get("Property").and_then(Value::as_str).unwrap_or("?");
        let sr = x.get("Expression").and_then(|e| e.get("SourceRef"));
        let entity = sr
            .and_then(|s| {
                source_entity(s, scope).or_else(|| {
                    s.get("Entity")
                        .or_else(|| s.get("Source"))
                        .and_then(Value::as_str)
                })
            })
            .unwrap_or("?");
        (entity.to_string(), prop.to_string())
    };
    if let Some(c) = e.get("Column") {
        let (t, p) = named(c);
        return dax_column(&t, &p);
    }
    if let Some(x) = e.get("Measure") {
        let (_, p) = named(x);
        return crate::edit::dax_measure(&p);
    }
    if let Some(a) = e.get("Aggregation") {
        if let Some(c) = a.get("Expression") {
            return format!("an aggregation of {}", field_label(c, scope));
        }
        return "an aggregation".into();
    }
    if let Some(hl) = e.get("HierarchyLevel") {
        let level = hl.get("Level").and_then(Value::as_str).unwrap_or("?");
        let h = hl.get("Expression").and_then(|x| x.get("Hierarchy"));
        let name = h
            .and_then(|h| h.get("Hierarchy"))
            .and_then(Value::as_str)
            .unwrap_or("?");
        return format!("level `{level}` of hierarchy `{name}`");
    }
    "the field".into()
}

// ---------------------------------------------------------------------------------------------
// Hierarchies and date variations
// ---------------------------------------------------------------------------------------------

/// The hierarchy `name` of a table (TMSL `hierarchies`, kept in `extra`).
fn hierarchy<'a>(t: &'a Table, name: &str) -> Option<&'a Value> {
    t.extra.get("hierarchies")?.as_array()?.iter().find(|h| {
        h.get("name")
            .and_then(Value::as_str)
            .is_some_and(|n| n.eq_ignore_ascii_case(name))
    })
}

/// The column of level `level` of hierarchy `h`.
fn level_column<'a>(h: &'a Value, level: &str) -> Option<&'a str> {
    h.get("levels")?
        .as_array()?
        .iter()
        .find(|l| {
            l.get("name")
                .and_then(Value::as_str)
                .is_some_and(|n| n.eq_ignore_ascii_case(level))
        })
        .and_then(|l| l.get("column").and_then(Value::as_str))
}

fn has_level(h: &Value, level: &str) -> bool {
    h.get("levels").and_then(Value::as_array).is_some_and(|ls| {
        ls.iter().any(|l| {
            l.get("name")
                .and_then(Value::as_str)
                .is_some_and(|n| n.eq_ignore_ascii_case(level))
        })
    })
}

/// The table a `PropertyVariationSource` points to. `Ok(None)` when its table or column is
/// missing (reported as a field) or it is not a model object; `Err` when the column has no such
/// variation.
fn variation_table<'a>(
    m: &'a SemanticModel,
    pvs: &Value,
    scope: &[(String, String)],
) -> Result<Option<&'a Table>, String> {
    let (Some(sr), Some(name), Some(prop)) = (
        pvs.get("Expression").and_then(|e| e.get("SourceRef")),
        pvs.get("Name").and_then(Value::as_str),
        pvs.get("Property").and_then(Value::as_str),
    ) else {
        return Ok(None);
    };
    let Some(t) = source_entity(sr, scope).and_then(|e| m.table(e)) else {
        return Ok(None);
    };
    let Some(c) = t.column(prop) else {
        return Ok(None);
    };
    let variations = c.variations();
    let Some(v) = variations.iter().find(|v| v.name.eq_ignore_ascii_case(name)) else {
        return Err(if variations.is_empty() {
            format!(
                "{} has no date variation `{name}` (it has none: auto date/time is off for it)",
                dax_column(&t.name, &c.name)
            )
        } else {
            format!("{} has no date variation `{name}`", dax_column(&t.name, &c.name))
        });
    };
    match m.variation_table(&t.name, &c.name, v) {
        Some(target) => Ok(Some(target)),
        None => Err(format!(
            "date variation `{name}` of {} points to no table of the model",
            dax_column(&t.name, &c.name)
        )),
    }
}

/// The table and hierarchy a `Hierarchy` expression names. `Ok(None)`: not resolvable for a
/// reason reported elsewhere.
fn resolve_hierarchy<'a>(
    m: &'a SemanticModel,
    h: &Value,
    scope: &[(String, String)],
) -> Result<Option<(&'a Table, &'a Value)>, String> {
    let Some(name) = h.get("Hierarchy").and_then(Value::as_str) else {
        return Ok(None);
    };
    let Some(expr) = h.get("Expression") else {
        return Ok(None);
    };
    let table = if let Some(sr) = expr.get("SourceRef") {
        source_entity(sr, scope).and_then(|e| m.table(e))
    } else if let Some(pvs) = expr.get("PropertyVariationSource") {
        // A missing variation is reported on the `PropertyVariationSource` itself.
        variation_table(m, pvs, scope).ok().flatten()
    } else {
        None
    };
    let Some(t) = table else {
        return Ok(None);
    };
    match hierarchy(t, name) {
        Some(found) => Ok(Some((t, found))),
        None => Err(format!("table `{}` has no hierarchy `{name}`", t.name)),
    }
}

/// Called with each object and the `From` aliases in scope.
type Visitor<'a> = dyn FnMut(&Map<String, Value>, &[(String, String)]) + 'a;

/// Calls `f` for every object of `v` with the `From` aliases in scope.
fn walk(v: &Value, scope: &mut Vec<(String, String)>, depth: usize, f: &mut Visitor<'_>) {
    if depth > MAX_DEPTH {
        return;
    }
    match v {
        Value::Array(items) => {
            for x in items {
                walk(x, scope, depth.saturating_add(1), f);
            }
        }
        Value::Object(o) => {
            let before = scope.len();
            if let Some(from) = o.get("From") {
                scope.extend(from_scope(from));
            }
            f(o, scope);
            for x in o.values() {
                if x.is_array() || x.is_object() {
                    walk(x, scope, depth.saturating_add(1), f);
                }
            }
            scope.truncate(before);
        }
        _ => {}
    }
}

/// Unresolved hierarchies, levels and date variations, anywhere in the report. Errors, except in
/// bookmarks.
fn hierarchy_issues(m: &SemanticModel, r: &Report, out: &mut Vec<Issue>) {
    let mut seen: HashSet<(Loc, String)> = HashSet::new();
    for (loc, doc) in raw_documents_ref(r) {
        let bookmark = loc.part.starts_with("bookmark");
        // A visual's typed hierarchy-level binding and its raw query are the same field.
        let key = Loc {
            part: if bookmark { loc.part.clone() } else { String::new() },
            ..loc.clone()
        };
        let mut found = Vec::new();
        walk(doc, &mut Vec::new(), 0, &mut |o, scope| {
            if let Some(pvs) = o.get("PropertyVariationSource")
                && let Err(e) = variation_table(m, pvs, scope)
            {
                found.push(e);
            }
            // a column of a variation's date table (an auto date/time level bound without its
            // hierarchy)
            if let Some(c) = o.get("Column")
                && let Some(prop) = c.get("Property").and_then(Value::as_str)
                && let Some(pvs) = c.get("Expression").and_then(|e| e.get("PropertyVariationSource"))
                && let Ok(Some(t)) = variation_table(m, pvs, scope)
                && t.column(prop).is_none()
            {
                let name = pvs.get("Name").and_then(Value::as_str).unwrap_or_default();
                let base = pvs
                    .get("Expression")
                    .and_then(|e| e.get("SourceRef"))
                    .and_then(|sr| source_entity(sr, scope))
                    .zip(pvs.get("Property").and_then(Value::as_str))
                    .and_then(|(e, p)| m.table(e).and_then(|t| t.column(p).map(|c| (t, c))))
                    .map(|(bt, bc)| dax_column(&bt.name, &bc.name))
                    .unwrap_or_default();
                found.push(format!(
                    "date variation `{name}` of {base} has no column `{prop}` (its date table `{}`)",
                    t.name
                ));
            }
            if let Some(h) = o.get("Hierarchy").filter(|h| h.is_object())
                && let Err(e) = resolve_hierarchy(m, h, scope)
            {
                found.push(e);
            }
            if let Some(hl) = o.get("HierarchyLevel")
                && let Some(level) = hl.get("Level").and_then(Value::as_str)
                && let Some(h) = hl.get("Expression").and_then(|x| x.get("Hierarchy"))
                && let Ok(Some((t, found_h))) = resolve_hierarchy(m, h, scope)
                && !has_level(found_h, level)
            {
                let name = found_h.get("name").and_then(Value::as_str).unwrap_or_default();
                found.push(format!(
                    "hierarchy `{name}` of table `{}` has no level `{level}`",
                    t.name
                ));
            }
        });
        for message in found {
            if seen.insert((key.clone(), message.clone())) {
                out.push((!bookmark, loc.to_json(), message));
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Bookmarks
// ---------------------------------------------------------------------------------------------

/// A bookmark's targets: the active page, the pages, visuals and groups of its sections, its
/// target visuals; and its filter states. Desktop opens a report whatever a bookmark points to
/// and keeps the bookmark, so every finding is a warning.
fn bookmark_issues(model: Option<&SemanticModel>, r: &Report, name: &str, doc: &Value, out: &mut Vec<Issue>) {
    let part = format!("bookmark {name}");
    let at = json!({"part": part});
    let mut warn = |message: String| out.push((false, at.clone(), message));
    let Some(es) = doc.get("explorationState").and_then(Value::as_object) else {
        return;
    };
    let page_of = |key: &str| r.pages.iter().find(|p| p.name.eq_ignore_ascii_case(key));
    let active = es.get("activeSection").and_then(Value::as_str);
    if let Some(a) = active
        && page_of(a).is_none()
    {
        warn(format!("its active page `{a}` does not exist"));
    }
    if let Some(Value::Object(sections)) = es.get("sections") {
        for (key, section) in sections {
            let Some(page) = page_of(key) else {
                warn(format!(
                    "it holds the state of page `{key}`, which does not exist"
                ));
                continue;
            };
            if let Some(Value::Object(vcs)) = section.get("visualContainers") {
                for (vname, state) in vcs {
                    let Some(v) = page.visuals.iter().find(|v| v.name == *vname) else {
                        warn(format!(
                            "it holds the state of visual `{vname}`, which is not on page `{}`",
                            page.display_name
                        ));
                        continue;
                    };
                    let captured = state
                        .get("singleVisual")
                        .and_then(|s| s.get("visualType"))
                        .and_then(Value::as_str);
                    if let (Some(c), Some(actual)) = (captured, v.visual_type.as_deref())
                        && !c.eq_ignore_ascii_case(actual)
                        && state
                            .get("singleVisual")
                            .and_then(|s| s.get("targetType"))
                            .is_none()
                    {
                        warn(format!(
                            "it captured visual `{vname}` on page `{}` as a {c}, but it is a {actual}",
                            page.display_name
                        ));
                    }
                }
            }
            if let Some(Value::Object(groups)) = section.get("visualContainerGroups") {
                for gname in groups.keys() {
                    let is_group = page
                        .visuals
                        .iter()
                        .any(|v| v.name == *gname && v.visual_type.is_none());
                    if !is_group {
                        warn(format!(
                            "it holds the state of group `{gname}`, which is not a visual group on page `{}`",
                            page.display_name
                        ));
                    }
                }
            }
        }
    }
    if let Some(Value::Array(targets)) = doc.get("options").and_then(|o| o.get("targetVisualNames")) {
        let page = active.and_then(page_of);
        for t in targets.iter().filter_map(Value::as_str) {
            let found = match page {
                Some(p) => p.visuals.iter().any(|v| v.name == t),
                None => r.pages.iter().any(|p| p.visuals.iter().any(|v| v.name == t)),
            };
            if !found {
                warn(format!("its target visual `{t}` is not on its page"));
            }
        }
    }
    // Filter states.
    if let Some(fs) = es.get("filters") {
        filter_state(model, fs, Level::Report, &at, out);
    }
    if let Some(Value::Object(sections)) = es.get("sections") {
        for section in sections.values() {
            if let Some(fs) = section.get("filters") {
                filter_state(model, fs, Level::Page, &at, out);
            }
            if let Some(Value::Object(vcs)) = section.get("visualContainers") {
                for state in vcs.values() {
                    if let Some(fs) = state.get("filters") {
                        filter_state(model, fs, Level::Visual, &at, out);
                    }
                }
            }
        }
    }
}
