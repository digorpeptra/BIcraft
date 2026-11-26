//! Date and time functions.

use super::*;

const C: &str = "date and time";

#[rustfmt::skip]
pub(super) const FUNCTIONS: &[Signature] = &[
    f(C, "CALENDAR", &[p("start_date", DATE), p("end_date", DATE)], R_TABLE),
    f(C, "CALENDARAUTO", &[o("fiscal_year_end_month", INT)], R_TABLE),
    f(C, "DATE", &[p("year", INT), p("month", INT), p("day", INT)], R_DATE),
    f(C, "DATEDIFF", &[p("date1", DATE), p("date2", DATE), p("interval", DIFF_INTERVAL)], R_INT),
    f(C, "DATEVALUE", &[p("date_text", STR)], R_DATE),
    f(C, "DAY", &[p("date", DATE)], R_INT),
    f(C, "EDATE", &[p("start_date", DATE), p("months", INT)], R_DATE),
    f(C, "EOMONTH", &[p("start_date", DATE), p("months", INT)], R_DATE),
    f(C, "HOUR", &[p("datetime", DATE)], R_INT),
    f(C, "MINUTE", &[p("datetime", DATE)], R_INT),
    f(C, "MONTH", &[p("date", DATE)], R_INT),
    f(C, "NETWORKDAYS", &[p("start_date", DATE), p("end_date", DATE), o("weekend", INT), o("holidays", TABLE)], R_INT),
    f(C, "NOW", &[], R_DATE),
    f(C, "QUARTER", &[p("date", DATE)], R_INT),
    f(C, "SECOND", &[p("datetime", DATE)], R_INT),
    f(C, "TIME", &[p("hour", INT), p("minute", INT), p("second", INT)], R_DATE),
    f(C, "TIMEVALUE", &[p("time_text", STR)], R_DATE),
    f(C, "TODAY", &[], R_DATE),
    f(C, "UTCNOW", &[], R_DATE),
    f(C, "UTCTODAY", &[], R_DATE),
    f(C, "WEEKDAY", &[p("date", DATE), o("return_type", INT)], R_INT),
    f(C, "WEEKNUM", &[p("date", DATE), o("return_type", INT)], R_INT),
    f(C, "YEAR", &[p("date", DATE)], R_INT),
    f(C, "YEARFRAC", &[p("start_date", DATE), p("end_date", DATE), o("basis", INT)], R_NUM),
];
