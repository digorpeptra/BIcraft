//! The typed DAX function catalog: one [`Signature`] per function of the public DAX reference,
//! checked against the engine's own function list (`$SYSTEM.MDSCHEMA_FUNCTIONS`) and argument
//! counts. Conventions are documented in `docs/formats/dax-functions.md`; the short version:
//!
//! - `params[..repeat_from]` is a fixed prefix. `params[repeat_from..]` starts with the *repeating
//!   unit* (the run of params with `repeating: true`), which may be followed by a non-repeating
//!   *tail* (`SWITCH`'s `else`, `LOOKUPVALUE`'s `alternate_result`).
//! - The unit is mandatory (at least one occurrence) iff it contains a required param.
//! - A few functions have several repeating sections in the engine (`SUMMARIZECOLUMNS`'s
//!   group-by columns, filter tables and name/expression pairs). They are flattened into one unit
//!   of optional params, matched by argument shape.
//! - `Returns::SameAsArg`/`CommonOfArgs` indices are *parameter* indices: every argument bound to a
//!   repeating param counts, and omitted arguments are ignored.
//! - Numeric params use `Decimal` (any number), `Integer` where a whole number is expected.

use std::cmp::Ordering;
use std::sync::OnceLock;

use crate::types::{Param, ParamKind, Returns, ScalarType, Signature, ValueType};

mod aggregation;
mod date_time;
mod descriptions;
mod filter;
mod financial;
mod info;
mod information;
mod logical;
mod math;
mod other;
mod parent_child;
mod relationship;
mod statistical;
mod table;
mod text;
mod time_intelligence;
mod undocumented;
mod visual;

#[cfg(test)]
mod tests;

/// Every category table, in the order of the public reference.
const TABLES: &[&[Signature]] = &[
    aggregation::FUNCTIONS,
    date_time::FUNCTIONS,
    filter::FUNCTIONS,
    financial::FUNCTIONS,
    info::FUNCTIONS,
    information::FUNCTIONS,
    logical::FUNCTIONS,
    math::FUNCTIONS,
    other::FUNCTIONS,
    parent_child::FUNCTIONS,
    relationship::FUNCTIONS,
    statistical::FUNCTIONS,
    table::FUNCTIONS,
    text::FUNCTIONS,
    time_intelligence::FUNCTIONS,
    visual::FUNCTIONS,
    undocumented::FUNCTIONS,
];

/// All signatures, sorted by name.
pub(crate) fn all() -> &'static [&'static Signature] {
    static INDEX: OnceLock<Vec<&'static Signature>> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut v: Vec<&'static Signature> = TABLES.iter().flat_map(|t| t.iter()).collect();
        v.sort_by(|a, b| a.name.cmp(b.name));
        v
    })
}

/// The one-sentence description of function `name`: (English, French).
pub(crate) fn description(name: &str) -> Option<(&'static str, &'static str)> {
    descriptions::DESCRIPTIONS
        .iter()
        .find(|(n, _, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, en, fr)| (*en, *fr))
}

/// Case-insensitive lookup by binary search (catalog names are upper-case ASCII).
pub(crate) fn find(name: &str) -> Option<&'static Signature> {
    let all = all();
    all.binary_search_by(|s| cmp_upper(s.name, name))
        .ok()
        .and_then(|i| all.get(i).copied())
}

/// Orders an upper-case catalog name against `query` as if `query` were upper-cased.
fn cmp_upper(catalog: &str, query: &str) -> Ordering {
    catalog.bytes().cmp(query.bytes().map(|b| b.to_ascii_uppercase()))
}

/// Whether some parameter accepts `word` as a bare keyword (case-insensitive).
pub(crate) fn is_keyword(word: &str) -> bool {
    keywords().binary_search_by(|k| cmp_upper(k, word)).is_ok()
}

/// Every bare keyword some parameter accepts, sorted and deduplicated (upper-case).
pub(crate) fn keywords() -> &'static [&'static str] {
    static WORDS: OnceLock<Vec<&'static str>> = OnceLock::new();
    WORDS.get_or_init(|| {
        let mut v: Vec<&'static str> = all()
            .iter()
            .flat_map(|s| s.params.iter())
            .filter_map(|p| match p.kind {
                ParamKind::Keyword(set) | ParamKind::KeywordOrScalar(set) => Some(set),
                _ => None,
            })
            .flat_map(|set| set.iter().copied())
            .chain(ANY_KIND_KEYWORDS.iter().copied())
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    })
}

/// Keywords accepted by params typed `Any` because they also take expressions: the visual-shape
/// axes of window functions (`relation` or `axis`), the `reset` modes, and the series selection
/// criterion of `SAMPLEAXISWITHLOCALMINMAX` (`ALPHABETICAL`, in Desktop 2.159's line chart
/// queries).
const ANY_KIND_KEYWORDS: &[&str] = &[
    "ROWS",
    "COLUMNS",
    "NONE",
    "LOWESTPARENT",
    "HIGHESTPARENT",
    "ALPHABETICAL",
];

// ---- Construction helpers (const, so every table is plain static data) ----

const fn param(name: &'static str, kind: ParamKind, optional: bool, repeating: bool) -> Param {
    Param {
        name,
        kind,
        optional,
        repeating,
        group: 0,
    }
}

/// Required parameter.
pub(super) const fn p(name: &'static str, kind: ParamKind) -> Param {
    param(name, kind, false, false)
}

/// Optional parameter.
pub(super) const fn o(name: &'static str, kind: ParamKind) -> Param {
    param(name, kind, true, false)
}

/// Required member of the repeating unit.
pub(super) const fn r(name: &'static str, kind: ParamKind) -> Param {
    param(name, kind, false, true)
}

/// Optional member of the repeating unit.
pub(super) const fn ro(name: &'static str, kind: ParamKind) -> Param {
    param(name, kind, true, true)
}

/// A fixed-arity signature.
pub(super) const fn f(
    category: &'static str,
    name: &'static str,
    params: &'static [Param],
    returns: Returns,
) -> Signature {
    Signature {
        name,
        params,
        repeat_from: None,
        returns,
        category,
        deprecated: false,
        min_args: None,
    }
}

/// A variadic signature: the repeating unit starts at `params[repeat_from]`.
pub(super) const fn v(
    category: &'static str,
    name: &'static str,
    params: &'static [Param],
    repeat_from: u8,
    returns: Returns,
) -> Signature {
    Signature {
        name,
        params,
        repeat_from: Some(repeat_from),
        returns,
        category,
        deprecated: false,
        min_args: None,
    }
}

/// Puts a repeating param in the engine's repeat group `group` (multi-section functions).
pub(super) const fn section(p: Param, group: u8) -> Param {
    Param { group, ..p }
}

/// Sets the minimum argument count of a multi-section signature.
pub(super) const fn min_args(sig: Signature, min: u8) -> Signature {
    Signature {
        min_args: Some(min),
        ..sig
    }
}

/// Marks a signature deprecated or not recommended.
#[allow(dead_code)] // no function of the current reference is deprecated; kept for future rows
pub(super) const fn deprecated(sig: Signature) -> Signature {
    Signature {
        deprecated: true,
        ..sig
    }
}

// ---- Parameter kinds ----

/// Any number (integer, decimal or currency).
pub(super) const NUM: ParamKind = ParamKind::Scalar(Some(ScalarType::Decimal));
/// A whole number.
pub(super) const INT: ParamKind = ParamKind::Scalar(Some(ScalarType::Integer));
pub(super) const STR: ParamKind = ParamKind::Scalar(Some(ScalarType::String));
pub(super) const BOOL: ParamKind = ParamKind::Scalar(Some(ScalarType::Boolean));
pub(super) const DATE: ParamKind = ParamKind::Scalar(Some(ScalarType::DateTime));
/// Any scalar.
pub(super) const SCALAR: ParamKind = ParamKind::Scalar(None);
pub(super) const TABLE: ParamKind = ParamKind::Table;
pub(super) const TABLE_REF: ParamKind = ParamKind::TableRef;
pub(super) const COL: ParamKind = ParamKind::ColumnRef;
pub(super) const TABLE_OR_COL: ParamKind = ParamKind::TableOrColumnRef;
/// Scalar expression evaluated in a row context the function creates.
pub(super) const ROW: ParamKind = ParamKind::RowExpression;
pub(super) const FILTER: ParamKind = ParamKind::Filter;
pub(super) const NAME: ParamKind = ParamKind::NewColumnName;
pub(super) const ANY: ParamKind = ParamKind::Any;
/// Time-intelligence `dates` argument: a date column, a single-column table of dates, or a
/// calendar reference.
pub(super) const DATES: ParamKind = ParamKind::Dates;
/// A calendar reference (calendar-based time intelligence).
pub(super) const CALENDAR: ParamKind = ParamKind::Calendar;
/// A measure reference (`[Measure]`).
pub(super) const MEASURE: ParamKind = ParamKind::MeasureRef;
/// A window function's relation (a table expression).
pub(super) const RELATION: ParamKind = ParamKind::Relation;
/// An `ORDERBY(…)` clause.
pub(super) const ORDER_BY: ParamKind = ParamKind::OrderBy;
/// A `PARTITIONBY(…)` clause.
pub(super) const PARTITION_BY: ParamKind = ParamKind::PartitionBy;
/// A `MATCHBY(…)` clause.
pub(super) const MATCH_BY: ParamKind = ParamKind::MatchBy;

// ---- Keyword sets ----

/// Sort order. The engine also accepts the constants `0`/`FALSE` (descending) and `1`/`TRUE`.
pub(super) const ORDER: ParamKind = ParamKind::KeywordOrScalar(&["ASC", "DESC"]);
/// Ranking of ties.
pub(super) const TIES: ParamKind = ParamKind::Keyword(&["SKIP", "DENSE"]);
/// Interval of `DATEADD`, `PARALLELPERIOD`, `DATESINPERIOD` (`WEEK` with a calendar only).
pub(super) const INTERVAL: ParamKind = ParamKind::Keyword(&["DAY", "WEEK", "MONTH", "QUARTER", "YEAR"]);
/// Interval of `PARALLELPERIOD`: the engine refuses `DAY` (M11.40 signature probes).
pub(super) const PERIOD_INTERVAL: ParamKind = ParamKind::Keyword(&["WEEK", "MONTH", "QUARTER", "YEAR"]);
/// Interval of `DATEDIFF`.
pub(super) const DIFF_INTERVAL: ParamKind = ParamKind::Keyword(&[
    "SECOND", "MINUTE", "HOUR", "DAY", "WEEK", "MONTH", "QUARTER", "YEAR",
]);
pub(super) const CROSSFILTER_DIRECTION: ParamKind = ParamKind::Keyword(&[
    "NONE",
    "ONEWAY",
    "BOTH",
    "ONEWAY_LEFTFILTERSRIGHT",
    "ONEWAY_RIGHTFILTERSLEFT",
]);
/// Placement of blanks when sorting (window functions).
pub(super) const BLANKS: ParamKind = ParamKind::Keyword(&["DEFAULT", "FIRST", "LAST"]);
/// `WINDOW` bound type.
pub(super) const BOUND_TYPE: ParamKind = ParamKind::Keyword(&["ABS", "REL"]);
/// Visual-calculation axis.
pub(super) const AXIS: ParamKind = ParamKind::Keyword(&["ROWS", "COLUMNS"]);
/// Data types of `CONVERT` and `DATATABLE`.
pub(super) const DATA_TYPE: ParamKind = ParamKind::Keyword(&[
    "BOOLEAN", "LOGICAL", "CURRENCY", "DECIMAL", "DATETIME", "DOUBLE", "INTEGER", "INT64", "STRING", "TEXT",
]);
/// Data types of `EXTERNALMEASURE` (adds `VARIANT`).
pub(super) const MEASURE_TYPE: ParamKind = ParamKind::Keyword(&[
    "BOOLEAN", "LOGICAL", "CURRENCY", "DECIMAL", "DATETIME", "DOUBLE", "INTEGER", "INT64", "STRING", "TEXT",
    "VARIANT",
]);
/// `DATEADD` extension with a calendar.
pub(super) const EXTENSION: ParamKind = ParamKind::Keyword(&["EXTENDING", "PRECISE", "ENDALIGNED"]);
/// `DATEADD` truncation with a calendar.
pub(super) const TRUNCATION: ParamKind = ParamKind::Keyword(&["BLANKS", "ANCHORED"]);
/// `DATESINPERIOD` end behavior with a calendar.
pub(super) const END_BEHAVIOR: ParamKind = ParamKind::Keyword(&["PRECISE", "ENDALIGNED"]);
/// `TEXTCONTAINS`/`TEXTSIMILARITY` match mode.
pub(super) const MATCH_MODE: ParamKind =
    ParamKind::Keyword(&["TEXTMATCHING", "FUZZYMATCHING", "PHRASEMATCHING"]);
/// `NAMEOF` component.
pub(super) const NAME_COMPONENT: ParamKind =
    ParamKind::Keyword(&["FULL", "SELF", "PARENT", "TABLE", "COLUMN", "MEASURE", "CALENDAR"]);
/// `NAMEOF` escaping.
pub(super) const ESCAPING: ParamKind = ParamKind::Keyword(&["ESCAPED", "UNESCAPED"]);
/// `LOOKUP` associated-columns behavior.
pub(super) const ASSOCIATED_COLUMNS: ParamKind = ParamKind::Keyword(&["EXPLICIT", "INFERRED"]);

// ---- Return types ----

const fn scalar(t: ScalarType) -> Returns {
    Returns::Fixed(ValueType::Scalar(t))
}

pub(super) const R_NUM: Returns = scalar(ScalarType::Decimal);
pub(super) const R_INT: Returns = scalar(ScalarType::Integer);
pub(super) const R_CUR: Returns = scalar(ScalarType::Currency);
pub(super) const R_STR: Returns = scalar(ScalarType::String);
pub(super) const R_BOOL: Returns = scalar(ScalarType::Boolean);
pub(super) const R_DATE: Returns = scalar(ScalarType::DateTime);
pub(super) const R_VARIANT: Returns = scalar(ScalarType::Variant);
pub(super) const R_TABLE: Returns = Returns::Fixed(ValueType::Table);
/// Filter modifiers and clause builders (`USERELATIONSHIP`, `ORDERBY`, `ROLLUP`…): not a value.
pub(super) const R_UNKNOWN: Returns = Returns::Fixed(ValueType::Unknown);

pub(super) const fn same(index: u8) -> Returns {
    Returns::SameAsArg(index)
}

pub(super) const fn common(indices: &'static [u8]) -> Returns {
    Returns::CommonOfArgs(indices)
}
