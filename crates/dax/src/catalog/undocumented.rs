//! Functions the engine exposes (`$SYSTEM.MDSCHEMA_FUNCTIONS`) that have no page in the public
//! reference. Power BI visuals generate them; signatures follow the engine's parameter list.

use super::*;

const C: &str = "undocumented";

#[rustfmt::skip]
pub(super) const FUNCTIONS: &[Signature] = &[
    f(C, "NATURALJOINUSAGE", &[p("expression", ANY)], R_UNKNOWN),
    // The engine's second repeating section (`dynamic_series`) is kept as a single param.
    v(C, "SAMPLEAXISWITHLOCALMINMAX", &[
        p("size", INT), p("table", TABLE), p("axis", ANY), r("measure", ANY),
        p("min_resolution", INT), o("dynamic_series", ANY), o("dynamic_series_selection_criteria", ANY),
        o("dynamic_series_selection_order", ANY), o("max_resolution", INT),
        o("max_dynamic_series", INT), o("max_iterations", INT),
    ], 3, R_TABLE),
    v(C, "TOPNSKIP", &[
        // skip_rows: the engine evaluates text there too (M11.40 signature probes).
        p("n", INT), p("skip_rows", SCALAR), p("table", TABLE), ro("order_by", ROW), ro("order", ORDER),
        o("ties", ANY),
    ], 3, R_TABLE),
];
