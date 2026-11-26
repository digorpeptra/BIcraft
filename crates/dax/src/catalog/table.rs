//! Table manipulation functions.
//!
//! `ADDMISSINGITEMS`, `GROUPBY`, `SUMMARIZE` and `SUMMARIZECOLUMNS` have several repeating
//! sections in the engine; each is flattened into one unit of optional params, in engine order,
//! and arguments are matched to them by shape (column reference, table, name string, expression).

use super::*;

const C: &str = "table manipulation";

#[rustfmt::skip]
pub(super) const FUNCTIONS: &[Signature] = &[
    v(C, "ADDCOLUMNS", &[p("table", TABLE), r("name", NAME), r("expression", ROW)], 1, R_TABLE),
    // ADDMISSINGITEMS([show_all_column…,] table [, group_by_column [, filter_table]]…)
    v(C, "ADDMISSINGITEMS", &[
        section(ro("show_all_column", COL), 1), r("table", TABLE), section(ro("group_by_column", COL), 2),
        section(ro("filter_table", TABLE), 3),
    ], 0, R_TABLE),
    v(C, "CROSSJOIN", &[p("table1", TABLE), r("table", TABLE)], 1, R_TABLE),
    f(C, "CURRENTGROUP", &[], R_TABLE),
    // DATATABLE(name, type [, name, type]…, {{values}…})
    v(C, "DATATABLE", &[r("name", NAME), r("data_type", DATA_TYPE), p("data", TABLE)], 0, R_TABLE),
    f(C, "DETAILROWS", &[p("measure", MEASURE)], R_TABLE),
    // A column reference or a table expression.
    f(C, "DISTINCT", &[p("column_or_table", ParamKind::TableOrColumn)], R_TABLE),
    f(C, "EXCEPT", &[p("left_table", TABLE), p("right_table", TABLE)], R_TABLE),
    f(C, "FILTERS", &[p("column", COL)], R_TABLE),
    f(C, "GENERATE", &[p("table1", TABLE), p("table2", TABLE)], R_TABLE),
    f(C, "GENERATEALL", &[p("table1", TABLE), p("table2", TABLE)], R_TABLE),
    f(C, "GENERATESERIES", &[p("start_value", SCALAR), p("end_value", SCALAR), o("increment_value", SCALAR)], R_TABLE),
    min_args(v(C, "GROUPBY", &[
        p("table", TABLE), section(ro("group_by_column", COL), 1), section(ro("name", NAME), 2),
        section(ro("expression", SCALAR), 2),
    ], 1, R_TABLE), 2),
    // Marks a SUMMARIZECOLUMNS expression as not affecting row removal.
    f(C, "IGNORE", &[p("expression", SCALAR)], same(0)),
    f(C, "INTERSECT", &[p("left_table", TABLE), p("right_table", TABLE)], R_TABLE),
    f(C, "NATURALINNERJOIN", &[p("left_table", TABLE), p("right_table", TABLE)], R_TABLE),
    f(C, "NATURALLEFTOUTERJOIN", &[p("left_table", TABLE), p("right_table", TABLE)], R_TABLE),
    // ROLLUP* are SUMMARIZE/SUMMARIZECOLUMNS group-by modifiers, not values.
    v(C, "ROLLUP", &[r("group_by_column", COL)], 0, R_UNKNOWN),
    v(C, "ROLLUPADDISSUBTOTAL", &[
        o("grand_total_filter", TABLE), r("group_by_column", COL), r("name", NAME),
        ro("group_level_filter", TABLE),
    ], 1, R_UNKNOWN),
    v(C, "ROLLUPGROUP", &[r("group_by_column", COL)], 0, R_UNKNOWN),
    v(C, "ROLLUPISSUBTOTAL", &[
        o("grand_total_filter", TABLE), r("group_by_column", COL), r("is_subtotal_column", COL),
        ro("group_level_filter", TABLE),
    ], 1, R_UNKNOWN),
    v(C, "ROW", &[r("name", NAME), r("expression", SCALAR)], 0, R_TABLE),
    // The name may be omitted when the expression is a column reference.
    v(C, "SELECTCOLUMNS", &[p("table", TABLE), ro("name", NAME), r("expression", ROW)], 1, R_TABLE),
    v(C, "SUBSTITUTEWITHINDEX", &[
        p("table", TABLE), p("index_column_name", NAME), p("index_columns_table", TABLE),
        r("order_by", ROW), ro("order", ORDER),
    ], 3, R_TABLE),
    min_args(v(C, "SUMMARIZE", &[
        p("table", TABLE), section(ro("group_by_column", COL), 1), section(ro("name", NAME), 2),
        section(ro("expression", SCALAR), 2),
    ], 1, R_TABLE), 2),
    min_args(v(C, "SUMMARIZECOLUMNS", &[
        section(ro("group_by_column", COL), 1), section(ro("filter_table", TABLE), 2),
        section(ro("name", NAME), 3), section(ro("expression", SCALAR), 3),
    ], 0, R_TABLE), 1),
    v(C, "TOPN", &[p("n", INT), p("table", TABLE), ro("order_by", ROW), ro("order", ORDER)], 2, R_TABLE),
    v(C, "TREATAS", &[p("table", TABLE), r("column", COL)], 1, R_TABLE),
    v(C, "UNION", &[p("table1", TABLE), r("table", TABLE)], 1, R_TABLE),
    f(C, "VALUES", &[p("table_or_column", TABLE_OR_COL)], R_TABLE),
];
