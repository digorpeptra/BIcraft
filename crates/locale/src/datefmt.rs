//! .NET-style custom date and time patterns (`yyyy-MM-dd`, `dddd d MMMM yyyy`, `HH:mm:ss.fff`,
//! `h:mm tt`, `zzz`), with the names and separators of a culture.
//!
//! Tokens: `d`/`dd` day, `ddd`/`dddd` day name, `M`/`MM` month, `MMM`/`MMMM` month name (the
//! genitive form when the pattern also has a day number, as .NET does), `y`… `yyyyy` year,
//! `h`/`hh` 12-hour, `H`/`HH` 24-hour, `m`/`mm`, `s`/`ss`, `f`… / `F`… fractions, `t`/`tt`
//! designator, `z`/`zz`/`zzz` offset, `g` era (`A.D.`), `/` date separator, `:` time
//! separator, `'…'` / `"…"` literal text, `\x` the character `x`, `%x` the single token `x`.

use crate::{Locale, calendar};

/// Names and separators a pattern prints.
#[derive(Debug, Clone, Copy)]
pub struct Names<'a> {
    pub months: &'a [&'a str; 12],
    pub months_genitive: &'a [&'a str; 12],
    pub months_abbr: &'a [&'a str; 12],
    pub months_abbr_genitive: &'a [&'a str; 12],
    pub days: &'a [&'a str; 7],
    pub days_abbr: &'a [&'a str; 7],
    pub am: &'a str,
    pub pm: &'a str,
    pub date_separator: &'a str,
    pub time_separator: &'a str,
}

/// A date and time broken into fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateParts {
    pub year: i64,
    /// 1–12.
    pub month: u32,
    /// 1–31.
    pub day: u32,
    /// 0 = Sunday.
    pub weekday: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    /// Fraction of the second in units of 10⁻⁷ s (.NET ticks).
    pub ticks: u32,
    /// UTC offset in minutes, for `z` tokens (0 when absent).
    pub offset: Option<i32>,
}

/// Days from 1970-01-01 to a proleptic Gregorian date ([`calendar::days_from_civil`]).
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    calendar::days_from_civil(y, i64::from(m), i64::from(d))
}

/// The date of a day count from 1970-01-01: (year, month, day) ([`calendar::civil_from_days`]).
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let (y, m, d) = calendar::civil_from_days(z);
    (y, m as u32, d as u32)
}

/// OLE day 0 (1899-12-30) in days from 1970-01-01.
const OLE_EPOCH: i64 = calendar::SERIAL_EPOCH;

/// Bound of the day numbers [`DateParts::from_days_ms`] reads.
const MAX_DAYS: i64 = 100_000_000;

/// Largest OLE serial accepted (year 9999).
const MAX_SERIAL: f64 = 2_958_466.0;

impl DateParts {
    /// The parts of an OLE Automation date serial (days since 1899-12-30, the fraction is the
    /// time of day), rounded to the millisecond. `None` outside years 100–9999 or for NaN.
    pub fn from_serial(x: f64) -> Option<DateParts> {
        if !x.is_finite() || !(-657_435.0..MAX_SERIAL).contains(&x) {
            return None;
        }
        let ms_total = (x * 86_400_000.0).round();
        if !ms_total.is_finite() {
            return None;
        }
        let ms_total = ms_total as i64;
        let day = ms_total.div_euclid(86_400_000);
        let ms = ms_total.rem_euclid(86_400_000);
        Some(DateParts::from_days_ms(day, ms))
    }

    /// The parts of an OLE day number and the milliseconds of that day.
    /// Days beyond ±100 million (far outside any calendar Power BI accepts) are clamped.
    pub fn from_days_ms(ole_day: i64, ms: i64) -> DateParts {
        let ole_day = ole_day.clamp(-MAX_DAYS, MAX_DAYS);
        let (year, month, day) = civil_from_days(ole_day + OLE_EPOCH);
        let ms = ms.clamp(0, 86_399_999);
        DateParts {
            year,
            month,
            day,
            weekday: calendar::weekday_of_serial(ole_day) as u32,
            hour: (ms / 3_600_000) as u32,
            minute: (ms / 60_000 % 60) as u32,
            second: (ms / 1000 % 60) as u32,
            ticks: (ms % 1000) as u32 * 10_000,
            offset: None,
        }
    }

    /// Day of the year (1 = 1 January).
    pub fn day_of_year(&self) -> i64 {
        calendar::day_of_year(self.year, i64::from(self.month), i64::from(self.day))
    }
}

fn idx(n: u32) -> usize {
    usize::try_from(n).unwrap_or(0)
}

fn pad(out: &mut String, v: i64, width: usize) {
    let s = v.unsigned_abs().to_string();
    if v < 0 {
        out.push('-');
    }
    for _ in s.len()..width {
        out.push('0');
    }
    out.push_str(&s);
}

/// True when the pattern has a day-number token (`d` or `dd` outside quotes): month names are
/// then genitive.
fn has_day_number(chars: &[char]) -> bool {
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        match c {
            '\'' | '"' => {
                i += 1;
                while chars.get(i).is_some_and(|x| *x != c) {
                    i += 1;
                }
                i += 1;
            }
            '\\' => i += 2,
            'd' => {
                let n = chars
                    .get(i..)
                    .map_or(0, |r| r.iter().take_while(|x| **x == 'd').count());
                if n <= 2 {
                    return true;
                }
                i += n;
            }
            _ => i += 1,
        }
    }
    false
}

/// Most characters a formatted pattern may produce.
const MAX_OUT: usize = 4096;

/// Formats `p` with a .NET custom pattern.
pub fn format_pattern(p: &DateParts, pattern: &str, names: &Names) -> String {
    let chars: Vec<char> = pattern.chars().take(MAX_OUT).collect();
    let genitive = has_day_number(&chars);
    let mut out = String::new();
    let mut i = 0usize;
    while let Some(&ch) = chars.get(i) {
        if out.len() > MAX_OUT {
            break;
        }
        let mut n = 1usize;
        while chars.get(i + n) == Some(&ch) {
            n += 1;
        }
        let h12 = if p.hour.is_multiple_of(12) {
            12
        } else {
            p.hour % 12
        };
        match ch {
            'd' => match n {
                1 => pad(&mut out, i64::from(p.day), 1),
                2 => pad(&mut out, i64::from(p.day), 2),
                3 => out.push_str(names.days_abbr.get(idx(p.weekday)).copied().unwrap_or("")),
                _ => out.push_str(names.days.get(idx(p.weekday)).copied().unwrap_or("")),
            },
            'M' => {
                let m = idx(p.month.saturating_sub(1));
                match n {
                    1 => pad(&mut out, i64::from(p.month), 1),
                    2 => pad(&mut out, i64::from(p.month), 2),
                    3 => {
                        let list = if genitive {
                            names.months_abbr_genitive
                        } else {
                            names.months_abbr
                        };
                        out.push_str(list.get(m).copied().unwrap_or(""));
                    }
                    _ => {
                        let list = if genitive {
                            names.months_genitive
                        } else {
                            names.months
                        };
                        out.push_str(list.get(m).copied().unwrap_or(""));
                    }
                }
            }
            'y' => {
                if n <= 2 {
                    pad(&mut out, p.year.rem_euclid(100), n);
                } else {
                    pad(&mut out, p.year, n.min(9));
                }
            }
            'h' => pad(&mut out, i64::from(h12), n.min(2)),
            'H' => pad(&mut out, i64::from(p.hour), n.min(2)),
            'm' => pad(&mut out, i64::from(p.minute), n.min(2)),
            's' => pad(&mut out, i64::from(p.second), n.min(2)),
            'f' | 'F' => {
                let digits = format!("{:07}", p.ticks.min(9_999_999));
                let mut t: String = digits.chars().take(n.min(7)).collect();
                if ch == 'F' {
                    t = t.trim_end_matches('0').to_string();
                    // `.FFF` with no fraction drops the separator before it.
                    if t.is_empty() && out.ends_with('.') {
                        out.pop();
                    }
                }
                out.push_str(&t);
            }
            't' => {
                let d = if p.hour < 12 { names.am } else { names.pm };
                if n == 1 {
                    if let Some(c) = d.chars().next() {
                        out.push(c);
                    }
                } else {
                    out.push_str(d);
                }
            }
            'z' => {
                let o = p.offset.unwrap_or(0);
                let sign = if o < 0 { '-' } else { '+' };
                let a = o.unsigned_abs();
                out.push(sign);
                match n {
                    1 => out.push_str(&(a / 60).to_string()),
                    2 => out.push_str(&format!("{:02}", a / 60)),
                    _ => out.push_str(&format!("{:02}:{:02}", a / 60, a % 60)),
                }
            }
            'g' => out.push_str(if p.year > 0 { "A.D." } else { "B.C." }),
            '\'' | '"' => {
                let mut j = i + 1;
                while let Some(&q) = chars.get(j) {
                    if q == ch {
                        break;
                    }
                    if q == '\\' {
                        j += 1;
                        if let Some(&e) = chars.get(j) {
                            out.push(e);
                        }
                    } else {
                        out.push(q);
                    }
                    j += 1;
                }
                i = j + 1;
                continue;
            }
            '\\' => {
                if let Some(&next) = chars.get(i + 1) {
                    out.push(next);
                }
                i += 2;
                continue;
            }
            '%' => {
                // `%d`: the single-letter token on its own.
                i += 1;
                continue;
            }
            '/' => {
                for _ in 0..n {
                    out.push_str(names.date_separator);
                }
            }
            ':' => {
                for _ in 0..n {
                    out.push_str(names.time_separator);
                }
            }
            other => {
                for _ in 0..n {
                    out.push(other);
                }
            }
        }
        i += n;
    }
    out
}

/// .NET standard date/time formats of one letter (`d`, `D`, `t`, `T`, `f`, `F`, `g`, `G`, `M`,
/// `Y`, `s`, `u`); `None` for others (`o`, `r`, which callers handle, or custom patterns).
pub fn standard_pattern(spec: &str, loc: &Locale) -> Option<String> {
    let (date, time) = (loc.short_date, loc.long_time);
    Some(match spec {
        "d" => date.to_string(),
        "D" => loc.long_date.to_string(),
        "t" => loc.short_time.to_string(),
        "T" => time.to_string(),
        "f" => format!("{} {}", loc.long_date, loc.short_time),
        "F" => format!("{} {}", loc.long_date, time),
        "g" => format!("{} {}", date, loc.short_time),
        "G" => format!("{date} {time}"),
        "M" | "m" => loc.month_day.to_string(),
        "Y" | "y" => loc.year_month.to_string(),
        "s" => "yyyy'-'MM'-'dd'T'HH':'mm':'ss".to_string(),
        "u" => "yyyy'-'MM'-'dd HH':'mm':'ss'Z'".to_string(),
        _ => return None,
    })
}

/// Formats with a standard (one letter) or custom .NET pattern in culture `loc`.
pub fn format(p: &DateParts, spec: &str, loc: &Locale) -> String {
    let names = loc.names();
    match standard_pattern(spec, loc) {
        Some(pattern) => format_pattern(p, &pattern, &names),
        None => format_pattern(p, spec, &names),
    }
}
