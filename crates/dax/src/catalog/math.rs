//! Math and trig functions.

use super::*;

const C: &str = "math and trig";

const NUMBER: &[Param] = &[p("number", NUM)];
const ROUNDING: &[Param] = &[p("number", NUM), p("num_digits", INT)];
const SIGNIFICANCE: &[Param] = &[p("number", NUM), p("significance", NUM)];

#[rustfmt::skip]
pub(super) const FUNCTIONS: &[Signature] = &[
    f(C, "ABS", NUMBER, same(0)),
    f(C, "ACOS", NUMBER, R_NUM),
    f(C, "ACOSH", NUMBER, R_NUM),
    f(C, "ACOT", NUMBER, R_NUM),
    f(C, "ACOTH", NUMBER, R_NUM),
    f(C, "ASIN", NUMBER, R_NUM),
    f(C, "ASINH", NUMBER, R_NUM),
    f(C, "ATAN", NUMBER, R_NUM),
    f(C, "ATANH", NUMBER, R_NUM),
    f(C, "CEILING", SIGNIFICANCE, same(0)),
    // The result type is the one named by `data_type`.
    f(C, "CONVERT", &[p("expression", SCALAR), p("data_type", DATA_TYPE)], R_VARIANT),
    f(C, "COS", NUMBER, R_NUM),
    f(C, "COSH", NUMBER, R_NUM),
    f(C, "COT", NUMBER, R_NUM),
    f(C, "COTH", NUMBER, R_NUM),
    f(C, "CURRENCY", &[p("value", SCALAR)], R_CUR),
    f(C, "DEGREES", NUMBER, R_NUM),
    f(C, "DIVIDE", &[p("numerator", NUM), p("denominator", NUM), o("alternate_result", SCALAR)], R_NUM),
    f(C, "EVEN", NUMBER, R_INT),
    f(C, "EXP", NUMBER, R_NUM),
    f(C, "FACT", NUMBER, R_NUM),
    f(C, "FLOOR", SIGNIFICANCE, same(0)),
    f(C, "GCD", &[p("number1", INT), p("number2", INT)], R_INT),
    f(C, "INT", NUMBER, R_INT),
    f(C, "ISO.CEILING", &[p("number", NUM), o("significance", NUM)], same(0)),
    f(C, "LCM", &[p("number1", INT), p("number2", INT)], R_INT),
    f(C, "LN", NUMBER, R_NUM),
    f(C, "LOG", &[p("number", NUM), o("base", NUM)], R_NUM),
    f(C, "LOG10", NUMBER, R_NUM),
    f(C, "MOD", &[p("number", NUM), p("divisor", NUM)], same(0)),
    f(C, "MROUND", &[p("number", NUM), p("multiple", NUM)], same(0)),
    f(C, "ODD", NUMBER, R_INT),
    f(C, "PI", &[], R_NUM),
    f(C, "POWER", &[p("number", NUM), p("power", NUM)], R_NUM),
    f(C, "QUOTIENT", &[p("numerator", NUM), p("denominator", NUM)], R_INT),
    f(C, "RADIANS", NUMBER, R_NUM),
    f(C, "RAND", &[], R_NUM),
    f(C, "RANDBETWEEN", &[p("bottom", INT), p("top", INT)], R_INT),
    f(C, "ROUND", ROUNDING, R_NUM),
    f(C, "ROUNDDOWN", ROUNDING, R_NUM),
    f(C, "ROUNDUP", ROUNDING, R_NUM),
    f(C, "SIGN", NUMBER, R_INT),
    f(C, "SIN", NUMBER, R_NUM),
    f(C, "SINH", NUMBER, R_NUM),
    f(C, "SQRT", NUMBER, R_NUM),
    f(C, "SQRTPI", NUMBER, R_NUM),
    f(C, "TAN", NUMBER, R_NUM),
    f(C, "TANH", NUMBER, R_NUM),
    f(C, "TRUNC", &[p("number", NUM), o("num_digits", INT)], R_NUM),
];
