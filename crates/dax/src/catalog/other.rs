//! Other functions.

use super::*;

const C: &str = "other";

#[rustfmt::skip]
pub(super) const FUNCTIONS: &[Signature] = &[
    f(C, "BLANK", &[], R_VARIANT),
    // Raises an error; never returns.
    f(C, "ERROR", &[p("error_text", STR)], R_VARIANT),
    // Returns its first argument (scalar or table) unchanged.
    // The engine accepts any value as max_rows, a table too (M11.40 signature probes).
    f(C, "EVALUATEANDLOG", &[p("value", ANY), o("label", STR), o("max_rows", ANY)], same(0)),
    f(C, "EXTERNALMEASURE", &[p("measure_name", STR), o("data_type", MEASURE_TYPE), o("connection", STR)], R_VARIANT),
    f(C, "TOCSV", &[p("table", TABLE), o("max_rows", INT), o("delimiter", STR), o("include_headers", BOOL)], R_STR),
    f(C, "TOJSON", &[p("table", TABLE), o("max_rows", INT)], R_STR),
];
