//! Relationship functions.

use super::*;

const C: &str = "relationship";

#[rustfmt::skip]
pub(super) const FUNCTIONS: &[Signature] = &[
    // Only valid as a CALCULATE filter modifier.
    f(C, "CROSSFILTER", &[p("column1", COL), p("column2", COL), p("direction", CROSSFILTER_DIRECTION)], R_UNKNOWN),
    f(C, "RELATED", &[p("column", COL)], same(0)),
    f(C, "RELATEDTABLE", &[p("table", TABLE_REF)], R_TABLE),
    // Only valid as a CALCULATE filter modifier.
    f(C, "USERELATIONSHIP", &[p("column1", COL), p("column2", COL)], R_UNKNOWN),
];
