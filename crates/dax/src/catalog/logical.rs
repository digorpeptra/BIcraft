//! Logical functions (the reference also files the `BIT*` functions here).

use super::*;

const C: &str = "logical";

const IF_PARAMS: &[Param] = &[
    p("logical_test", BOOL),
    p("result_if_true", SCALAR),
    o("result_if_false", SCALAR),
];

#[rustfmt::skip]
pub(super) const FUNCTIONS: &[Signature] = &[
    f(C, "AND", &[p("logical1", BOOL), p("logical2", BOOL)], R_BOOL),
    f(C, "BITAND", &[p("number1", INT), p("number2", INT)], R_INT),
    f(C, "BITLSHIFT", &[p("number", INT), p("shift_amount", INT)], R_INT),
    f(C, "BITOR", &[p("number1", INT), p("number2", INT)], R_INT),
    f(C, "BITRSHIFT", &[p("number", INT), p("shift_amount", INT)], R_INT),
    f(C, "BITXOR", &[p("number1", INT), p("number2", INT)], R_INT),
    v(C, "COALESCE", &[p("value1", SCALAR), r("value", SCALAR)], 1, common(&[0, 1])),
    f(C, "FALSE", &[], R_BOOL),
    f(C, "IF", IF_PARAMS, common(&[1, 2])),
    f(C, "IF.EAGER", IF_PARAMS, common(&[1, 2])),
    f(C, "IFERROR", &[p("value", SCALAR), p("value_if_error", SCALAR)], common(&[0, 1])),
    f(C, "NOT", &[p("logical", BOOL)], R_BOOL),
    f(C, "OR", &[p("logical1", BOOL), p("logical2", BOOL)], R_BOOL),
    // SWITCH(expression, value, result [, value, result]… [, else])
    v(C, "SWITCH", &[p("expression", SCALAR), r("value", SCALAR), r("result", SCALAR), o("else", SCALAR)], 1, common(&[2, 3])),
    f(C, "TRUE", &[], R_BOOL),
];
