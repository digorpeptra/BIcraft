//! Statistical functions.

use super::*;

const C: &str = "statistical";

const COLUMN: &[Param] = &[p("column", COL)];
const ITERATOR: &[Param] = &[p("table", TABLE), p("expression", ROW)];
const X_DF: &[Param] = &[p("x", NUM), p("deg_freedom", NUM)];
const P_DF: &[Param] = &[p("probability", NUM), p("deg_freedom", NUM)];
const CONFIDENCE: &[Param] = &[p("alpha", NUM), p("standard_dev", NUM), p("size", NUM)];

#[rustfmt::skip]
pub(super) const FUNCTIONS: &[Signature] = &[
    f(C, "BETA.DIST", &[
        p("x", NUM), p("alpha", NUM), p("beta", NUM), p("cumulative", BOOL), o("a", NUM), o("b", NUM),
    ], R_NUM),
    f(C, "BETA.INV", &[p("probability", NUM), p("alpha", NUM), p("beta", NUM), o("a", NUM), o("b", NUM)], R_NUM),
    f(C, "CHISQ.DIST", &[p("x", NUM), p("deg_freedom", NUM), p("cumulative", BOOL)], R_NUM),
    f(C, "CHISQ.DIST.RT", X_DF, R_NUM),
    f(C, "CHISQ.INV", P_DF, R_NUM),
    f(C, "CHISQ.INV.RT", P_DF, R_NUM),
    f(C, "COMBIN", &[p("number", INT), p("number_chosen", INT)], R_INT),
    f(C, "COMBINA", &[p("number", INT), p("number_chosen", INT)], R_INT),
    f(C, "CONFIDENCE.NORM", CONFIDENCE, R_NUM),
    f(C, "CONFIDENCE.T", CONFIDENCE, R_NUM),
    f(C, "EXPON.DIST", &[p("x", NUM), p("lambda", NUM), p("cumulative", BOOL)], R_NUM),
    f(C, "GEOMEAN", COLUMN, R_NUM),
    f(C, "GEOMEANX", ITERATOR, R_NUM),
    // LINEST(column_y, column_x [, column_x]… [, const])
    v(C, "LINEST", &[p("column_y", COL), r("column_x", COL), o("const", BOOL)], 1, R_TABLE),
    v(C, "LINESTX", &[p("table", TABLE), p("expression_y", ROW), r("expression_x", ROW), o("const", BOOL)], 2, R_TABLE),
    f(C, "MEDIAN", COLUMN, R_NUM),
    f(C, "MEDIANX", ITERATOR, R_NUM),
    f(C, "NORM.DIST", &[p("x", NUM), p("mean", NUM), p("standard_dev", NUM), p("cumulative", BOOL)], R_NUM),
    f(C, "NORM.INV", &[p("probability", NUM), p("mean", NUM), p("standard_dev", NUM)], R_NUM),
    f(C, "NORM.S.DIST", &[p("z", NUM), p("cumulative", BOOL)], R_NUM),
    f(C, "NORM.S.INV", &[p("probability", NUM)], R_NUM),
    f(C, "PERCENTILE.EXC", &[p("column", COL), p("k", NUM)], R_NUM),
    f(C, "PERCENTILE.INC", &[p("column", COL), p("k", NUM)], R_NUM),
    f(C, "PERCENTILEX.EXC", &[p("table", TABLE), p("expression", ROW), p("k", NUM)], R_NUM),
    f(C, "PERCENTILEX.INC", &[p("table", TABLE), p("expression", ROW), p("k", NUM)], R_NUM),
    f(C, "PERMUT", &[p("number", INT), p("number_chosen", INT)], R_INT),
    f(C, "POISSON.DIST", &[p("x", NUM), p("mean", NUM), p("cumulative", BOOL)], R_NUM),
    f(C, "RANK.EQ", &[p("value", SCALAR), p("column", COL), o("order", ORDER)], R_INT),
    f(C, "RANKX", &[
        p("table", TABLE), p("expression", ROW), o("value", SCALAR), o("order", ORDER), o("ties", TIES),
    ], R_INT),
    v(C, "SAMPLE", &[p("n", INT), p("table", TABLE), r("order_by", ROW), ro("order", ORDER)], 2, R_TABLE),
    f(C, "SAMPLECARTESIANPOINTSBYCOVER", &[
        p("size", INT), p("table", TABLE), p("x_axis", ANY), p("y_axis", ANY), o("radius", NUM),
        o("max_min_ratio", NUM), o("max_blank_ratio", NUM),
    ], R_TABLE),
    f(C, "STDEV.P", COLUMN, R_NUM),
    f(C, "STDEV.S", COLUMN, R_NUM),
    f(C, "STDEVX.P", ITERATOR, R_NUM),
    f(C, "STDEVX.S", ITERATOR, R_NUM),
    f(C, "T.DIST", &[p("x", NUM), p("deg_freedom", NUM), p("cumulative", BOOL)], R_NUM),
    f(C, "T.DIST.2T", X_DF, R_NUM),
    f(C, "T.DIST.RT", X_DF, R_NUM),
    f(C, "T.INV", P_DF, R_NUM),
    f(C, "T.INV.2T", P_DF, R_NUM),
    f(C, "VAR.P", COLUMN, R_NUM),
    f(C, "VAR.S", COLUMN, R_NUM),
    f(C, "VARX.P", ITERATOR, R_NUM),
    f(C, "VARX.S", ITERATOR, R_NUM),
];
