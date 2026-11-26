//! The DAX type vocabulary shared by the function catalog (`functions`) and the type checker
//! (`binder`/`typecheck`). Signatures are static data so the catalog can be a plain table.

use serde::Serialize;

/// Scalar value types, as the DAX engine names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ScalarType {
    Integer,
    /// Floating point (`Decimal Number` in the UI, `double` in TMSL).
    Decimal,
    /// Fixed decimal (`Fixed Decimal Number`, `decimal` in TMSL).
    Currency,
    Boolean,
    String,
    DateTime,
    /// Any scalar; the result type depends on runtime values (e.g. `IF` with mixed branches).
    Variant,
}

/// The shape of a value: what an expression evaluates to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(tag = "kind", content = "type", rename_all = "camelCase")]
pub enum ValueType {
    Scalar(ScalarType),
    /// A table expression.
    Table,
    /// Not determinable statically.
    Unknown,
}

/// What a function parameter accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ParamKind {
    /// A scalar value; `None` means any scalar type.
    Scalar(Option<ScalarType>),
    /// A table expression.
    Table,
    /// A table *reference* (a model table or table variable), not an arbitrary table expression.
    TableRef,
    /// A column reference (`T[C]`), e.g. `SUM`, `VALUES`, `RELATED`.
    ColumnRef,
    /// A column or a table reference (e.g. `ALL`, `REMOVEFILTERS`, `COUNTROWS` arg is Table).
    TableOrColumnRef,
    /// A scalar expression evaluated in a row context the function creates (e.g. `SUMX` arg 2).
    RowExpression,
    /// A `CALCULATE` filter argument: boolean expression, table, or filter modifier
    /// (`ALL`, `KEEPFILTERS`, `USERELATIONSHIP`, `CROSSFILTER`, `REMOVEFILTERS`…).
    Filter,
    /// A string literal that names a new column (`ADDCOLUMNS`, `SUMMARIZECOLUMNS`…).
    NewColumnName,
    /// One of a fixed set of bare keywords (e.g. `ASC`/`DESC`, `DAY`/`MONTH`/`YEAR`).
    Keyword(&'static [&'static str]),
    /// Accepted as-is; not checked.
    Any,
    /// A time-intelligence `dates` argument: a date column, a one-column table of dates (with
    /// the lineage of a model column), or a calendar reference.
    Dates,
    /// A calendar reference (calendar-based time intelligence).
    Calendar,
    /// A measure reference (`[Measure]`), e.g. `DETAILROWS`, `ISSELECTEDMEASURE`.
    MeasureRef,
    /// A table expression or a column reference (`DISTINCT`).
    TableOrColumn,
    /// A window function's `relation` (a table expression) or a visual calculation's axis.
    Relation,
    /// An `ORDERBY(…)` clause of a window function.
    OrderBy,
    /// A `PARTITIONBY(…)` clause of a window function.
    PartitionBy,
    /// A `MATCHBY(…)` clause of a window function.
    MatchBy,
    /// One of a fixed set of bare keywords, or a scalar standing for one (sort orders: `ASC`/`DESC`
    /// or `0`/`1`/`FALSE`/`TRUE`).
    KeywordOrScalar(&'static [&'static str]),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Param {
    pub name: &'static str,
    pub kind: ParamKind,
    /// May be omitted (trailing) or left empty (`f(a, , c)`).
    pub optional: bool,
    /// The parameter (and any following ones in the same group) may repeat; see `Signature`.
    pub repeating: bool,
    /// For functions with several repeating sections (`SUMMARIZECOLUMNS`: group-by columns, filter
    /// tables, name/expression pairs), the engine's repeat group of this param (`REPEATGROUP`,
    /// 1-based); params of one group repeat together. 0 for every other param: a signature with a
    /// single repeating unit describes it with `repeating` and `Signature::repeat_from` alone.
    pub group: u8,
}

/// How the return type is determined.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Returns {
    Fixed(ValueType),
    /// Same type as the argument at this index (e.g. `MAX(col)` returns the column's type).
    SameAsArg(u8),
    /// The common (widest) scalar type of the arguments at these indices (e.g. `IF` branches).
    CommonOfArgs(&'static [u8]),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Signature {
    /// Upper-case function name, e.g. `PERCENTILE.INC`.
    pub name: &'static str,
    pub params: &'static [Param],
    /// For variadic functions: the repeating group is `params[repeat_from..]`, repeated as a unit
    /// (e.g. `SWITCH(expr, value, result, …, else)`, `SUMMARIZECOLUMNS(groupBy…, filter…, name, expr…)`).
    pub repeat_from: Option<u8>,
    pub returns: Returns,
    /// Category in the public DAX reference (aggregation, filter, time intelligence…).
    pub category: &'static str,
    /// Deprecated or not recommended (still valid).
    pub deprecated: bool,
    /// The minimum argument count when the params alone do not tell it: the multi-section
    /// functions (`SUMMARIZECOLUMNS`, `SUMMARIZE`, `GROUPBY`) need at least one argument in their
    /// optional sections. `None`: one per required param.
    pub min_args: Option<u8>,
}
