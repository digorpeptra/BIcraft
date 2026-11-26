//! Visual-calculation functions documented with Power BI visual calculations rather than in the
//! DAX function reference. Valid only in a visual calculation's expression.

use super::*;

const C: &str = "visual calculations";

/// `NONE`, `LOWESTPARENT`, `HIGHESTPARENT`, an integer or a column.
const RESET: ParamKind = ANY;

#[rustfmt::skip]
pub(super) const FUNCTIONS: &[Signature] = &[
    // COLLAPSE(expression [, axis] [, column]… [, n])
    v(C, "COLLAPSE", &[p("expression", SCALAR), o("axis", AXIS), ro("column", COL), o("n", INT)], 2, same(0)),
    // The engine marks `expression` optional and `axis` required.
    f(C, "COLLAPSEALL", &[o("expression", SCALAR), p("axis", AXIS)], same(0)),
    v(C, "EXPAND", &[p("expression", SCALAR), o("axis", AXIS), ro("column", COL), o("n", INT)], 2, same(0)),
    f(C, "EXPANDALL", &[o("expression", SCALAR), p("axis", AXIS)], same(0)),
    f(C, "FIRST", &[
        p("column", COL), o("axis", AXIS), o("order_by", ORDER_BY), o("blanks", BLANKS),
        o("reset", RESET),
    ], same(0)),
    f(C, "ISATLEVEL", &[p("column", COL)], R_BOOL),
    f(C, "LAST", &[
        p("column", COL), o("axis", AXIS), o("order_by", ORDER_BY), o("blanks", BLANKS),
        o("reset", RESET),
    ], same(0)),
    f(C, "NEXT", &[
        p("column", COL), o("steps", INT), o("axis", AXIS), o("order_by", ORDER_BY),
        o("blanks", BLANKS), o("reset", RESET),
    ], same(0)),
    f(C, "PREVIOUS", &[
        p("column", COL), o("steps", INT), o("axis", AXIS), o("order_by", ORDER_BY),
        o("blanks", BLANKS), o("reset", RESET),
    ], same(0)),
];
