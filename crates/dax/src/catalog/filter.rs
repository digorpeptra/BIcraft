//! Filter functions, including the window functions (`INDEX`, `OFFSET`, `WINDOW`, `RANK`,
//! `ROWNUMBER` with their `ORDERBY`/`PARTITIONBY`/`MATCHBY` clauses) and the visual-calculation
//! helpers the reference files under filter functions.

use super::*;

const C: &str = "filter";

/// `NONE`, `LOWESTPARENT`, `HIGHESTPARENT`, an integer or a column.
const RESET: ParamKind = ANY;

#[rustfmt::skip]
pub(super) const FUNCTIONS: &[Signature] = &[
    v(C, "ALL", &[o("table_or_column", TABLE_OR_COL), ro("column", COL)], 1, R_TABLE),
    // Only valid as a CALCULATE filter modifier.
    f(C, "ALLCROSSFILTERED", &[p("table", TABLE_REF)], R_UNKNOWN),
    v(C, "ALLEXCEPT", &[p("table", TABLE_REF), r("column", TABLE_OR_COL)], 1, R_TABLE),
    v(C, "ALLNOBLANKROW", &[p("table_or_column", TABLE_OR_COL), ro("column", COL)], 1, R_TABLE),
    v(C, "ALLSELECTED", &[o("table_or_column", TABLE_OR_COL), ro("column", COL)], 1, R_TABLE),
    v(C, "CALCULATE", &[p("expression", SCALAR), ro("filter", FILTER)], 1, same(0)),
    v(C, "CALCULATETABLE", &[p("table", TABLE), ro("filter", FILTER)], 1, R_TABLE),
    f(C, "EARLIER", &[p("column", COL), o("number", INT)], same(0)),
    f(C, "EARLIEST", &[p("column", COL)], same(0)),
    f(C, "FILTER", &[p("table", TABLE), p("filter_expression", ROW)], R_TABLE),
    // A column or a table reference (M11.40 signature probes).
    f(C, "FIRSTNONBLANK", &[p("column", TABLE_OR_COL), p("expression", ROW)], R_TABLE),
    // The expression is evaluated in a filter context per value, with no row context (M11.40).
    f(C, "FIRSTNONBLANKVALUE", &[p("column", COL), p("expression", SCALAR)], same(1)),
    f(C, "INDEX", &[
        p("position", INT), o("relation", RELATION), o("order_by", ORDER_BY), o("blanks", BLANKS),
        o("partition_by", PARTITION_BY), o("match_by", MATCH_BY), o("reset", RESET),
    ], R_TABLE),
    f(C, "KEEPFILTERS", &[p("expression", FILTER)], R_TABLE),
    f(C, "LASTNONBLANK", &[p("column", TABLE_OR_COL), p("expression", ROW)], R_TABLE),
    f(C, "LASTNONBLANKVALUE", &[p("column", COL), p("expression", SCALAR)], same(1)),
    // Visual calculations only. Column/value pairs, then an optional behavior keyword.
    v(C, "LOOKUP", &[
        p("expression", SCALAR), ro("search_column", COL), ro("search_value", SCALAR),
        o("associated_columns_behavior", ASSOCIATED_COLUMNS),
    ], 1, same(0)),
    v(C, "LOOKUPVALUE", &[
        p("result_column", COL), r("search_column", COL), r("search_value", SCALAR),
        o("alternate_result", SCALAR),
    ], 1, common(&[0, 3])),
    // Visual calculations only.
    v(C, "LOOKUPWITHTOTALS", &[p("expression", SCALAR), ro("search_column", COL), ro("search_value", SCALAR)], 1, same(0)),
    v(C, "MATCHBY", &[ro("column", COL)], 0, R_UNKNOWN),
    // Visual calculations only.
    f(C, "MOVINGAVERAGE", &[
        p("column", COL), p("window_size", INT), o("include_current", BOOL), o("axis", AXIS),
        o("order_by", ORDER_BY), o("blanks", BLANKS), o("reset", RESET),
    ], R_NUM),
    f(C, "OFFSET", &[
        p("delta", INT), o("relation", RELATION), o("order_by", ORDER_BY), o("blanks", BLANKS),
        o("partition_by", PARTITION_BY), o("match_by", MATCH_BY), o("reset", RESET),
    ], R_TABLE),
    v(C, "ORDERBY", &[ro("expression", ROW), ro("order", ORDER)], 0, R_UNKNOWN),
    v(C, "PARTITIONBY", &[ro("column", COL)], 0, R_UNKNOWN),
    // Visual calculations only.
    f(C, "RANGE", &[
        p("step", INT), o("include_current", BOOL), o("axis", AXIS), o("order_by", ORDER_BY),
        o("blanks", BLANKS), o("reset", RESET),
    ], R_TABLE),
    f(C, "RANK", &[
        o("ties", TIES), o("relation", RELATION), o("order_by", ORDER_BY), o("blanks", BLANKS),
        o("partition_by", PARTITION_BY), o("match_by", MATCH_BY), o("reset", RESET),
    ], R_INT),
    // Only valid as a CALCULATE filter modifier.
    v(C, "REMOVEFILTERS", &[o("table_or_column", TABLE_OR_COL), ro("column", COL)], 1, R_UNKNOWN),
    f(C, "ROWNUMBER", &[
        o("relation", RELATION), o("order_by", ORDER_BY), o("blanks", BLANKS),
        o("partition_by", PARTITION_BY), o("match_by", MATCH_BY), o("reset", RESET),
    ], R_INT),
    // Visual calculations only.
    f(C, "RUNNINGSUM", &[
        p("column", COL), o("axis", AXIS), o("order_by", ORDER_BY), o("blanks", BLANKS),
        o("reset", RESET),
    ], same(0)),
    f(C, "SELECTEDVALUE", &[p("column", COL), o("alternate_result", SCALAR)], common(&[0, 1])),
    // `from_type`/`to_type` are told apart from `to` by shape (ABS/REL keywords).
    f(C, "WINDOW", &[
        p("from", INT), o("from_type", BOUND_TYPE), p("to", INT), o("to_type", BOUND_TYPE),
        o("relation", RELATION), o("order_by", ORDER_BY), o("blanks", BLANKS),
        o("partition_by", PARTITION_BY), o("match_by", MATCH_BY), o("reset", RESET),
    ], R_TABLE),
];
