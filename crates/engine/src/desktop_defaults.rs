//! M9.7: the properties and annotations Power BI Desktop gives the objects it creates, given
//! by bicraft to the objects it creates, so that Desktop's first save of a bicraft project
//! changes nothing. Observed on Desktop 2.158.1304.0, see `docs/formats/tmdl.md`, *Desktop's
//! default annotations*.
//!
//! Only new objects get them: models read from files and objects edited by `*.set` keep their
//! own annotations byte for byte.

use bicraft_data::m::{Analysis, analyze_all};
use bicraft_data::{DateKind, Limits};
use bicraft_model::{Annotation, Column, DataType, SemanticModel, Table};

use crate::partition::set_annotation;

/// How the summarization of a column was chosen (`Automatic`, or `User` when it was given).
pub(crate) const SUMMARIZATION_SET_BY: &str = "SummarizationSetBy";
/// The value kind of a query (`Table` for a table's query).
pub(crate) const RESULT_TYPE: &str = "PBI_ResultType";
/// The step Desktop's navigator wrote (`Navigation`).
pub(crate) const NAVIGATION_STEP: &str = "PBI_NavigationStepName";
/// Desktop's number-format hint. Its `currencyCulture` form (and the currency format string of
/// a `decimal` column) is not written: Desktop takes both from the machine's regional format,
/// not from the model culture (docs/formats/tmdl.md, *Currency columns*).
pub(crate) const FORMAT_HINT: &str = "PBI_FormatHint";
/// The date/time type of a `dateTime` column's source (`Date`, `Time`, `DateTime`).
pub(crate) const UNDERLYING_DATE_TIME: &str = "UnderlyingDateTimeDataType";

/// The summarization Desktop gives a new column: numbers are summed.
pub(crate) fn default_summarize(t: DataType) -> &'static str {
    match t {
        DataType::Int64 | DataType::Double | DataType::Decimal => "sum",
        _ => "none",
    }
}

fn analysis(expression: &str) -> Analysis {
    analyze_all(vec![expression.to_string()], &Limits::default())
        .into_iter()
        .next()
        .unwrap_or_default()
}

fn query_annotations(list: &mut Vec<Annotation>, a: &Analysis, result: Option<&str>) {
    if a.steps.iter().any(|s| s == "Navigation") {
        set_annotation(list, NAVIGATION_STEP, "Navigation");
    }
    if let Some(r) = result {
        set_annotation(list, RESULT_TYPE, r);
    }
}

/// A table that gets its first M partition.
pub(crate) fn m_table(t: &mut Table, expression: &str) {
    query_annotations(&mut t.annotations, &analysis(expression), Some("Table"));
}

/// A new shared expression.
pub(crate) fn m_expression(list: &mut Vec<Annotation>, a: &Analysis) {
    query_annotations(list, a, a.result_type);
}

/// A new table (`table.add`, `table.import`): its query's annotations.
pub(crate) fn new_table(t: &mut Table) {
    let m = t
        .partitions
        .iter()
        .find(|p| crate::mquery::is_m(&p.source_type))
        .and_then(|p| p.expression.clone());
    if let Some(e) = m {
        m_table(t, &e);
    }
}

/// Desktop's format string of a new `boolean` column (TMSL `"TRUE";"TRUE";"FALSE"`, TMDL
/// `"""TRUE"";""TRUE"";""FALSE"""`).
const BOOLEAN_FORMAT: &str = "\"TRUE\";\"TRUE\";\"FALSE\"";

/// The boolean format earlier bicraft versions wrote (the TMDL spelling stored as the value,
/// so quoted twice in the files): still a default, replaced when the column's type changes.
const LEGACY_BOOLEAN_FORMAT: &str = "\"\"\"TRUE\"\";\"\"TRUE\"\";\"\"FALSE\"\"\"";

/// A new column (`column.add`, the columns of `table.import`). `summarize_given`: the caller
/// chose the summarization; `date_kind`: the M type of a `dateTime` column's source, when
/// known (an M load, or the declared types of the table's query, see [`source_date_kind`]).
pub(crate) fn new_column(c: &mut Column, summarize_given: bool, date_kind: Option<DateKind>) {
    if c.summarize_by.is_none() {
        c.summarize_by = Some(default_summarize(c.data_type).into());
    }
    let by = if summarize_given { "User" } else { "Automatic" };
    set_annotation(&mut c.annotations, SUMMARIZATION_SET_BY, by);
    let given_format = c.format_string.is_some();
    if !given_format && let Some(f) = default_format(c.data_type, date_kind) {
        c.format_string = Some(f.into());
    }
    if let Some(u) = underlying(c.data_type, date_kind) {
        set_annotation(&mut c.annotations, UNDERLYING_DATE_TIME, u);
    }
    if !given_format && c.data_type == DataType::Double {
        set_annotation(&mut c.annotations, FORMAT_HINT, "{\"isGeneralNumber\":true}");
    }
}

/// The format string Desktop gives a new column of type `t` (`date_kind`: the M type of a
/// `dateTime` column's source).
fn default_format(t: DataType, date_kind: Option<DateKind>) -> Option<&'static str> {
    match (t, date_kind) {
        (DataType::Int64, _) => Some("0"),
        (DataType::Boolean, _) => Some(BOOLEAN_FORMAT),
        (DataType::DateTime, Some(DateKind::Date)) => Some("Long Date"),
        (DataType::DateTime, Some(DateKind::Time)) => Some("Long Time"),
        (DataType::DateTime, Some(DateKind::DateTime | DateKind::DateTimeZone)) => Some("General Date"),
        _ => None,
    }
}

/// The `UnderlyingDateTimeDataType` annotation of a `dateTime` column (only date and time
/// sources get one).
fn underlying(t: DataType, date_kind: Option<DateKind>) -> Option<&'static str> {
    match (t, date_kind) {
        (DataType::DateTime, Some(DateKind::Date)) => Some("Date"),
        (DataType::DateTime, Some(DateKind::Time)) => Some("Time"),
        _ => None,
    }
}

/// Most other queries looked into for a column's type (`Source = OtherQuery`).
const MAX_REFERENCED: usize = 8;

fn lookup(kinds: &[(String, DateKind)], column: &str) -> Option<DateKind> {
    kinds
        .iter()
        .find(|(n, _)| n == column)
        .or_else(|| kinds.iter().find(|(n, _)| n.eq_ignore_ascii_case(column)))
        .map(|(_, k)| *k)
}

/// The date/time type table `t`'s M query declares for its source column `source`, read from
/// the query text (typed `#table`, `Table.TransformColumnTypes`, typed `Table.AddColumn`), then
/// from the shared queries it references. `None` when the text does not tell.
pub(crate) fn source_date_kind(m: &SemanticModel, t: &Table, source: &str) -> Option<DateKind> {
    let e = t
        .partitions
        .iter()
        .find(|p| crate::mquery::is_m(&p.source_type))
        .and_then(|p| p.expression.as_deref())?;
    let a = analysis(e);
    if let Some(k) = lookup(&a.date_kinds, source) {
        return Some(k);
    }
    let mut seen: Vec<&str> = Vec::new();
    for r in &a.references {
        if seen.len() >= MAX_REFERENCED {
            break;
        }
        if seen.contains(&r.name.as_str()) {
            continue;
        }
        seen.push(&r.name);
        if let Some(x) = m
            .expressions
            .iter()
            .find(|x| x.name == r.name && crate::mquery::is_m(&x.kind))
            && let Some(k) = lookup(&analysis(&x.expression).date_kinds, source)
        {
            return Some(k);
        }
    }
    None
}

/// The format strings Desktop gives new columns: replaced when the column's type changes.
fn is_default_format(f: &str) -> bool {
    matches!(f, "0" | "Long Date" | "Long Time" | "General Date")
        || f == BOOLEAN_FORMAT
        || f == LEGACY_BOOLEAN_FORMAT
}

/// `column.set` changed data column `c`'s type from `old` (the format string was not given):
/// the type's default format string replaces a default one (or none), and the
/// `UnderlyingDateTimeDataType` annotation follows the new type, as Desktop does.
pub(crate) fn data_type_changed(
    c: &mut Column,
    old: DataType,
    format_given: bool,
    date_kind: Option<DateKind>,
) {
    if c.data_type == old {
        return;
    }
    if !format_given && c.format_string.as_deref().is_none_or(is_default_format) {
        c.format_string = default_format(c.data_type, date_kind).map(str::to_string);
    }
    c.annotations.retain(|a| a.name != UNDERLYING_DATE_TIME);
    if let Some(u) = underlying(c.data_type, date_kind) {
        set_annotation(&mut c.annotations, UNDERLYING_DATE_TIME, u);
    }
}
