//! Time intelligence functions. `dates` accepts a date column, a single-column table of dates or
//! (calendar-based time intelligence) a calendar reference; the `*WEEK`/`*WTD` functions take a
//! calendar only.

use super::*;

const C: &str = "time intelligence";

const DATES_ONLY: &[Param] = &[p("dates", DATES)];
const DATES_YEAR_END: &[Param] = &[p("dates", DATES), o("year_end_date", STR)];
const CALENDAR_ONLY: &[Param] = &[p("calendar", CALENDAR)];
/// `(expression, dates [, filter])`
const PERIOD: &[Param] = &[p("expression", SCALAR), p("dates", DATES), o("filter", FILTER)];
/// `(expression, dates [, filter [, year_end_date]])`
const PERIOD_YEAR: &[Param] = &[
    p("expression", SCALAR),
    p("dates", DATES),
    o("filter", FILTER),
    // The engine also accepts a table here (M11.40 signature probes); a text year-end date is
    // checked by its own rule (`invalid-year-end-date`).
    o("year_end_date", ANY),
];
/// `(expression, calendar [, filter])`
const PERIOD_WEEK: &[Param] = &[
    p("expression", SCALAR),
    p("calendar", CALENDAR),
    o("filter", FILTER),
];

#[rustfmt::skip]
pub(super) const FUNCTIONS: &[Signature] = &[
    f(C, "CLOSINGBALANCEMONTH", PERIOD, same(0)),
    f(C, "CLOSINGBALANCEQUARTER", PERIOD, same(0)),
    f(C, "CLOSINGBALANCEWEEK", PERIOD_WEEK, same(0)),
    f(C, "CLOSINGBALANCEYEAR", PERIOD_YEAR, same(0)),
    f(C, "DATEADD", &[
        p("dates", DATES), p("number_of_intervals", INT), p("interval", INTERVAL),
        o("extension", EXTENSION), o("truncation", TRUNCATION),
    ], R_TABLE),
    f(C, "DATESBETWEEN", &[p("dates", DATES), p("start_date", DATE), p("end_date", DATE)], R_TABLE),
    f(C, "DATESINPERIOD", &[
        p("dates", DATES), p("start_date", DATE), p("number_of_intervals", INT),
        p("interval", INTERVAL), o("end_behavior", END_BEHAVIOR),
    ], R_TABLE),
    f(C, "DATESMTD", DATES_ONLY, R_TABLE),
    f(C, "DATESQTD", DATES_ONLY, R_TABLE),
    f(C, "DATESWTD", CALENDAR_ONLY, R_TABLE),
    f(C, "DATESYTD", DATES_YEAR_END, R_TABLE),
    f(C, "ENDOFMONTH", DATES_ONLY, R_TABLE),
    f(C, "ENDOFQUARTER", DATES_ONLY, R_TABLE),
    f(C, "ENDOFWEEK", CALENDAR_ONLY, R_TABLE),
    f(C, "ENDOFYEAR", DATES_YEAR_END, R_TABLE),
    f(C, "FIRSTDATE", DATES_ONLY, R_TABLE),
    f(C, "LASTDATE", DATES_ONLY, R_TABLE),
    f(C, "NEXTDAY", DATES_ONLY, R_TABLE),
    f(C, "NEXTMONTH", DATES_ONLY, R_TABLE),
    f(C, "NEXTQUARTER", DATES_ONLY, R_TABLE),
    f(C, "NEXTWEEK", CALENDAR_ONLY, R_TABLE),
    f(C, "NEXTYEAR", DATES_YEAR_END, R_TABLE),
    f(C, "OPENINGBALANCEMONTH", PERIOD, same(0)),
    f(C, "OPENINGBALANCEQUARTER", PERIOD, same(0)),
    f(C, "OPENINGBALANCEWEEK", PERIOD_WEEK, same(0)),
    f(C, "OPENINGBALANCEYEAR", PERIOD_YEAR, same(0)),
    f(C, "PARALLELPERIOD", &[p("dates", DATES), p("number_of_intervals", INT), p("interval", PERIOD_INTERVAL)], R_TABLE),
    f(C, "PREVIOUSDAY", DATES_ONLY, R_TABLE),
    f(C, "PREVIOUSMONTH", DATES_ONLY, R_TABLE),
    f(C, "PREVIOUSQUARTER", DATES_ONLY, R_TABLE),
    f(C, "PREVIOUSWEEK", CALENDAR_ONLY, R_TABLE),
    f(C, "PREVIOUSYEAR", DATES_YEAR_END, R_TABLE),
    f(C, "SAMEPERIODLASTYEAR", DATES_ONLY, R_TABLE),
    f(C, "STARTOFMONTH", DATES_ONLY, R_TABLE),
    f(C, "STARTOFQUARTER", DATES_ONLY, R_TABLE),
    f(C, "STARTOFWEEK", CALENDAR_ONLY, R_TABLE),
    f(C, "STARTOFYEAR", DATES_YEAR_END, R_TABLE),
    f(C, "TOTALMTD", PERIOD, same(0)),
    f(C, "TOTALQTD", PERIOD, same(0)),
    f(C, "TOTALWTD", PERIOD_WEEK, same(0)),
    f(C, "TOTALYTD", PERIOD_YEAR, same(0)),
];
