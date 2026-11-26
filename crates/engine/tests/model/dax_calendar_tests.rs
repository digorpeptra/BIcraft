//! Calendar-based time intelligence beyond the balances (gap G30), run by `dax.evaluate`:
//! `TOTAL*TD`, `DATES*TD`, `STARTOF*`/`ENDOF*`, `FIRSTDATE`/`LASTDATE`, `PREVIOUS*`/`NEXT*`,
//! `PARALLELPERIOD`, `DATEADD` (its `extension` and `truncation`), `SAMEPERIODLASTYEAR`,
//! `DATESINPERIOD` (its `endbehavior`) and `DATESBETWEEN` over a calendar of the model, the
//! clearing of tagged columns, their sort-by columns and time-related columns; then
//! `ROLLUPISSUBTOTAL`'s filter arguments and several `ROLLUPISSUBTOTAL` in one
//! `ADDMISSINGITEMS`, and the `utc_offset_minutes` of `model.refresh`.
//!
//! The expected values follow Microsoft Learn's public documentation (the Power BI page
//! "Implement time-based calculations in Power BI" and the DAX reference pages of the
//! functions; the rules kept are in `docs/formats/dax-functions.md`, "Calendar-based time
//! intelligence"). They are documented, **not oracle-checked**: no engine oracle ran for them.

use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::Session;

fn col(name: &str, ty: &str) -> Value {
    json!({"type": "calculatedTableColumn", "name": name, "dataType": ty,
           "isNameInferred": true, "isDataTypeInferred": true, "sourceColumn": format!("[{name}]")})
}

/// Days since 1970-01-01 of a proleptic Gregorian date.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The date `n` days after 1970-01-01.
fn civil_from_days(n: i64) -> (i64, i64, i64) {
    let z = n + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn group(unit: &str, primary: &str) -> Value {
    json!({"timeUnit": unit, "primaryColumn": primary})
}

/// `Cal` holds every day of 2023 and 2024 (2023-01-01 is a Sunday, 2024 a leap year): `Year`,
/// `Quarter` ("2024-Q2"), `Month` ("2024-05", with `MonthName` "May" sorted by `MonthSort`),
/// `MonthNo`, `Week` (the Monday that starts it, "2024-05-13"), `FYear` (July to June, named by
/// the year it ends), `IsWorking` (Monday to Friday, tagged in no calendar) and `Workday` (the
/// same, time-related in "GregTR"). Calendars: "Greg" (year, quarter, month with MonthName,
/// week, date), "GregTR" (year, date, Workday time-related), "YMQ" (year, month of year,
/// quarter: no date), "Fiscal" (FYear as the year, date). `Biz` holds the weekdays of May 2024
/// (sparse) with the calendar "Business" (year, date). `Stock[Qty]` is 1 on every day of `Cal`,
/// so `[Q]` counts the days in context.
fn session() -> Session {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let iso = |(y, m, d): (i64, i64, i64)| format!("{y:04}-{m:02}-{d:02}");
    let (mut cal_rows, mut stock_rows, mut biz_rows) = (Vec::new(), Vec::new(), Vec::new());
    // days since 1970-01-01 (a Thursday)
    for n in days_from_civil(2023, 1, 1)..=days_from_civil(2024, 12, 31) {
        let (y, m, d) = civil_from_days(n);
        let monday0 = (n + 3).rem_euclid(7); // 0 on Mondays
        let working = monday0 < 5;
        let week = iso(civil_from_days(n - monday0));
        let b = if working { "TRUE" } else { "FALSE" };
        cal_rows.push(format!(
            "{{\"{}\", {y}, \"{y}-Q{}\", \"{y}-{m:02}\", {m}, \"{}\", {m}, \"{week}\", {}, {b}, {b}}}",
            iso((y, m, d)),
            (m - 1) / 3 + 1,
            MONTHS[(m - 1) as usize],
            y + i64::from(m >= 7),
        ));
        stock_rows.push(format!("{{\"{}\", 1}}", iso((y, m, d))));
        if y == 2024 && m == 5 && working {
            biz_rows.push(format!("{{\"{}\", 2024}}", iso((y, m, d))));
        }
    }
    let cal = format!(
        "DATATABLE(\"Date\", DATETIME, \"Year\", INTEGER, \"Quarter\", STRING, \"Month\", STRING, \
         \"MonthNo\", INTEGER, \"MonthName\", STRING, \"MonthSort\", INTEGER, \"Week\", STRING, \
         \"FYear\", INTEGER, \"IsWorking\", BOOLEAN, \"Workday\", BOOLEAN, {{{}}})",
        cal_rows.join(", ")
    );
    let stock = format!(
        "DATATABLE(\"Day\", DATETIME, \"Qty\", INTEGER, {{{}}})",
        stock_rows.join(", ")
    );
    let biz = format!(
        "DATATABLE(\"Date\", DATETIME, \"Year\", INTEGER, {{{}}})",
        biz_rows.join(", ")
    );
    let mut month_name = col("MonthName", "string");
    month_name["sortByColumn"] = json!("MonthSort");
    let model = json!({
        "name": "BicraftCalendars",
        "compatibilityLevel": 1702,
        "model": {
            "culture": "en-US",
            "tables": [
                {
                    "name": "Cal",
                    "columns": [col("Date", "dateTime"), col("Year", "int64"), col("Quarter", "string"),
                                col("Month", "string"), col("MonthNo", "int64"), month_name,
                                col("MonthSort", "int64"), col("Week", "string"), col("FYear", "int64"),
                                col("IsWorking", "boolean"), col("Workday", "boolean")],
                    "partitions": [{"name": "Cal", "source": {"type": "calculated", "expression": cal}}],
                    "calendars": [
                        {"name": "Greg", "calendarColumnGroups": [
                            group("year", "Year"), group("quarter", "Quarter"),
                            {"timeUnit": "month", "primaryColumn": "Month", "associatedColumns": ["MonthName"]},
                            group("week", "Week"), group("date", "Date")]},
                        {"name": "GregTR", "calendarColumnGroups": [
                            group("year", "Year"), group("date", "Date"), {"columns": ["Workday"]}]},
                        {"name": "YMQ", "calendarColumnGroups": [
                            group("year", "Year"), group("monthOfYear", "MonthNo"), group("quarter", "Quarter")]},
                        {"name": "Fiscal", "calendarColumnGroups": [
                            group("year", "FYear"), group("date", "Date")]}
                    ]
                },
                {
                    "name": "Stock",
                    "columns": [col("Day", "dateTime"), col("Qty", "int64")],
                    "partitions": [{"name": "Stock", "source": {"type": "calculated", "expression": stock}}],
                    "measures": [{"name": "Q", "expression": "SUM(Stock[Qty])"}]
                },
                {
                    "name": "Biz",
                    "columns": [col("Date", "dateTime"), col("Year", "int64")],
                    "partitions": [{"name": "Biz", "source": {"type": "calculated", "expression": biz}}],
                    "calendars": [
                        {"name": "Business", "calendarColumnGroups": [group("year", "Year"), group("date", "Date")]}
                    ]
                }
            ],
            "relationships": [
                {"name": "Stock_Cal", "fromTable": "Stock", "fromColumn": "Day", "toTable": "Cal", "toColumn": "Date"}
            ]
        }
    });
    let mut s = Session::new();
    s.model = Some(bicraft_tmsl::read(&model.to_string()).unwrap());
    s
}

fn eval(s: &mut Session, query: &str) -> Value {
    s.execute("dax.evaluate", &json!({ "query": query }))
        .unwrap_or_else(|e| panic!("{query}: {e}"))
}

/// The one value of `EVALUATE ROW("v", expression)`; numbers as doubles.
fn value(s: &mut Session, expression: &str) -> Value {
    let q = format!("EVALUATE ROW(\"v\", {expression})");
    let v = eval(s, &q);
    let x = v["results"][0]["rows"][0]["[v]"].clone();
    match x.as_f64() {
        Some(n) => json!(n),
        None => x,
    }
}

/// `expression` (a date) as `YYYY-MM-DD`, empty when blank.
fn day(s: &mut Session, expression: &str) -> String {
    match value(s, &format!("FORMAT({expression}, \"YYYY-MM-DD\")")) {
        Value::String(t) => t,
        other => panic!("{expression}: {other}"),
    }
}

/// `[Q]` (the days) and the first and last day of `table` used as a filter, under `context`
/// (filter arguments of CALCULATE).
fn span(s: &mut Session, table: &str, context: &str) -> (Value, String, String) {
    let ctx = if context.is_empty() {
        String::new()
    } else {
        format!(", {context}")
    };
    (
        value(s, &format!("CALCULATE(CALCULATE([Q], {table}){ctx})")),
        day(s, &format!("CALCULATE(CALCULATE(MIN(Stock[Day]), {table}){ctx})")),
        day(s, &format!("CALCULATE(CALCULATE(MAX(Stock[Day]), {table}){ctx})")),
    )
}

fn error(s: &mut Session, expression: &str) -> String {
    let q = format!("EVALUATE ROW(\"v\", {expression})");
    match s.execute("dax.evaluate", &json!({ "query": q })) {
        Ok(v) => panic!("{q}: expected an error, got {v}"),
        Err(e) => e,
    }
}

const MAY15: &str = "Cal[Date] = DATE(2024, 5, 15)";

// --- TOTAL*TD and DATES*TD -------------------------------------------------------------------

#[test]
fn totals_to_date_over_a_calendar() {
    let mut s = session();
    let on = |f: &str| format!("CALCULATE({f}, {MAY15})");
    // 2024-05-15 is a Wednesday: the week started on Monday the 13th
    assert_eq!(value(&mut s, &on("TOTALYTD([Q], Greg)")), json!(136.0));
    assert_eq!(value(&mut s, &on("TOTALQTD([Q], Greg)")), json!(45.0));
    assert_eq!(value(&mut s, &on("TOTALMTD([Q], Greg)")), json!(15.0));
    assert_eq!(value(&mut s, &on("TOTALWTD([Q], Greg)")), json!(3.0));
    // the fiscal year starts on July 1: 184 days of 2023 and 136 of 2024
    assert_eq!(value(&mut s, &on("TOTALYTD([Q], Fiscal)")), json!(320.0));
    // the filter argument applies after the dates
    assert_eq!(
        value(&mut s, &on("TOTALYTD([Q], Greg, Cal[MonthNo] = 2)")),
        json!(29.0)
    );
    // a month in context: to its last day
    assert_eq!(
        value(&mut s, "CALCULATE(TOTALYTD([Q], Greg), Cal[Month] = \"2024-03\")"),
        json!(91.0)
    );
    // in a row context: context transition first
    assert_eq!(
        value(
            &mut s,
            "SUMX(FILTER(VALUES(Cal[Month]), Cal[Month] IN {\"2024-01\", \"2024-02\"}), TOTALYTD([Q], Greg))"
        ),
        json!(31.0 + 60.0)
    );
}

#[test]
fn dates_to_date_over_a_calendar() {
    let mut s = session();
    for (f, days, first) in [
        ("DATESYTD(Greg)", 136.0, "2024-01-01"),
        ("DATESQTD(Greg)", 45.0, "2024-04-01"),
        ("DATESMTD(Greg)", 15.0, "2024-05-01"),
        ("DATESWTD(Greg)", 3.0, "2024-05-13"),
        ("DATESYTD(Fiscal)", 320.0, "2023-07-01"),
    ] {
        assert_eq!(
            span(&mut s, f, MAY15),
            (json!(days), first.to_string(), "2024-05-15".to_string()),
            "{f}"
        );
    }
    // the table holds the primary columns of the selected rows: one row per day here
    assert_eq!(
        value(&mut s, &format!("CALCULATE(COUNTROWS(DATESYTD(Greg)), {MAY15})")),
        json!(136.0)
    );
    // a calendar without a Date category: positions are its months (Year, Month of Year)
    assert_eq!(
        value(
            &mut s,
            "CALCULATE(COUNTROWS(DATESYTD(YMQ)), Cal[Year] = 2024, Cal[MonthNo] = 5)"
        ),
        json!(5.0)
    );
    assert_eq!(
        value(
            &mut s,
            "CALCULATE(CALCULATE([Q], DATESQTD(YMQ)), Cal[Year] = 2024, Cal[MonthNo] = 5)"
        ),
        json!(61.0)
    );
}

// --- STARTOF*/ENDOF*, FIRSTDATE/LASTDATE --------------------------------------------------------

#[test]
fn start_and_end_of_periods_over_a_calendar() {
    let mut s = session();
    for (f, want) in [
        ("STARTOFWEEK", "2024-05-13"),
        ("ENDOFWEEK", "2024-05-19"),
        ("STARTOFMONTH", "2024-05-01"),
        ("ENDOFMONTH", "2024-05-31"),
        ("STARTOFQUARTER", "2024-04-01"),
        ("ENDOFQUARTER", "2024-06-30"),
        ("STARTOFYEAR", "2024-01-01"),
        ("ENDOFYEAR", "2024-12-31"),
    ] {
        assert_eq!(
            day(&mut s, &format!("CALCULATE(MINX({f}(Greg), Cal[Date]), {MAY15})")),
            want,
            "{f}"
        );
        // one day, as a filter
        assert_eq!(
            value(&mut s, &format!("CALCULATE(CALCULATE([Q], {f}(Greg)), {MAY15})")),
            json!(1.0),
            "{f}"
        );
    }
    // the first week of the table holds one day (2023-01-01, a Sunday): its first day is it
    assert_eq!(
        day(
            &mut s,
            "CALCULATE(MINX(STARTOFWEEK(Greg), Cal[Date]), Cal[Date] = DATE(2023, 1, 1))"
        ),
        "2023-01-01"
    );
    // the fiscal year ends on June 30
    assert_eq!(
        day(
            &mut s,
            &format!("CALCULATE(MINX(ENDOFYEAR(Fiscal), Cal[Date]), {MAY15})")
        ),
        "2024-06-30"
    );
    // a context of several days: the first one's period for STARTOF, the last one's for ENDOF
    let feb_mar = "DATESBETWEEN(Cal[Date], DATE(2024, 2, 20), DATE(2024, 3, 5))";
    assert_eq!(
        day(
            &mut s,
            &format!("CALCULATE(MINX(STARTOFMONTH(Greg), Cal[Date]), {feb_mar})")
        ),
        "2024-02-01"
    );
    assert_eq!(
        day(
            &mut s,
            &format!("CALCULATE(MINX(ENDOFMONTH(Greg), Cal[Date]), {feb_mar})")
        ),
        "2024-03-31"
    );
    assert_eq!(
        day(
            &mut s,
            &format!("CALCULATE(MINX(FIRSTDATE(Greg), Cal[Date]), {feb_mar})")
        ),
        "2024-02-20"
    );
    assert_eq!(
        day(
            &mut s,
            &format!("CALCULATE(MINX(LASTDATE(Greg), Cal[Date]), {feb_mar})")
        ),
        "2024-03-05"
    );
    // nothing in context: an empty table
    assert_eq!(
        value(
            &mut s,
            "CALCULATE(COUNTROWS(STARTOFMONTH(Greg)), Cal[Year] = 2030)"
        ),
        Value::Null
    );
}

// --- PREVIOUS*/NEXT* ---------------------------------------------------------------------------

#[test]
fn previous_and_next_periods_over_a_calendar() {
    let mut s = session();
    for (f, days, first, last) in [
        ("PREVIOUSWEEK(Greg)", 7.0, "2024-05-06", "2024-05-12"),
        ("NEXTWEEK(Greg)", 7.0, "2024-05-20", "2024-05-26"),
        ("PREVIOUSMONTH(Greg)", 30.0, "2024-04-01", "2024-04-30"),
        ("NEXTMONTH(Greg)", 30.0, "2024-06-01", "2024-06-30"),
        ("PREVIOUSQUARTER(Greg)", 91.0, "2024-01-01", "2024-03-31"),
        ("NEXTQUARTER(Greg)", 92.0, "2024-07-01", "2024-09-30"),
        ("PREVIOUSYEAR(Greg)", 365.0, "2023-01-01", "2023-12-31"),
        ("PREVIOUSDAY(Greg)", 1.0, "2024-05-14", "2024-05-14"),
        ("NEXTDAY(Greg)", 1.0, "2024-05-16", "2024-05-16"),
        ("PREVIOUSYEAR(Fiscal)", 181.0, "2023-01-01", "2023-06-30"),
    ] {
        assert_eq!(
            span(&mut s, f, MAY15),
            (json!(days), first.to_string(), last.to_string()),
            "{f}"
        );
    }
    // the calendar has no year after 2024: an empty table
    assert_eq!(
        value(
            &mut s,
            &format!("CALCULATE(CALCULATE([Q], NEXTYEAR(Greg)), {MAY15})")
        ),
        Value::Null
    );
    // PREVIOUS* from the first day in context, NEXT* from the last
    let ctx = "DATESBETWEEN(Cal[Date], DATE(2024, 2, 20), DATE(2024, 3, 5))";
    assert_eq!(
        span(&mut s, "PREVIOUSMONTH(Greg)", ctx).1,
        "2024-01-01".to_string()
    );
    assert_eq!(span(&mut s, "NEXTMONTH(Greg)", ctx).1, "2024-04-01".to_string());
    // sparse dates: the day before a Monday is the Friday the calendar's table has
    assert_eq!(
        day(
            &mut s,
            "CALCULATE(CALCULATE(MAX(Biz[Date]), PREVIOUSDAY(Business)), Biz[Date] = DATE(2024, 5, 13))"
        ),
        "2024-05-10"
    );
    assert_eq!(
        day(
            &mut s,
            "CALCULATE(CALCULATE(MAX(Biz[Date]), NEXTDAY(Business)), Biz[Date] = DATE(2024, 5, 10))"
        ),
        "2024-05-13"
    );
}

// --- PARALLELPERIOD ----------------------------------------------------------------------------

#[test]
fn parallel_period_over_a_calendar() {
    let mut s = session();
    for (f, days, first, last) in [
        (
            "PARALLELPERIOD(Greg, -1, YEAR)",
            365.0,
            "2023-01-01",
            "2023-12-31",
        ),
        (
            "PARALLELPERIOD(Greg, -1, MONTH)",
            30.0,
            "2024-04-01",
            "2024-04-30",
        ),
        (
            "PARALLELPERIOD(Greg, 1, QUARTER)",
            92.0,
            "2024-07-01",
            "2024-09-30",
        ),
        ("PARALLELPERIOD(Greg, -1, WEEK)", 7.0, "2024-05-06", "2024-05-12"),
        (
            "PARALLELPERIOD(Fiscal, -1, YEAR)",
            181.0,
            "2023-01-01",
            "2023-06-30",
        ),
    ] {
        assert_eq!(
            span(&mut s, f, MAY15),
            (json!(days), first.to_string(), last.to_string()),
            "{f}"
        );
    }
    // the first and the last day's periods, and every period between
    let ctx = "DATESBETWEEN(Cal[Date], DATE(2024, 2, 20), DATE(2024, 3, 5))";
    assert_eq!(
        span(&mut s, "PARALLELPERIOD(Greg, 2, MONTH)", ctx),
        (json!(61.0), "2024-04-01".to_string(), "2024-05-31".to_string())
    );
    // beyond the calendar: clamped to the periods it has, empty when none
    assert_eq!(
        value(
            &mut s,
            &format!("CALCULATE(CALCULATE([Q], PARALLELPERIOD(Greg, 5, YEAR)), {MAY15})")
        ),
        Value::Null
    );
}

// --- DATEADD and SAMEPERIODLASTYEAR ------------------------------------------------------------

#[test]
fn dateadd_over_a_calendar_keeps_positions_within_the_period() {
    let mut s = session();
    let mar3_10 = "DATESBETWEEN(Cal[Date], DATE(2024, 3, 3), DATE(2024, 3, 10))";
    assert_eq!(
        span(&mut s, "DATEADD(Greg, 1, MONTH)", mar3_10),
        (json!(8.0), "2024-04-03".to_string(), "2024-04-10".to_string())
    );
    // a whole period moves to the whole target period
    assert_eq!(
        span(&mut s, "DATEADD(Greg, 1, MONTH)", "Cal[Month] = \"2024-02\""),
        (json!(31.0), "2024-03-01".to_string(), "2024-03-31".to_string())
    );
    assert_eq!(
        span(&mut s, "DATEADD(Greg, -1, MONTH)", "Cal[Month] = \"2024-03\""),
        (json!(29.0), "2024-02-01".to_string(), "2024-02-29".to_string())
    );
    // a week, a day, a quarter
    assert_eq!(
        span(&mut s, "DATEADD(Greg, -1, WEEK)", MAY15).1,
        "2024-05-08".to_string()
    );
    assert_eq!(
        span(&mut s, "DATEADD(Greg, -3, DAY)", MAY15).1,
        "2024-05-12".to_string()
    );
    // May 15 is the 45th day of its quarter; the 45th day of the next is August 14
    assert_eq!(
        span(&mut s, "DATEADD(Greg, 1, QUARTER)", MAY15).1,
        "2024-08-14".to_string()
    );
    // whole months keep their index among the months: February a year back is February
    assert_eq!(
        span(&mut s, "DATEADD(Greg, -1, YEAR)", "Cal[Month] = \"2024-02\"").0,
        json!(28.0)
    );
    // whole weeks keep theirs among the weeks starting in the month (May 13–19 is the second
    // starting in May, after May 6; June's second is June 10–16)
    assert_eq!(
        span(&mut s, "DATEADD(Greg, 1, MONTH)", "Cal[Week] = \"2024-05-13\""),
        (json!(7.0), "2024-06-10".to_string(), "2024-06-16".to_string())
    );
    // sparse dates: two days back from Tuesday the 14th is Friday the 10th
    assert_eq!(
        day(
            &mut s,
            "CALCULATE(CALCULATE(MAX(Biz[Date]), DATEADD(Business, -2, DAY)), Biz[Date] = DATE(2024, 5, 14))"
        ),
        "2024-05-10"
    );
    // beyond the calendar: empty
    assert_eq!(
        value(
            &mut s,
            &format!("CALCULATE(CALCULATE([Q], DATEADD(Greg, 3, YEAR)), {MAY15})")
        ),
        Value::Null
    );
}

#[test]
fn dateadd_extension_and_truncation() {
    let mut s = session();
    // February 25–28, 2023 one month on (February has 28 days that year)
    let feb = "DATESBETWEEN(Cal[Date], DATE(2023, 2, 25), DATE(2023, 2, 28))";
    for (ext, days, first, last) in [
        ("", 7.0, "2023-03-25", "2023-03-31"),
        (", EXTENDING", 7.0, "2023-03-25", "2023-03-31"),
        (", PRECISE", 4.0, "2023-03-25", "2023-03-28"),
        (", ENDALIGNED", 4.0, "2023-03-28", "2023-03-31"),
    ] {
        assert_eq!(
            span(&mut s, &format!("DATEADD(Greg, 1, MONTH{ext})"), feb),
            (json!(days), first.to_string(), last.to_string()),
            "{ext}"
        );
    }
    // the reference's EndAligned examples: Feb 28 → March 31, Feb 27 → March 27
    let one = |d: u32| format!("Cal[Date] = DATE(2023, 2, {d})");
    assert_eq!(
        span(&mut s, "DATEADD(Greg, 1, MONTH, ENDALIGNED)", &one(28)).1,
        "2023-03-31".to_string()
    );
    assert_eq!(
        span(&mut s, "DATEADD(Greg, 1, MONTH, ENDALIGNED)", &one(27)).1,
        "2023-03-27".to_string()
    );
    // March 31 one month back: blank by default (BLANKS), February's last day when ANCHORED
    let mar31 = "Cal[Date] = DATE(2023, 3, 31)";
    for trunc in ["", ", EXTENDING, BLANKS"] {
        assert_eq!(
            value(
                &mut s,
                &format!("CALCULATE(CALCULATE([Q], DATEADD(Greg, -1, MONTH{trunc})), {mar31})")
            ),
            Value::Null,
            "{trunc}"
        );
    }
    assert_eq!(
        span(&mut s, "DATEADD(Greg, -1, MONTH, EXTENDING, ANCHORED)", mar31).1,
        "2023-02-28".to_string()
    );
    // a bad keyword is an error
    let e = error(&mut s, "CALCULATE([Q], DATEADD(Greg, 1, MONTH, SIDEWAYS))");
    assert!(e.contains("extension") || e.contains("SIDEWAYS"), "{e}");
}

#[test]
fn sameperiodlastyear_over_a_calendar() {
    let mut s = session();
    // the 60th day of 2024 (February 29) is March 1 in 2023, as the reference says
    assert_eq!(
        span(
            &mut s,
            "SAMEPERIODLASTYEAR(Greg)",
            "Cal[Date] = DATE(2024, 2, 29)"
        )
        .1,
        "2023-03-01".to_string()
    );
    assert_eq!(
        span(&mut s, "SAMEPERIODLASTYEAR(Greg)", "Cal[Month] = \"2024-02\""),
        (json!(28.0), "2023-02-01".to_string(), "2023-02-28".to_string())
    );
    assert_eq!(
        span(&mut s, "SAMEPERIODLASTYEAR(Greg)", "Cal[Year] = 2024").0,
        json!(365.0)
    );
}

#[test]
fn dateadd_by_quarter_on_a_month_grain_calendar() {
    // the page's context-clearing example: Year and Month of Year browsed, a quarter back
    let mut s = session();
    let q = |m: u32| {
        format!("CALCULATE(CALCULATE([Q], DATEADD(YMQ, -1, QUARTER)), Cal[Year] = 2024, Cal[MonthNo] = {m})")
    };
    // April 2024 → January 2024, May → February (29 days)
    assert_eq!(value(&mut s, &q(4)), json!(31.0));
    assert_eq!(value(&mut s, &q(5)), json!(29.0));
    // January 2024 → October 2023
    assert_eq!(value(&mut s, &q(1)), json!(31.0));
}

// --- DATESINPERIOD and DATESBETWEEN --------------------------------------------------------------

#[test]
fn datesinperiod_over_a_calendar() {
    let mut s = session();
    let dip = |args: &str| format!("DATESINPERIOD(Greg, {args})");
    // February 29, 2024 six months back: PRECISE ends on August 29 (excluded), ENDALIGNED on
    // August 31 (the reference's table, a year later)
    assert_eq!(
        span(&mut s, &dip("DATE(2024, 2, 29), -6, MONTH"), ""),
        (json!(184.0), "2023-08-30".to_string(), "2024-02-29".to_string())
    );
    assert_eq!(
        span(&mut s, &dip("DATE(2024, 2, 29), -6, MONTH, PRECISE"), "").0,
        json!(184.0)
    );
    assert_eq!(
        span(&mut s, &dip("DATE(2024, 2, 29), -6, MONTH, ENDALIGNED"), ""),
        (json!(182.0), "2023-09-01".to_string(), "2024-02-29".to_string())
    );
    // forward: [start, boundary)
    assert_eq!(
        span(&mut s, &dip("DATE(2024, 1, 10), 7, DAY"), ""),
        (json!(7.0), "2024-01-10".to_string(), "2024-01-16".to_string())
    );
    // a year back from June 30, 2024 (the 182nd day): July 1, 2023 is excluded
    assert_eq!(
        span(&mut s, &dip("DATE(2024, 6, 30), -1, YEAR"), ""),
        (json!(365.0), "2023-07-02".to_string(), "2024-06-30".to_string())
    );
    // the reference's fiscal example: the last day of a 366-day fiscal year has no match in the
    // previous year (its first half only in the table): anchored on that year's last day
    assert_eq!(
        span(&mut s, "DATESINPERIOD(Fiscal, DATE(2024, 6, 30), -1, YEAR)", ""),
        (json!(366.0), "2023-07-01".to_string(), "2024-06-30".to_string())
    );
    // with MAX of the context, as the reference's measure
    assert_eq!(
        value(
            &mut s,
            "CALCULATE(CALCULATE([Q], DATESINPERIOD(Greg, MAX(Cal[Date]), -7, DAY)), Cal[Month] = \"2024-03\")"
        ),
        json!(7.0)
    );
    // a start the calendar does not have, or no interval: empty
    for args in ["DATE(2030, 1, 1), -1, YEAR", "DATE(2024, 1, 10), 0, DAY"] {
        assert_eq!(
            value(&mut s, &format!("CALCULATE([Q], {})", dip(args))),
            Value::Null,
            "{args}"
        );
    }
}

#[test]
fn datesbetween_over_a_calendar() {
    let mut s = session();
    assert_eq!(
        span(
            &mut s,
            "DATESBETWEEN(Greg, DATE(2024, 2, 1), DATE(2024, 2, 10))",
            ""
        ),
        (json!(10.0), "2024-02-01".to_string(), "2024-02-10".to_string())
    );
    // a blank start is the calendar's first day; a day it does not have is treated as blank
    for start in ["BLANK()", "DATE(2020, 1, 1)"] {
        assert_eq!(
            span(
                &mut s,
                &format!("DATESBETWEEN(Greg, {start}, DATE(2024, 2, 10))"),
                ""
            ),
            (json!(406.0), "2023-01-01".to_string(), "2024-02-10".to_string()),
            "{start}"
        );
    }
    assert_eq!(
        span(&mut s, "DATESBETWEEN(Greg, DATE(2024, 12, 1), BLANK())", "").0,
        json!(31.0)
    );
}

// --- context clearing --------------------------------------------------------------------------

#[test]
fn calendar_functions_clear_tagged_and_sort_by_columns() {
    let mut s = session();
    // MonthSort is tagged in no calendar, but it is the sort-by column of MonthName (associated
    // with Month in "Greg"): its filter is cleared as the tagged columns' are
    assert_eq!(
        value(
            &mut s,
            "CALCULATE(TOTALYTD([Q], Greg), Cal[Year] = 2024, Cal[MonthSort] = 3)"
        ),
        json!(91.0)
    );
    // the balances too: the year's last day counts although it is in December
    assert_eq!(
        value(
            &mut s,
            "CALCULATE(CLOSINGBALANCEYEAR([Q], Greg), Cal[Year] = 2024, Cal[MonthSort] = 3)"
        ),
        json!(1.0)
    );
    // a column tagged in another calendar of the table (MonthNo, in "YMQ") is cleared as well
    assert_eq!(
        value(
            &mut s,
            "CALCULATE(TOTALYTD([Q], Greg), Cal[Year] = 2024, Cal[MonthNo] = 3)"
        ),
        json!(91.0)
    );
    // IsWorking is tagged in no calendar: kept (98 weekdays from January 1 to May 15, 2024)
    assert_eq!(
        value(
            &mut s,
            &format!("CALCULATE(TOTALYTD([Q], Greg), {MAY15}, Cal[IsWorking] = TRUE())")
        ),
        json!(98.0)
    );
    // a filter on the fact table stays
    assert_eq!(
        value(
            &mut s,
            &format!("CALCULATE(TOTALYTD([Q], Greg), {MAY15}, Stock[Day] <= DATE(2024, 1, 31))")
        ),
        json!(31.0)
    );
}

#[test]
fn time_related_columns_are_cleared_except_by_the_lateral_shifts() {
    let mut s = session();
    let ctx = "Cal[Year] = 2024, Cal[Workday] = TRUE()";
    // hierarchical (PARALLELPERIOD): Workday, time-related, is cleared: the whole of 2023
    assert_eq!(
        value(
            &mut s,
            &format!("CALCULATE(CALCULATE([Q], PARALLELPERIOD(GregTR, -1, YEAR)), {ctx})")
        ),
        json!(365.0)
    );
    // lateral (DATEADD, SAMEPERIODLASTYEAR): kept, the working days of 2023 (260)
    for f in ["DATEADD(GregTR, -1, YEAR)", "SAMEPERIODLASTYEAR(GregTR)"] {
        assert_eq!(
            value(&mut s, &format!("CALCULATE(CALCULATE([Q], {f}), {ctx})")),
            json!(260.0),
            "{f}"
        );
    }
    // an untagged column (IsWorking) is kept by both, as the page's example shows
    let ctx = "Cal[Year] = 2024, Cal[IsWorking] = TRUE()";
    for f in ["PARALLELPERIOD(Greg, -1, YEAR)", "DATEADD(Greg, -1, YEAR)"] {
        assert_eq!(
            value(&mut s, &format!("CALCULATE(CALCULATE([Q], {f}), {ctx})")),
            json!(260.0),
            "{f}"
        );
    }
    // the table of a flexible function holds the time-related columns, the lateral one's not
    assert_eq!(
        value(
            &mut s,
            &format!(
                "CALCULATE(COUNTROWS(DISTINCT(SELECTCOLUMNS(PARALLELPERIOD(GregTR, -1, YEAR), \"w\", Cal[Workday]))), {ctx})"
            )
        ),
        json!(2.0)
    );
}

#[test]
fn calendar_function_errors() {
    let mut s = session();
    // a calendar without the categories of the period
    let e = error(&mut s, "TOTALWTD([Q], YMQ)");
    assert!(e.contains("week"), "{e}");
    let e = error(&mut s, "CALCULATE([Q], DATEADD(Fiscal, -1, MONTH))");
    assert!(e.contains("month"), "{e}");
    // a year-end date only with a date column
    let e = error(&mut s, "COUNTROWS(DATESYTD(Greg, \"6-30\"))");
    assert!(e.contains("year-end"), "{e}");
    // week functions need a calendar
    for f in ["TOTALWTD([Q], Cal[Date])", "COUNTROWS(STARTOFWEEK(Cal[Date]))"] {
        let e = error(&mut s, f);
        assert!(e.contains("calendar"), "{f}: {e}");
    }
    // DATESINPERIOD and DATESBETWEEN need a Date category
    let e = error(&mut s, "COUNTROWS(DATESBETWEEN(YMQ, BLANK(), BLANK()))");
    assert!(e.contains("Date category"), "{e}");
}

/// Calendar functions cannot be nested (the page's considerations: `PREVIOUSDAY(PREVIOUSMONTH(
/// 'Calendar'))` is not supported): an error that names the nesting, as filter argument, as table
/// and in `TOTAL*TD` and the balances; the page's alternative, the inner function as a
/// `CALCULATETABLE` filter, works.
#[test]
fn nested_calendar_functions_are_an_error() {
    let mut s = session();
    for f in [
        "CALCULATE([Q], PREVIOUSDAY(PREVIOUSMONTH(Greg)))",
        "COUNTROWS(PREVIOUSDAY(PREVIOUSMONTH(Greg)))",
        "COUNTROWS(SAMEPERIODLASTYEAR(DATESYTD(Greg)))",
        "COUNTROWS(DATESBETWEEN(NEXTYEAR(Greg), BLANK(), BLANK()))",
        "TOTALYTD([Q], DATEADD(Greg, -1, YEAR))",
        "CLOSINGBALANCEMONTH([Q], PREVIOUSYEAR(Greg))",
    ] {
        let e = error(&mut s, &format!("CALCULATE({f}, {MAY15})"));
        assert!(e.contains("nested"), "{f}: {e}");
    }
    // the page's alternative: the day before the previous month's first day
    let (days, first, last) = span(
        &mut s,
        "CALCULATETABLE(PREVIOUSDAY(Greg), PREVIOUSMONTH(Greg))",
        MAY15,
    );
    assert_eq!(
        (days, first.as_str(), last.as_str()),
        (json!(1.0), "2024-03-31", "2024-03-31")
    );
}

// --- ROLLUPISSUBTOTAL --------------------------------------------------------------------------

/// The small model of `tests/window_functions` (Sales by region and day, `[Total]`).
fn window_session() -> Session {
    let mut s = Session::new();
    s.model = Some(bicraft_tmsl::read(include_str!("../window_functions/model.bim")).unwrap());
    s
}

fn rows(s: &mut Session, query: &str) -> Vec<Value> {
    let v = eval(s, query);
    v["results"][0]["rows"]
        .as_array()
        .unwrap_or_else(|| panic!("{query}: no rows in {v}"))
        .clone()
}

#[test]
fn rollupissubtotal_filters_apply_to_their_level() {
    let mut s = window_session();
    let sc = "SUMMARIZECOLUMNS(ROLLUPADDISSUBTOTAL(Region[Region], \"IsTotal\"), \"T\", [Total])";
    let base = rows(
        &mut s,
        &format!(
            "EVALUATE ADDMISSINGITEMS(Region[Region], {sc}, ROLLUPISSUBTOTAL(Region[Region], [IsTotal]))"
        ),
    );
    // a level filter after the group: the missing regions added at that level are only those
    // the filter keeps (North has no sales)
    let kept = rows(
        &mut s,
        &format!(
            "EVALUATE ADDMISSINGITEMS(Region[Region], {sc}, ROLLUPISSUBTOTAL(Region[Region], [IsTotal], \
             TREATAS({{\"East\"}}, Region[Region])))"
        ),
    );
    let regions = |rs: &[Value]| -> Vec<String> {
        let mut v: Vec<String> = rs
            .iter()
            .filter(|r| r["[IsTotal]"] == json!(false))
            .map(|r| r["Region[Region]"].as_str().unwrap_or("").to_string())
            .collect();
        v.sort();
        v
    };
    assert!(regions(&base).contains(&"North".to_string()), "{base:?}");
    assert!(!regions(&kept).contains(&"North".to_string()), "{kept:?}");
    // a grand total filter (before the group) applies to the rolled-up level only: the detail
    // rows are completed as without it
    let grand = rows(
        &mut s,
        &format!(
            "EVALUATE ADDMISSINGITEMS(Region[Region], {sc}, ROLLUPISSUBTOTAL(TREATAS({{\"East\"}}, Region[Region]), \
             Region[Region], [IsTotal]))"
        ),
    );
    assert_eq!(regions(&grand), regions(&base));
    assert_eq!(grand.len(), base.len());
    // two filters for one level are an error
    let e = s
        .execute(
            "dax.evaluate",
            &json!({"query": format!(
                "EVALUATE ADDMISSINGITEMS(Region[Region], {sc}, ROLLUPISSUBTOTAL(Region[Region], [IsTotal], \
                 TREATAS({{\"East\"}}, Region[Region]), TREATAS({{\"West\"}}, Region[Region])))"
            )}),
        )
        .unwrap_err();
    assert!(e.contains("two filters"), "{e}");
}

#[test]
fn two_rollupissubtotal_are_crossed() {
    let mut s = window_session();
    // a table with two subtotal flags: regions (with their subtotal rows) by product, and a
    // product flag FALSE everywhere
    let t = "ADDCOLUMNS(SUMMARIZECOLUMNS(ROLLUPADDISSUBTOTAL(Region[Region], \"IsR\"), Sales[Product], \
             \"T\", [Total]), \"IsP\", FALSE())";
    let q = format!(
        "EVALUATE ADDMISSINGITEMS(Region[Region], Sales[Product], {t}, ROLLUPISSUBTOTAL(Region[Region], [IsR]), \
         ROLLUPISSUBTOTAL(Sales[Product], [IsP]))"
    );
    let all = rows(&mut s, &q);
    let plain = rows(&mut s, &format!("EVALUATE {t}"));
    assert!(all.len() > plain.len(), "{all:?}");
    // the level with the products rolled up and the regions grouped (IsR FALSE, IsP TRUE) has
    // no row in the table: every region is added there once, North (no sales) too
    let level: Vec<&Value> = all
        .iter()
        .filter(|r| r["[IsR]"] == json!(false) && r["[IsP]"] == json!(true))
        .collect();
    assert!(
        level.iter().any(|r| r["Region[Region]"] == json!("North")),
        "{level:?}"
    );
    assert!(level.iter().all(|r| r["Sales[Product]"].is_null()), "{level:?}");
    let mut names: Vec<String> = level
        .iter()
        .map(|r| r["Region[Region]"].as_str().unwrap_or("").to_string())
        .collect();
    let n = names.len();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), n, "{level:?}");
    // the grand total level of both (IsR TRUE, IsP TRUE): one row
    assert_eq!(
        all.iter()
            .filter(|r| r["[IsR]"] == json!(true) && r["[IsP]"] == json!(true))
            .count(),
        1,
        "{all:?}"
    );
}

// --- the clock of calculated tables and columns ------------------------------------------------

#[test]
fn refresh_takes_utc_offset_minutes_for_calculated_columns() {
    let mut s = Session::new();
    s.execute("model.new", &json!({"name": "Clock"})).unwrap();
    s.execute(
        "table.import",
        &json!({"name": "T", "m": "#table(type table [A = number], {{1}})"}),
    )
    .unwrap();
    s.execute(
        "column.add",
        &json!({"table": "T", "name": "D", "expression": "ROUND((NOW() - UTCNOW()) * 1440, 0)"}),
    )
    .unwrap();
    let dir = TempDir::new("engine-calendar-clock");
    let path = dir.join("clock.pbip");
    s.execute("file.save_as", &json!({"path": path})).unwrap();
    let r = s
        .execute("model.refresh", &json!({"utc_offset_minutes": 90}))
        .unwrap_or_else(|e| panic!("refresh: {e}"));
    let v = eval(&mut s, "EVALUATE ROW(\"d\", MAX(T[D]))");
    assert_eq!(v["results"][0]["rows"][0]["[d]"].as_f64(), Some(90.0), "{v} {r}");
    // out of range: refused
    let e = s
        .execute("model.refresh", &json!({"utc_offset_minutes": 5000}))
        .unwrap_err();
    assert!(e.contains("utc_offset_minutes"), "{e}");
}
