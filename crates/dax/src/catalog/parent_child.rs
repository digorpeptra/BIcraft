//! Parent and child functions.

use super::*;

const C: &str = "parent and child";

#[rustfmt::skip]
pub(super) const FUNCTIONS: &[Signature] = &[
    f(C, "PATH", &[p("id_column", COL), p("parent_column", COL)], R_STR),
    f(C, "PATHCONTAINS", &[p("path", STR), p("item", SCALAR)], R_BOOL),
    // `type`: 0 returns text (the default), 1 an integer.
    f(C, "PATHITEM", &[p("path", STR), p("position", INT), o("type", INT)], R_VARIANT),
    f(C, "PATHITEMREVERSE", &[p("path", STR), p("position", INT), o("type", INT)], R_VARIANT),
    f(C, "PATHLENGTH", &[p("path", STR)], R_INT),
];
