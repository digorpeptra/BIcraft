//! Information functions.

use super::*;

const C: &str = "information";

const VALUE: &[Param] = &[p("value", SCALAR)];

#[rustfmt::skip]
pub(super) const FUNCTIONS: &[Signature] = &[
    f(C, "COLUMNSTATISTICS", &[], R_TABLE),
    v(C, "CONTAINS", &[p("table", TABLE), r("column", COL), r("value", SCALAR)], 1, R_BOOL),
    v(C, "CONTAINSROW", &[p("table", TABLE), r("value", SCALAR)], 1, R_BOOL),
    f(C, "CONTAINSSTRING", &[p("within_text", STR), p("find_text", STR)], R_BOOL),
    f(C, "CONTAINSSTRINGEXACT", &[p("within_text", STR), p("find_text", STR)], R_BOOL),
    f(C, "CUSTOMDATA", &[], R_STR),
    f(C, "HASONEFILTER", &[p("column", COL)], R_BOOL),
    f(C, "HASONEVALUE", &[p("column", COL)], R_BOOL),
    v(C, "ISAFTER", &[r("value1", SCALAR), r("value2", SCALAR), ro("order", ORDER)], 0, R_BOOL),
    f(C, "ISBLANK", VALUE, R_BOOL),
    f(C, "ISBOOLEAN", VALUE, R_BOOL),
    f(C, "ISCROSSFILTERED", &[p("table_or_column", TABLE_OR_COL)], R_BOOL),
    f(C, "ISCURRENCY", VALUE, R_BOOL),
    f(C, "ISDATETIME", VALUE, R_BOOL),
    f(C, "ISDECIMAL", VALUE, R_BOOL),
    f(C, "ISDOUBLE", VALUE, R_BOOL),
    f(C, "ISEMPTY", &[p("table", TABLE)], R_BOOL),
    f(C, "ISERROR", VALUE, R_BOOL),
    f(C, "ISEVEN", &[p("number", NUM)], R_BOOL),
    f(C, "ISFILTERED", &[p("table_or_column", TABLE_OR_COL)], R_BOOL),
    f(C, "ISINSCOPE", &[p("column", COL)], R_BOOL),
    f(C, "ISINT64", VALUE, R_BOOL),
    f(C, "ISINTEGER", VALUE, R_BOOL),
    f(C, "ISLOGICAL", VALUE, R_BOOL),
    f(C, "ISNONTEXT", VALUE, R_BOOL),
    f(C, "ISNUMBER", VALUE, R_BOOL),
    f(C, "ISNUMERIC", VALUE, R_BOOL),
    f(C, "ISODD", &[p("number", NUM)], R_BOOL),
    v(C, "ISONORAFTER", &[r("value1", SCALAR), r("value2", SCALAR), ro("order", ORDER)], 0, R_BOOL),
    v(C, "ISSELECTEDMEASURE", &[r("measure", MEASURE)], 0, R_BOOL),
    f(C, "ISSTRING", VALUE, R_BOOL),
    f(C, "ISSUBTOTAL", &[p("column", COL)], R_BOOL),
    f(C, "ISTEXT", VALUE, R_BOOL),
    // `object`: a table, column, measure or calendar reference.
    f(C, "NAMEOF", &[p("object", ANY), o("component", NAME_COMPONENT), o("escaped", ESCAPING)], R_STR),
    // Wraps a SUMMARIZECOLUMNS filter table.
    f(C, "NONVISUAL", &[p("filter_table", TABLE)], R_TABLE),
    f(C, "SELECTEDMEASURE", &[], R_VARIANT),
    f(C, "SELECTEDMEASUREFORMATSTRING", &[], R_STR),
    f(C, "SELECTEDMEASURENAME", &[], R_STR),
    // `reference`: a table or calendar reference (user-defined function parameters).
    f(C, "TABLEOF", &[p("reference", ANY)], R_TABLE),
    f(C, "TEXTCONTAINS", &[p("within_text_column", COL), p("search_text", STR), o("match_mode", MATCH_MODE)], R_BOOL),
    f(C, "TEXTSIMILARITY", &[p("within_text_column", COL), p("search_text", STR), o("match_mode", MATCH_MODE)], R_NUM),
    f(C, "USERCULTURE", &[], R_STR),
    f(C, "USERNAME", &[], R_STR),
    f(C, "USEROBJECTID", &[], R_STR),
    f(C, "USERPRINCIPALNAME", &[], R_STR),
];
