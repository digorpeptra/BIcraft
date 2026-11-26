//! Calendar math of the proleptic Gregorian calendar, through the `jiff` crate (ROADMAP M11.9,
//! M11.43, docs/method.md decision 3). Every crate that turns day numbers into dates and back
//! (DAX, M, the readers and writers, the timestamps of logs and files) goes through these
//! functions.
//!
//! Two day numbers are used:
//! - *Unix days*: days from 1970-01-01;
//! - *serials* (OLE Automation dates): days from 1899-12-30, what DAX, M and the VertiPaq
//!   storage keep, with the time of day as the fraction. They have no 1900-02-29 (Excel's
//!   1900 leap-year bug is not part of them): serial 60 is 1900-02-28, 61 is 1900-03-01.
//!
//! jiff's `civil::Date` spans years −9999 to 9999, which holds every day DAX and M accept
//! (years 100 to 9999). Day numbers beyond that (only hostile input or carries past 9999 reach
//! them) are answered by the same calendar's 400-year-cycle arithmetic, [`fast`], which gives
//! jiff's answers inside its range (tested over every day of it). The hot paths of the storage
//! engine and of the readers use [`fast`] directly where it measured faster
//! (docs/formats/dax-functions.md, "Calendar").

use jiff::civil::Date;
use jiff::{SignedDuration, Span};

/// Unix day of 1899-12-30, serial 0.
pub const SERIAL_EPOCH: i64 = -25_569;

/// Serial of 0100-01-01, the first day DAX and M accept.
pub const SERIAL_MIN: i64 = -657_434;

/// Serial of 9999-12-31, the last day DAX and M accept.
pub const SERIAL_MAX: i64 = 2_958_465;

/// 1970-01-01, Unix day 0.
const UNIX_EPOCH: Date = Date::constant(1970, 1, 1);

/// Seconds in a civil day.
const DAY_SECS: i64 = 86_400;

/// The machine's local time minus UTC, in minutes, at the instant `unix_ms` (Unix milliseconds):
/// the operating system's time zone, daylight saving included (on Unix, `TZ` first, then
/// `/etc/localtime`; UTC when neither gives a zone). `None` outside jiff's range (years −9999 to
/// 9999).
#[cfg(feature = "clock")]
pub fn machine_offset_minutes(unix_ms: i64) -> Option<i32> {
    let at = jiff::Timestamp::from_millisecond(unix_ms).ok()?;
    Some(jiff::tz::TimeZone::system().to_offset(at).seconds() / 60)
}

/// The jiff date of a Unix day, inside jiff's range.
pub fn date_of_days(z: i64) -> Option<Date> {
    let span = SignedDuration::from_secs(z.checked_mul(DAY_SECS)?);
    UNIX_EPOCH.checked_add(span).ok()
}

/// The Unix day of a jiff date.
pub fn days_of_date(d: Date) -> i64 {
    d.duration_since(UNIX_EPOCH).as_secs() / DAY_SECS
}

/// The jiff date of a civil date, `None` outside jiff's range or for a day that does not exist.
fn date_of_civil(y: i64, m: i64, d: i64) -> Option<Date> {
    let (y, m, d) = (
        i16::try_from(y).ok()?,
        i8::try_from(m).ok()?,
        i8::try_from(d).ok()?,
    );
    Date::new(y, m, d).ok()
}

/// (year, month 1–12, day 1–31) of a Unix day.
pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
    match date_of_days(z) {
        Some(d) => (i64::from(d.year()), i64::from(d.month()), i64::from(d.day())),
        None => fast::civil_from_days(z),
    }
}

/// Unix day of a civil date. A month outside 1–12 carries into the years around (month 13 is
/// January of the next year, month 0 December of the previous one) and the day counts from the
/// first of that month (day 0 is the last day of the month before, day 32 runs into the next).
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let (y, m) = carry_month(y, m);
    let first = date_of_civil(y, m, 1).map_or_else(|| fast::days_from_civil(y, m, 1), days_of_date);
    first.saturating_add(d.saturating_sub(1))
}

/// (year, month 1–12) of a month that may be outside 1–12.
fn carry_month(y: i64, m: i64) -> (i64, i64) {
    let k = m.saturating_sub(1);
    (y.saturating_add(k.div_euclid(12)), k.rem_euclid(12) + 1)
}

/// (year, month 1–12, day 1–31) of a serial's day.
pub fn civil_from_serial(x: i64) -> (i64, i64, i64) {
    civil_from_days(x.saturating_add(SERIAL_EPOCH))
}

/// Serial of a civil date (months and days carry as in [`days_from_civil`]).
pub fn serial_from_civil(y: i64, m: i64, d: i64) -> i64 {
    days_from_civil(y, m, d).saturating_sub(SERIAL_EPOCH)
}

/// Serial of a valid civil date, `None` when the month or day does not exist.
pub fn serial_of_valid(y: i64, m: i64, d: i64) -> Option<i64> {
    let valid = (1..=12).contains(&m) && (1..=days_in_month(y, m)).contains(&d);
    valid.then(|| serial_from_civil(y, m, d))
}

/// Whether `y` is a leap year.
pub fn is_leap(y: i64) -> bool {
    match date_of_civil(y, 1, 1) {
        Some(d) => d.in_leap_year(),
        None => fast::is_leap(y),
    }
}

/// Days in month `m` (1–12) of year `y`; 0 for a month outside 1–12.
pub fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        2 if is_leap(y) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        1..=12 => 31,
        _ => 0,
    }
}

/// Day of the week of a serial: 0 = Sunday … 6 = Saturday (serial 0, 1899-12-30, is a
/// Saturday).
pub fn weekday_of_serial(x: i64) -> i64 {
    // the arithmetic: measured faster than a library's weekday, same answers (tested)
    fast::weekday_of_serial(x)
}

/// Day of the year (1 = 1 January) of a civil date.
pub fn day_of_year(y: i64, m: i64, d: i64) -> i64 {
    days_from_civil(y, m, d)
        .saturating_sub(days_from_civil(y, 1, 1))
        .saturating_add(1)
}

/// Months since year 0 of a serial's day (`year * 12 + month - 1`).
pub fn month_index(x: i64) -> i64 {
    let (y, m, _) = civil_from_serial(x);
    y.saturating_mul(12).saturating_add(m - 1)
}

/// Serial of the first day of month index `mi` (`year * 12 + month - 1`).
pub fn month_start(mi: i64) -> i64 {
    serial_from_civil(mi.div_euclid(12), mi.rem_euclid(12) + 1, 1)
}

/// Serial of the last day of month index `mi`.
pub fn month_end(mi: i64) -> i64 {
    month_start(mi.saturating_add(1)).saturating_sub(1)
}

/// The serial `k` months after `x`, its day of the month clipped to the target month's length
/// (EDATE, `Date.AddMonths`): 2024-01-31 plus one month is 2024-02-29.
pub fn add_months_clip(x: i64, k: i64) -> i64 {
    // jiff clips the day to the target month's length, as EDATE does
    let moved = date_of_days(x.saturating_add(SERIAL_EPOCH))
        .and_then(|d| d.checked_add(Span::new().try_months(k).ok()?).ok());
    if let Some(d) = moved {
        return days_of_date(d) - SERIAL_EPOCH;
    }
    let (y, m, d) = civil_from_serial(x);
    let mi = y.saturating_mul(12).saturating_add(m - 1).saturating_add(k);
    let (ty, tm) = (mi.div_euclid(12), mi.rem_euclid(12) + 1);
    serial_from_civil(ty, tm, d.min(days_in_month(ty, tm)))
}

/// Whether a serial's day is the last day of its month.
pub fn is_month_end(x: i64) -> bool {
    let (y, m, d) = civil_from_serial(x);
    d == days_in_month(y, m)
}

/// ISO 8601 week number (1–53) of a serial's day: weeks start on Monday and week 1 holds the
/// year's first Thursday.
pub fn iso_week(x: i64) -> i64 {
    match date_of_days(x.saturating_add(SERIAL_EPOCH)) {
        Some(d) => i64::from(d.iso_week_date().week()),
        None => fast::iso_week(x),
    }
}

/// The 400-year-cycle arithmetic of the same calendar (H. Hinnant's `days_from_civil` and
/// `civil_from_days`), for day numbers beyond jiff's range and for the hot loops where it
/// measured faster (the tests check it gives jiff's answers on every day jiff has).
pub mod fast {
    /// (year, month 1–12, day 1–31) of a Unix day.
    #[inline]
    pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
        let z = z.saturating_add(719_468);
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        (
            yoe.saturating_add(era.saturating_mul(400))
                .saturating_add(i64::from(m <= 2)),
            m,
            d,
        )
    }

    /// Unix day of a civil date with month 1–12 (the day counts from the first of the month).
    #[inline]
    pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
        let y = if m <= 2 { y.saturating_sub(1) } else { y };
        let era = y.div_euclid(400);
        let yoe = y.rem_euclid(400);
        let mp = (m + 9).rem_euclid(12);
        let doy = d.saturating_sub(1).saturating_add((153 * mp + 2) / 5);
        let doe = doy.saturating_add(yoe * 365 + yoe / 4 - yoe / 100);
        era.saturating_mul(146_097)
            .saturating_add(doe.saturating_sub(719_468))
    }

    /// (year, month, day) of a serial's day.
    #[inline]
    pub fn civil_from_serial(x: i64) -> (i64, i64, i64) {
        civil_from_days(x.saturating_add(super::SERIAL_EPOCH))
    }

    /// Whether `y` is a leap year.
    #[inline]
    pub fn is_leap(y: i64) -> bool {
        (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
    }

    /// Day of the week of a serial, 0 = Sunday.
    #[inline]
    pub fn weekday_of_serial(x: i64) -> i64 {
        x.rem_euclid(7).saturating_add(6) % 7
    }

    /// ISO 8601 week of a serial's day.
    pub fn iso_week(x: i64) -> i64 {
        let mon0 = (weekday_of_serial(x) + 6) % 7;
        let thursday = x.saturating_sub(mon0).saturating_add(3);
        let (ty, _, _) = civil_from_serial(thursday);
        let jan1 = days_from_civil(ty, 1, 1).saturating_sub(super::SERIAL_EPOCH);
        thursday.saturating_sub(jan1) / 7 + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn same_day(z: i64) {
        let d = date_of_days(z).unwrap();
        let civil = (i64::from(d.year()), i64::from(d.month()), i64::from(d.day()));
        assert_eq!(fast::civil_from_days(z), civil, "{z}");
        assert_eq!(fast::days_from_civil(civil.0, civil.1, civil.2), z, "{d}");
        assert_eq!(civil_from_days(z), civil, "{z}");
        assert_eq!(days_from_civil(civil.0, civil.1, civil.2), z, "{d}");
    }

    /// The arithmetic and jiff agree: on every day DAX and M accept, every day of the first
    /// and last thousand of jiff's range, and every 97th day in between.
    #[test]
    fn fast_path_matches_jiff_on_its_whole_range() {
        let lo = days_of_date(Date::MIN);
        let hi = days_of_date(Date::MAX);
        assert_eq!(date_of_days(lo - 1), None);
        assert_eq!(date_of_days(hi + 1), None);
        // jiff stops at 9999-12-31, the last day DAX and M accept
        assert_eq!(hi, SERIAL_MAX + SERIAL_EPOCH);
        for z in (SERIAL_MIN + SERIAL_EPOCH - 1000)..=hi {
            same_day(z);
        }
        for z in (lo..lo + 1000).chain(hi - 1000..=hi).chain((lo..=hi).step_by(97)) {
            same_day(z);
        }
        // beyond jiff's range the arithmetic continues the calendar
        assert_eq!(civil_from_days(hi + 1), fast::civil_from_days(hi + 1));
        assert_eq!(civil_from_days(lo - 1), fast::civil_from_days(lo - 1));
        assert_eq!(fast::civil_from_days(hi + 1).0, i64::from(Date::MAX.year()) + 1);
        for y in i64::from(Date::MIN.year())..=i64::from(Date::MAX.year()) {
            assert_eq!(fast::is_leap(y), is_leap(y), "{y}");
        }
        for y in [-300_000, -10_000, 10_000, 262_143, 300_000] {
            assert_eq!(is_leap(y), fast::is_leap(y), "{y}");
            for m in 1..=12 {
                let last = days_in_month(y, m);
                assert!(serial_of_valid(y, m, last).is_some(), "{y}-{m}");
                assert_eq!(serial_of_valid(y, m, last + 1), None, "{y}-{m}");
            }
        }
        // jiff's month arithmetic clips the day as the civil arithmetic does
        for x in (SERIAL_MIN..=SERIAL_MAX).step_by(13) {
            for k in [-1200, -13, -1, 1, 11, 12, 13, 1200] {
                let (y, m, d) = civil_from_serial(x);
                let mi = y * 12 + m - 1 + k;
                let (ty, tm) = (mi.div_euclid(12), mi.rem_euclid(12) + 1);
                let want = serial_from_civil(ty, tm, d.min(days_in_month(ty, tm)));
                assert_eq!(add_months_clip(x, k), want, "{x} {k}");
            }
        }
        for x in SERIAL_MIN - 400..=SERIAL_MAX {
            let d = date_of_days(x + SERIAL_EPOCH).unwrap();
            assert_eq!(
                fast::weekday_of_serial(x),
                i64::from(d.weekday().to_sunday_zero_offset()),
                "{x}"
            );
            assert_eq!(fast::iso_week(x), iso_week(x), "{x}");
        }
    }

    #[test]
    fn serial_epoch_and_known_days() {
        assert_eq!(civil_from_serial(0), (1899, 12, 30));
        // no 1900-02-29 in serials (unlike Excel's 1900 date system)
        assert_eq!(civil_from_serial(60), (1900, 2, 28));
        assert_eq!(civil_from_serial(61), (1900, 3, 1));
        assert_eq!(serial_from_civil(100, 1, 1), SERIAL_MIN);
        assert_eq!(serial_from_civil(9999, 12, 31), SERIAL_MAX);
        assert_eq!(serial_from_civil(2025, 1, 1), 45_658);
        assert_eq!(weekday_of_serial(0), 6);
        assert_eq!(serial_of_valid(2023, 2, 29), None);
        assert_eq!(serial_of_valid(2024, 2, 29), Some(45_351));
        assert!(!is_leap(1900) && is_leap(2000) && !is_leap(2100) && is_leap(2024));
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2024, 13), 0);
    }

    #[test]
    fn months_and_days_carry() {
        assert_eq!(serial_from_civil(2024, 13, 1), serial_from_civil(2025, 1, 1));
        assert_eq!(serial_from_civil(2024, 0, 1), serial_from_civil(2023, 12, 1));
        assert_eq!(serial_from_civil(2024, -25, 1), serial_from_civil(2021, 11, 1));
        assert_eq!(serial_from_civil(2024, 3, 0), serial_from_civil(2024, 2, 29));
        assert_eq!(serial_from_civil(2024, 2, 30), serial_from_civil(2024, 3, 1));
        assert_eq!(
            add_months_clip(serial_from_civil(2024, 1, 31), 1),
            serial_from_civil(2024, 2, 29)
        );
        assert_eq!(month_end(2024 * 12 + 1), serial_from_civil(2024, 2, 29));
    }

    /// jiff against the arithmetic on the conversions the hot loops make (run with
    /// `cargo test -p bicraft-locale calendar_speed -- --ignored --nocapture`).
    #[test]
    #[ignore = "timing run"]
    fn calendar_speed() {
        use std::hint::black_box;
        use std::time::Instant;
        let days: Vec<i64> = (0..20_000_000i64)
            .map(|i| (i * 7919) % 3_600_000 - 657_434)
            .collect();
        let time = |name: &str, f: &dyn Fn(i64) -> i64| {
            let t = Instant::now();
            let mut acc = 0i64;
            for &x in &days {
                acc = acc.wrapping_add(f(black_box(x)));
            }
            println!(
                "{name}: {:.2} ns/day ({acc})",
                t.elapsed().as_secs_f64() * 1e9 / days.len() as f64
            );
        };
        time("jiff civil_from_serial", &|x| {
            let (y, m, d) = civil_from_serial(x);
            y + m + d
        });
        time("fast civil_from_serial", &|x| {
            let (y, m, d) = fast::civil_from_serial(x);
            y + m + d
        });
        time("jiff serial_from_civil", &|x| {
            serial_from_civil(2000 + x % 7000, 1 + x % 12, 1 + x % 28)
        });
        time("fast days_from_civil", &|x| {
            fast::days_from_civil(2000 + x % 7000, 1 + x % 12, 1 + x % 28) - SERIAL_EPOCH
        });
        time("jiff add_months_clip", &|x| add_months_clip(x, 13));
        time("jiff weekday", &weekday_of_serial);
        time("fast weekday", &fast::weekday_of_serial);
        time("jiff iso_week", &iso_week);
        time("fast iso_week", &fast::iso_week);
    }

    /// Hostile day numbers stay answers, never panics.
    #[test]
    fn extreme_day_numbers_answer() {
        for z in [
            i64::MIN,
            i64::MIN / 2,
            -1_000_000_000_000,
            1_000_000_000_000,
            i64::MAX / 2,
            i64::MAX,
        ] {
            let _ = civil_from_days(z);
            let _ = civil_from_serial(z);
            let _ = weekday_of_serial(z);
            let _ = iso_week(z);
            let _ = add_months_clip(z, i64::MAX);
            let _ = month_end(z);
            let _ = month_index(z);
        }
        for y in [i64::MIN, i64::MAX, -1_000_000, 1_000_000] {
            let _ = days_from_civil(y, i64::MIN, i64::MAX);
            let _ = days_in_month(y, 2);
            let _ = is_leap(y);
        }
    }

    #[cfg(feature = "clock")]
    #[test]
    fn the_machine_offset_is_a_real_offset() {
        for ms in [0, 1_000_000_000_000, 1_790_000_000_000] {
            let off = machine_offset_minutes(ms).unwrap();
            assert!(off.abs() <= 14 * 60, "{off}");
        }
        assert_eq!(machine_offset_minutes(i64::MAX), None);
    }
}
