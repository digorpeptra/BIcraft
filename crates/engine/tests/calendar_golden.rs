//! Differential test of BIcraft's calendar math (ROADMAP M11.9, docs/method.md decision 3).
//!
//! A generated corpus of dates (leap years, the 1899-12-30 serial epoch and the days around it,
//! month ends, ISO week edges, negative serials, years 100 and 9999 and beyond, fractions of a
//! day, time zones) goes through the user-visible date functions of the three places that do
//! calendar math: DAX (`dax.evaluate`: date functions, `DATE` argument carries, `DATEDIFF`,
//! `YEARFRAC`, `WEEKNUM`, `FORMAT`, `CALENDAR`, time intelligence), Power Query M (`Date.*`,
//! `DateTime.*`, `DateTimeZone.*`, culture parsing and formatting) and `bicraft-locale` (.NET
//! and VB patterns, the civil-day conversions). Every answer is one line of
//! `tests/calendar/golden.txt`; the test fails on the first line that changed.
//!
//! The golden file was recorded from the hand-written calendar code before it moved to `chrono`
//! (M11.9), so the port is held to the same answers; the move from `chrono` to `jiff` (M11.43)
//! is held to them too. `BICRAFT_BLESS=1` rewrites it: only do that for a
//! change you mean, and say why in the commit (docs/formats/dax.md, "Calendar").

use std::fmt::Write as _;
use std::path::PathBuf;

use bicraft_engine::Session;
use bicraft_locale::{Locale, datefmt, vb};
use serde_json::{Value, json};

// --- the corpus ------------------------------------------------------------------------------

/// Days from 1899-12-30 of a proleptic Gregorian date (the test's own reference, independent of
/// the code under test).
fn serial(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468 + 25_569
}

/// Deterministic pseudo-random numbers (a 64-bit LCG).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, lo: i64, hi: i64) -> i64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let span = u64::try_from(hi - lo + 1).unwrap();
        lo + i64::try_from((self.0 >> 33) % span).unwrap()
    }
}

/// Whole days of the corpus (serials).
fn days() -> Vec<i64> {
    let mut v = Vec::new();
    let ymd: &[(i64, i64, i64)] = &[
        // the serial epoch and the days around it (DAX and OLE have no 1900-02-29)
        (1899, 12, 29),
        (1899, 12, 30),
        (1899, 12, 31),
        (1900, 1, 1),
        (1900, 1, 7),
        (1900, 2, 28),
        (1900, 3, 1),
        (1899, 1, 1),
        (1800, 2, 28),
        (1800, 3, 1),
        (1700, 12, 31),
        (1600, 2, 29),
        (1582, 10, 4),
        (1582, 10, 15),
        (1752, 9, 2),
        (1752, 9, 14),
        (100, 1, 1),
        (100, 2, 28),
        (100, 3, 1),
        (101, 1, 1),
        (400, 2, 29),
        (1000, 6, 15),
        // leap years
        (1904, 2, 29),
        (1996, 2, 29),
        (2000, 2, 28),
        (2000, 2, 29),
        (2000, 3, 1),
        (2023, 2, 28),
        (2023, 3, 1),
        (2024, 2, 28),
        (2024, 2, 29),
        (2024, 3, 1),
        (2100, 2, 28),
        (2100, 3, 1),
        (2400, 2, 29),
        // ISO week and year edges
        (2004, 12, 27),
        (2005, 1, 1),
        (2005, 1, 2),
        (2005, 1, 3),
        (2008, 12, 29),
        (2009, 12, 31),
        (2010, 1, 3),
        (2015, 12, 31),
        (2016, 1, 1),
        (2016, 1, 3),
        (2020, 12, 31),
        (2021, 1, 1),
        (2021, 1, 4),
        (2026, 11, 5),
        (1969, 12, 31),
        (1970, 1, 1),
        // the top of the range
        (9998, 12, 31),
        (9999, 11, 30),
        (9999, 12, 1),
        (9999, 12, 31),
    ];
    v.extend(ymd.iter().map(|&(y, m, d)| serial(y, m, d)));
    // every month start and end of 2023 and 2024
    for y in [2023, 2024] {
        for m in 1..=12 {
            v.push(serial(y, m, 1));
            v.push(serial(y, m + 1, 1) - 1);
        }
    }
    let mut r = Lcg(0x5EED_CA1E_DA75);
    for _ in 0..30 {
        v.push(r.next(serial(100, 1, 1), serial(9999, 12, 31)));
    }
    for _ in 0..20 {
        v.push(r.next(serial(1990, 1, 1), serial(2040, 12, 31)));
    }
    v
}

/// Values a date function may see: the corpus days, fractions of a day and out-of-range
/// numbers.
fn values() -> Vec<f64> {
    let mut v: Vec<f64> = days().into_iter().map(|d| d as f64).collect();
    v.extend([
        45_658.25,
        45_658.999_988_425_9,
        45_351.5,
        0.75,
        -0.5,
        -1.25,
        59.999,
        60.000_01,
        2_958_465.999,
        // outside 100..9999
        -657_435.0,
        2_958_466.0,
        3_000_000.0,
        -700_000.0,
        1.0e7,
    ]);
    v
}

fn iso(x: f64) -> String {
    let d = x.floor() as i64;
    // the test's own inverse of `serial`, by search over years then days
    let mut y = 1899 + d.div_euclid(366);
    while serial(y + 1, 1, 1) <= d {
        y += 1;
    }
    while serial(y, 1, 1) > d {
        y -= 1;
    }
    let mut m = 1;
    while m < 12 && serial(y, m + 1, 1) <= d {
        m += 1;
    }
    let day = d - serial(y, m, 1) + 1;
    let frac = x - x.floor();
    if frac == 0.0 {
        format!("{y:04}-{m:02}-{day:02}")
    } else {
        format!("{y:04}-{m:02}-{day:02}+{frac:.6}")
    }
}

// --- DAX -------------------------------------------------------------------------------------

fn cell(v: &Value) -> String {
    match v {
        Value::Null => "∅".into(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// One `EVALUATE` answered as `a b c` (its first row), or `ERR <message>`.
fn dax(s: &mut Session, query: &str) -> String {
    match s.execute("dax.evaluate", &json!({ "query": query })) {
        Err(e) => format!("ERR {}", e.lines().next().unwrap_or("")),
        Ok(v) => {
            let r = &v["results"][0];
            let mut out = Vec::new();
            for row in r["rows"].as_array().into_iter().flatten() {
                let mut cells = Vec::new();
                for c in r["columns"].as_array().into_iter().flatten() {
                    cells.push(cell(&row[c["name"].as_str().unwrap_or("")]));
                }
                out.push(cells.join(" "));
            }
            out.join(" / ")
        }
    }
}

fn row(cols: &[String]) -> String {
    let parts: Vec<String> = cols
        .iter()
        .enumerate()
        .map(|(i, e)| format!("\"c{i}\", {e}"))
        .collect();
    format!("EVALUATE ROW({})", parts.join(", "))
}

fn dax_scalars(out: &mut String) {
    let mut s = Session::new();
    s.execute("model.new", &json!({"name": "Calendar"})).unwrap();
    let shifts = [-1200, -25, -13, -12, -1, 0, 1, 11, 12, 13, 1200];
    let deltas = [1.0, 31.0, 366.0, -400.0, 1461.25];
    for x in values() {
        let lit = format!("{x:?}");
        let tag = format!("{} {lit}", iso(x));
        let parts: Vec<String> = ["YEAR", "MONTH", "DAY", "QUARTER", "HOUR", "MINUTE", "SECOND"]
            .iter()
            .map(|f| format!("{f}({lit})"))
            .collect();
        let _ = writeln!(out, "dax.parts {tag} = {}", dax(&mut s, &row(&parts)));
        let wk: Vec<String> = [1, 2, 3]
            .iter()
            .map(|k| format!("WEEKDAY({lit}, {k})"))
            .chain(
                [1, 2, 11, 12, 13, 14, 15, 16, 17, 21]
                    .iter()
                    .map(|k| format!("WEEKNUM({lit}, {k})")),
            )
            .collect();
        let _ = writeln!(out, "dax.week {tag} = {}", dax(&mut s, &row(&wk)));
        for f in ["EOMONTH", "EDATE"] {
            let c: Vec<String> = shifts.iter().map(|k| format!("{f}({lit}, {k})")).collect();
            let _ = writeln!(out, "dax.{f} {tag} = {}", dax(&mut s, &row(&c)));
        }
        for dl in deltas {
            let y = x + dl;
            let c: Vec<String> = [
                "SECOND", "MINUTE", "HOUR", "DAY", "WEEK", "MONTH", "QUARTER", "YEAR",
            ]
            .iter()
            .map(|u| format!("DATEDIFF({lit}, {y:?}, {u})"))
            .collect();
            let _ = writeln!(out, "dax.DATEDIFF {tag} {dl:?} = {}", dax(&mut s, &row(&c)));
            let c: Vec<String> = (0..=4)
                .map(|b| format!("YEARFRAC({lit}, {y:?}, {b})"))
                .chain([format!("NETWORKDAYS({lit}, {y:?})")])
                .collect();
            let _ = writeln!(out, "dax.YEARFRAC {tag} {dl:?} = {}", dax(&mut s, &row(&c)));
        }
        let c: Vec<String> = [
            "dddd, mmmm d, yyyy",
            "ddd dd/mm/yy",
            "yyyy-mm-dd hh:nn:ss",
            "h:nn AM/PM",
            "Short Date",
            "Long Date",
            "General Date",
            "q w ww y",
        ]
        .iter()
        .map(|f| format!("FORMAT({lit}, \"{f}\")"))
        .collect();
        let _ = writeln!(out, "dax.FORMAT {tag} = {}", dax(&mut s, &row(&c)));
        let q = format!(
            "EVALUATE VAR t = CALENDAR({lit}, {lit} + 40) RETURN ROW(\"n\", COUNTROWS(t), \
             \"lo\", MINX(t, [Date]), \"hi\", MAXX(t, [Date]))"
        );
        let _ = writeln!(out, "dax.CALENDAR {tag} = {}", dax(&mut s, &q));
    }
    // DATE's argument carries
    for y in [-1900, -1, 0, 49, 50, 99, 100, 1899, 1900, 2024, 9999, 10_000] {
        for m in [-25, -1, 0, 1, 2, 12, 13, 14, 15, 25, 120_000] {
            let c: Vec<String> = [-400, -1, 0, 1, 28, 29, 30, 31, 32, 60, 400]
                .iter()
                .map(|d| format!("IFERROR(DATE({y}, {m}, {d}), -1)"))
                .collect();
            let _ = writeln!(out, "dax.DATE {y} {m} = {}", dax(&mut s, &row(&c)));
        }
    }
    for (h, mi, sec) in [
        (0, 0, 0),
        (23, 59, 59),
        (24, 0, 0),
        (25, 61, 61),
        (-1, 0, 0),
        (12, -30, 0),
        (0, 0, 86_400),
    ] {
        let _ = writeln!(
            out,
            "dax.TIME {h} {mi} {sec} = {}",
            dax(&mut s, &format!("EVALUATE ROW(\"t\", TIME({h}, {mi}, {sec}))"))
        );
    }
    for t in [
        "2024-02-29",
        "2023-02-29",
        "1900-02-29",
        "12/31/2026",
        "31/12/2026",
        "2026-11-05T13:45:00",
        "March 3, 2024",
        "3 March 2024",
        "0100-01-01",
        "9999-12-31",
        "10000-01-01",
        "",
    ] {
        let _ = writeln!(
            out,
            "dax.DATEVALUE {t:?} = {}",
            dax(&mut s, &format!("EVALUATE ROW(\"d\", DATEVALUE(\"{t}\"))"))
        );
    }
}

/// A 2019-2025 calendar table and time intelligence over single days and whole months.
fn dax_time_intelligence(out: &mut String) {
    let mut rows = Vec::new();
    for x in serial(2019, 1, 1)..=serial(2025, 12, 31) {
        let t = iso(x as f64);
        let (y, m, d) = (&t[0..4], &t[5..7], &t[8..10]);
        let (m, d) = (m.trim_start_matches('0'), d.trim_start_matches('0'));
        rows.push(format!("{{#date({y},{m},{d}), {y}, {m}}}"));
    }
    let cal = format!(
        "#table({{\"Date\", \"Year\", \"Month\"}}, {{{}}})",
        rows.join(", ")
    );
    let mut s = Session::new();
    for (id, p) in [
        ("model.new", json!({"name": "TI"})),
        (
            "table.add",
            json!({"name": "C", "partition": {"source_type": "m", "expression": cal}}),
        ),
        (
            "column.add",
            json!({"table": "C", "name": "Date", "data_type": "dateTime"}),
        ),
        (
            "column.add",
            json!({"table": "C", "name": "Year", "data_type": "int64"}),
        ),
        (
            "column.add",
            json!({"table": "C", "name": "Month", "data_type": "int64"}),
        ),
    ] {
        s.execute(id, &p).unwrap();
    }
    let tis = [
        "DATESYTD(C[Date])",
        "DATESYTD(C[Date], \"06-30\")",
        "DATESYTD(C[Date], \"02-28\")",
        "DATESQTD(C[Date])",
        "DATESMTD(C[Date])",
        "SAMEPERIODLASTYEAR(C[Date])",
        "DATEADD(C[Date], -1, MONTH)",
        "DATEADD(C[Date], 1, MONTH)",
        "DATEADD(C[Date], 1, QUARTER)",
        "DATEADD(C[Date], -1, YEAR)",
        "DATEADD(C[Date], 10, DAY)",
        "PARALLELPERIOD(C[Date], -1, MONTH)",
        "PARALLELPERIOD(C[Date], 1, QUARTER)",
        "PARALLELPERIOD(C[Date], -1, YEAR)",
        "PREVIOUSDAY(C[Date])",
        "PREVIOUSMONTH(C[Date])",
        "PREVIOUSQUARTER(C[Date])",
        "PREVIOUSYEAR(C[Date])",
        "PREVIOUSYEAR(C[Date], \"03-31\")",
        "NEXTMONTH(C[Date])",
        "NEXTQUARTER(C[Date])",
        "NEXTYEAR(C[Date])",
        "STARTOFMONTH(C[Date])",
        "STARTOFQUARTER(C[Date])",
        "STARTOFYEAR(C[Date])",
        "ENDOFMONTH(C[Date])",
        "ENDOFQUARTER(C[Date])",
        "ENDOFYEAR(C[Date])",
        "ENDOFYEAR(C[Date], \"09-30\")",
        "DATESINPERIOD(C[Date], MAX(C[Date]), -1, MONTH)",
        "DATESINPERIOD(C[Date], MAX(C[Date]), 1, QUARTER)",
        "DATESINPERIOD(C[Date], MAX(C[Date]), -1, YEAR)",
        "DATESINPERIOD(C[Date], MAX(C[Date]), -7, DAY)",
    ];
    let mut filters: Vec<(String, String)> = Vec::new();
    for (y, m, d) in [
        (2020, 2, 29),
        (2021, 2, 28),
        (2021, 3, 1),
        (2024, 2, 29),
        (2024, 3, 31),
        (2024, 1, 31),
        (2023, 12, 31),
        (2024, 1, 1),
        (2022, 6, 30),
        (2022, 7, 1),
        (2023, 9, 30),
        (2023, 5, 15),
        (2020, 1, 1),
        (2025, 12, 31),
        (2024, 5, 31),
        (2024, 4, 30),
    ] {
        filters.push((
            format!("{y:04}-{m:02}-{d:02}"),
            format!("C[Date] = DATE({y}, {m}, {d})"),
        ));
    }
    for (y, m) in [(2024, 2), (2023, 2), (2024, 12), (2020, 1), (2022, 6), (2025, 12)] {
        filters.push((format!("{y:04}-{m:02}"), format!("C[Year] = {y}, C[Month] = {m}")));
    }
    for (tag, f) in &filters {
        for ti in tis {
            let q = format!(
                "EVALUATE ROW(\"lo\", CALCULATE(CALCULATE(MIN(C[Date]), {ti}), {f}), \
                 \"hi\", CALCULATE(CALCULATE(MAX(C[Date]), {ti}), {f}), \
                 \"n\", CALCULATE(CALCULATE(COUNTROWS(C), {ti}), {f}))"
            );
            let _ = writeln!(out, "dax.ti {tag} {ti} = {}", dax(&mut s, &q));
        }
    }
}

// --- M ---------------------------------------------------------------------------------------

/// The answers of M expressions: each one's `Text.From`, or `ERR <message>`.
fn m_values(exprs: &[String]) -> Vec<String> {
    let rows: Vec<String> = exprs
        .iter()
        .map(|e| {
            format!(
                "{{let r = try ({e}) in if r[HasError] then \"ERR \" & r[Error][Message] \
                 else if r[Value] = null then \"null\" else Text.From(r[Value])}}"
            )
        })
        .collect();
    let q = format!("#table({{\"v\"}}, {{{}}})", rows.join(", "));
    let limits = bicraft_data::Limits::default();
    match bicraft_data::m::load_m(&q, &limits) {
        Err(e) => vec![format!("ERR {e}")],
        Ok(t) => t.columns()[0]
            .values
            .iter()
            .map(|v| match v {
                bicraft_data::CellValue::Text(t) => t.to_string(),
                other => format!("{other:?}"),
            })
            .collect(),
    }
}

/// One line per expression.
fn m_lines(out: &mut String, prefix: &str, exprs: &[String]) {
    for (e, v) in exprs.iter().zip(m_values(exprs)) {
        let _ = writeln!(out, "{prefix} {e} = {v}");
    }
}

fn m_date(x: i64) -> String {
    let t = iso(x as f64);
    let y: i64 = t[..t.len() - 6].parse().unwrap();
    let m: i64 = t[t.len() - 5..t.len() - 3].parse().unwrap();
    let d: i64 = t[t.len() - 2..].parse().unwrap();
    format!("#date({y}, {m}, {d})")
}

/// Date functions applied to `{d}`, one line per corpus day (the header line names them).
const M_PER_DAY: &[&str] = &[
    "Date.AddMonths({d}, -1200)",
    "Date.AddMonths({d}, -13)",
    "Date.AddMonths({d}, -1)",
    "Date.AddMonths({d}, 1)",
    "Date.AddMonths({d}, 12)",
    "Date.AddMonths({d}, 13)",
    "Date.AddYears({d}, -400)",
    "Date.AddYears({d}, -1)",
    "Date.AddYears({d}, 1)",
    "Date.AddYears({d}, 4)",
    "Date.AddQuarters({d}, 1)",
    "Date.AddQuarters({d}, -5)",
    "Date.AddWeeks({d}, 1)",
    "Date.AddDays({d}, -1)",
    "Date.StartOfMonth({d})",
    "Date.EndOfMonth({d})",
    "Date.StartOfQuarter({d})",
    "Date.EndOfQuarter({d})",
    "Date.StartOfYear({d})",
    "Date.EndOfYear({d})",
    "Date.StartOfWeek({d})",
    "Date.StartOfWeek({d}, Day.Monday)",
    "Date.StartOfWeek({d}, Day.Saturday)",
    "Date.EndOfWeek({d}, Day.Monday)",
    "Date.DayOfWeek({d})",
    "Date.DayOfWeek({d}, Day.Monday)",
    "Date.DayOfWeekName({d}, \"fr-FR\")",
    "Date.DayOfYear({d})",
    "Date.DaysInMonth({d})",
    "Date.WeekOfYear({d})",
    "Date.WeekOfYear({d}, Day.Monday)",
    "Date.WeekOfMonth({d})",
    "Date.WeekOfMonth({d}, Day.Monday)",
    "Date.QuarterOfYear({d})",
    "Date.IsLeapYear({d})",
    "Date.MonthName({d}, \"de-DE\")",
    "Date.ToText({d})",
    "Date.ToText({d}, \"dddd d MMMM yyyy\", \"fr-FR\")",
    "Date.ToText({d}, \"D\", \"en-US\")",
    "Date.ToText({d}, \"d\", \"ja-JP\")",
    "Date.FromText(Date.ToText({d}, \"d\", \"en-US\"), \"en-US\")",
    "Date.FromText(Date.ToText({d}, \"d\", \"fr-FR\"), \"fr-FR\")",
    "Date.FromText(Date.ToText({d}, \"d\", \"de-DE\"), \"de-DE\")",
    "Number.From({d})",
    "Date.From(Number.From({d}))",
    "DateTime.From(Number.From({d}) + 0.75)",
    "Date.ToRecord({d})[Day]",
    "DateTime.ToText({d} & #time(23, 59, 59), \"yyyy-MM-ddTHH:mm:ss\")",
];

fn m_functions(out: &mut String) {
    let lo = serial(100, 1, 1);
    let hi = serial(9999, 12, 31);
    let _ = writeln!(out, "m.functions = {}", M_PER_DAY.join(" | "));
    for x in days().into_iter().filter(|x| (lo..=hi).contains(x)) {
        let d = m_date(x);
        let e: Vec<String> = M_PER_DAY.iter().map(|f| f.replace("{d}", &d)).collect();
        let _ = writeln!(out, "m {} = {}", iso(x as f64), m_values(&e).join(" | "));
    }
    // time zones: switching zones carries across days, months and years
    let mut e: Vec<String> = Vec::new();
    for (y, m, d, h) in [
        (2024, 12, 31, 23),
        (2024, 1, 1, 0),
        (2024, 2, 28, 22),
        (9999, 12, 31, 20),
        (100, 1, 1, 2),
    ] {
        for (zh, zm) in [(14, 0), (-12, 0), (5, 30), (-9, -30), (0, 0)] {
            let dtz = format!("#datetimezone({y}, {m}, {d}, {h}, 30, 0, {zh}, {zm})");
            e.push(format!("DateTimeZone.ToUtc({dtz})"));
            e.push(format!("DateTimeZone.SwitchZone({dtz}, -11)"));
            e.push(format!("DateTimeZone.SwitchZone({dtz}, 13, 45)"));
            e.push(format!("DateTimeZone.RemoveZone({dtz})"));
            e.push(format!("DateTimeZone.ZoneHours({dtz})"));
            e.push(format!(
                "DateTime.AddZone(#datetime({y}, {m}, {d}, {h}, 0, 0), {zh}, {zm})"
            ));
        }
    }
    for t in [
        "2024-02-29T23:30:00+05:30",
        "2024-12-31T23:59:59-11:00",
        "0100-01-01T00:00:00+01:00",
        "9999-12-31T23:00:00-02:00",
    ] {
        e.push(format!("DateTimeZone.FromText(\"{t}\")"));
        e.push(format!("DateTimeZone.ToUtc(DateTimeZone.FromText(\"{t}\"))"));
    }
    m_lines(out, "m.zone", &e);
    // culture parsing of text columns
    let mut e: Vec<String> = Vec::new();
    for (t, c) in [
        ("31/12/2026", "fr-FR"),
        ("12/31/2026", "en-US"),
        ("2026/12/31", "ja-JP"),
        ("31.12.2026", "de-DE"),
        ("29/02/2023", "fr-FR"),
        ("29/02/2024", "fr-FR"),
        ("2/29/1900", "en-US"),
        ("1/1/0100", "en-US"),
        ("12/31/9999 11:59 PM", "en-US"),
        ("31/12/2026 13:45", "en-GB"),
        ("2026-11-05", "fr-FR"),
        ("2026-11-05T25:00:00", "en-US"),
    ] {
        e.push(format!(
            "Table.TransformColumnTypes(#table({{\"t\"}}, {{{{\"{t}\"}}}}), {{{{\"t\", type datetime}}}}, \"{c}\"){{0}}[t]"
        ));
        e.push(format!("Date.FromText(\"{t}\", \"{c}\")"));
        e.push(format!("DateTime.FromText(\"{t}\", \"{c}\")"));
    }
    m_lines(out, "m.parse", &e);
}

// --- bicraft-locale --------------------------------------------------------------------------

fn locale(out: &mut String) {
    let locs = ["en-US", "fr-FR", "ja-JP"];
    for x in values() {
        let tag = format!("{} {x:?}", iso(x));
        let Some(p) = datefmt::DateParts::from_serial(x) else {
            let _ = writeln!(out, "locale {tag} = none");
            continue;
        };
        let mut cells = vec![format!("{p:?}"), format!("doy {}", p.day_of_year())];
        for l in locs {
            let loc = Locale::get(l);
            cells.push(datefmt::format_pattern(
                &p,
                "dddd d MMMM yyyy ddd MMM HH:mm:ss.fff g",
                &loc.names(),
            ));
            for spec in ["d", "D", "f", "s", "Y", "M"] {
                cells.push(datefmt::format(&p, spec, loc));
            }
            for f in ["dddd, mmmm d, yyyy hh:nn", "q w ww y"] {
                cells.push(vb::format_date(x, f, loc).unwrap_or_else(|| "none".into()));
            }
        }
        let _ = writeln!(out, "locale {tag} = {}", cells.join(" | "));
    }
    // the civil-day conversions over the whole i64-friendly range they accept
    let mut r = Lcg(0xD15C_0DA7);
    let mut zs: Vec<i64> = vec![
        -100_000_000,
        -95_000_000,
        -719_468,
        -719_162,
        -1,
        0,
        1,
        2_932_896,
        95_000_000,
        100_000_000,
    ];
    for _ in 0..60 {
        zs.push(r.next(-100_000_000, 100_000_000));
    }
    for z in zs {
        let (y, m, d) = datefmt::civil_from_days(z);
        let back = datefmt::days_from_civil(y, m, d);
        let p = datefmt::DateParts::from_days_ms(z + 25_569, 45_296_789);
        let _ = writeln!(
            out,
            "locale.civil {z} = {y} {m} {d} {back} {} {}",
            p.weekday,
            p.day_of_year()
        );
    }
}

#[test]
fn calendar_answers_match_the_golden_file() {
    let mut out = String::new();
    dax_scalars(&mut out);
    dax_time_intelligence(&mut out);
    m_functions(&mut out);
    locale(&mut out);
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("calendar")
        .join("golden.txt");
    if bicraft_env::var_os("BICRAFT_BLESS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &out).unwrap();
        return;
    }
    let golden = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e} (BICRAFT_BLESS=1 writes it)", path.display()))
        .replace("\r\n", "\n");
    let mut differ = Vec::new();
    for (i, (a, b)) in golden.lines().zip(out.lines()).enumerate() {
        if a != b {
            differ.push(format!("line {}:\n  golden: {a}\n  now:    {b}", i + 1));
        }
    }
    assert!(
        differ.is_empty() && golden.lines().count() == out.lines().count(),
        "{} lines differ from {} ({} golden lines, {} now):\n{}",
        differ.len(),
        path.display(),
        golden.lines().count(),
        out.lines().count(),
        differ.iter().take(20).cloned().collect::<Vec<_>>().join("\n")
    );
}
