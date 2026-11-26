//! `#date`, `#datetime`, `#datetimezone`, `#time`, `#duration` and the
//! `Date.*`, `DateTime.*`, `DateTimeZone.*`, `Time.*`, `Duration.*` functions.
//! Functions reading the clock are unsupported (non-deterministic).

use super::{Builtin, arg, b, conv_err, culture_arg, int, number, opt_number, opt_text};
use crate::m::convert::{self, CultureInfo, days_in_month, make_date, split_ms, weekday, ymd};
use crate::m::eval::{Interp, MS_PER_DAY, date_ms, whole};
use crate::m::value::{Kind, MType, Res, Value, err, expr_err, unsupported};

pub(crate) static FUNCS: &[Builtin] = &[
    b("#date", 3, 3, hash_date),
    b("#datetime", 6, 6, hash_datetime),
    b("#datetimezone", 8, 8, hash_datetimezone),
    b("#time", 3, 3, hash_time),
    b("#duration", 4, 4, hash_duration),
    b("Date.From", 1, 2, date_from),
    b("Date.FromText", 1, 2, date_from_text),
    b("Date.ToText", 1, 3, to_text),
    b("Date.Year", 1, 1, year),
    b("Date.Month", 1, 1, month),
    b("Date.Day", 1, 1, day),
    b("Date.DayOfWeek", 1, 2, day_of_week),
    b("Date.DayOfWeekName", 1, 2, day_of_week_name),
    b("Date.DayOfYear", 1, 1, day_of_year),
    b("Date.DaysInMonth", 1, 1, days_in_month_fn),
    b("Date.WeekOfYear", 1, 2, week_of_year),
    b("Date.WeekOfMonth", 1, 2, week_of_month),
    b("Date.QuarterOfYear", 1, 1, quarter_of_year),
    b("Date.MonthName", 1, 2, month_name),
    b("Date.IsLeapYear", 1, 1, is_leap_year),
    b("Date.AddDays", 2, 2, add_days),
    b("Date.AddWeeks", 2, 2, add_weeks),
    b("Date.AddMonths", 2, 2, add_months),
    b("Date.AddQuarters", 2, 2, add_quarters),
    b("Date.AddYears", 2, 2, add_years),
    b("Date.StartOfDay", 1, 1, start_of_day),
    b("Date.EndOfDay", 1, 1, end_of_day),
    b("Date.StartOfWeek", 1, 2, start_of_week),
    b("Date.EndOfWeek", 1, 2, end_of_week),
    b("Date.StartOfMonth", 1, 1, start_of_month),
    b("Date.EndOfMonth", 1, 1, end_of_month),
    b("Date.StartOfQuarter", 1, 1, start_of_quarter),
    b("Date.EndOfQuarter", 1, 1, end_of_quarter),
    b("Date.StartOfYear", 1, 1, start_of_year),
    b("Date.EndOfYear", 1, 1, end_of_year),
    b("Date.ToRecord", 1, 1, date_to_record),
    b("DateTime.From", 1, 2, datetime_from),
    b("DateTime.FromText", 1, 2, datetime_from_text),
    b("DateTime.ToText", 1, 3, to_text),
    b("DateTime.Date", 1, 1, datetime_date),
    b("DateTime.Time", 1, 1, datetime_time),
    b("DateTime.AddZone", 2, 3, add_zone),
    b("DateTime.ToRecord", 1, 1, datetime_to_record),
    b("DateTime.LocalNow", 0, 0, local_now),
    b("DateTime.FixedLocalNow", 0, 0, local_now),
    b("DateTimeZone.LocalNow", 0, 0, local_now_zone),
    b("DateTimeZone.FixedLocalNow", 0, 0, local_now_zone),
    b("DateTimeZone.UtcNow", 0, 0, utc_now),
    b("DateTimeZone.FixedUtcNow", 0, 0, utc_now),
    b("Date.IsInCurrentYear", 1, 1, clock),
    b("Date.IsInCurrentMonth", 1, 1, clock),
    b("Date.IsInPreviousNDays", 2, 2, clock),
    b("DateTimeZone.From", 1, 2, dtz_from),
    b("DateTimeZone.FromText", 1, 2, dtz_from),
    b("DateTimeZone.ToText", 1, 3, to_text),
    b("DateTimeZone.RemoveZone", 1, 1, remove_zone),
    b("DateTimeZone.ZoneHours", 1, 1, zone_hours),
    b("DateTimeZone.ZoneMinutes", 1, 1, zone_minutes),
    b("DateTimeZone.ToUtc", 1, 1, to_utc),
    b("DateTimeZone.SwitchZone", 2, 3, switch_zone),
    b("DateTimeZone.ToLocal", 1, 1, clock),
    b("Time.From", 1, 2, time_from),
    b("Time.FromText", 1, 2, time_from_text),
    b("Time.ToText", 1, 3, to_text),
    b("Time.Hour", 1, 1, hour),
    b("Time.Minute", 1, 1, minute),
    b("Time.Second", 1, 1, second),
    b("Time.StartOfHour", 1, 1, start_of_hour),
    b("Time.EndOfHour", 1, 1, end_of_hour),
    b("Duration.From", 1, 1, duration_from),
    b("Duration.FromText", 1, 1, duration_from_text),
    b("Duration.ToText", 1, 2, duration_to_text),
    b("Duration.Days", 1, 1, duration_days),
    b("Duration.Hours", 1, 1, duration_hours),
    b("Duration.Minutes", 1, 1, duration_minutes),
    b("Duration.Seconds", 1, 1, duration_seconds),
    b("Duration.TotalDays", 1, 1, total_days),
    b("Duration.TotalHours", 1, 1, total_hours),
    b("Duration.TotalMinutes", 1, 1, total_minutes),
    b("Duration.TotalSeconds", 1, 1, total_seconds),
];

fn clock(_: &mut Interp, _: &[Value]) -> Res<Value> {
    unsupported("a function that reads the current date or time (non-deterministic)")
}

/// The UTC clock, when the caller gave one (`SourceOptions::clock_utc_ms`, set by a refresh).
fn utc_now(interp: &mut Interp, _: &[Value]) -> Res<Value> {
    match interp.clock_utc_ms {
        Some(ms) => super::super::eval::in_range(Value::DateTimeZone(ms, 0)),
        None => clock(interp, &[]),
    }
}

/// The local clock: the UTC clock plus the caller's offset (`SourceOptions::clock_local_offset_minutes`),
/// as a `datetime`.
fn local_now(interp: &mut Interp, _: &[Value]) -> Res<Value> {
    match local_ms(interp) {
        Some(ms) => super::super::eval::in_range(Value::DateTime(ms)),
        None => clock(interp, &[]),
    }
}

/// The local clock as a `datetimezone` with the offset.
fn local_now_zone(interp: &mut Interp, _: &[Value]) -> Res<Value> {
    match (local_ms(interp), interp.clock_local_offset_minutes) {
        (Some(ms), Some(off)) => super::super::eval::in_range(Value::DateTimeZone(ms, off)),
        _ => clock(interp, &[]),
    }
}

fn local_ms(interp: &Interp) -> Option<i64> {
    let off = interp.clock_local_offset_minutes?;
    interp
        .clock_utc_ms?
        .checked_add(i64::from(off).checked_mul(60_000)?)
}

/// Hours, minutes and (fractional) seconds → ms of day.
fn clock_ms(h: f64, mi: f64, s: f64) -> Res<i64> {
    let (Some(h), Some(mi)) = (whole(h), whole(mi)) else {
        return expr_err("Hours and minutes must be whole numbers.");
    };
    if !(0.0..60.0).contains(&s) {
        return expr_err(format!("The seconds value {s} is out of range."));
    }
    let whole_s = s.trunc();
    let ms = ((s - whole_s) * 1000.0).round().min(999.0);
    crate::value::time_ms(h, mi, whole_s as i64, ms as i64).or_else(expr_err)
}

fn ints3(a: &[Value]) -> Res<(i64, i64, i64)> {
    Ok((int(arg(a, 0))?, int(arg(a, 1))?, int(arg(a, 2))?))
}

fn hash_date(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let (y, m, d) = ints3(a)?;
    Ok(Value::Date(make_date(y, m, d)?))
}

fn hash_datetime(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let (y, m, d) = ints3(a)?;
    let day = make_date(y, m, d)?;
    let ms = clock_ms(number(arg(a, 3))?, number(arg(a, 4))?, number(arg(a, 5))?)?;
    Ok(Value::DateTime(date_ms(day)? + ms))
}

fn hash_datetimezone(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let (y, m, d) = ints3(a)?;
    let day = make_date(y, m, d)?;
    let ms = clock_ms(number(arg(a, 3))?, number(arg(a, 4))?, number(arg(a, 5))?)?;
    let oh = int(arg(a, 6))?;
    let om = int(arg(a, 7))?;
    if oh.abs() > 14 || om.abs() > 59 {
        return expr_err(format!("The time zone offset {oh}:{om} is out of range."));
    }
    Ok(Value::DateTimeZone(date_ms(day)? + ms, (oh * 60 + om) as i32))
}

fn hash_time(_: &mut Interp, a: &[Value]) -> Res<Value> {
    Ok(Value::Time(clock_ms(
        number(arg(a, 0))?,
        number(arg(a, 1))?,
        number(arg(a, 2))?,
    )?))
}

fn hash_duration(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let total = number(arg(a, 0))? * MS_PER_DAY as f64
        + number(arg(a, 1))? * 3_600_000.0
        + number(arg(a, 2))? * 60_000.0
        + number(arg(a, 3))? * 1000.0;
    Ok(Value::Duration(crate::m::eval::ms_from_f64(total)?))
}

// ---------------------------------------------------------------------------
// Decomposition.

#[derive(Clone, Copy)]
enum DKind {
    Date,
    DateTime,
    Zone(i32),
}

/// (OLE day, ms of day, kind) of a date, datetime or datetimezone.
fn parts(v: &Value) -> Res<Option<(i64, i64, DKind)>> {
    Ok(Some(match v {
        Value::Null => return Ok(None),
        Value::Date(d) => (*d, 0, DKind::Date),
        Value::DateTime(ms) => {
            let (d, t) = split_ms(*ms);
            (d, t, DKind::DateTime)
        }
        Value::DateTimeZone(ms, o) => {
            let (d, t) = split_ms(*ms);
            (d, t, DKind::Zone(*o))
        }
        other => return conv_err(other, "Date"),
    }))
}

fn rebuild(day: i64, ms: i64, k: DKind) -> Res<Value> {
    if !(crate::m::eval::MIN_DAY..=crate::m::eval::MAX_DAY).contains(&day) {
        return err("Expression.Error", "The date is out of range.");
    }
    Ok(match k {
        DKind::Date => Value::Date(day),
        DKind::DateTime => Value::DateTime(date_ms(day)? + ms),
        DKind::Zone(o) => Value::DateTimeZone(date_ms(day)? + ms, o),
    })
}

fn with_date(a: &[Value], f: impl FnOnce(i64, i64, i64, i64) -> Res<f64>) -> Res<Value> {
    match parts(arg(a, 0))? {
        None => Ok(Value::Null),
        Some((d, _, _)) => {
            let (y, m, dd) = ymd(d);
            Ok(Value::num(f(d, y, m, dd)?))
        }
    }
}

fn year(_: &mut Interp, a: &[Value]) -> Res<Value> {
    with_date(a, |_, y, _, _| Ok(y as f64))
}

fn month(_: &mut Interp, a: &[Value]) -> Res<Value> {
    with_date(a, |_, _, m, _| Ok(m as f64))
}

fn day(_: &mut Interp, a: &[Value]) -> Res<Value> {
    with_date(a, |_, _, _, d| Ok(d as f64))
}

fn first_day(interp: &Interp, v: &Value) -> Res<i64> {
    Ok(match opt_number(v)? {
        None => i64::from(interp.culture.first_day),
        Some(n) => whole(n)
            .filter(|d| (0..7).contains(d))
            .map_or_else(|| expr_err("Bad first day of week."), Ok)?,
    })
}

fn day_of_week(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let fd = first_day(interp, arg(a, 1))?;
    with_date(a, |d, _, _, _| Ok((weekday(d) - fd).rem_euclid(7) as f64))
}

fn day_of_week_name(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let c = culture_arg(interp, arg(a, 1))?;
    match parts(arg(a, 0))? {
        None => Ok(Value::Null),
        Some((d, _, _)) => Ok(Value::text(
            c.days.get(weekday(d) as usize).copied().unwrap_or(""),
        )),
    }
}

fn day_of_year(_: &mut Interp, a: &[Value]) -> Res<Value> {
    with_date(a, |d, y, _, _| Ok((d - make_date(y, 1, 1)? + 1) as f64))
}

fn days_in_month_fn(_: &mut Interp, a: &[Value]) -> Res<Value> {
    with_date(a, |_, y, m, _| Ok(days_in_month(y, m) as f64))
}

fn week_of_year(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let fd = first_day(interp, arg(a, 1))?;
    with_date(a, |d, y, _, _| {
        let jan1 = make_date(y, 1, 1)?;
        let offset = (weekday(jan1) - fd).rem_euclid(7);
        Ok(((d - jan1 + offset) / 7 + 1) as f64)
    })
}

fn week_of_month(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let fd = first_day(interp, arg(a, 1))?;
    with_date(a, |d, y, m, _| {
        let first = make_date(y, m, 1)?;
        let offset = (weekday(first) - fd).rem_euclid(7);
        Ok(((d - first + offset) / 7 + 1) as f64)
    })
}

fn quarter_of_year(_: &mut Interp, a: &[Value]) -> Res<Value> {
    with_date(a, |_, _, m, _| Ok(((m - 1) / 3 + 1) as f64))
}

fn month_name(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let c = culture_arg(interp, arg(a, 1))?;
    match parts(arg(a, 0))? {
        None => Ok(Value::Null),
        Some((d, _, _)) => {
            let (_, m, _) = ymd(d);
            Ok(Value::text(c.months.get((m - 1) as usize).copied().unwrap_or("")))
        }
    }
}

fn is_leap_year(_: &mut Interp, a: &[Value]) -> Res<Value> {
    match parts(arg(a, 0))? {
        None => Ok(Value::Null),
        Some((d, _, _)) => {
            let (y, _, _) = ymd(d);
            Ok(Value::Logical(days_in_month(y, 2) == 29))
        }
    }
}

fn shift_days(a: &[Value], per: i64) -> Res<Value> {
    let n = int(arg(a, 1))?;
    match parts(arg(a, 0))? {
        None => Ok(Value::Null),
        Some((d, ms, k)) => {
            let Some(nd) = n.checked_mul(per).and_then(|x| x.checked_add(d)) else {
                return err("Expression.Error", "The date is out of range.");
            };
            rebuild(nd, ms, k)
        }
    }
}

fn add_days(_: &mut Interp, a: &[Value]) -> Res<Value> {
    shift_days(a, 1)
}

fn add_weeks(_: &mut Interp, a: &[Value]) -> Res<Value> {
    shift_days(a, 7)
}

fn shift_months(a: &[Value], per: i64) -> Res<Value> {
    let n = int(arg(a, 1))?;
    match parts(arg(a, 0))? {
        None => Ok(Value::Null),
        Some((d, ms, k)) => {
            let (y, m, dd) = ymd(d);
            let Some(total) = n.checked_mul(per).and_then(|x| x.checked_add(y * 12 + m - 1)) else {
                return err("Expression.Error", "The date is out of range.");
            };
            let (ny, nm) = (total.div_euclid(12), total.rem_euclid(12) + 1);
            if !(1..=9999).contains(&ny) {
                return err("Expression.Error", "The date is out of range.");
            }
            let nd = dd.min(days_in_month(ny, nm));
            rebuild(make_date(ny, nm, nd)?, ms, k)
        }
    }
}

fn add_months(_: &mut Interp, a: &[Value]) -> Res<Value> {
    shift_months(a, 1)
}

fn add_quarters(_: &mut Interp, a: &[Value]) -> Res<Value> {
    shift_months(a, 3)
}

fn add_years(_: &mut Interp, a: &[Value]) -> Res<Value> {
    shift_months(a, 12)
}

/// Applies `f(day, y, m, d) -> new day` and `end` (whether to set the time to the end of day).
fn boundary(a: &[Value], end: bool, f: impl FnOnce(i64, i64, i64, i64) -> Res<i64>) -> Res<Value> {
    match parts(arg(a, 0))? {
        None => Ok(Value::Null),
        Some((d, _, k)) => {
            let (y, m, dd) = ymd(d);
            let nd = f(d, y, m, dd)?;
            let ms = if end { MS_PER_DAY - 1 } else { 0 };
            rebuild(nd, ms, k)
        }
    }
}

fn start_of_day(_: &mut Interp, a: &[Value]) -> Res<Value> {
    boundary(a, false, |d, _, _, _| Ok(d))
}

fn end_of_day(_: &mut Interp, a: &[Value]) -> Res<Value> {
    boundary(a, true, |d, _, _, _| Ok(d))
}

fn start_of_week(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let fd = first_day(interp, arg(a, 1))?;
    boundary(a, false, |d, _, _, _| Ok(d - (weekday(d) - fd).rem_euclid(7)))
}

fn end_of_week(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let fd = first_day(interp, arg(a, 1))?;
    boundary(a, true, |d, _, _, _| Ok(d - (weekday(d) - fd).rem_euclid(7) + 6))
}

fn start_of_month(_: &mut Interp, a: &[Value]) -> Res<Value> {
    boundary(a, false, |_, y, m, _| make_date(y, m, 1))
}

fn end_of_month(_: &mut Interp, a: &[Value]) -> Res<Value> {
    boundary(a, true, |_, y, m, _| make_date(y, m, days_in_month(y, m)))
}

fn start_of_quarter(_: &mut Interp, a: &[Value]) -> Res<Value> {
    boundary(a, false, |_, y, m, _| make_date(y, (m - 1) / 3 * 3 + 1, 1))
}

fn end_of_quarter(_: &mut Interp, a: &[Value]) -> Res<Value> {
    boundary(a, true, |_, y, m, _| {
        let qm = (m - 1) / 3 * 3 + 3;
        make_date(y, qm, days_in_month(y, qm))
    })
}

fn start_of_year(_: &mut Interp, a: &[Value]) -> Res<Value> {
    boundary(a, false, |_, y, _, _| make_date(y, 1, 1))
}

fn end_of_year(_: &mut Interp, a: &[Value]) -> Res<Value> {
    boundary(a, true, |_, y, _, _| make_date(y, 12, 31))
}

fn date_to_record(_: &mut Interp, a: &[Value]) -> Res<Value> {
    match parts(arg(a, 0))? {
        None => Ok(Value::Null),
        Some((d, _, _)) => {
            let (y, m, dd) = ymd(d);
            Ok(Value::record(vec![
                ("Year", Value::num(y as f64)),
                ("Month", Value::num(m as f64)),
                ("Day", Value::num(dd as f64)),
            ]))
        }
    }
}

fn datetime_to_record(_: &mut Interp, a: &[Value]) -> Res<Value> {
    match parts(arg(a, 0))? {
        None => Ok(Value::Null),
        Some((d, ms, _)) => {
            let (y, m, dd) = ymd(d);
            Ok(Value::record(vec![
                ("Year", Value::num(y as f64)),
                ("Month", Value::num(m as f64)),
                ("Day", Value::num(dd as f64)),
                ("Hour", Value::num((ms / 3_600_000) as f64)),
                ("Minute", Value::num((ms / 60_000 % 60) as f64)),
                ("Second", Value::num((ms % 60_000) as f64 / 1000.0)),
            ]))
        }
    }
}

// ---------------------------------------------------------------------------
// Conversions.

/// Options of the `*.FromText` / `*.ToText` functions: a record `[Format, Culture]`,
/// or (older signatures) a format text and a culture text.
/// Options of the `*.FromText` (`parse`) and `*.ToText` functions: a record
/// `[Format, Culture]`, or the older forms `FromText(text, culture)` and
/// `ToText(value, format, culture)`.
fn options(interp: &Interp, a: &[Value], parse: bool) -> Res<(Option<String>, CultureInfo)> {
    match arg(a, 1) {
        Value::Record(r) => {
            let fmt = match r.get("Format") {
                Some(Value::Text(t)) => Some(t.to_string()),
                _ => None,
            };
            let c = match r.get("Culture") {
                Some(Value::Text(t)) => CultureInfo::from_name(t),
                _ => interp.culture.clone(),
            };
            Ok((fmt, c))
        }
        v if parse => Ok((None, culture_arg(interp, v)?)),
        v => {
            let fmt = opt_text(v)?.map(str::to_string);
            let c = culture_arg(interp, arg(a, 2))?;
            Ok((fmt, c))
        }
    }
}

fn convert_value(interp: &Interp, v: &Value, kind: Kind, culture: &Value) -> Res<Value> {
    let c = culture_arg(interp, culture)?;
    convert::convert_to(v, &MType::prim(kind), &c)
}

fn date_from(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    convert_value(interp, arg(a, 0), Kind::Date, arg(a, 1))
}

fn datetime_from(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    convert_value(interp, arg(a, 0), Kind::DateTime, arg(a, 1))
}

fn time_from(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    convert_value(interp, arg(a, 0), Kind::Time, arg(a, 1))
}

fn dtz_from(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    convert_value(interp, arg(a, 0), Kind::DateTimeZone, &Value::Null)
}

fn duration_from(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    convert_value(interp, arg(a, 0), Kind::Duration, &Value::Null)
}

/// Parses text with a .NET custom format (`dd/MM/yyyy`, `yyyyMMdd HH:mm`, …).
fn parse_with_format(s: &str, fmt: &str, c: &CultureInfo) -> Option<i64> {
    let f: Vec<char> = fmt.chars().collect();
    let t: Vec<char> = s.trim().chars().collect();
    let (mut i, mut j) = (0usize, 0usize);
    let (mut y, mut mo, mut d, mut h, mut mi, mut sec, mut ms) = (1i64, 1i64, 1i64, 0i64, 0i64, 0i64, 0i64);
    let mut pm: Option<bool> = None;
    let digits = |j: &mut usize, min: usize, max: usize| -> Option<i64> {
        let start = *j;
        while *j < t.len() && *j - start < max && t.get(*j).is_some_and(char::is_ascii_digit) {
            *j += 1;
        }
        if *j - start < min {
            return None;
        }
        t.get(start..*j)?.iter().collect::<String>().parse().ok()
    };
    while let Some(&ch) = f.get(i) {
        let mut n = 1;
        while f.get(i + n) == Some(&ch) {
            n += 1;
        }
        match ch {
            'y' => {
                y = digits(&mut j, n.min(4), if n <= 2 { 2 } else { 4 })?;
                if n <= 2 {
                    y += 2000;
                }
            }
            'M' if n >= 3 => {
                let rest: String = t.get(j..)?.iter().collect();
                let lower = rest.to_lowercase();
                let mut hit = None;
                for (k, name) in c.months.iter().enumerate() {
                    let cand = if n == 3 {
                        name.chars().take(3).collect::<String>()
                    } else {
                        name.to_string()
                    };
                    if lower.starts_with(&cand.to_lowercase()) {
                        hit = Some((k as i64 + 1, cand.chars().count()));
                        break;
                    }
                }
                let (m, len) = hit?;
                mo = m;
                j += len;
            }
            'M' => mo = digits(&mut j, n.min(2), 2)?,
            'd' if n >= 3 => {
                while j < t.len() && t.get(j).is_some_and(|c| c.is_alphabetic()) {
                    j += 1;
                }
            }
            'd' => d = digits(&mut j, n.min(2), 2)?,
            'H' | 'h' => h = digits(&mut j, n.min(2), 2)?,
            'm' => mi = digits(&mut j, n.min(2), 2)?,
            's' => sec = digits(&mut j, n.min(2), 2)?,
            'f' | 'F' => {
                let v = digits(&mut j, 1, n)?;
                let scale = 10i64.pow(3u32.saturating_sub(n.min(3) as u32));
                ms = if n >= 3 {
                    v / 10i64.pow((n - 3) as u32)
                } else {
                    v * scale
                };
            }
            't' => {
                let rest: String = t.get(j..)?.iter().collect::<String>().to_ascii_uppercase();
                if rest.starts_with("PM") {
                    pm = Some(true);
                } else if rest.starts_with("AM") {
                    pm = Some(false);
                } else {
                    return None;
                }
                j += if n == 1 { 1 } else { 2 };
            }
            '\'' | '"' => {
                i += 1;
                while let Some(&q) = f.get(i) {
                    if q == ch {
                        break;
                    }
                    if t.get(j) != Some(&q) {
                        return None;
                    }
                    i += 1;
                    j += 1;
                }
                i += 1;
                continue;
            }
            other => {
                for _ in 0..n {
                    let lit = if other == '/' {
                        c.date_sep.chars().next().unwrap_or('/')
                    } else {
                        other
                    };
                    if t.get(j) != Some(&lit) && t.get(j) != Some(&other) {
                        return None;
                    }
                    j += 1;
                }
            }
        }
        i += n;
    }
    if j != t.len() {
        return None;
    }
    if let Some(pm) = pm {
        h = if pm { h % 12 + 12 } else { h % 12 };
    }
    let day = crate::value::ole_date(y, mo, d).ok()?;
    let t = crate::value::time_ms(h, mi, sec, ms).ok()?;
    day.checked_mul(MS_PER_DAY)?.checked_add(t)
}

/// Milliseconds and optional offset parsed from text.
type Parsed = Option<(i64, Option<i32>)>;

/// `None` for a null input; `Some(None)` when the text does not parse.
fn text_to_ms(interp: &Interp, a: &[Value]) -> Res<Option<Parsed>> {
    let Some(s) = opt_text(arg(a, 0))? else {
        return Ok(None);
    };
    let (fmt, c) = options(interp, a, true)?;
    Ok(Some(match fmt {
        Some(f) => parse_with_format(s, &f, &c).map(|ms| (ms, None)),
        None => convert::datetime_from_text(s, &c),
    }))
}

fn date_from_text(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    match text_to_ms(interp, a)? {
        None => Ok(Value::Null),
        Some(Some((ms, _))) => Ok(Value::Date(split_ms(ms).0)),
        Some(None) => err(
            "DataFormat.Error",
            "We couldn't parse the input provided as a Date value.",
        ),
    }
}

fn datetime_from_text(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    match text_to_ms(interp, a)? {
        None => Ok(Value::Null),
        Some(Some((ms, _))) => Ok(Value::DateTime(ms)),
        Some(None) => err(
            "DataFormat.Error",
            "We couldn't parse the input provided as a DateTime value.",
        ),
    }
}

fn time_from_text(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let Some(s) = opt_text(arg(a, 0))? else {
        return Ok(Value::Null);
    };
    let (fmt, c) = options(interp, a, true)?;
    let r = match fmt {
        Some(f) => parse_with_format(s, &f, &c).map(|ms| split_ms(ms).1),
        None => convert::time_from_text(s),
    };
    match r {
        Some(ms) => Ok(Value::Time(ms)),
        None => err(
            "DataFormat.Error",
            "We couldn't parse the input provided as a Time value.",
        ),
    }
}

fn to_text(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let v = arg(a, 0);
    if v.is_null() {
        return Ok(Value::Null);
    }
    let (fmt, c) = options(interp, a, false)?;
    let out = match fmt {
        None => convert::to_text(v, &c),
        Some(f) => convert::format_datetime(v, &f, &c),
    };
    match out {
        Some(s) => Ok(Value::text(s)),
        None => conv_err(v, "Date"),
    }
}

fn datetime_date(_: &mut Interp, a: &[Value]) -> Res<Value> {
    match parts(arg(a, 0))? {
        None => Ok(Value::Null),
        Some((d, _, _)) => Ok(Value::Date(d)),
    }
}

fn datetime_time(_: &mut Interp, a: &[Value]) -> Res<Value> {
    match parts(arg(a, 0))? {
        None => Ok(Value::Null),
        Some((_, ms, _)) => Ok(Value::Time(ms)),
    }
}

/// A time zone offset in minutes from hours and optional minutes (±14:00 at most).
fn offset_arg(a: &[Value]) -> Res<i32> {
    let h = int(arg(a, 1))?;
    let m = opt_number(arg(a, 2))?.map_or(Ok(0), |_| int(arg(a, 2)))?;
    if h.abs() > 14 || m.abs() > 59 {
        return expr_err(format!("The time zone offset {h}:{m} is out of range."));
    }
    Ok((h * 60 + m) as i32)
}

fn add_zone(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let o = offset_arg(a)?;
    match arg(a, 0) {
        Value::Null => Ok(Value::Null),
        Value::DateTime(ms) => Ok(Value::DateTimeZone(*ms, o)),
        Value::Date(d) => Ok(Value::DateTimeZone(date_ms(*d)?, o)),
        other => conv_err(other, "DateTime"),
    }
}

fn remove_zone(_: &mut Interp, a: &[Value]) -> Res<Value> {
    match arg(a, 0) {
        Value::Null => Ok(Value::Null),
        Value::DateTimeZone(ms, _) => Ok(Value::DateTime(*ms)),
        other => conv_err(other, "DateTimeZone"),
    }
}

fn zone_hours(_: &mut Interp, a: &[Value]) -> Res<Value> {
    match arg(a, 0) {
        Value::Null => Ok(Value::Null),
        Value::DateTimeZone(_, o) => Ok(Value::num(f64::from(o / 60))),
        other => conv_err(other, "DateTimeZone"),
    }
}

fn zone_minutes(_: &mut Interp, a: &[Value]) -> Res<Value> {
    match arg(a, 0) {
        Value::Null => Ok(Value::Null),
        Value::DateTimeZone(_, o) => Ok(Value::num(f64::from(o % 60))),
        other => conv_err(other, "DateTimeZone"),
    }
}

fn to_utc(_: &mut Interp, a: &[Value]) -> Res<Value> {
    match arg(a, 0) {
        Value::Null => Ok(Value::Null),
        Value::DateTimeZone(ms, o) => {
            crate::m::eval::in_range(Value::DateTimeZone(ms.saturating_sub(i64::from(*o) * 60_000), 0))
        }
        other => conv_err(other, "DateTimeZone"),
    }
}

fn switch_zone(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let o = offset_arg(a)?;
    match arg(a, 0) {
        Value::Null => Ok(Value::Null),
        Value::DateTimeZone(ms, old) => crate::m::eval::in_range(Value::DateTimeZone(
            ms.saturating_sub(i64::from(*old) * 60_000)
                .saturating_add(i64::from(o) * 60_000),
            o,
        )),
        other => conv_err(other, "DateTimeZone"),
    }
}

fn time_part(a: &[Value]) -> Res<Option<i64>> {
    Ok(match arg(a, 0) {
        Value::Null => None,
        Value::Time(ms) => Some(*ms),
        Value::DateTime(ms) | Value::DateTimeZone(ms, _) => Some(split_ms(*ms).1),
        other => return conv_err(other, "Time"),
    })
}

fn hour(_: &mut Interp, a: &[Value]) -> Res<Value> {
    Ok(time_part(a)?.map_or(Value::Null, |ms| Value::num((ms / 3_600_000) as f64)))
}

fn minute(_: &mut Interp, a: &[Value]) -> Res<Value> {
    Ok(time_part(a)?.map_or(Value::Null, |ms| Value::num((ms / 60_000 % 60) as f64)))
}

fn second(_: &mut Interp, a: &[Value]) -> Res<Value> {
    Ok(time_part(a)?.map_or(Value::Null, |ms| Value::num((ms % 60_000) as f64 / 1000.0)))
}

fn start_of_hour(_: &mut Interp, a: &[Value]) -> Res<Value> {
    Ok(match arg(a, 0) {
        Value::Null => Value::Null,
        Value::Time(ms) => Value::Time(ms - ms % 3_600_000),
        Value::DateTime(ms) => Value::DateTime(ms - ms.rem_euclid(3_600_000)),
        Value::DateTimeZone(ms, o) => Value::DateTimeZone(ms - ms.rem_euclid(3_600_000), *o),
        other => return conv_err(other, "Time"),
    })
}

fn end_of_hour(_: &mut Interp, a: &[Value]) -> Res<Value> {
    Ok(match arg(a, 0) {
        Value::Null => Value::Null,
        Value::Time(ms) => Value::Time(ms - ms % 3_600_000 + 3_599_999),
        Value::DateTime(ms) => Value::DateTime(ms - ms.rem_euclid(3_600_000) + 3_599_999),
        Value::DateTimeZone(ms, o) => Value::DateTimeZone(ms - ms.rem_euclid(3_600_000) + 3_599_999, *o),
        other => return conv_err(other, "Time"),
    })
}

fn duration_from_text(_: &mut Interp, a: &[Value]) -> Res<Value> {
    match opt_text(arg(a, 0))? {
        None => Ok(Value::Null),
        Some(s) => match convert::duration_from_text(s) {
            Some(ms) => Ok(Value::Duration(ms)),
            None => err(
                "DataFormat.Error",
                "We couldn't parse the input provided as a Duration value.",
            ),
        },
    }
}

fn duration_to_text(_: &mut Interp, a: &[Value]) -> Res<Value> {
    match arg(a, 0) {
        Value::Null => Ok(Value::Null),
        Value::Duration(ms) => Ok(Value::text(convert::duration_text(*ms))),
        other => conv_err(other, "Duration"),
    }
}

fn dur(a: &[Value], f: impl FnOnce(i64) -> f64) -> Res<Value> {
    match arg(a, 0) {
        Value::Null => Ok(Value::Null),
        Value::Duration(ms) => Ok(Value::num(f(*ms))),
        other => conv_err(other, "Duration"),
    }
}

fn duration_days(_: &mut Interp, a: &[Value]) -> Res<Value> {
    dur(a, |ms| (ms / MS_PER_DAY) as f64)
}

fn duration_hours(_: &mut Interp, a: &[Value]) -> Res<Value> {
    dur(a, |ms| (ms % MS_PER_DAY / 3_600_000) as f64)
}

fn duration_minutes(_: &mut Interp, a: &[Value]) -> Res<Value> {
    dur(a, |ms| (ms % 3_600_000 / 60_000) as f64)
}

fn duration_seconds(_: &mut Interp, a: &[Value]) -> Res<Value> {
    dur(a, |ms| (ms % 60_000) as f64 / 1000.0)
}

fn total_days(_: &mut Interp, a: &[Value]) -> Res<Value> {
    dur(a, |ms| ms as f64 / MS_PER_DAY as f64)
}

fn total_hours(_: &mut Interp, a: &[Value]) -> Res<Value> {
    dur(a, |ms| ms as f64 / 3_600_000.0)
}

fn total_minutes(_: &mut Interp, a: &[Value]) -> Res<Value> {
    dur(a, |ms| ms as f64 / 60_000.0)
}

fn total_seconds(_: &mut Interp, a: &[Value]) -> Res<Value> {
    dur(a, |ms| ms as f64 / 1000.0)
}
