//! M9.1: model object properties that Power BI Desktop keeps and agents set.
//!
//! - measures: `format_string_definition` (dynamic format string, TMSL
//!   `formatStringDefinition.expression`), validated as a scalar DAX expression of the measure's
//!   table. As in TOM, setting it clears the static `formatString` (the engine refuses a measure
//!   with both); `null` removes it and leaves no format string unless `format_string` is passed
//!   in the same call;
//! - columns: `sort_by_column`, `is_available_in_mdx`, `is_key`, `data_category`, with the
//!   refusals of Power BI Desktop / Analysis Services (a sort-by column with more than one value
//!   per value of the sorted column, sort-by cycles, attribute hierarchies needed by sort-by,
//!   hierarchies, relationships and variations, two key columns);
//! - annotations of the model, tables, columns and measures (`*.set_annotation`);
//! - `model.set`: culture, discourage implicit measures, source query culture;
//! - date variations: `column.remove_variation`, and `table.remove` / `column.remove` take the
//!   variations and auto date tables (`LocalDateTable_…`) along.
//!
//! `measure.add`/`measure.set`/`column.add`/`column.set`/`table.remove`/`column.remove` are
//! wrappers: the base command in [`crate::edit`] runs first, then the properties here are applied
//! as a second model edit. When the second part fails the session is put back as it was, so the
//! command stays all-or-nothing (and one undo step, like every command).

use bicraft_dax::deps::{ObjectRef, expression_dependencies};
use bicraft_dax::types::{ScalarType, ValueType};
use bicraft_dax::{Context, Diagnostic, ExprKind, Severity, validate_as};
use bicraft_model::{Annotation, Column, SemanticModel, Table};
use bicraft_vertipaq::CellValue;
use serde_json::{Map, Value, json};

use crate::Session;
use crate::commands::model;
use crate::edit::{self, Edit, apply, check_name, dax_column, dax_measure, dax_table, table_index};
use crate::params::{allow, change_str, force, opt_bool, opt_str, str_param};

const MEASURE_KEYS: &[&str] = &["format_string_definition"];
const COLUMN_KEYS: &[&str] = &["sort_by_column", "is_available_in_mdx", "is_key", "data_category"];
/// TMSL property of a measure's dynamic format string.
const FORMAT_STRING_DEFINITION: &str = "formatStringDefinition";
const AVAILABLE_IN_MDX: &str = "isAvailableInMdx";
const AVAILABLE_IN_MDX_LEGACY: &str = "isAvailableInMDX";
/// Most rows read for the data checks of `sort_by_column` and `is_key`.
const MAX_CHECK_ROWS: usize = 5_000_000;
/// Most offending values named in a message.
const MAX_LISTED: usize = 5;

/// Column data categories TOM knows, in their TMSL spelling (Desktop's "Data category" menu).
const DATA_CATEGORIES: &[&str] = &[
    "Address",
    "Place",
    "City",
    "County",
    "StateOrProvince",
    "PostalCode",
    "Country",
    "Continent",
    "Latitude",
    "Longitude",
    "WebUrl",
    "ImageUrl",
    "Barcode",
    "PaddedDateTableDates",
];

type Run = fn(&mut Session, &Value) -> Result<Value, String>;

// ---------------------------------------------------------------------------------------------
// Running a base command and the properties as one command
// ---------------------------------------------------------------------------------------------

/// What a failed second step puts back.
pub(crate) struct Saved {
    model: Option<SemanticModel>,
    model_error: Option<String>,
    dirty: bool,
    report: Option<bicraft_report::Report>,
    report_dirty: bool,
}

impl Saved {
    pub(crate) fn take(s: &Session) -> Self {
        Self {
            model: s.model.clone(),
            model_error: s.model_error.clone(),
            dirty: s.dirty,
            report: s.report.clone(),
            report_dirty: s.report_dirty,
        }
    }

    pub(crate) fn restore(self, s: &mut Session) {
        s.model = self.model;
        s.model_error = self.model_error;
        s.dirty = self.dirty;
        s.report = self.report;
        s.report_dirty = self.report_dirty;
    }
}

/// Splits `p` into the base command's parameters and the keys of `mine`.
fn split(p: &Value, mine: &[&str]) -> Result<(Value, Map<String, Value>), String> {
    let obj = match p {
        Value::Null => return Ok((Value::Null, Map::new())),
        Value::Object(o) => o,
        _ => return Err("parameters must be a JSON object".into()),
    };
    let mut base = Map::new();
    let mut own = Map::new();
    for (k, v) in obj {
        if mine.contains(&k.as_str()) {
            own.insert(k.clone(), v.clone());
        } else {
            base.insert(k.clone(), v.clone());
        }
    }
    Ok((Value::Object(base), own))
}

/// Runs `base` (when given), then `second`; a failing `second` puts the session back.
fn combine(
    s: &mut Session,
    base: Option<(Run, &Value)>,
    second: impl FnOnce(&mut Session, Option<&Value>) -> Result<Value, String>,
) -> Result<Value, String> {
    let saved = base.is_some().then(|| Saved::take(s));
    let first = match base {
        Some((run, p)) => Some(run(s, p)?),
        None => None,
    };
    match second(s, first.as_ref()) {
        Ok(v) => Ok(merge(first, v)),
        Err(e) => {
            if let Some(saved) = saved {
                saved.restore(s);
            }
            Err(e)
        }
    }
}

/// The base command's result with the second step's fields added (`warnings` and
/// `diagnostics` concatenated).
fn merge(first: Option<Value>, second: Value) -> Value {
    let mut out = match first {
        Some(Value::Object(o)) => o,
        _ => Map::new(),
    };
    if let Value::Object(o) = second {
        for (k, v) in o {
            match (out.get_mut(&k), v) {
                (Some(Value::Array(a)), Value::Array(b)) if k == "warnings" || k == "diagnostics" => {
                    a.extend(b);
                }
                (_, v) => {
                    out.insert(k, v);
                }
            }
        }
    }
    Value::Object(out)
}

fn force_only(force: bool) -> Value {
    json!({ "force": force })
}

fn summarize(diags: &[Diagnostic]) -> String {
    let errors: Vec<String> = diags
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .map(|d| format!("{}:{}: {} [{}]", d.line, d.column, d.message, d.code))
        .collect();
    if errors.is_empty() {
        "the expression is incomplete".into()
    } else {
        errors.join(", ")
    }
}

// ---------------------------------------------------------------------------------------------
// Measures: format_string_definition
// ---------------------------------------------------------------------------------------------

/// `format_string_definition`: absent → `None`, `null`/`""` → remove, text → set.
fn parse_definition(own: &Map<String, Value>) -> Result<Option<Option<String>>, String> {
    let p = Value::Object(own.clone());
    change_str(&p, "format_string_definition")
}

fn cleared_note(measure: &str, format: &str) -> String {
    format!(
        "measure {} has a dynamic format string: its static format string `{format}` was cleared \
         (the engine refuses a measure with both, as TOM clears it)",
        dax_measure(measure)
    )
}

/// After a `measure.set` that set `format_string` on a measure with a dynamic format string (and
/// left the dynamic one in place): refused, the engine refuses a measure with both.
fn check_static(s: &mut Session, name: &str) -> Result<Value, String> {
    let both = model(s)?
        .measure(name)
        .is_some_and(|(_, x)| x.extra.contains_key(FORMAT_STRING_DEFINITION) && x.format_string.is_some());
    if both {
        return Err(format!(
            "measure {} has a dynamic format string; remove it (\"format_string_definition\": null) \
             in the same call to give it a static format string",
            dax_measure(name)
        ));
    }
    Ok(json!({}))
}

pub(crate) fn measure_add(s: &mut Session, p: &Value) -> Result<Value, String> {
    let (base, own) = split(p, MEASURE_KEYS)?;
    if own.is_empty() {
        return edit::measure_add(s, p);
    }
    let definition = parse_definition(&own)?;
    let force = force(p)?;
    let name = str_param(p, "name")?.to_string();
    combine(s, Some((edit::measure_add, &base)), |s, _| match definition {
        Some(d) => set_definition(s, &name, None, d, force),
        None => Ok(json!({})),
    })
}

pub(crate) fn measure_set(s: &mut Session, p: &Value) -> Result<Value, String> {
    let (base, own) = split(p, MEASURE_KEYS)?;
    let static_given = p.get("format_string").is_some();
    if own.is_empty() && !static_given {
        return edit::measure_set(s, p);
    }
    let definition = parse_definition(&own)?;
    let force = force(p)?;
    let name = str_param(p, "name")?.to_string();
    let table = opt_str(p, "table")?.map(str::to_string);
    let base_changes = base.as_object().is_some_and(|o| {
        o.keys().any(|k| {
            !matches!(
                k.as_str(),
                "table" | "name" | "force" | "update_report" | "update_dax"
            )
        })
    });
    let base = base_changes.then_some((edit::measure_set as Run, &base));
    if !base_changes {
        // The base command checks its parameters; without it, check them here.
        allow(
            p,
            &[
                "table",
                "name",
                "force",
                "update_report",
                "update_dax",
                "format_string_definition",
            ],
        )?;
    }
    combine(s, base, |s, first| {
        let current = first
            .and_then(|r| r.get("measure"))
            .and_then(Value::as_str)
            .unwrap_or(&name)
            .to_string();
        match definition {
            Some(d) => set_definition(s, &current, table.as_deref(), d, force),
            None => check_static(s, &current),
        }
    })
}

fn find_measure(m: &SemanticModel, name: &str) -> Result<(usize, usize), String> {
    m.tables
        .iter()
        .enumerate()
        .find_map(|(ti, t)| {
            t.measures
                .iter()
                .position(|x| x.name.eq_ignore_ascii_case(name))
                .map(|mi| (ti, mi))
        })
        .ok_or_else(|| format!("no measure named `{name}`"))
}

fn set_definition(
    s: &mut Session,
    name: &str,
    table: Option<&str>,
    definition: Option<String>,
    force: bool,
) -> Result<Value, String> {
    apply(s, &force_only(force), |m| {
        let (ti, mi) = find_measure(m, name)?;
        let t = m
            .tables
            .get_mut(ti)
            .ok_or_else(|| "internal: table index out of range".to_string())?;
        if let Some(expected) = table
            && !t.name.eq_ignore_ascii_case(expected)
        {
            return Err(format!(
                "measure `{name}` is in table `{}`, not `{expected}`",
                t.name
            ));
        }
        let table_name = t.name.clone();
        let x = t
            .measures
            .get_mut(mi)
            .ok_or_else(|| "internal: measure index out of range".to_string())?;
        let measure_name = x.name.clone();
        let mut edit = Edit::new(json!({
            "table": table_name,
            "measure": measure_name,
            "format_string_definition": definition,
        }));
        match &definition {
            None => {
                if x.extra.shift_remove(FORMAT_STRING_DEFINITION).is_none() {
                    edit.notes.push(format!(
                        "measure {} had no dynamic format string",
                        dax_measure(&measure_name)
                    ));
                }
            }
            Some(text) => {
                if text.trim().is_empty() {
                    return Err("format_string_definition must not be blank (null removes it)".into());
                }
                if let Some(fs) = x.format_string.take() {
                    edit.notes.push(cleared_note(&measure_name, &fs));
                }
                let entry = x
                    .extra
                    .entry(FORMAT_STRING_DEFINITION)
                    .or_insert_with(|| Value::Object(Map::new()));
                match entry {
                    Value::Object(o) => {
                        o.insert("expression".into(), Value::String(text.clone()));
                    }
                    other => *other = json!({ "expression": text }),
                }
            }
        }
        if let Some(text) = &definition {
            let v = validate_as(
                text,
                &Context {
                    model: Some(m),
                    table: Some(&table_name),
                },
                ExprKind::Measure,
            );
            let object = format!(
                "the format string expression of measure {}",
                dax_measure(&measure_name)
            );
            if !v.valid {
                if !force {
                    return Err(format!(
                        "invalid DAX, nothing was changed: {object}: {} (pass \"force\": true to apply anyway)",
                        summarize(&v.diagnostics)
                    ));
                }
                edit.notes.push(format!(
                    "applied with invalid DAX (force): {object}: {}",
                    summarize(&v.diagnostics)
                ));
            } else if let Some(ValueType::Scalar(t)) = v.result_type
                && !matches!(t, ScalarType::String | ScalarType::Variant)
            {
                edit.notes.push(format!(
                    "{object} returns {t:?}, not text; the engine converts the value to a format string"
                ));
            }
        }
        Ok(edit)
    })
}

// ---------------------------------------------------------------------------------------------
// Columns: sort_by_column, is_available_in_mdx, is_key, data_category
// ---------------------------------------------------------------------------------------------

#[derive(Default)]
struct ColumnProps {
    sort_by: Option<Option<String>>,
    available_in_mdx: Option<bool>,
    is_key: Option<bool>,
    data_category: Option<Option<String>>,
}

impl ColumnProps {
    fn parse(own: &Map<String, Value>) -> Result<(Self, Vec<String>), String> {
        let p = Value::Object(own.clone());
        let mut notes = Vec::new();
        let data_category = match change_str(&p, "data_category")? {
            Some(Some(c)) => Some(normalize_category(&c, &mut notes)),
            other => other,
        };
        Ok((
            Self {
                sort_by: change_str(&p, "sort_by_column")?,
                available_in_mdx: opt_bool(&p, "is_available_in_mdx")?,
                is_key: opt_bool(&p, "is_key")?,
                data_category,
            },
            notes,
        ))
    }

    fn is_empty(&self) -> bool {
        self.sort_by.is_none()
            && self.available_in_mdx.is_none()
            && self.is_key.is_none()
            && self.data_category.is_none()
    }
}

/// The TMSL spelling of a data category (`Uncategorized` clears it); unknown categories are kept
/// as given, with a warning.
fn normalize_category(c: &str, notes: &mut Vec<String>) -> Option<String> {
    let squashed: String = c.chars().filter(|ch| !ch.is_whitespace()).collect();
    if squashed.eq_ignore_ascii_case("uncategorized") {
        return None;
    }
    if let Some(known) = DATA_CATEGORIES.iter().find(|k| k.eq_ignore_ascii_case(&squashed)) {
        return Some((*known).to_string());
    }
    notes.push(format!(
        "data_category `{c}` is not one Power BI Desktop offers ({}); it is kept as given",
        DATA_CATEGORIES.join(", ")
    ));
    Some(c.to_string())
}

fn available_in_mdx(c: &Column) -> bool {
    c.extra
        .get(AVAILABLE_IN_MDX)
        .or_else(|| c.extra.get(AVAILABLE_IN_MDX_LEGACY))
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

fn is_key(c: &Column) -> bool {
    c.extra.get("isKey").and_then(Value::as_bool) == Some(true)
}

pub(crate) fn column_add(s: &mut Session, p: &Value) -> Result<Value, String> {
    let (base, own) = split(p, COLUMN_KEYS)?;
    if own.is_empty() {
        return edit::column_add(s, p);
    }
    let (props, notes) = ColumnProps::parse(&own)?;
    let force = force(p)?;
    let table = str_param(p, "table")?.to_string();
    let name = str_param(p, "name")?.to_string();
    combine(s, Some((edit::column_add, &base)), |s, first| {
        let table = first
            .and_then(|r| r.get("table"))
            .and_then(Value::as_str)
            .unwrap_or(&table)
            .to_string();
        set_column_props(s, &table, &name, &props, notes, force)
    })
}

pub(crate) fn column_set(s: &mut Session, p: &Value) -> Result<Value, String> {
    let (base, own) = split(p, COLUMN_KEYS)?;
    if own.is_empty() {
        return edit::column_set(s, p);
    }
    let (props, notes) = ColumnProps::parse(&own)?;
    let force = force(p)?;
    let table = str_param(p, "table")?.to_string();
    let name = str_param(p, "name")?.to_string();
    let base_changes = base.as_object().is_some_and(|o| {
        o.keys().any(|k| {
            !matches!(
                k.as_str(),
                "table" | "name" | "force" | "update_report" | "update_dax"
            )
        })
    });
    if !base_changes {
        let mut allowed = vec!["table", "name", "force", "update_report", "update_dax"];
        allowed.extend_from_slice(COLUMN_KEYS);
        allow(p, &allowed)?;
    }
    let base = base_changes.then_some((edit::column_set as Run, &base));
    combine(s, base, |s, first| {
        let current = first
            .and_then(|r| r.get("column"))
            .and_then(Value::as_str)
            .unwrap_or(&name)
            .to_string();
        set_column_props(s, &table, &current, &props, notes, force)
    })
}

/// One row key for the data checks: texts compare case-insensitively, as the engine does.
fn key(v: &CellValue) -> String {
    match v {
        CellValue::Null => "\u{0}blank".into(),
        CellValue::Text(t) => format!("t:{}", t.to_lowercase()),
        CellValue::Int(n) => format!("n:{}", *n as f64),
        CellValue::Real(x) | CellValue::DateTime(x) => format!("n:{x}"),
        CellValue::Decimal(n) => format!("n:{}", *n as f64 / 10_000.0),
        CellValue::Bool(b) => format!("b:{b}"),
    }
}

fn shown(v: &CellValue) -> String {
    match v {
        CellValue::Null => "(blank)".into(),
        CellValue::Text(t) => format!("\"{t}\""),
        CellValue::Int(n) => n.to_string(),
        CellValue::Real(x) | CellValue::DateTime(x) => x.to_string(),
        CellValue::Decimal(n) => format!("{}", *n as f64 / 10_000.0),
        CellValue::Bool(b) => b.to_string(),
    }
}

/// The values of `table[column]`: `Ok(None)` with the reason when the session has no data.
fn values(s: &Session, table: &str, column: &str) -> Result<Vec<CellValue>, String> {
    match crate::preview::column_values(s, table, column, MAX_CHECK_ROWS) {
        Ok(Some(v)) => Ok(v),
        Ok(None) => Err(format!("no data is available for {}", dax_column(table, column))),
        Err(e) => Err(e),
    }
}

/// Values of `column` that have more than one value of `sort_by` (at most [`MAX_LISTED`]).
fn sort_conflicts(a: &[CellValue], b: &[CellValue]) -> Vec<String> {
    let mut seen: std::collections::HashMap<String, (usize, &CellValue)> = std::collections::HashMap::new();
    let mut out: Vec<String> = Vec::new();
    let mut reported: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (i, (x, y)) in a.iter().zip(b).enumerate() {
        let kx = key(x);
        match seen.get(&kx) {
            None => {
                seen.insert(kx, (i, y));
            }
            Some((_, first)) => {
                if key(first) != key(y) && reported.insert(kx) {
                    out.push(format!(
                        "{} (sort values {} and {})",
                        shown(x),
                        shown(first),
                        shown(y)
                    ));
                    if out.len() >= MAX_LISTED {
                        break;
                    }
                }
            }
        }
    }
    out
}

/// Duplicate (or blank) values of a key column (at most [`MAX_LISTED`]).
fn key_conflicts(a: &[CellValue]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for v in a {
        let problem = if matches!(v, CellValue::Null) {
            Some("(blank)".to_string())
        } else if !seen.insert(key(v)) {
            Some(format!("{} (duplicate)", shown(v)))
        } else {
            None
        };
        if let Some(p) = problem
            && !out.contains(&p)
        {
            out.push(p);
            if out.len() >= MAX_LISTED {
                break;
            }
        }
    }
    out
}

/// The data checks of Desktop: a sort-by column has one value per value of the sorted column;
/// a key column has unique, non-blank values. Errors are refusals unless `force`; missing data is
/// a warning.
fn data_checks(
    s: &Session,
    table: &str,
    column: &str,
    props: &ColumnProps,
    force: bool,
    notes: &mut Vec<String>,
) -> Result<(), String> {
    let m = model(s)?;
    let Some(t) = m.table(table) else {
        return Ok(());
    };
    let Some(c) = t.column(column) else {
        return Ok(());
    };
    let (tn, cn) = (t.name.clone(), c.name.clone());
    let mut problems = Vec::new();
    if let Some(Some(target)) = &props.sort_by
        && let Some(b) = t.column(target)
    {
        let bn = b.name.clone();
        match values(s, &tn, &cn).and_then(|a| Ok((a, values(s, &tn, &bn)?))) {
            Ok((a, b)) if a.len() == b.len() => {
                let conflicts = sort_conflicts(&a, &b);
                if !conflicts.is_empty() {
                    problems.push(format!(
                        "{} cannot be sorted by {}: it has more than one {} value for the same {} value, \
                         which Power BI Desktop refuses: {}",
                        dax_column(&tn, &cn),
                        dax_column(&tn, &bn),
                        dax_column(&tn, &bn),
                        dax_column(&tn, &cn),
                        conflicts.join("; ")
                    ));
                }
            }
            Ok(_) => notes.push(format!(
                "the sort-by values of {} were not checked: the columns' data differ in length",
                dax_column(&tn, &cn)
            )),
            Err(e) => notes.push(format!(
                "the sort-by values of {} were not checked against the data ({e}); Power BI Desktop \
                 refuses a sort-by column with more than one value per value of the sorted column",
                dax_column(&tn, &cn)
            )),
        }
    }
    if props.is_key == Some(true) {
        match values(s, &tn, &cn) {
            Ok(a) => {
                let conflicts = key_conflicts(&a);
                if !conflicts.is_empty() {
                    problems.push(format!(
                        "{} cannot be a key column: its values must be unique and not blank: {}",
                        dax_column(&tn, &cn),
                        conflicts.join("; ")
                    ));
                }
            }
            Err(e) => notes.push(format!(
                "the values of key column {} were not checked against the data ({e}); a key column \
                 must have unique, non-blank values",
                dax_column(&tn, &cn)
            )),
        }
    }
    if problems.is_empty() {
        return Ok(());
    }
    if force {
        notes.extend(
            problems
                .into_iter()
                .map(|p| format!("applied anyway (force): {p}")),
        );
        Ok(())
    } else {
        Err(format!(
            "{}. Nothing was changed (pass \"force\": true to apply anyway)",
            problems.join("; ")
        ))
    }
}

/// The uses of `table[column]` that involve its attribute hierarchy: `(refused, what)`. The engine
/// of Desktop 2.158 refuses `isAvailableInMdx: false` on a sort-by column and on a hierarchy
/// level's column (checked on the engine, 2026-10-14); a sorted column, a relationship key and a
/// variation's default column without hierarchy deploy and query fine, so they only warn.
fn hierarchy_uses(m: &SemanticModel, t: &Table, column: &str) -> Vec<(bool, String)> {
    let mut out: Vec<(bool, String)> = Vec::new();
    let me = |tn: &str, cn: &str| tn.eq_ignore_ascii_case(&t.name) && cn.eq_ignore_ascii_case(column);
    for other in &t.columns {
        if other
            .sort_by_column
            .as_deref()
            .is_some_and(|x| x.eq_ignore_ascii_case(column))
        {
            out.push((
                true,
                format!("the sort-by column of {}", dax_column(&t.name, &other.name)),
            ));
        }
    }
    if let Some(c) = t.column(column)
        && let Some(b) = &c.sort_by_column
    {
        out.push((false, format!("sorted by {}", dax_column(&t.name, b))));
    }
    for h in t
        .extra
        .get("hierarchies")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice)
    {
        let levels = h
            .get("levels")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        if levels.iter().any(|l| {
            l.get("column")
                .and_then(Value::as_str)
                .is_some_and(|c| c.eq_ignore_ascii_case(column))
        }) {
            let name = h.get("name").and_then(Value::as_str).unwrap_or_default();
            out.push((true, format!("a level of hierarchy `{name}`")));
        }
    }
    for r in &m.relationships {
        if me(&r.from_table, &r.from_column) || me(&r.to_table, &r.to_column) {
            out.push((false, format!("a key of relationship `{}`", r.name)));
        }
    }
    for ot in &m.tables {
        for oc in &ot.columns {
            for v in oc.variations() {
                if v.default_column.as_ref().is_some_and(|(vt, vc)| me(vt, vc)) {
                    out.push((
                        false,
                        format!(
                            "the default column of variation `{}` of {}",
                            v.name,
                            dax_column(&ot.name, &oc.name)
                        ),
                    ));
                }
            }
        }
    }
    out
}

fn set_column_props(
    s: &mut Session,
    table: &str,
    column: &str,
    props: &ColumnProps,
    mut notes: Vec<String>,
    force: bool,
) -> Result<Value, String> {
    if props.is_empty() {
        return Ok(json!({ "warnings": notes }));
    }
    // Structural refusals first (cheap), then the data checks.
    {
        let m = model(s)?;
        notes.extend(check_column_props(m, table, column, props)?);
    }
    data_checks(s, table, column, props, force, &mut notes)?;
    apply(s, &force_only(force), |m| {
        check_column_props(m, table, column, props)?;
        let ti = table_index(m, table)?;
        let t = m
            .tables
            .get_mut(ti)
            .ok_or_else(|| "internal: table index out of range".to_string())?;
        let table_name = t.name.clone();
        let sort_target = match &props.sort_by {
            Some(Some(b)) => Some(Some(
                t.column(b).map(|c| c.name.clone()).unwrap_or_else(|| b.clone()),
            )),
            Some(None) => Some(None),
            None => None,
        };
        let c = t
            .columns
            .iter_mut()
            .find(|c| !c.is_row_number() && c.name.eq_ignore_ascii_case(column))
            .ok_or_else(|| format!("no column `{column}` in table `{table_name}`"))?;
        if let Some(v) = sort_target {
            c.sort_by_column = v;
        }
        if let Some(v) = props.available_in_mdx {
            c.extra.shift_remove(AVAILABLE_IN_MDX_LEGACY);
            if v {
                c.extra.shift_remove(AVAILABLE_IN_MDX);
            } else {
                c.extra.insert(AVAILABLE_IN_MDX.into(), Value::Bool(false));
            }
        }
        if let Some(v) = props.is_key {
            if v {
                c.extra.insert("isKey".into(), Value::Bool(true));
            } else {
                c.extra.shift_remove("isKey");
            }
        }
        if let Some(v) = &props.data_category {
            c.data_category = v.clone();
        }
        let mut edit = Edit::new(json!({
            "table": table_name,
            "column": c.name,
            "sort_by_column": c.sort_by_column,
            "is_available_in_mdx": available_in_mdx(c),
            "is_key": is_key(c),
            "data_category": c.data_category,
        }));
        edit.notes.append(&mut notes);
        Ok(edit)
    })
}

/// The refusals that need no data.
fn check_column_props(
    m: &SemanticModel,
    table: &str,
    column: &str,
    props: &ColumnProps,
) -> Result<Vec<String>, String> {
    let t = m
        .table(table)
        .ok_or_else(|| format!("no table named `{table}`"))?;
    let c = t
        .column(column)
        .ok_or_else(|| format!("no column `{column}` in table `{}`", t.name))?;
    let me = dax_column(&t.name, &c.name);
    if let Some(Some(target)) = &props.sort_by {
        let b = t.column(target).ok_or_else(|| {
            format!(
                "no column `{target}` in table `{}`: a column is sorted by a column of its own table",
                t.name
            )
        })?;
        let other = dax_column(&t.name, &b.name);
        if b.name.eq_ignore_ascii_case(&c.name) {
            return Err(format!("{me} cannot be sorted by itself"));
        }
        if b.sort_by_column
            .as_deref()
            .is_some_and(|x| x.eq_ignore_ascii_case(&c.name))
        {
            return Err(format!(
                "{me} cannot be sorted by {other}: {other} is sorted by {me} (a sort-by cycle)"
            ));
        }
        if let Some(e) = &b.expression {
            let target = ObjectRef::Column {
                table: t.name.clone(),
                name: c.name.clone(),
            };
            let uses = |deps: &[ObjectRef]| {
                deps.iter().any(|d| match (d, &target) {
                    (ObjectRef::Column { table: a, name: x }, ObjectRef::Column { table: b, name: y }) => {
                        a.eq_ignore_ascii_case(b) && x.eq_ignore_ascii_case(y)
                    }
                    _ => false,
                })
            };
            if uses(&expression_dependencies(m, e, Some(&t.name), true)) {
                return Err(format!(
                    "{me} cannot be sorted by {other}: {other} is calculated from {me} (a circular dependency)"
                ));
            }
        }
        if !available_in_mdx(b) {
            return Err(format!(
                "{me} cannot be sorted by {other}: {other} has no attribute hierarchy \
                 (is_available_in_mdx false)"
            ));
        }
    }
    let mut notes = Vec::new();
    if props.available_in_mdx == Some(false) {
        let mut uses = hierarchy_uses(m, t, &c.name);
        if matches!(props.sort_by, Some(None)) {
            uses.retain(|(_, u)| !u.starts_with("sorted by"));
        }
        let refused: Vec<&str> = uses.iter().filter(|(r, _)| *r).map(|(_, u)| u.as_str()).collect();
        if !refused.is_empty() {
            return Err(format!(
                "is_available_in_mdx cannot be false on {me}: the engine needs its attribute \
                 hierarchy, it is {}",
                refused.join(", ")
            ));
        }
        let others: Vec<&str> = uses.iter().map(|(_, u)| u.as_str()).collect();
        if !others.is_empty() {
            notes.push(format!(
                "{me} has no attribute hierarchy now and is {}: the engine accepts it, but \
                 visuals that group by it may be slower",
                others.join(", ")
            ));
        }
    }
    if props.is_key == Some(true)
        && let Some(other) = t
            .columns
            .iter()
            .find(|x| !x.is_row_number() && is_key(x) && !x.name.eq_ignore_ascii_case(&c.name))
    {
        return Err(format!(
            "{} is already the key column of table {}; a table has one key column \
             (set is_key false on it first)",
            dax_column(&t.name, &other.name),
            dax_table(&t.name)
        ));
    }
    Ok(notes)
}

// ---------------------------------------------------------------------------------------------
// Annotations
// ---------------------------------------------------------------------------------------------

/// `value`: a string sets, `null` removes; required.
fn annotation_value(p: &Value) -> Result<Option<String>, String> {
    match p.get("value") {
        None => Err("missing parameter `value` (a string, or null to remove the annotation)".into()),
        Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err("parameter `value` must be a string or null".into()),
    }
}

/// Sets or removes the annotation `name`; returns the previous value.
fn set_in(list: &mut Vec<Annotation>, name: &str, value: Option<String>) -> Option<String> {
    let at = list.iter().position(|a| a.name == name);
    match (at, value) {
        (Some(i), Some(v)) => list.get_mut(i).map(|a| std::mem::replace(&mut a.value, v)),
        (Some(i), None) => Some(list.remove(i).value),
        (None, Some(v)) => {
            list.push(Annotation {
                name: name.to_string(),
                value: v,
            });
            None
        }
        (None, None) => None,
    }
}

/// Which object an annotation command targets.
enum Target {
    Model,
    Table(String),
    Column(String, String),
    Measure(String, Option<String>),
}

fn set_annotation(s: &mut Session, p: &Value, target: Target) -> Result<Value, String> {
    let name = str_param(p, "name")?.to_string();
    check_name("annotation", &name)?;
    let value = annotation_value(p)?;
    apply(s, &Value::Null, |m| {
        let (mut result, list): (Map<String, Value>, &mut Vec<Annotation>) = match &target {
            Target::Model => (Map::new(), &mut m.annotations),
            Target::Table(table) => {
                let ti = table_index(m, table)?;
                let t = m
                    .tables
                    .get_mut(ti)
                    .ok_or_else(|| "internal: table index out of range".to_string())?;
                let mut r = Map::new();
                r.insert("table".into(), json!(t.name));
                (r, &mut t.annotations)
            }
            Target::Column(table, column) => {
                let ti = table_index(m, table)?;
                let t = m
                    .tables
                    .get_mut(ti)
                    .ok_or_else(|| "internal: table index out of range".to_string())?;
                let tn = t.name.clone();
                let c = t
                    .columns
                    .iter_mut()
                    .find(|c| !c.is_row_number() && c.name.eq_ignore_ascii_case(column))
                    .ok_or_else(|| format!("no column `{column}` in table `{tn}`"))?;
                let mut r = Map::new();
                r.insert("table".into(), json!(tn));
                r.insert("column".into(), json!(c.name));
                (r, &mut c.annotations)
            }
            Target::Measure(measure, table) => {
                let (ti, mi) = find_measure(m, measure)?;
                let t = m
                    .tables
                    .get_mut(ti)
                    .ok_or_else(|| "internal: table index out of range".to_string())?;
                if let Some(expected) = table
                    && !t.name.eq_ignore_ascii_case(expected)
                {
                    return Err(format!(
                        "measure `{measure}` is in table `{}`, not `{expected}`",
                        t.name
                    ));
                }
                let tn = t.name.clone();
                let x = t
                    .measures
                    .get_mut(mi)
                    .ok_or_else(|| "internal: measure index out of range".to_string())?;
                let mut r = Map::new();
                r.insert("table".into(), json!(tn));
                r.insert("measure".into(), json!(x.name));
                (r, &mut x.annotations)
            }
        };
        let previous = set_in(list, &name, value.clone());
        let mut notes = Vec::new();
        if value.is_none() && previous.is_none() {
            notes.push(format!("there was no annotation `{name}`; nothing to remove"));
        }
        if name.starts_with("PBI_") || name.starts_with("__PBI_") {
            notes.push(format!(
                "annotation `{name}` is managed by Power BI Desktop, which may rewrite it"
            ));
        }
        result.insert("name".into(), json!(name));
        result.insert("value".into(), json!(value));
        result.insert("previous".into(), json!(previous));
        let mut edit = Edit::new(Value::Object(result));
        edit.notes.extend(notes);
        Ok(edit)
    })
}

pub(crate) fn model_set_annotation(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["name", "value"])?;
    set_annotation(s, p, Target::Model)
}

pub(crate) fn table_set_annotation(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["table", "name", "value"])?;
    let table = str_param(p, "table")?.to_string();
    set_annotation(s, p, Target::Table(table))
}

pub(crate) fn column_set_annotation(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["table", "column", "name", "value"])?;
    let table = str_param(p, "table")?.to_string();
    let column = str_param(p, "column")?.to_string();
    set_annotation(s, p, Target::Column(table, column))
}

pub(crate) fn measure_set_annotation(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["measure", "table", "name", "value"])?;
    let measure = str_param(p, "measure")?.to_string();
    let table = opt_str(p, "table")?.map(str::to_string);
    set_annotation(s, p, Target::Measure(measure, table))
}

// ---------------------------------------------------------------------------------------------
// model.set
// ---------------------------------------------------------------------------------------------

/// A culture name such as `en-US`, `fr-FR`, `zh-Hans-CN` (BCP 47 shape).
pub(crate) fn check_culture(key: &str, c: &str) -> Result<(), String> {
    let mut parts = c.split('-');
    let lang_ok = parts
        .next()
        .is_some_and(|l| (2..=3).contains(&l.len()) && l.chars().all(|ch| ch.is_ascii_alphabetic()));
    let rest_ok = parts.all(|x| (2..=8).contains(&x.len()) && x.chars().all(|ch| ch.is_ascii_alphanumeric()));
    if lang_ok && rest_ok {
        Ok(())
    } else {
        Err(format!(
            "`{key}`: `{c}` is not a culture name such as en-US or fr-FR"
        ))
    }
}

pub(crate) fn model_set(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &["culture", "discourage_implicit_measures", "source_query_culture"],
    )?;
    let culture = opt_str(p, "culture")?.map(str::to_string);
    if let Some(c) = &culture {
        check_culture("culture", c)?;
    }
    let discourage = opt_bool(p, "discourage_implicit_measures")?;
    let source_culture = change_str(p, "source_query_culture")?;
    if let Some(Some(c)) = &source_culture {
        check_culture("source_query_culture", c)?;
    }
    if culture.is_none() && discourage.is_none() && source_culture.is_none() {
        return Err(
            "model.set needs at least one of `culture`, `discourage_implicit_measures`, `source_query_culture`"
                .into(),
        );
    }
    let has_data = crate::preview::stored_backup(s).is_some();
    apply(s, &Value::Null, |m| {
        let mut notes = Vec::new();
        if let Some(c) = &culture
            && m.culture.as_deref() != Some(c.as_str())
        {
            if has_data {
                notes.push(
                    "the open file stores data collated for the previous culture: a PBIX save refuses \
                     a culture change (save as a PBIT or a PBIP without cache instead), and Power BI \
                     Desktop applies the new culture to the data after a refresh"
                        .to_string(),
                );
            }
            let old = m.culture.replace(c.clone());
            if let Some(old) = old
                && m.extra
                    .get("cultures")
                    .and_then(Value::as_array)
                    .is_some_and(|a| {
                        a.iter()
                            .any(|x| x.get("name").and_then(Value::as_str) == Some(old.as_str()))
                    })
            {
                notes.push(format!(
                    "the model keeps its culture `{old}` entry (linguistic schema, translations)"
                ));
            }
        }
        if let Some(d) = discourage {
            if d {
                m.extra
                    .insert("discourageImplicitMeasures".into(), Value::Bool(true));
            } else {
                m.extra.shift_remove("discourageImplicitMeasures");
            }
        }
        if let Some(c) = &source_culture {
            match c {
                Some(c) => {
                    m.extra
                        .insert("sourceQueryCulture".into(), Value::String(c.clone()));
                }
                None => {
                    m.extra.shift_remove("sourceQueryCulture");
                }
            }
        }
        let mut edit = Edit::new(json!({
            "culture": m.culture,
            "discourage_implicit_measures": m.extra.get("discourageImplicitMeasures").and_then(Value::as_bool).unwrap_or(false),
            "source_query_culture": m.extra.get("sourceQueryCulture"),
        }));
        edit.notes.extend(notes);
        Ok(edit)
    })
}

// ---------------------------------------------------------------------------------------------
// Date variations and auto date tables
// ---------------------------------------------------------------------------------------------

/// Power BI Desktop's auto date/time tables: `LocalDateTable_<guid>`, annotated
/// `__PBI_LocalDateTable`.
pub(crate) fn is_auto_date_table(t: &Table) -> bool {
    t.name
        .get(..15)
        .is_some_and(|p| p.eq_ignore_ascii_case("LocalDateTable_"))
        || t.annotations
            .iter()
            .any(|a| a.name == "__PBI_LocalDateTable" && a.value.eq_ignore_ascii_case("true"))
}

/// A variation of a column: (table, column, variation name, relationship, target table).
#[derive(Debug, Clone)]
struct VarRef {
    table: String,
    column: String,
    name: String,
    relationship: Option<String>,
    target: Option<String>,
}

fn all_variations(m: &SemanticModel) -> Vec<VarRef> {
    let mut out = Vec::new();
    for t in &m.tables {
        for c in &t.columns {
            for v in c.variations() {
                let target = m.variation_table(&t.name, &c.name, &v).map(|x| x.name.clone());
                out.push(VarRef {
                    table: t.name.clone(),
                    column: c.name.clone(),
                    name: v.name.clone(),
                    relationship: v.relationship.clone(),
                    target,
                });
            }
        }
    }
    out
}

fn same(a: &VarRef, b: &VarRef) -> bool {
    a.table.eq_ignore_ascii_case(&b.table)
        && a.column.eq_ignore_ascii_case(&b.column)
        && a.name.eq_ignore_ascii_case(&b.name)
}

/// Removes the variations `gone` from the raw `variations` collections (other properties of the
/// remaining entries are kept as they are).
fn drop_variations(m: &mut SemanticModel, gone: &[VarRef]) {
    for t in &mut m.tables {
        for c in &mut t.columns {
            let Some(Value::Array(list)) = c.extra.get_mut("variations") else {
                continue;
            };
            list.retain(|v| {
                let name = v.get("name").and_then(Value::as_str).unwrap_or_default();
                !gone.iter().any(|g| {
                    g.table.eq_ignore_ascii_case(&t.name)
                        && g.column.eq_ignore_ascii_case(&c.name)
                        && g.name.eq_ignore_ascii_case(name)
                })
            });
            if list.is_empty() {
                c.extra.shift_remove("variations");
            }
        }
    }
}

/// Removes table `name` with its relationships and row filters; returns the relationships.
fn drop_table(m: &mut SemanticModel, name: &str) -> Vec<String> {
    m.tables.retain(|t| !t.name.eq_ignore_ascii_case(name));
    let mut removed = Vec::new();
    m.relationships.retain(|r| {
        let hit = r.from_table.eq_ignore_ascii_case(name) || r.to_table.eq_ignore_ascii_case(name);
        if hit {
            removed.push(r.name.clone());
        }
        !hit
    });
    for r in &mut m.roles {
        r.table_permissions
            .retain(|tp| !tp.table.eq_ignore_ascii_case(name));
    }
    removed
}

/// DAX (model and report-level measures) that uses table `name`.
fn table_used_by(s: &Session, name: &str) -> Result<Vec<(Value, String)>, String> {
    let m = model(s)?;
    let target = ObjectRef::Table {
        name: name.to_string(),
    };
    let mut found = crate::dax_rename::dependants(m, &target);
    if let Some(r) = s.report.as_ref() {
        found.extend(crate::report_measures::dependants(r, m, &target));
    }
    Ok(found)
}

/// The auto date tables `removed` pointed to that no variation of `rest` points to any more.
fn orphaned(m: &SemanticModel, removed: &[VarRef], rest: &[VarRef]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for v in removed {
        let Some(target) = &v.target else { continue };
        let still_used = rest.iter().any(|r| {
            r.target
                .as_deref()
                .is_some_and(|x| x.eq_ignore_ascii_case(target))
        });
        let auto = m.table(target).is_some_and(is_auto_date_table);
        if auto && !still_used && !out.iter().any(|x| x.eq_ignore_ascii_case(target)) {
            out.push(target.clone());
        }
    }
    out
}

pub(crate) fn column_remove_variation(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["table", "name", "variation", "force"])?;
    let table = str_param(p, "table")?;
    let column = str_param(p, "name")?;
    let variation = opt_str(p, "variation")?;
    let force = force(p)?;
    let (gone, tables) = {
        let m = model(s)?;
        let t = m
            .table(table)
            .ok_or_else(|| format!("no table named `{table}`"))?;
        let c = t
            .column(column)
            .ok_or_else(|| format!("no column `{column}` in table `{}`", t.name))?;
        let all = all_variations(m);
        let mine: Vec<VarRef> = all
            .iter()
            .filter(|v| v.table.eq_ignore_ascii_case(&t.name) && v.column.eq_ignore_ascii_case(&c.name))
            .cloned()
            .collect();
        if mine.is_empty() {
            return Err(format!("{} has no variations", dax_column(&t.name, &c.name)));
        }
        let gone: Vec<VarRef> = match variation {
            None => mine.clone(),
            Some(n) => mine
                .iter()
                .filter(|v| v.name.eq_ignore_ascii_case(n))
                .cloned()
                .collect(),
        };
        if gone.is_empty() {
            let names: Vec<&str> = mine.iter().map(|v| v.name.as_str()).collect();
            return Err(format!(
                "{} has no variation `{}` (it has: {})",
                dax_column(&t.name, &c.name),
                variation.unwrap_or_default(),
                names.join(", ")
            ));
        }
        let rest: Vec<VarRef> = all
            .iter()
            .filter(|v| !gone.iter().any(|g| same(g, v)))
            .cloned()
            .collect();
        (gone.clone(), orphaned(m, &gone, &rest))
    };
    let mut used_by = Vec::new();
    for t in &tables {
        used_by.extend(table_used_by(s, t)?);
    }
    if !used_by.is_empty() && !force {
        let labels: Vec<String> = used_by.iter().map(|(_, l)| l.clone()).collect();
        return Err(crate::dax_rename::refusal(
            &format!("auto date table(s) {}", tables.join(", ")),
            &labels,
        ));
    }
    let used_by: Vec<Value> = used_by.into_iter().map(|(v, _)| v).collect();
    apply(s, &force_only(force), |m| {
        drop_variations(m, &gone);
        let rest = all_variations(m);
        let mut removed_relationships = Vec::new();
        for v in &gone {
            let Some(r) = &v.relationship else { continue };
            let in_use = rest.iter().any(|x| {
                x.relationship
                    .as_deref()
                    .is_some_and(|y| y.eq_ignore_ascii_case(r))
            });
            if !in_use {
                let before = m.relationships.len();
                m.relationships.retain(|x| !x.name.eq_ignore_ascii_case(r));
                if m.relationships.len() != before {
                    removed_relationships.push(r.clone());
                }
            }
        }
        for t in &tables {
            removed_relationships.extend(drop_table(m, t));
        }
        let names: Vec<&str> = gone.iter().map(|v| v.name.as_str()).collect();
        let (table, column) = gone
            .first()
            .map(|v| (v.table.clone(), v.column.clone()))
            .unwrap_or_default();
        Ok(Edit::new(json!({
            "table": table,
            "column": column,
            "removed_variations": names,
            "removed_relationships": removed_relationships,
            "removed_tables": tables,
            "used_by": used_by,
        }))
        .breaking())
    })
}

/// `table.remove`: the variations that point to the table go first (a variation without its
/// relationship or table does not load), then the table.
pub(crate) fn table_remove(s: &mut Session, p: &Value) -> Result<Value, String> {
    let pointing: Vec<VarRef> = match s.model.as_ref() {
        Some(m) => {
            let name = p.get("name").and_then(Value::as_str).unwrap_or_default();
            all_variations(m)
                .into_iter()
                .filter(|v| {
                    v.target.as_deref().is_some_and(|t| t.eq_ignore_ascii_case(name))
                        && !v.table.eq_ignore_ascii_case(name)
                })
                .collect()
        }
        None => Vec::new(),
    };
    if pointing.is_empty() {
        return edit::table_remove(s, p);
    }
    let force = force(p)?;
    combine(s, Some((edit::table_remove, p)), |s, _| {
        apply(s, &force_only(force), |m| {
            drop_variations(m, &pointing);
            let list: Vec<Value> = pointing
                .iter()
                .map(|v| json!({"table": v.table, "column": v.column, "variation": v.name}))
                .collect();
            Ok(Edit::new(json!({ "removed_variations": list })))
        })
    })
}

/// `column.remove`: the auto date tables only the column's variations pointed to go with it
/// (unless DAX uses them).
pub(crate) fn column_remove(s: &mut Session, p: &Value) -> Result<Value, String> {
    let (gone, candidates) = match s.model.as_ref() {
        Some(m) => {
            let table = p.get("table").and_then(Value::as_str).unwrap_or_default();
            let column = p.get("name").and_then(Value::as_str).unwrap_or_default();
            let all = all_variations(m);
            let (gone, rest): (Vec<VarRef>, Vec<VarRef>) = all
                .into_iter()
                .partition(|v| v.table.eq_ignore_ascii_case(table) && v.column.eq_ignore_ascii_case(column));
            let candidates = orphaned(m, &gone, &rest);
            (gone, candidates)
        }
        None => (Vec::new(), Vec::new()),
    };
    if gone.is_empty() || candidates.is_empty() {
        return edit::column_remove(s, p);
    }
    let force = force(p)?;
    combine(s, Some((edit::column_remove, p)), |s, _| {
        let mut kept = Vec::new();
        let mut drop = Vec::new();
        for t in &candidates {
            if table_used_by(s, t)?.is_empty() {
                drop.push(t.clone());
            } else {
                kept.push(t.clone());
            }
        }
        apply(s, &force_only(force), |m| {
            let mut removed_relationships = Vec::new();
            for t in &drop {
                removed_relationships.extend(drop_table(m, t));
            }
            let mut edit = Edit::new(json!({
                "removed_tables": drop,
                "removed_date_table_relationships": removed_relationships,
            }));
            for t in &kept {
                edit.notes.push(format!(
                    "auto date table {} is kept: DAX uses it (remove it with table.remove)",
                    dax_table(t)
                ));
            }
            Ok(edit)
        })
    })
}
