//! Text functions.

use super::*;

const C: &str = "text";

const TEXT: &[Param] = &[p("text", STR)];

#[rustfmt::skip]
pub(super) const FUNCTIONS: &[Signature] = &[
    v(C, "COMBINEVALUES", &[p("delimiter", STR), p("expression1", SCALAR), r("expression", SCALAR)], 2, R_STR),
    f(C, "CONCATENATE", &[p("text1", STR), p("text2", STR)], R_STR),
    // CONCATENATEX(table, expression [, delimiter [, order_by [, order]]…])
    v(C, "CONCATENATEX", &[
        p("table", TABLE), p("expression", ROW), o("delimiter", STR), ro("order_by", ROW),
        ro("order", ORDER),
    ], 3, R_STR),
    f(C, "EXACT", &[p("text1", STR), p("text2", STR)], R_BOOL),
    f(C, "FIND", &[p("find_text", STR), p("within_text", STR), o("start_position", INT), o("not_found_value", SCALAR)], R_INT),
    f(C, "FIXED", &[p("number", NUM), o("decimals", INT), o("no_commas", BOOL)], R_STR),
    f(C, "FORMAT", &[p("value", SCALAR), p("format", STR), o("locale_name", STR)], R_STR),
    f(C, "LEFT", &[p("text", STR), o("num_chars", INT)], R_STR),
    f(C, "LEN", TEXT, R_INT),
    f(C, "LOWER", TEXT, R_STR),
    f(C, "MID", &[p("text", STR), p("start_position", INT), p("num_chars", INT)], R_STR),
    f(C, "REPLACE", &[p("old_text", STR), p("start_position", INT), p("num_chars", INT), p("new_text", STR)], R_STR),
    f(C, "REPT", &[p("text", STR), p("number_of_times", INT)], R_STR),
    f(C, "RIGHT", &[p("text", STR), o("num_chars", INT)], R_STR),
    f(C, "SEARCH", &[p("find_text", STR), p("within_text", STR), o("start_position", INT), o("not_found_value", SCALAR)], R_INT),
    f(C, "SUBSTITUTE", &[p("text", STR), p("old_text", STR), p("new_text", STR), o("instance_number", INT)], R_STR),
    f(C, "TRIM", TEXT, R_STR),
    f(C, "UNICHAR", &[p("number", INT)], R_STR),
    f(C, "UNICODE", TEXT, R_INT),
    f(C, "UPPER", TEXT, R_STR),
    f(C, "VALUE", TEXT, R_NUM),
];
