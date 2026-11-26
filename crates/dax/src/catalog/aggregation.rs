//! Aggregation functions.

use super::*;

const C: &str = "aggregation";

#[rustfmt::skip]
pub(super) const FUNCTIONS: &[Signature] = &[
    f(C, "APPROXIMATEDISTINCTCOUNT", &[p("column", COL)], R_INT),
    f(C, "AVERAGE", &[p("column", COL)], R_NUM),
    f(C, "AVERAGEA", &[p("column", COL)], R_NUM),
    f(C, "AVERAGEX", &[p("table", TABLE), p("expression", ROW)], R_NUM),
    f(C, "COUNT", &[p("column", COL)], R_INT),
    f(C, "COUNTA", &[p("column", COL)], R_INT),
    f(C, "COUNTAX", &[p("table", TABLE), p("expression", ROW)], R_INT),
    f(C, "COUNTBLANK", &[p("column", COL)], R_INT),
    f(C, "COUNTROWS", &[o("table", TABLE)], R_INT),
    f(C, "COUNTX", &[p("table", TABLE), p("expression", ROW)], R_INT),
    f(C, "DISTINCTCOUNT", &[p("column", COL)], R_INT),
    f(C, "DISTINCTCOUNTNOBLANK", &[p("column", COL)], R_INT),
    // One argument: a column. Two arguments: two scalars.
    f(C, "MAX", &[p("column_or_scalar1", ANY), o("scalar2", SCALAR)], common(&[0, 1])),
    f(C, "MAXA", &[p("column", COL)], same(0)),
    f(C, "MAXX", &[p("table", TABLE), p("expression", ROW), o("variant", BOOL)], same(1)),
    f(C, "MIN", &[p("column_or_scalar1", ANY), o("scalar2", SCALAR)], common(&[0, 1])),
    f(C, "MINA", &[p("column", COL)], same(0)),
    f(C, "MINX", &[p("table", TABLE), p("expression", ROW), o("variant", BOOL)], same(1)),
    f(C, "PRODUCT", &[p("column", COL)], R_NUM),
    f(C, "PRODUCTX", &[p("table", TABLE), p("expression", ROW)], R_NUM),
    f(C, "SUM", &[p("column", COL)], same(0)),
    f(C, "SUMX", &[p("table", TABLE), p("expression", ROW)], same(1)),
];
