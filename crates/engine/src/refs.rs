//! Model references in a report.
//!
//! - The `field` strings report commands take (`Sales[Amount]`, `[Total]`, `Sum(Sales[Amount])`):
//!   parsing, resolving against the model, rendering.
//! - References inside the raw PBIR JSON the report keeps verbatim (filters, queries, formatting,
//!   bookmarks): a visitor that finds `Column` / `Measure` / `PropertyVariationSource` objects
//!   (`{Expression: {SourceRef: {Entity | Source}}, Property}`, with `Source` aliases resolved
//!   through the enclosing `From` lists) and every `Entity` name.
//! - What a model rename or removal does to the report ([`ModelChange`]): the bindings and raw
//!   references it affects, rewritten on request.

use bicraft_model::{DataType, SemanticModel};
use bicraft_report::{FieldRef, Report};
use serde_json::{Map, Value, json};

use crate::edit::{dax_column, dax_measure, parse_column_ref};

/// PBIR aggregation function names (`QueryAggregateFunction`), keyed by the lower-case names and
/// aliases accepted in `Function(Table[Column])`.
const AGGREGATIONS: &[(&str, &str)] = &[
    ("sum", "Sum"),
    ("avg", "Avg"),
    ("average", "Avg"),
    ("count", "Count"),
    ("distinctcount", "Count"),
    ("min", "Min"),
    ("max", "Max"),
    ("countnonnull", "CountNonNull"),
    ("median", "Median"),
    ("standarddeviation", "StandardDeviation"),
    ("stddev", "StandardDeviation"),
    ("stdev", "StandardDeviation"),
    ("variance", "Variance"),
    ("var", "Variance"),
];

/// Aggregations that only make sense on numbers.
const NUMERIC_ONLY: &[&str] = &["Sum", "Avg", "Median", "StandardDeviation", "Variance"];

/// Deepest JSON nesting the visitor descends into (serde_json already stops at 128).
const MAX_DEPTH: usize = 256;

/// Most affected bindings listed in a model edit's result; the count is always exact.
const MAX_LISTED: usize = 200;

// ---------------------------------------------------------------------------------------------
// Field strings
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ParsedField {
    /// `[Measure]`
    Measure(String),
    /// `Table[Name]`: a column, or a measure of that table.
    Qualified { table: String, name: String },
    /// `Function(Table[Column])`
    Aggregation {
        function: &'static str,
        table: String,
        column: String,
    },
}

fn aggregation_names() -> String {
    let mut names: Vec<&str> = AGGREGATIONS.iter().map(|(_, f)| *f).collect();
    names.dedup();
    names.join(", ")
}

pub(crate) fn parse_field(text: &str) -> Result<ParsedField, String> {
    let s = text.trim();
    let bad = || format!("`{text}` is not a field like Sales[Amount], [Total Sales] or Sum(Sales[Amount])");
    if let Some(inner) = s.strip_prefix('[') {
        let name = inner.strip_suffix(']').ok_or_else(bad)?;
        if name.is_empty() || name.replace("]]", "").contains(']') {
            return Err(bad());
        }
        return Ok(ParsedField::Measure(name.replace("]]", "]")));
    }
    if let Some(open) = s.find('(') {
        let function = s.get(..open).unwrap_or_default().trim();
        if !function.is_empty() && function.chars().all(|c| c.is_ascii_alphabetic()) {
            let inner = s
                .strip_suffix(')')
                .and_then(|x| x.get(open.saturating_add(1)..))
                .ok_or_else(bad)?;
            let canonical = AGGREGATIONS
                .iter()
                .find(|(alias, _)| alias.eq_ignore_ascii_case(function))
                .map(|(_, f)| *f)
                .ok_or_else(|| {
                    format!(
                        "unknown aggregation `{function}` (expected {})",
                        aggregation_names()
                    )
                })?;
            let (table, column) = parse_column_ref(inner)?;
            return Ok(ParsedField::Aggregation {
                function: canonical,
                table,
                column,
            });
        }
    }
    let (table, name) = parse_column_ref(s).map_err(|_| bad())?;
    Ok(ParsedField::Qualified { table, name })
}

/// A field string resolved to a binding, with the model's spelling of every name.
pub(crate) struct Resolved {
    pub field: FieldRef,
    pub warnings: Vec<String>,
}

/// Parses `text` and resolves it against `model`. Without a model, `Table[Name]` is taken as a
/// column (with a warning) and `[Measure]` is an error: its table cannot be known.
pub(crate) fn resolve_field(model: Option<&SemanticModel>, text: &str) -> Result<Resolved, String> {
    let parsed = parse_field(text)?;
    let Some(m) = model else {
        let unchecked = format!("no model is open: `{text}` was not checked against it");
        return match parsed {
            ParsedField::Measure(name) => Err(format!(
                "no model is open, so the table of measure {} cannot be found",
                dax_measure(&name)
            )),
            ParsedField::Qualified { table, name } => Ok(Resolved {
                field: FieldRef::Column { table, column: name },
                warnings: vec![format!("{unchecked}; it is bound as a column")],
            }),
            ParsedField::Aggregation {
                function,
                table,
                column,
            } => Ok(Resolved {
                field: FieldRef::Aggregation {
                    table,
                    column,
                    function: function.to_string(),
                },
                warnings: vec![unchecked],
            }),
        };
    };
    match parsed {
        ParsedField::Measure(name) => {
            if let Some((t, x)) = m.measure(&name) {
                return Ok(Resolved {
                    field: FieldRef::Measure {
                        table: t.name.clone(),
                        measure: x.name.clone(),
                    },
                    warnings: Vec::new(),
                });
            }
            match m.tables.iter().find_map(|t| t.column(&name).map(|c| (t, c))) {
                Some((t, c)) => Err(format!(
                    "{} is a column, not a measure: write {}",
                    dax_measure(&name),
                    dax_column(&t.name, &c.name)
                )),
                None => Err(format!("no measure named {}", dax_measure(&name))),
            }
        }
        ParsedField::Qualified { table, name } => {
            let t = m
                .table(&table)
                .ok_or_else(|| format!("no table named `{table}`"))?;
            if let Some(c) = t.column(&name) {
                return Ok(Resolved {
                    field: FieldRef::Column {
                        table: t.name.clone(),
                        column: c.name.clone(),
                    },
                    warnings: Vec::new(),
                });
            }
            if let Some(x) = t.measure(&name) {
                return Ok(Resolved {
                    field: FieldRef::Measure {
                        table: t.name.clone(),
                        measure: x.name.clone(),
                    },
                    warnings: Vec::new(),
                });
            }
            match m.measure(&name) {
                Some((other, x)) => Err(format!(
                    "measure {} is in table `{}`, not `{}`: write {}",
                    dax_measure(&x.name),
                    other.name,
                    t.name,
                    dax_measure(&x.name)
                )),
                None => Err(format!("table `{}` has no column or measure `{name}`", t.name)),
            }
        }
        ParsedField::Aggregation {
            function,
            table,
            column,
        } => {
            let t = m
                .table(&table)
                .ok_or_else(|| format!("no table named `{table}`"))?;
            let Some(c) = t.column(&column) else {
                return Err(if t.measure(&column).is_some() {
                    format!(
                        "{} is a measure; measures cannot be aggregated: bind {} instead",
                        dax_column(&t.name, &column),
                        dax_measure(&column)
                    )
                } else {
                    format!("table `{}` has no column `{column}`", t.name)
                });
            };
            let mut warnings = Vec::new();
            if NUMERIC_ONLY.contains(&function)
                && matches!(
                    c.data_type,
                    DataType::String | DataType::Boolean | DataType::DateTime | DataType::Binary
                )
            {
                warnings.push(format!(
                    "{function} of {} ({}) is not a numeric aggregation Power BI offers for this type",
                    dax_column(&t.name, &c.name),
                    c.data_type.as_tmsl().unwrap_or("unknown")
                ));
            }
            Ok(Resolved {
                field: FieldRef::Aggregation {
                    table: t.name.clone(),
                    column: c.name.clone(),
                    function: function.to_string(),
                },
                warnings,
            })
        }
    }
}

/// The field as commands write it: `Sales[Amount]`, `[Total]`, `Sum(Sales[Amount])`.
pub(crate) fn field_text(f: &FieldRef) -> String {
    match f {
        FieldRef::Column { table, column } => dax_column(table, column),
        FieldRef::Measure { measure, .. } => dax_measure(measure),
        FieldRef::Aggregation {
            table,
            column,
            function,
        } => format!("{function}({})", dax_column(table, column)),
        FieldRef::Other { .. } => match bicraft_report::query::visual_calc(f) {
            Some(c) => format!("visual calculation [{}]", c.name),
            None => match bicraft_report::query::sparkline(f) {
                Some(s) => format!(
                    "sparkline of {} by {}",
                    field_text(&s.measure),
                    s.groupings.iter().map(field_text).collect::<Vec<_>>().join(", ")
                ),
                None => "(other field)".to_string(),
            },
        },
    }
}

pub(crate) fn field_json(f: &FieldRef) -> Value {
    let (kind, table) = match f {
        FieldRef::Column { table, .. } => ("column", Some(table)),
        FieldRef::Measure { table, .. } => ("measure", Some(table)),
        FieldRef::Aggregation { table, .. } => ("aggregation", Some(table)),
        FieldRef::Other { .. } if bicraft_report::query::visual_calc(f).is_some() => {
            ("visual_calculation", None)
        }
        FieldRef::Other { .. } if bicraft_report::query::sparkline(f).is_some() => ("sparkline", None),
        FieldRef::Other { .. } => ("other", None),
    };
    json!({"field": field_text(f), "kind": kind, "table": table})
}

/// Same binding, names compared case-insensitively.
pub(crate) fn same_field(a: &FieldRef, b: &FieldRef) -> bool {
    let eq = |x: &str, y: &str| x.eq_ignore_ascii_case(y);
    match (a, b) {
        (
            FieldRef::Column { table, column },
            FieldRef::Column {
                table: t2,
                column: c2,
            },
        ) => eq(table, t2) && eq(column, c2),
        (
            FieldRef::Measure { table, measure },
            FieldRef::Measure {
                table: t2,
                measure: m2,
            },
        ) => eq(table, t2) && eq(measure, m2),
        (
            FieldRef::Aggregation {
                table,
                column,
                function,
            },
            FieldRef::Aggregation {
                table: t2,
                column: c2,
                function: f2,
            },
        ) => eq(table, t2) && eq(column, c2) && eq(function, f2),
        (FieldRef::Other { raw }, FieldRef::Other { raw: r2 }) => raw == r2,
        _ => false,
    }
}

/// Why a binding does not resolve in `m`, or `None` when it does (or cannot be checked).
pub(crate) fn check_field(m: &SemanticModel, f: &FieldRef) -> Option<String> {
    match f {
        FieldRef::Column { table, column } | FieldRef::Aggregation { table, column, .. } => {
            check_site(m, SiteKind::Column, table, column)
        }
        FieldRef::Measure { table, measure } => check_site(m, SiteKind::Measure, table, measure),
        FieldRef::Other { .. } => None,
    }
}

pub(crate) fn check_site(m: &SemanticModel, kind: SiteKind, table: &str, name: &str) -> Option<String> {
    let Some(t) = m.table(table) else {
        return Some(format!("no table named `{table}` in the model"));
    };
    match kind {
        SiteKind::Table => None,
        SiteKind::Column => {
            if t.column(name).is_some() {
                None
            } else if t.measure(name).is_some() {
                Some(format!(
                    "{} is a measure, but it is used as a column",
                    dax_column(&t.name, name)
                ))
            } else {
                Some(format!("table `{}` has no column `{name}`", t.name))
            }
        }
        SiteKind::Measure => {
            if t.measure(name).is_some() {
                None
            } else if let Some((other, x)) = m.measure(name) {
                Some(format!(
                    "measure {} is in table `{}`, not `{}`",
                    dax_measure(&x.name),
                    other.name,
                    t.name
                ))
            } else if t.column(name).is_some() {
                Some(format!(
                    "{} is a column, but it is used as a measure",
                    dax_column(&t.name, name)
                ))
            } else {
                Some(format!("no measure named {}", dax_measure(name)))
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Raw PBIR JSON
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SiteKind {
    /// An `Entity` name (a `SourceRef` or a `From` item).
    Table,
    Column,
    Measure,
}

/// One model reference found in raw JSON.
pub(crate) struct Site<'a> {
    pub kind: SiteKind,
    /// The table, with `Source` aliases resolved.
    pub entity: &'a str,
    /// Column or measure name (empty for [`SiteKind::Table`]).
    pub property: &'a str,
    /// The table is named in place (`SourceRef.Entity`), not through a `From` alias.
    pub direct: bool,
    /// `SourceRef.Schema` is `extension`: a report-level object (`reportExtensions.json`), not a
    /// model object.
    pub extension: bool,
}

pub(crate) enum Action {
    Keep,
    SetEntity(String),
    SetProperty(String),
    /// A field site's table (when named in place) and its property.
    SetField(String, String),
}

/// Calls `f` for every model reference in `v`, applying the [`Action`] it returns.
pub(crate) fn visit(v: &mut Value, f: &mut dyn FnMut(&Site<'_>) -> Action) {
    let mut scope = Vec::new();
    visit_at(v, &mut scope, 0, f);
}

fn visit_at(
    v: &mut Value,
    scope: &mut Vec<(String, String)>,
    depth: usize,
    f: &mut dyn FnMut(&Site<'_>) -> Action,
) {
    if depth > MAX_DEPTH {
        return;
    }
    match v {
        Value::Array(items) => {
            for item in items {
                visit_at(item, scope, depth.saturating_add(1), f);
            }
        }
        Value::Object(o) => {
            let before = scope.len();
            if let Some(Value::Array(from)) = o.get("From") {
                for item in from {
                    if let (Some(name), Some(entity)) = (
                        item.get("Name").and_then(Value::as_str),
                        item.get("Entity").and_then(Value::as_str),
                    ) {
                        scope.push((name.to_string(), entity.to_string()));
                    }
                }
            }
            for (key, kind) in [
                ("Column", SiteKind::Column),
                ("Measure", SiteKind::Measure),
                ("PropertyVariationSource", SiteKind::Column),
            ] {
                if let Some(Value::Object(inner)) = o.get_mut(key) {
                    field_site(inner, kind, scope, f);
                }
            }
            let action = match o.get("Entity") {
                Some(Value::String(entity)) => Some(f(&Site {
                    kind: SiteKind::Table,
                    entity,
                    property: "",
                    direct: true,
                    extension: o.get("Schema").and_then(Value::as_str) == Some("extension"),
                })),
                _ => None,
            };
            if let Some(Action::SetEntity(new)) = action {
                o.insert("Entity".into(), Value::String(new));
            }
            for child in o.values_mut() {
                visit_at(child, scope, depth.saturating_add(1), f);
            }
            scope.truncate(before);
        }
        _ => {}
    }
}

/// `{Expression: {SourceRef: {Entity | Source, Schema?}}, Property}`.
fn field_site(
    inner: &mut Map<String, Value>,
    kind: SiteKind,
    scope: &[(String, String)],
    f: &mut dyn FnMut(&Site<'_>) -> Action,
) {
    let Some((entity, property, direct, extension)) = field_parts(inner, scope) else {
        return;
    };
    let action = f(&Site {
        kind,
        entity: &entity,
        property: &property,
        direct,
        extension,
    });
    match action {
        Action::Keep => {}
        Action::SetProperty(new) => {
            inner.insert("Property".into(), Value::String(new));
        }
        Action::SetField(table, property) => {
            inner.insert("Property".into(), Value::String(property));
            if direct
                && let Some(Value::Object(sr)) =
                    inner.get_mut("Expression").and_then(|e| e.get_mut("SourceRef"))
            {
                sr.insert("Entity".into(), Value::String(table));
            }
        }
        Action::SetEntity(new) => {
            if direct
                && let Some(Value::Object(sr)) =
                    inner.get_mut("Expression").and_then(|e| e.get_mut("SourceRef"))
            {
                sr.insert("Entity".into(), Value::String(new));
            }
        }
    }
}

/// The (entity, property, direct, extension) of a field site.
pub(crate) fn field_parts(
    inner: &Map<String, Value>,
    scope: &[(String, String)],
) -> Option<(String, String, bool, bool)> {
    let property = inner.get("Property").and_then(Value::as_str)?.to_string();
    let source_ref = inner.get("Expression").and_then(|e| e.get("SourceRef"))?;
    let extension = source_ref.get("Schema").and_then(Value::as_str) == Some("extension");
    let (entity, direct) = if let Some(e) = source_ref.get("Entity").and_then(Value::as_str) {
        (e.to_string(), true)
    } else {
        let alias = source_ref.get("Source").and_then(Value::as_str)?;
        let (_, entity) = scope.iter().rev().find(|(name, _)| name == alias)?;
        (entity.clone(), false)
    };
    Some((entity, property, direct, extension))
}

/// Read-only [`visit`]: calls `f` for every model reference in `v`, in the same order.
pub(crate) fn find_refs(v: &Value, f: &mut dyn FnMut(&Site<'_>)) {
    let mut scope = Vec::new();
    find_refs_at(v, &mut scope, 0, f);
}

fn find_refs_at(v: &Value, scope: &mut Vec<(String, String)>, depth: usize, f: &mut dyn FnMut(&Site<'_>)) {
    if depth > MAX_DEPTH {
        return;
    }
    match v {
        Value::Array(items) => {
            for item in items {
                find_refs_at(item, scope, depth.saturating_add(1), f);
            }
        }
        Value::Object(o) => {
            // One pass over the keys instead of a hash lookup per key: this walk runs over every
            // document of the report for each viewer snapshot.
            let (mut from, mut column, mut measure, mut variation, mut entity, mut schema) =
                (None, None, None, None, None, None);
            for (k, x) in o {
                match k.as_str() {
                    "From" => from = Some(x),
                    "Column" => column = Some(x),
                    "Measure" => measure = Some(x),
                    "PropertyVariationSource" => variation = Some(x),
                    "Entity" => entity = Some(x),
                    "Schema" => schema = Some(x),
                    _ => {}
                }
            }
            let before = scope.len();
            if let Some(Value::Array(from)) = from {
                for item in from {
                    if let (Some(name), Some(entity)) = (
                        item.get("Name").and_then(Value::as_str),
                        item.get("Entity").and_then(Value::as_str),
                    ) {
                        scope.push((name.to_string(), entity.to_string()));
                    }
                }
            }
            for (found, kind) in [
                (column, SiteKind::Column),
                (measure, SiteKind::Measure),
                (variation, SiteKind::Column),
            ] {
                if let Some(Value::Object(inner)) = found
                    && let Some((entity, property, direct, extension)) = field_parts(inner, scope)
                {
                    f(&Site {
                        kind,
                        entity: &entity,
                        property: &property,
                        direct,
                        extension,
                    });
                }
            }
            if let Some(Value::String(entity)) = entity {
                f(&Site {
                    kind: SiteKind::Table,
                    entity,
                    property: "",
                    direct: true,
                    extension: schema.and_then(Value::as_str) == Some("extension"),
                });
            }
            for child in o.values() {
                if child.is_array() || child.is_object() {
                    find_refs_at(child, scope, depth.saturating_add(1), f);
                }
            }
            scope.truncate(before);
        }
        _ => {}
    }
}

/// Where a raw JSON document sits in the report.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct Loc {
    /// Page display name.
    pub page: Option<String>,
    pub visual: Option<String>,
    /// `filters`, `properties`, `fields` or `bookmark <name>`.
    pub part: String,
}

impl Loc {
    fn new(page: Option<&str>, visual: Option<&str>, part: &str) -> Self {
        Self {
            page: page.map(str::to_string),
            visual: visual.map(str::to_string),
            part: part.to_string(),
        }
    }

    pub fn to_json(&self) -> Value {
        json!({"page": self.page, "visual": self.visual, "part": self.part})
    }
}

/// Every JSON value of the report that is kept verbatim (and so may hold model references),
/// with its location. Typed bindings (`Visual::fields` other than `FieldRef::Other`) are not
/// included: callers handle them directly.
pub(crate) fn raw_documents(report: &mut Report) -> Vec<(Loc, &mut Value)> {
    let mut out: Vec<(Loc, &mut Value)> = Vec::new();
    if let Some(v) = report.filter_config.as_mut() {
        out.push((Loc::new(None, None, "filters"), v));
    }
    for v in report.extra.values_mut().chain(report.pages_extra.values_mut()) {
        out.push((Loc::new(None, None, "properties"), v));
    }
    for b in &mut report.bookmarks {
        out.push((
            Loc::new(None, None, &format!("bookmark {}", b.name)),
            &mut b.document,
        ));
    }
    for page in &mut report.pages {
        let display = page.display_name.as_str();
        if let Some(v) = page.filter_config.as_mut() {
            out.push((Loc::new(Some(display), None, "filters"), v));
        }
        for v in page.extra.values_mut() {
            out.push((Loc::new(Some(display), None, "properties"), v));
        }
        for visual in &mut page.visuals {
            let name = visual.name.as_str();
            if let Some(v) = visual.filter_config.as_mut() {
                out.push((Loc::new(Some(display), Some(name), "filters"), v));
            }
            for v in visual.visual_extra.values_mut().chain(visual.extra.values_mut()) {
                out.push((Loc::new(Some(display), Some(name), "properties"), v));
            }
            for b in &mut visual.fields {
                if let FieldRef::Other { raw } = &mut b.field {
                    out.push((Loc::new(Some(display), Some(name), "fields"), raw));
                }
            }
        }
    }
    out
}

/// [`raw_documents`] on a shared borrow.
pub(crate) fn raw_documents_ref(report: &Report) -> Vec<(Loc, &Value)> {
    let mut out: Vec<(Loc, &Value)> = Vec::new();
    if let Some(v) = report.filter_config.as_ref() {
        out.push((Loc::new(None, None, "filters"), v));
    }
    for v in report.extra.values().chain(report.pages_extra.values()) {
        out.push((Loc::new(None, None, "properties"), v));
    }
    for b in &report.bookmarks {
        out.push((Loc::new(None, None, &format!("bookmark {}", b.name)), &b.document));
    }
    for page in &report.pages {
        let display = page.display_name.as_str();
        if let Some(v) = page.filter_config.as_ref() {
            out.push((Loc::new(Some(display), None, "filters"), v));
        }
        for v in page.extra.values() {
            out.push((Loc::new(Some(display), None, "properties"), v));
        }
        for visual in &page.visuals {
            let name = visual.name.as_str();
            if let Some(v) = visual.filter_config.as_ref() {
                out.push((Loc::new(Some(display), Some(name), "filters"), v));
            }
            for v in visual.visual_extra.values().chain(visual.extra.values()) {
                out.push((Loc::new(Some(display), Some(name), "properties"), v));
            }
            for b in &visual.fields {
                if let FieldRef::Other { raw } = &b.field {
                    out.push((Loc::new(Some(display), Some(name), "fields"), raw));
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Model changes
// ---------------------------------------------------------------------------------------------

/// A model edit that can break report bindings.
#[derive(Debug, Clone)]
pub(crate) enum ModelChange {
    TableRenamed {
        old: String,
        new: String,
    },
    TableRemoved {
        table: String,
    },
    ColumnRenamed {
        table: String,
        old: String,
        new: String,
    },
    ColumnRemoved {
        table: String,
        column: String,
    },
    MeasureRenamed {
        table: String,
        old: String,
        new: String,
    },
    MeasureRemoved {
        table: String,
        measure: String,
    },
    MeasureMoved {
        measure: String,
        from: String,
        to: String,
    },
    /// Not a model edit: the report's references to the column (or measure) `table[name]` are
    /// pointed at `to_table[to_name]` (`report.rebind`).
    Rebound {
        measure: bool,
        table: String,
        name: String,
        to_table: String,
        to_name: String,
    },
}

impl ModelChange {
    /// Renames and moves can be followed by rewriting the report; removals cannot.
    pub fn rewritable(&self) -> bool {
        matches!(
            self,
            Self::TableRenamed { .. }
                | Self::ColumnRenamed { .. }
                | Self::MeasureRenamed { .. }
                | Self::MeasureMoved { .. }
                | Self::Rebound { .. }
        )
    }

    fn matches_field(&self, f: &FieldRef) -> bool {
        let eq = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
        match (self, f) {
            (
                Self::TableRenamed { old: t, .. } | Self::TableRemoved { table: t },
                FieldRef::Column { table, .. }
                | FieldRef::Measure { table, .. }
                | FieldRef::Aggregation { table, .. },
            ) => eq(t, table),
            (
                Self::ColumnRenamed { table: t, old: c, .. } | Self::ColumnRemoved { table: t, column: c },
                FieldRef::Column { table, column } | FieldRef::Aggregation { table, column, .. },
            ) => eq(t, table) && eq(c, column),
            (
                Self::MeasureRenamed { table: t, old: x, .. }
                | Self::MeasureRemoved { table: t, measure: x }
                | Self::MeasureMoved {
                    from: t, measure: x, ..
                },
                FieldRef::Measure { table, measure },
            ) => eq(t, table) && eq(x, measure),
            (
                Self::Rebound {
                    measure: true,
                    table: t,
                    name: x,
                    ..
                },
                FieldRef::Measure { table, measure },
            ) => eq(t, table) && eq(x, measure),
            (
                Self::Rebound {
                    measure: false,
                    table: t,
                    name: c,
                    ..
                },
                FieldRef::Column { table, column } | FieldRef::Aggregation { table, column, .. },
            ) => eq(t, table) && eq(c, column),
            _ => false,
        }
    }

    fn rewrite_field(&self, f: &mut FieldRef) {
        match (self, f) {
            (
                Self::TableRenamed { new, .. } | Self::MeasureMoved { to: new, .. },
                FieldRef::Column { table, .. }
                | FieldRef::Measure { table, .. }
                | FieldRef::Aggregation { table, .. },
            ) => *table = new.clone(),
            (
                Self::ColumnRenamed { new, .. },
                FieldRef::Column { column, .. } | FieldRef::Aggregation { column, .. },
            ) => *column = new.clone(),
            (Self::MeasureRenamed { new, .. }, FieldRef::Measure { measure, .. }) => *measure = new.clone(),
            (
                Self::Rebound {
                    to_table, to_name, ..
                },
                FieldRef::Column { table, column } | FieldRef::Aggregation { table, column, .. },
            )
            | (
                Self::Rebound {
                    to_table, to_name, ..
                },
                FieldRef::Measure {
                    table,
                    measure: column,
                },
            ) => {
                *table = to_table.clone();
                *column = to_name.clone();
            }
            _ => {}
        }
    }

    fn matches_site(&self, s: &Site<'_>) -> bool {
        let eq = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
        match self {
            Self::TableRenamed { old: t, .. } | Self::TableRemoved { table: t } => {
                s.kind == SiteKind::Table && eq(t, s.entity)
            }
            Self::ColumnRenamed { table, old: c, .. } | Self::ColumnRemoved { table, column: c } => {
                s.kind == SiteKind::Column && !s.extension && eq(table, s.entity) && eq(c, s.property)
            }
            Self::MeasureRenamed { table, old: x, .. }
            | Self::MeasureRemoved { table, measure: x }
            | Self::MeasureMoved {
                from: table,
                measure: x,
                ..
            } => s.kind == SiteKind::Measure && !s.extension && eq(table, s.entity) && eq(x, s.property),
            Self::Rebound {
                measure, table, name, ..
            } => {
                let kind = if *measure {
                    SiteKind::Measure
                } else {
                    SiteKind::Column
                };
                s.kind == kind && !s.extension && eq(table, s.entity) && eq(name, s.property)
            }
        }
    }

    /// The rewrite for a matching site, `None` when it cannot be rewritten in place (a measure
    /// moved but referenced through a `From` alias that other references share).
    fn site_action(&self, s: &Site<'_>) -> Option<Action> {
        match self {
            Self::TableRenamed { new, .. } => Some(Action::SetEntity(new.clone())),
            Self::ColumnRenamed { new, .. } | Self::MeasureRenamed { new, .. } => {
                Some(Action::SetProperty(new.clone()))
            }
            Self::MeasureMoved { to, .. } if s.direct => Some(Action::SetEntity(to.clone())),
            Self::Rebound {
                to_table, to_name, ..
            } if s.direct => Some(Action::SetField(to_table.clone(), to_name.clone())),
            // through a `From` alias, only the property can change in place
            Self::Rebound {
                to_table, to_name, ..
            } if to_table.eq_ignore_ascii_case(s.entity) => Some(Action::SetProperty(to_name.clone())),
            _ => None,
        }
    }
}

/// Finds what `change` affects in `report`, and rewrites it when `update` is set (only for
/// [`ModelChange::rewritable`] changes). Returns the JSON for the edit result and whether the
/// report was modified.
pub(crate) fn apply_change(report: &mut Report, change: &ModelChange, update: bool) -> (Value, bool) {
    let update = update && change.rewritable();
    // queryRefs as they are before the change; stored ones that name the changed field are
    // renamed first (they are recognized from the fields as they are before the change).
    let refs_before = if update {
        let before = snapshot_query_refs(report);
        rename_stored_query_refs(report, change);
        Some(before)
    } else {
        None
    };
    let mut bindings = Vec::new();
    let mut binding_count = 0usize;
    let mut modified = false;
    for page in &mut report.pages {
        for visual in &mut page.visuals {
            for b in &mut visual.fields {
                if !change.matches_field(&b.field) {
                    continue;
                }
                binding_count = binding_count.saturating_add(1);
                if bindings.len() < MAX_LISTED {
                    bindings.push(json!({
                        "page": page.display_name,
                        "visual": visual.name,
                        "visual_type": visual.visual_type,
                        "role": b.role,
                        "field": field_text(&b.field),
                    }));
                }
                if update {
                    change.rewrite_field(&mut b.field);
                    modified = true;
                }
            }
        }
    }
    let mut others = Vec::new();
    let mut other_count = 0usize;
    let mut not_updated = 0usize;
    for (loc, doc) in raw_documents(report) {
        let mut count = 0usize;
        visit(doc, &mut |site| {
            if !change.matches_site(site) {
                return Action::Keep;
            }
            count = count.saturating_add(1);
            if !update {
                return Action::Keep;
            }
            match change.site_action(site) {
                Some(action) => {
                    modified = true;
                    action
                }
                None => {
                    not_updated = not_updated.saturating_add(1);
                    Action::Keep
                }
            }
        });
        if count > 0 {
            other_count = other_count.saturating_add(count);
            let mut entry = loc.to_json();
            if let Some(o) = entry.as_object_mut() {
                o.insert("count".into(), json!(count));
            }
            others.push(entry);
        }
    }
    let query_refs = match &refs_before {
        Some(before) => follow_query_refs(report, before),
        None => 0,
    };
    let mut out = json!({
        "affected_bindings": bindings,
        "affected_binding_count": binding_count,
        "other_references": others,
        "other_reference_count": other_count,
        "updated": update && (binding_count > 0 || other_count > 0),
    });
    if let Some(o) = out.as_object_mut() {
        if query_refs > 0 {
            o.insert("query_refs_updated".into(), json!(query_refs));
            modified = true;
        }
        if not_updated > 0 {
            o.insert(
                "not_updated".into(),
                json!(format!(
                    "{not_updated} reference(s) go through a query alias shared with other fields and were \
                     not rewritten; fix them in Power BI Desktop"
                )),
            );
        }
        let affected = binding_count > 0 || other_count > 0;
        if affected && !update {
            o.insert(
                "hint".into(),
                json!(if change.rewritable() {
                    "the report still uses the old name: pass \"update_report\": true to rewrite it, \
                     or run report.validate to list what is broken"
                } else {
                    "these report bindings no longer resolve: rebind or remove them (visual.unbind, \
                     visual.remove); report.validate lists them"
                }),
            );
        }
    }
    (out, modified)
}

// ---------------------------------------------------------------------------------------------
// queryRefs
// ---------------------------------------------------------------------------------------------

/// The `queryRef` a projection of `field` (as it is before `change`) gets after `change`, when
/// it carries Desktop's name for the field (`Sales.Total`, `Sum(Sales.Amount)`; for other
/// expressions of a renamed table, any `Table.…` name). `None` when it keeps its name.
fn renamed_query_ref(change: &ModelChange, field: &FieldRef, query_ref: &str) -> Option<String> {
    use bicraft_report::query::default_query_ref;
    match field {
        FieldRef::Other { raw } => {
            let ModelChange::TableRenamed { old, new } = change else {
                return None;
            };
            let mut uses = false;
            find_refs(raw, &mut |s| {
                if s.kind == SiteKind::Table && s.entity.eq_ignore_ascii_case(old) {
                    uses = true;
                }
            });
            let prefix = format!("{old}.");
            let rest = query_ref
                .get(..prefix.len())
                .filter(|p| p.eq_ignore_ascii_case(&prefix))
                .and_then(|_| query_ref.get(prefix.len()..))?;
            uses.then(|| format!("{new}.{rest}"))
        }
        f => {
            if !change.matches_field(f) {
                return None;
            }
            let mut renamed = f.clone();
            change.rewrite_field(&mut renamed);
            let old = default_query_ref(f)?;
            let new = default_query_ref(&renamed)?;
            (old.eq_ignore_ascii_case(query_ref) && new != query_ref).then_some(new)
        }
    }
}

/// `(old, new)` queryRefs of the visual's projections that `change` renames. A new name that
/// another projection of the visual already has (or that two projections would share) is not
/// used: those projections keep their name.
fn visual_query_ref_renames(visual: &bicraft_report::Visual, change: &ModelChange) -> Vec<(String, String)> {
    let Some(state) = visual
        .visual_extra
        .get("query")
        .and_then(|q| q.get("queryState"))
        .and_then(Value::as_object)
    else {
        return Vec::new();
    };
    let mut all: Vec<&str> = Vec::new();
    let mut map: Vec<(String, String)> = Vec::new();
    for role in state.values() {
        for p in role
            .get("projections")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(r) = p.get("queryRef").and_then(Value::as_str) else {
                continue;
            };
            all.push(r);
            let field = bicraft_report::query::field_ref(p.get("field"));
            if let Some(n) = renamed_query_ref(change, &field, r)
                && !map.iter().any(|(o, _)| o == r)
            {
                map.push((r.to_string(), n));
            }
        }
    }
    let taken = |n: &str, o: &str| {
        all.iter().any(|a| *a == n && *a != o) || map.iter().filter(|(_, m)| m == n).count() > 1
    };
    let clashes: Vec<bool> = map.iter().map(|(o, n)| taken(n, o)).collect();
    map.into_iter()
        .zip(clashes)
        .filter(|(_, clash)| !clash)
        .map(|(m, _)| m)
        .collect()
}

/// Replaces the queryRefs of `map` wherever a visual's JSON names one: projection `queryRef`,
/// selector `metadata` (a string, or a list in bookmark highlights), `SelectRef.ExpressionName`,
/// expansion-state `queryRefs`. Returns how many strings changed.
fn rewrite_query_refs(v: &mut Value, map: &[(String, String)], depth: usize) -> usize {
    if depth > MAX_DEPTH {
        return 0;
    }
    let lookup = |s: &str| map.iter().find(|(o, _)| o == s).map(|(_, n)| n.clone());
    let mut n = 0usize;
    match v {
        Value::Array(items) => {
            for item in items {
                n = n.saturating_add(rewrite_query_refs(item, map, depth.saturating_add(1)));
            }
        }
        Value::Object(o) => {
            for (k, x) in o.iter_mut() {
                let named = matches!(
                    k.as_str(),
                    "queryRef" | "metadata" | "ExpressionName" | "queryRefs"
                );
                match x {
                    Value::String(s) if named => {
                        if let Some(new) = lookup(s) {
                            *s = new;
                            n = n.saturating_add(1);
                        }
                    }
                    Value::Array(items) if named => {
                        for item in items.iter_mut() {
                            match item {
                                Value::String(s) => {
                                    if let Some(new) = lookup(s) {
                                        *s = new;
                                        n = n.saturating_add(1);
                                    }
                                }
                                other => {
                                    n = n.saturating_add(rewrite_query_refs(
                                        other,
                                        map,
                                        depth.saturating_add(1),
                                    ));
                                }
                            }
                        }
                    }
                    _ => n = n.saturating_add(rewrite_query_refs(x, map, depth.saturating_add(1))),
                }
            }
        }
        _ => {}
    }
    n
}

/// Renames the stored queryRefs (a raw `query`, as in files Desktop saved) that `change`
/// affects (see [`visual_query_ref_renames`]). Visuals without a stored query get theirs from
/// their bindings, so they follow the rename by themselves.
fn rename_stored_query_refs(report: &mut Report, change: &ModelChange) {
    for page in &mut report.pages {
        for visual in &mut page.visuals {
            let map = visual_query_ref_renames(visual, change);
            if map.is_empty() {
                continue;
            }
            if let Some(state) = visual
                .visual_extra
                .get_mut("query")
                .and_then(|q| q.get_mut("queryState"))
                .and_then(Value::as_object_mut)
            {
                for role in state.values_mut() {
                    for p in role
                        .get_mut("projections")
                        .and_then(Value::as_array_mut)
                        .into_iter()
                        .flatten()
                    {
                        if let Some(Value::String(r)) = p.get_mut("queryRef")
                            && let Some((_, new)) = map.iter().find(|(o, _)| o == r)
                        {
                            *r = new.clone();
                        }
                    }
                }
            }
        }
    }
}

/// `(old, new)` queryRef pairs.
type RefMap = Vec<(String, String)>;

/// The queryRefs a visual's query has (stored, or derived from its bindings), by role.
pub(crate) type QueryRefs = Vec<(String, Vec<Option<String>>)>;

fn query_refs(v: &bicraft_report::Visual) -> QueryRefs {
    let query = bicraft_report::query::effective(v);
    let Some(state) = query
        .as_ref()
        .and_then(|q| q.get("queryState"))
        .and_then(Value::as_object)
    else {
        return Vec::new();
    };
    state
        .iter()
        .map(|(role, rs)| {
            let refs = rs
                .get("projections")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|p| p.get("queryRef").and_then(Value::as_str).map(str::to_string))
                .collect();
            (role.clone(), refs)
        })
        .collect()
}

/// Every visual's [`QueryRefs`], by page then visual.
pub(crate) fn snapshot_query_refs(report: &Report) -> Vec<Vec<QueryRefs>> {
    report
        .pages
        .iter()
        .map(|p| p.visuals.iter().map(query_refs).collect())
        .collect()
}

/// After a rename: the queryRefs that changed since `before` (a projection keeps its role and
/// place), replaced wherever the visual names them (selectors of series formatting, conditional
/// formats and analytics lines, `SelectRef` values, expansion states, the phone layout
/// `mobile.json`, the visual's state in bookmarks), so that a renamed field keeps its
/// formatting. Returns how many projections and references changed.
pub(crate) fn follow_query_refs(report: &mut Report, before: &[Vec<QueryRefs>]) -> usize {
    let mut n = 0usize;
    // (page, visual, (old, new) queryRefs)
    let mut renamed: Vec<(String, String, RefMap)> = Vec::new();
    for (page, refs_page) in report.pages.iter_mut().zip(before) {
        for (visual, refs0) in page.visuals.iter_mut().zip(refs_page) {
            let refs1 = query_refs(visual);
            let mut map: Vec<(String, String)> = Vec::new();
            for (role, list0) in refs0 {
                let Some((_, list1)) = refs1.iter().find(|(r, _)| r == role) else {
                    continue;
                };
                for (a, b) in list0.iter().zip(list1) {
                    if let (Some(a), Some(b)) = (a, b)
                        && a != b
                        && !map.iter().any(|(o, _)| o == a)
                    {
                        map.push((a.clone(), b.clone()));
                    }
                }
            }
            if map.is_empty() {
                continue;
            }
            n = n.saturating_add(map.len());
            for (k, x) in visual.visual_extra.iter_mut() {
                if k != "query" {
                    n = n.saturating_add(rewrite_query_refs(x, &map, 0));
                }
            }
            for x in visual.extra.values_mut() {
                n = n.saturating_add(rewrite_query_refs(x, &map, 0));
            }
            renamed.push((page.name.clone(), visual.name.clone(), map));
        }
    }
    for (page, visual, map) in &renamed {
        let path = format!("definition/pages/{page}/visuals/{visual}/mobile.json");
        if let Some(res) = report
            .resources
            .iter_mut()
            .find(|r| r.path.replace('\\', "/").eq_ignore_ascii_case(&path))
            && let Ok(mut doc) = serde_json::from_slice::<Value>(&res.bytes)
        {
            let k = rewrite_query_refs(&mut doc, map, 0);
            if k > 0
                && let Ok(bytes) = serde_json::to_vec_pretty(&doc)
            {
                res.bytes = bytes;
                n = n.saturating_add(k);
            }
        }
        for b in &mut report.bookmarks {
            if let Some(state) = b
                .document
                .get_mut("explorationState")
                .and_then(|e| e.get_mut("sections"))
                .and_then(|s| s.get_mut(page.as_str()))
                .and_then(|s| s.get_mut("visualContainers"))
                .and_then(|c| c.get_mut(visual.as_str()))
            {
                n = n.saturating_add(rewrite_query_refs(state, map, 0));
            }
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_parse() {
        assert_eq!(
            parse_field(" Sales[Amount] ").unwrap(),
            ParsedField::Qualified {
                table: "Sales".into(),
                name: "Amount".into()
            }
        );
        assert_eq!(
            parse_field("[Total ]]x]").unwrap(),
            ParsedField::Measure("Total ]x".into())
        );
        assert_eq!(
            parse_field("average('Order Lines'[Qty])").unwrap(),
            ParsedField::Aggregation {
                function: "Avg",
                table: "Order Lines".into(),
                column: "Qty".into()
            }
        );
        assert_eq!(
            parse_field("'Sales (2024)'[Amount]").unwrap(),
            ParsedField::Qualified {
                table: "Sales (2024)".into(),
                name: "Amount".into()
            }
        );
        assert!(
            parse_field("Foo(Sales[Amount])")
                .unwrap_err()
                .contains("unknown aggregation")
        );
        for bad in ["", "[]", "[a]b]", "Sales", "Sum(Sales[Amount]", "Sum()", "[Total"] {
            assert!(parse_field(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_visitor_resolves_aliases_and_rewrites() {
        let mut v = json!({
            "field": {"Column": {"Expression": {"SourceRef": {"Entity": "Sales"}}, "Property": "Region"}},
            "filter": {
                "From": [{"Name": "s", "Entity": "Sales", "Type": 0}],
                "Where": [{"Condition": {"In": {"Expressions": [
                    {"Column": {"Expression": {"SourceRef": {"Source": "s"}}, "Property": "Region"}},
                    {"Measure": {"Expression": {"SourceRef": {"Source": "s"}}, "Property": "Total"}},
                    {"Column": {"Expression": {"SourceRef": {"Source": "unknown"}}, "Property": "X"}}
                ]}}}]
            },
            "ext": {"Measure": {"Expression": {"SourceRef": {"Schema": "extension", "Entity": "Sales"}}, "Property": "Local"}}
        });
        let mut seen = Vec::new();
        visit(&mut v, &mut |s| {
            seen.push((
                s.kind,
                s.entity.to_string(),
                s.property.to_string(),
                s.direct,
                s.extension,
            ));
            if s.kind == SiteKind::Column && s.property == "Region" {
                Action::SetProperty("Area".into())
            } else if s.kind == SiteKind::Table {
                Action::SetEntity("Fact".into())
            } else {
                Action::Keep
            }
        });
        let columns = seen.iter().filter(|s| s.0 == SiteKind::Column).count();
        assert_eq!(columns, 2);
        assert!(seen.contains(&(SiteKind::Measure, "Sales".into(), "Total".into(), false, false)));
        assert!(seen.contains(&(SiteKind::Measure, "Sales".into(), "Local".into(), true, true)));
        assert_eq!(v["field"]["Column"]["Property"], "Area");
        assert_eq!(v["field"]["Column"]["Expression"]["SourceRef"]["Entity"], "Fact");
        assert_eq!(v["filter"]["From"][0]["Entity"], "Fact");
        assert_eq!(
            v["filter"]["Where"][0]["Condition"]["In"]["Expressions"][0]["Column"]["Property"],
            "Area"
        );
    }

    #[test]
    fn deep_json_does_not_overflow() {
        let mut v = json!(1);
        for _ in 0..400 {
            v = json!({"a": [v]});
        }
        let mut n = 0;
        visit(&mut v, &mut |_| {
            n += 1;
            Action::Keep
        });
        assert_eq!(n, 0);
    }
}
