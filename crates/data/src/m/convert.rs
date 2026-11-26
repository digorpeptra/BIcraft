//! Culture-aware conversions between M values and text, and the type
//! conversions of `Table.TransformColumnTypes` / `*.From`.

use super::decimal::Dec;
use super::eval::MS_PER_DAY;
use super::value::{Facet, Kind, MType, Num, Res, Value, err};
use crate::value::{self as cv, DateOrder};
use bicraft_locale::Locale;
use bicraft_locale::datefmt::{self, DateParts};

/// The culture settings the M library needs: the `bicraft-locale` culture (separators, date
/// and time patterns, CLDR month and day names) plus the date order its parsers use.
#[derive(Debug, Clone)]
pub(crate) struct CultureInfo {
    pub name: String,
    pub loc: &'static Locale,
    pub decimal: char,
    pub group: char,
    pub order: DateOrder,
    pub date_sep: &'static str,
    pub months: [&'static str; 12],
    pub days: [&'static str; 7],
    /// First day of the week: 0 = Sunday, 1 = Monday.
    pub first_day: u8,
}

const EN_MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

impl CultureInfo {
    /// Settings for a culture name (`fr-FR`); a culture bicraft has no data for takes its
    /// language's, else en-US's; an empty name is the invariant culture.
    pub fn from_name(name: &str) -> Self {
        let loc = Locale::get(name);
        let order = if loc.year_first() {
            DateOrder::Ymd
        } else if loc.day_first() {
            DateOrder::Dmy
        } else {
            DateOrder::Mdy
        };
        Self {
            name: name.trim().to_string(),
            loc,
            decimal: loc.decimal,
            group: loc.group,
            order,
            date_sep: loc.date_separator,
            months: loc.months,
            days: loc.days,
            first_day: loc.first_day,
        }
    }

    fn parse_culture(&self) -> crate::Culture {
        crate::Culture {
            decimal_separator: self.decimal,
            date_order: self.order,
            currency_symbol: self.loc.pq.currency.symbol,
        }
    }
}

// ---------------------------------------------------------------------------
// Calendar helpers.

pub(crate) fn ymd(day: i64) -> (i64, i64, i64) {
    bicraft_locale::calendar::civil_from_serial(day)
}

pub(crate) fn make_date(y: i64, m: i64, d: i64) -> Res<i64> {
    match cv::ole_date(y, m, d) {
        Ok(v) => Ok(v),
        Err(e) => err("Expression.Error", format!("The date is not valid: {e}.")),
    }
}

/// Day of week of an OLE day: 0 = Sunday.
pub(crate) fn weekday(day: i64) -> i64 {
    bicraft_locale::calendar::weekday_of_serial(day)
}

pub(crate) fn days_in_month(y: i64, m: i64) -> i64 {
    bicraft_locale::calendar::days_in_month(y, m)
}

pub(crate) fn split_ms(ms: i64) -> (i64, i64) {
    (ms.div_euclid(MS_PER_DAY), ms.rem_euclid(MS_PER_DAY))
}

// ---------------------------------------------------------------------------
// Numbers to text.

/// .NET-style general format with `digits` significant digits (`G15`).
pub(crate) fn format_general(n: f64, digits: usize, culture: &CultureInfo) -> String {
    if n.is_nan() {
        return "NaN".into();
    }
    if n.is_infinite() {
        return if n > 0.0 {
            "\u{221e}".into()
        } else {
            "-\u{221e}".into()
        };
    }
    if n == 0.0 {
        return "0".into();
    }
    let digits = digits.clamp(1, 17);
    let (mant_digits, exp): (String, i32) = if digits <= 15 {
        // .NET Framework: 15 significant digits first, then rounded half away from zero.
        let (d, point) = number_digits(Num::F(n));
        let (d, point) = round_digits(d, point, digits);
        (digit_text(&d), i32::try_from(point - 1).unwrap_or(0))
    } else {
        let sci = format!("{:.*e}", digits - 1, n);
        let (mant, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
        (
            mant.chars().filter(char::is_ascii_digit).collect(),
            exp.parse().unwrap_or(0),
        )
    };
    let neg = n < 0.0;
    let mant_digits = mant_digits.trim_end_matches('0');
    let mant_digits = if mant_digits.is_empty() { "0" } else { mant_digits };
    let mut s = String::new();
    if neg {
        s.push('-');
    }
    if exp >= digits as i32 || exp < -4 {
        let (first, rest) = mant_digits.split_at(1);
        s.push_str(first);
        if !rest.is_empty() {
            s.push(culture.decimal);
            s.push_str(rest);
        }
        s.push('E');
        s.push(if exp < 0 { '-' } else { '+' });
        s.push_str(&format!("{:02}", exp.unsigned_abs()));
        return s;
    }
    if exp < 0 {
        s.push('0');
        s.push(culture.decimal);
        for _ in 0..(-exp - 1) {
            s.push('0');
        }
        s.push_str(mant_digits);
        return s;
    }
    let int_len = exp as usize + 1;
    if mant_digits.len() <= int_len {
        s.push_str(mant_digits);
        for _ in mant_digits.len()..int_len {
            s.push('0');
        }
    } else {
        let (a, b) = mant_digits.split_at(int_len);
        s.push_str(a);
        s.push(culture.decimal);
        s.push_str(b);
    }
    s
}

/// Default number → text (`Text.From`, `Number.ToText` without a format), as Desktop prints
/// it:
/// - decimals with every digit of their scale (`2.680`, `123456789012345678`), no sign on
///   zero;
/// - whole doubles in full (`100000000000000000000`);
/// - other doubles in the .NET Framework round-trip form (15 significant digits when they
///   read back to the same double, else 17: `0.33333333333333331`).
pub(crate) fn number_to_text(n: Num, culture: &CultureInfo) -> String {
    let n = match n {
        Num::D(d) => return d.to_text(culture.decimal),
        Num::F(x) => x,
    };
    if n == 0.0 {
        return "0".into();
    }
    // Whole numbers in the decimal range are written out in full.
    if n.is_finite() && n.fract() == 0.0 && n.abs() < 79_228_162_514_264_337_593_543_950_336.0 {
        return format!("{n:.0}");
    }
    let dot = CultureInfo::from_name("");
    let g15 = format_general(n, 15, &dot);
    let s = if g15.parse::<f64>().ok() == Some(n) {
        g15
    } else {
        format_general(n, 17, &dot)
    };
    if culture.decimal == '.' {
        s
    } else {
        s.replace('.', &culture.decimal.to_string())
    }
}

/// The significant digits of `|n|` (no leading zeros) and the number of digits before the
/// decimal point (`12.5` → `[1, 2, 5]`, 2; `0.05` → `[5]`, -1): doubles limited to 15
/// significant digits (as .NET Framework formats them), decimals exactly. Zero, NaN and
/// infinities give no digits.
fn number_digits(n: Num) -> (Vec<u8>, i64) {
    match n {
        Num::D(d) => {
            let (digits, point) = d.digits();
            let lead = digits.iter().take_while(|d| **d == 0).count();
            if lead == digits.len() {
                return (Vec::new(), 0);
            }
            let rest = digits.get(lead..).unwrap_or(&[]).to_vec();
            (rest, point - lead as i64)
        }
        Num::F(x) => {
            if !x.is_finite() || x == 0.0 {
                return (Vec::new(), 0);
            }
            let sci = format!("{:.14e}", x.abs());
            let (mant, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
            let exp: i64 = exp.parse().unwrap_or(0);
            let digits = mant
                .bytes()
                .filter(u8::is_ascii_digit)
                .map(|b| b - b'0')
                .collect();
            (digits, exp + 1)
        }
    }
}

fn is_negative(n: Num) -> bool {
    match n {
        Num::D(d) => d.is_negative(),
        Num::F(x) => x < 0.0,
    }
}

/// Keeps the first `keep` digits, rounding half away from zero (how .NET rounds formatted
/// numbers); a carry adds a leading digit and moves the point.
fn round_digits(mut digits: Vec<u8>, mut point: i64, keep: usize) -> (Vec<u8>, i64) {
    if keep >= digits.len() {
        return (digits, point);
    }
    let round_up = digits.get(keep).is_some_and(|d| *d >= 5);
    digits.truncate(keep);
    if round_up {
        let mut i = keep;
        loop {
            if i == 0 {
                digits.insert(0, 1);
                point += 1;
                break;
            }
            i -= 1;
            if let Some(d) = digits.get_mut(i) {
                if *d == 9 {
                    *d = 0;
                } else {
                    *d += 1;
                    break;
                }
            }
        }
    }
    (digits, point)
}

/// Digits of `|n|` rounded half away from zero at `decimals` places (see
/// [`number_digits`]). Returns the integer and fraction digits.
fn fixed_digits(n: Num, decimals: usize) -> (String, String) {
    let decimals = decimals.min(30);
    let (digits, point) = number_digits(n);
    if digits.is_empty() {
        return ("0".into(), "0".repeat(decimals));
    }
    // Keep the digits down to 10^-decimals.
    let keep = point + decimals as i64;
    if keep < 0 {
        return ("0".into(), "0".repeat(decimals));
    }
    let (mut digits, point) = round_digits(digits, point, keep as usize);
    let total = (point + decimals as i64).max(0) as usize;
    digits.resize(total.max(digits.len()), 0);
    let to_s = |ds: &[u8]| ds.iter().map(|d| char::from(b'0' + d)).collect::<String>();
    if point <= 0 {
        let lead = (-point) as usize;
        let mut frac = "0".repeat(lead.min(decimals));
        frac.push_str(&to_s(digits.get(..decimals.saturating_sub(lead)).unwrap_or(&[])));
        frac.truncate(decimals);
        while frac.len() < decimals {
            frac.push('0');
        }
        return ("0".into(), frac);
    }
    let p = point as usize;
    let int = to_s(digits.get(..p).unwrap_or(&[]));
    let mut frac = to_s(digits.get(p..).unwrap_or(&[]));
    frac.truncate(decimals);
    while frac.len() < decimals {
        frac.push('0');
    }
    (if int.is_empty() { "0".into() } else { int }, frac)
}

/// `int` with the culture's group separator (groups of three, or 3 then 2 in Indian grouping).
fn group_in(int: &str, c: &CultureInfo) -> String {
    if !c.loc.pq.indian_grouping {
        return group_digits(int, c.group);
    }
    let len = int.chars().count();
    let mut out = String::new();
    for (i, ch) in int.chars().enumerate() {
        let left = len - i;
        if i > 0 && (left == 3 || (left > 3 && (left - 3).is_multiple_of(2))) {
            out.push(c.group);
        }
        out.push(ch);
    }
    out
}

/// `Number.ToText(n, "C")`: the culture's currency layout.
fn currency_text(n: Num, decimals: Option<usize>, c: &CultureInfo) -> String {
    let cur = c.loc.pq.currency;
    let (int, frac) = fixed_digits(n, decimals.unwrap_or(usize::from(cur.digits)));
    let is_zero = int.chars().chain(frac.chars()).all(|ch| ch == '0');
    let affix = if is_negative(n) && !is_zero {
        cur.negative
    } else {
        cur.positive
    };
    let mut out = String::from(affix.prefix);
    out.push_str(&group_digits(&int, cur.group));
    if !frac.is_empty() {
        out.push(cur.decimal);
        out.push_str(&frac);
    }
    out.push_str(affix.suffix);
    out
}

fn group_digits(int: &str, group: char) -> String {
    let mut out = String::new();
    let len = int.chars().count();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            out.push(group);
        }
        out.push(c);
    }
    out
}

fn fixed(n: Num, decimals: usize, culture: &CultureInfo, grouped: bool) -> String {
    let (int, frac) = fixed_digits(n, decimals);
    let mut out = String::new();
    let is_zero = int.chars().chain(frac.chars()).all(|c| c == '0');
    if is_negative(n) && !is_zero {
        out.push('-');
    }
    out.push_str(&if grouped {
        group_in(&int, culture)
    } else {
        int.clone()
    });
    if !frac.is_empty() {
        out.push(culture.decimal);
        out.push_str(&frac);
    }
    out
}

/// `n × 100` for the percent formats: exact for decimals.
fn times_100(n: Num) -> Num {
    match n {
        Num::D(d) => match Dec::from_i128(100).and_then(|h| d.mul(h).ok()) {
            Some(r) => Num::D(r),
            None => Num::F(d.to_f64() * 100.0),
        },
        Num::F(x) => Num::F(x * 100.0),
    }
}

fn digit_text(ds: &[u8]) -> String {
    ds.iter().map(|d| char::from(b'0' + d)).collect()
}

/// .NET general format of a decimal with `prec` significant digits (`G5`): rounded half away
/// from zero, scientific when the exponent is at least `prec` or below -5.
fn decimal_general(d: Dec, prec: usize, culture: &CultureInfo) -> String {
    let (digits, point) = number_digits(Num::D(d));
    if digits.is_empty() {
        return "0".into();
    }
    let (mut digits, point) = round_digits(digits, point, prec.max(1));
    while digits.len() > 1 && digits.last() == Some(&0) {
        digits.pop();
    }
    let exp = point - 1;
    let mut s = String::new();
    if d.is_negative() {
        s.push('-');
    }
    if exp >= prec as i64 || exp < -5 {
        let (first, rest) = digits.split_at(1);
        s.push_str(&digit_text(first));
        if !rest.is_empty() {
            s.push(culture.decimal);
            s.push_str(&digit_text(rest));
        }
        s.push('E');
        s.push(if exp < 0 { '-' } else { '+' });
        s.push_str(&format!("{:02}", exp.unsigned_abs()));
        return s;
    }
    if point <= 0 {
        s.push('0');
        s.push(culture.decimal);
        s.push_str(&"0".repeat(point.unsigned_abs() as usize));
        s.push_str(&digit_text(&digits));
        return s;
    }
    let p = point as usize;
    if digits.len() <= p {
        s.push_str(&digit_text(&digits));
        s.push_str(&"0".repeat(p - digits.len()));
    } else {
        let (a, b) = digits.split_at(p);
        s.push_str(&digit_text(a));
        s.push(culture.decimal);
        s.push_str(&digit_text(b));
    }
    s
}

/// .NET exponential format with `p` decimals (`E3` → `1.235E+017`): decimals rounded half
/// away from zero from their exact digits.
fn decimal_exponential(d: Dec, p: usize, lower: bool, culture: &CultureInfo) -> String {
    let p = p.min(60);
    let (digits, point) = number_digits(Num::D(d));
    let (mut digits, point) = if digits.is_empty() {
        (vec![0], 1)
    } else {
        round_digits(digits, point, p + 1)
    };
    digits.resize(p + 1, 0);
    let exp = if d.is_zero() { 0 } else { point - 1 };
    let mut s = String::new();
    if d.is_negative() {
        s.push('-');
    }
    for (i, c) in digits.iter().enumerate() {
        if i == 1 {
            s.push(culture.decimal);
        }
        s.push(char::from(b'0' + c));
    }
    s.push(if lower { 'e' } else { 'E' });
    s.push(if exp < 0 { '-' } else { '+' });
    s.push_str(&format!("{:03}", exp.unsigned_abs()));
    s
}

/// A whole number for the `D` and `X` formats.
fn whole_number(n: Num) -> Option<i128> {
    match n {
        Num::D(d) => d.to_i128(),
        Num::F(x) => super::eval::whole(x).map(i128::from),
    }
}

/// `Number.ToText(n, format)`: standard formats `G`, `R`, `F`, `N`, `P`, `D`, `E`, `X` (with
/// precision) and simple custom patterns (`0`, `#`, `.`, `,`, `%`). Decimals format from their
/// exact digits, doubles from 15 significant digits.
pub(crate) fn format_number(n: Num, fmt: &str, culture: &CultureInfo) -> Option<String> {
    let mut chars = fmt.chars();
    let first = chars.next()?;
    let rest: String = chars.collect();
    if first.is_ascii_alphabetic() && (rest.is_empty() || rest.bytes().all(|b| b.is_ascii_digit())) {
        let prec: Option<usize> = if rest.is_empty() { None } else { rest.parse().ok() };
        return Some(match first.to_ascii_uppercase() {
            'G' => match n {
                Num::D(d) => match prec.filter(|p| *p > 0) {
                    None => d.to_text(culture.decimal),
                    Some(p) => decimal_general(d, p, culture),
                },
                Num::F(x) => format_general(x, prec.filter(|p| *p > 0).unwrap_or(15), culture),
            },
            'R' => match n {
                Num::D(d) => d.to_text(culture.decimal),
                Num::F(x) => {
                    let s = format!("{x}");
                    s.replace('.', &culture.decimal.to_string())
                }
            },
            'F' => fixed(n, prec.unwrap_or(2), culture, false),
            'N' => fixed(n, prec.unwrap_or(2), culture, true),
            'C' => currency_text(n, prec, culture),
            'P' => {
                let digits = fixed(times_100(n), prec.unwrap_or(2), culture, true);
                let p = culture.loc.percent;
                // The sign stays in front: "-25,6 %".
                match digits.strip_prefix('-') {
                    Some(d) => format!("-{}{d}{}", p.prefix, p.suffix),
                    None => format!("{}{digits}{}", p.prefix, p.suffix),
                }
            }
            'D' => {
                let i = whole_number(n)?;
                let width = prec.unwrap_or(1).min(100);
                let digits = format!("{:0width$}", i.unsigned_abs());
                if i < 0 { format!("-{digits}") } else { digits }
            }
            'E' => match n {
                Num::D(d) => decimal_exponential(d, prec.unwrap_or(6).min(99), first == 'e', culture),
                Num::F(x) if prec.unwrap_or(6) < 15 && x.is_finite() && x != 0.0 => {
                    match Dec::parse(&format!("{x:.14e}")) {
                        Some(d) => decimal_exponential(d, prec.unwrap_or(6).min(99), first == 'e', culture),
                        None => {
                            // Below the decimal range: digits as they are.
                            let p = prec.unwrap_or(6).min(99);
                            let s = format!("{:.*e}", p, x);
                            let (m, e) = s.split_once('e')?;
                            let e: i32 = e.parse().ok()?;
                            let m = m.replace('.', &culture.decimal.to_string());
                            let case = if first == 'e' { 'e' } else { 'E' };
                            format!(
                                "{m}{case}{}{:03}",
                                if e < 0 { '-' } else { '+' },
                                e.unsigned_abs()
                            )
                        }
                    }
                }
                Num::F(x) => {
                    let p = prec.unwrap_or(6).min(99);
                    let s = format!("{:.*e}", p, x);
                    let (m, e) = s.split_once('e')?;
                    let e: i32 = e.parse().ok()?;
                    let m = m.replace('.', &culture.decimal.to_string());
                    let case = if first == 'e' { 'e' } else { 'E' };
                    format!(
                        "{m}{case}{}{:03}",
                        if e < 0 { '-' } else { '+' },
                        e.unsigned_abs()
                    )
                }
            },
            'X' => {
                let i = whole_number(n)?;
                let s = match i64::try_from(i) {
                    Ok(v) if first == 'x' => format!("{v:x}"),
                    Ok(v) => format!("{v:X}"),
                    Err(_) if first == 'x' => format!("{i:x}"),
                    Err(_) => format!("{i:X}"),
                };
                format!("{:0>width$}", s, width = prec.unwrap_or(1).min(100))
            }
            _ => return None,
        });
    }
    custom_number(n, fmt, culture)
}

fn custom_number(n: Num, fmt: &str, culture: &CultureInfo) -> Option<String> {
    let section = fmt.split(';').next().unwrap_or(fmt);
    let mut prefix = String::new();
    let mut suffix = String::new();
    let mut int_zeros = 0usize;
    let mut dec_zeros = 0usize;
    let mut dec_hashes = 0usize;
    let mut grouped = false;
    let mut percent = false;
    let mut in_number = false;
    let mut after_number = false;
    let mut in_dec = false;
    let mut quote: Option<char> = None;
    for c in section.chars() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            } else if after_number {
                suffix.push(c);
            } else {
                prefix.push(c);
            }
            continue;
        }
        match c {
            '\'' | '"' => quote = Some(c),
            '0' | '#' if !after_number => {
                in_number = true;
                if in_dec {
                    if c == '0' {
                        dec_zeros += 1 + dec_hashes;
                        dec_hashes = 0;
                    } else {
                        dec_hashes += 1;
                    }
                } else if c == '0' {
                    int_zeros += 1;
                }
            }
            '.' if !after_number => {
                in_number = true;
                in_dec = true;
            }
            ',' if in_number && !in_dec => grouped = true,
            '%' => {
                percent = true;
                if in_number {
                    after_number = true;
                    suffix.push('%');
                } else {
                    prefix.push('%');
                }
            }
            c => {
                if in_number {
                    after_number = true;
                    suffix.push(c);
                } else {
                    prefix.push(c);
                }
            }
        }
    }
    if !in_number {
        return Some(section.to_string());
    }
    let v = if percent { times_100(n) } else { n };
    let total = dec_zeros + dec_hashes;
    let (int, frac) = fixed_digits(v, total);
    let mut frac = frac;
    while frac.len() > dec_zeros && frac.ends_with('0') {
        frac.pop();
    }
    let int = int.trim_start_matches('0');
    let int = if int.len() < int_zeros {
        format!("{}{int}", "0".repeat(int_zeros - int.len()))
    } else {
        int.to_string()
    };
    let mut out = String::new();
    let all_zero = int.chars().chain(frac.chars()).all(|c| c == '0');
    if is_negative(v) && !all_zero {
        out.push('-');
    }
    out.push_str(&prefix);
    out.push_str(&if grouped { group_in(&int, culture) } else { int });
    if !frac.is_empty() {
        out.push(culture.decimal);
        out.push_str(&frac);
    }
    out.push_str(&suffix);
    Some(out)
}

// ---------------------------------------------------------------------------
// Dates to text.

fn date_parts(day: i64, ms: i64) -> DateParts {
    DateParts::from_days_ms(day, ms)
}

/// The culture's short date (`1/5/2026`, `05/01/2026`, `05.01.2026`).
fn short_date(day: i64, c: &CultureInfo) -> String {
    datefmt::format_pattern(&date_parts(day, 0), c.loc.short_date, &c.loc.names())
}

/// The culture's long time (`1:04:05 PM`, `13:04:05`); fractions of a second are not shown.
fn long_time(ms: i64, c: &CultureInfo) -> String {
    datefmt::format_pattern(&date_parts(0, ms), c.loc.long_time, &c.loc.names())
}

/// The culture's short time (`9:05 AM`, `09:05`), Desktop's default text for `time` values.
fn short_time(ms: i64, c: &CultureInfo) -> String {
    datefmt::format_pattern(&date_parts(0, ms), c.loc.short_time, &c.loc.names())
}

fn offset_text(o: i32) -> String {
    let sign = if o < 0 { '-' } else { '+' };
    let a = o.unsigned_abs();
    format!("{sign}{:02}:{:02}", a / 60, a % 60)
}

pub(crate) fn duration_text(ms: i64) -> String {
    let neg = ms < 0;
    let a = ms.unsigned_abs();
    let days = a / MS_PER_DAY as u64;
    let rest = a % MS_PER_DAY as u64;
    let (h, mi, s, f) = (
        rest / 3_600_000,
        rest / 60_000 % 60,
        rest / 1000 % 60,
        rest % 1000,
    );
    let mut out = String::new();
    if neg {
        out.push('-');
    }
    if days > 0 {
        out.push_str(&format!("{days}."));
    }
    out.push_str(&format!("{h:02}:{mi:02}:{s:02}"));
    if f > 0 {
        out.push_str(&format!(".{f:03}0000"));
    }
    out
}

/// Default text of a value (`Text.From`); `None` for null.
pub(crate) fn to_text(v: &Value, c: &CultureInfo) -> Option<String> {
    Some(match v {
        Value::Null => return None,
        Value::Text(t) => t.to_string(),
        Value::Logical(b) => if *b { "true" } else { "false" }.into(),
        Value::Number(n) => number_to_text(*n, c),
        Value::Date(d) => short_date(*d, c),
        Value::DateTime(ms) => {
            let (d, t) = split_ms(*ms);
            format!("{} {}", short_date(d, c), long_time(t, c))
        }
        Value::DateTimeZone(ms, o) => {
            let (d, t) = split_ms(*ms);
            format!("{} {} {}", short_date(d, c), long_time(t, c), offset_text(*o))
        }
        Value::Time(t) => short_time(*t, c),
        Value::Duration(d) => duration_text(*d),
        _ => return None,
    })
}

/// .NET custom date/time format strings (`yyyy-MM-dd`, `dddd`, `HH:mm:ss.fff`, `tt`, …)
/// and the standard ones (`d`, `D`, `t`, `T`, `f`, `F`, `g`, `G`, `M`, `Y`, `s`, `u`, `o`) in
/// the culture (`bicraft_locale::datefmt`).
pub(crate) fn format_datetime(v: &Value, fmt: &str, c: &CultureInfo) -> Option<String> {
    let (day, ms, offset) = match v {
        Value::Date(d) => (*d, 0, None),
        Value::DateTime(t) => {
            let (d, ms) = split_ms(*t);
            (d, ms, None)
        }
        Value::DateTimeZone(t, o) => {
            let (d, ms) = split_ms(*t);
            (d, ms, Some(*o))
        }
        Value::Time(t) => (0, *t, None),
        _ => return None,
    };
    let mut p = date_parts(day, ms);
    p.offset = offset;
    if matches!(fmt, "o" | "O") {
        let mut s = datefmt::format_pattern(&p, "yyyy'-'MM'-'dd'T'HH':'mm':'ss'.'fffffff", &c.loc.names());
        if let Some(o) = offset {
            s.push_str(&offset_text(o));
        }
        return Some(s);
    }
    Some(datefmt::format(&p, fmt, c.loc))
}

// ---------------------------------------------------------------------------
// Text to values.

/// `Number.FromText`: sign, digits with optional group separators, the culture's
/// decimal separator, an exponent, surrounding blanks, the culture's currency symbol before or
/// after the number (`$2.675`, `1 234,50 €`); a trailing `%` divides by 100. The result is a
/// decimal when the value fits one (see [`Num::parse`]).
pub(crate) fn number_from_text(s: &str, c: &CultureInfo) -> Option<Num> {
    let t = s.trim().trim_matches('\u{a0}');
    let sym = c.loc.pq.currency.symbol;
    let t = bicraft_locale::parse::strip_symbol(t, sym).unwrap_or(t);
    if let Some(r) = t.strip_prefix('-')
        && let Some(inner) = bicraft_locale::parse::strip_symbol(r, sym)
    {
        return number_from_text(inner, c).map(Num::neg);
    }
    let (t, pct) = match t.strip_suffix('%') {
        Some(r) => (r.trim_end(), true),
        None => (t, false),
    };
    if t.eq_ignore_ascii_case("nan") {
        return Some(Num::F(f64::NAN));
    }
    // `(2.5)` is negative.
    if let Some(inner) = t.strip_prefix('(').and_then(|r| r.strip_suffix(')')) {
        return number_from_text(inner, c).map(Num::neg);
    }
    let mut norm = String::with_capacity(t.len());
    let mut seen_dec = false;
    let mut seen_exp = false;
    let mut digits = 0usize;
    let mut prev_digit = false;
    for (i, ch) in t.chars().enumerate() {
        if ch.is_ascii_digit() {
            norm.push(ch);
            digits += 1;
            prev_digit = true;
            continue;
        }
        let at_start = i == 0 || norm.ends_with('e');
        if (ch == '-' || ch == '+') && at_start {
            norm.push(ch);
        } else if (ch == c.decimal || ch == c.loc.pq.currency.decimal) && !seen_dec && !seen_exp {
            seen_dec = true;
            norm.push('.');
        } else if (ch == c.group || ch == c.loc.pq.currency.group || (c.group == '\u{a0}' && ch == ' '))
            && prev_digit
            && !seen_dec
            && !seen_exp
        {
            // Group separators are accepted between digits of the integer part; a culture whose
            // separator is U+00A0 also takes a plain space (as .NET does; U+202F does not).
        } else if (ch == 'e' || ch == 'E') && digits > 0 && !seen_exp {
            seen_exp = true;
            norm.push('e');
        } else {
            return None;
        }
        prev_digit = false;
    }
    if digits == 0 {
        return None;
    }
    let n = Num::parse(&norm)?;
    if !pct {
        return Some(n);
    }
    Some(match n {
        Num::D(d) => match Dec::from_i128(100).and_then(|h| d.div(h).ok()) {
            Some(r) => Num::D(r),
            None => Num::F(d.to_f64() / 100.0),
        },
        Num::F(x) => Num::F(x / 100.0),
    })
}

fn month_from_name(word: &str, c: &CultureInfo) -> Option<i64> {
    let w = word.trim_end_matches('.').to_lowercase();
    if w.chars().count() < 3 {
        return None;
    }
    for names in [&c.months, &c.loc.months_genitive, &c.loc.months_abbr, &EN_MONTHS] {
        for (i, m) in names.iter().enumerate() {
            let m = m.trim_end_matches('.').to_lowercase();
            if m == w || (m.starts_with(&w) && w.chars().count() >= 3) {
                return Some(i as i64 + 1);
            }
        }
    }
    None
}

/// Dates with a month name: `5 January 2026`, `January 5, 2026`, `05-Jan-2026`.
fn named_month_date(s: &str, c: &CultureInfo) -> Option<i64> {
    let words: Vec<&str> = s
        .split([' ', ',', '-', '/', '.'])
        .filter(|w| !w.is_empty())
        .collect();
    if words.len() != 3 {
        return None;
    }
    let mut month = None;
    let mut nums = Vec::new();
    for w in &words {
        if let Some(m) = month_from_name(w, c) {
            month = Some(m);
        } else if w.bytes().all(|b| b.is_ascii_digit()) && w.len() <= 4 {
            nums.push((w.parse::<i64>().ok()?, w.len()));
        } else {
            return None;
        }
    }
    let m = month?;
    let [(a, la), (b, lb)] = nums.as_slice() else {
        return None;
    };
    let (d, y) = if *la == 4 {
        (*b, *a)
    } else if *lb == 4 {
        (*a, *b)
    } else {
        (*a, 2000 + *b)
    };
    cv::ole_date(y, m, d).ok()
}

/// `DateTime.FromText`/`Date.FromText`: ISO 8601, the culture's short date
/// (with optional time and AM/PM), or a date with a month name. Returns OLE
/// milliseconds, plus the offset when one was written.
pub(crate) fn datetime_from_text(s: &str, c: &CultureInfo) -> Option<(i64, Option<i32>)> {
    let t = s.trim();
    if let Some(r) = iso_datetime(t) {
        return Some(r);
    }
    if let Some(v) = cv::parse_datetime(t, c.parse_culture()) {
        let (d, ms) = cv::split_ole(v)?;
        return Some((d.checked_mul(MS_PER_DAY)?.checked_add(ms)?, None));
    }
    // Year-first cultures read a date with the year last month first (ja-JP, sv-SE, en-CA:
    // "05/03/2024" is 3 May, observed in Desktop).
    if c.order == DateOrder::Ymd
        && let Some(v) = cv::parse_datetime(
            t,
            crate::Culture {
                date_order: DateOrder::Mdy,
                ..c.parse_culture()
            },
        )
    {
        let (d, ms) = cv::split_ole(v)?;
        return Some((d.checked_mul(MS_PER_DAY)?.checked_add(ms)?, None));
    }
    // Dates in an order other than the culture's, with a 4-digit year first.
    if c.order != DateOrder::Ymd
        && let Some(v) = cv::parse_datetime(
            t,
            crate::Culture {
                date_order: DateOrder::Ymd,
                ..c.parse_culture()
            },
        )
    {
        let (d, ms) = cv::split_ole(v)?;
        return Some((d.checked_mul(MS_PER_DAY)?.checked_add(ms)?, None));
    }
    let day = named_month_date(t, c)?;
    Some((day.checked_mul(MS_PER_DAY)?, None))
}

/// ISO 8601 `yyyy-MM-dd[(T| )HH:mm[:ss[.fff]]][Z|±hh:mm]`, keeping the offset.
fn iso_datetime(s: &str) -> Option<(i64, Option<i32>)> {
    let b = s.as_bytes();
    if b.len() < 10 || b.get(4) != Some(&b'-') || b.get(7) != Some(&b'-') {
        return None;
    }
    let num = |r: std::ops::Range<usize>| -> Option<i64> {
        let p = s.get(r)?;
        if p.bytes().all(|c| c.is_ascii_digit()) {
            p.parse().ok()
        } else {
            None
        }
    };
    let day = cv::ole_date(num(0..4)?, num(5..7)?, num(8..10)?).ok()?;
    let mut ms = 0i64;
    let mut rest = s.get(10..)?;
    if let Some(r) = rest.strip_prefix(['T', ' ']) {
        let end = r.find(['Z', '+', '-']).unwrap_or(r.len());
        let time = r.get(..end)?;
        ms = time_from_text_24(time)?;
        rest = r.get(end..)?;
    }
    let offset = if rest.is_empty() {
        None
    } else if rest == "Z" {
        Some(0)
    } else {
        let sign = if rest.starts_with('-') { -1 } else { 1 };
        let body = rest.get(1..)?;
        let (h, m) = match body.split_once(':') {
            Some((h, m)) => (h, m),
            None if body.len() == 4 => (body.get(..2)?, body.get(2..)?),
            None => (body, "0"),
        };
        let h: i32 = h.parse().ok()?;
        let m: i32 = m.parse().ok()?;
        if h > 14 || m > 59 {
            return None;
        }
        Some(sign * (h * 60 + m))
    };
    Some((day.checked_mul(MS_PER_DAY)?.checked_add(ms)?, offset))
}

fn time_from_text_24(t: &str) -> Option<i64> {
    let mut parts = t.split(':');
    let h: i64 = parts.next()?.trim().parse().ok()?;
    let mi: i64 = parts.next()?.trim().parse().ok()?;
    let (s, ms) = match parts.next() {
        None => (0, 0),
        Some(p) => {
            let (w, f) = p.split_once(['.', ',']).unwrap_or((p, ""));
            let s: i64 = w.trim().parse().ok()?;
            if !f.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let mut ms = 0;
            for i in 0..3 {
                ms = ms * 10 + f.as_bytes().get(i).map_or(0, |b| i64::from(b - b'0'));
            }
            (s, ms)
        }
    };
    if parts.next().is_some() {
        return None;
    }
    cv::time_ms(h, mi, s, ms).ok()
}

/// `Time.FromText`: `13:45`, `13:45:30.5`, `1:45 PM`.
pub(crate) fn time_from_text(s: &str) -> Option<i64> {
    let t = s.trim();
    let upper = t.to_ascii_uppercase();
    let (body, pm) = if let Some(b) = upper.strip_suffix("PM") {
        (b.trim_end().to_string(), Some(true))
    } else if let Some(b) = upper.strip_suffix("AM") {
        (b.trim_end().to_string(), Some(false))
    } else {
        (upper.clone(), None)
    };
    let ms = time_from_text_24(&body)?;
    match pm {
        None => Some(ms),
        Some(pm) => {
            let h = ms / 3_600_000;
            if !(1..=12).contains(&h) {
                return None;
            }
            let h24 = if pm { h % 12 + 12 } else { h % 12 };
            Some(ms - h * 3_600_000 + h24 * 3_600_000)
        }
    }
}

/// `Duration.FromText`: `[-][d.]hh:mm[:ss[.fffffff]]` or `[-]d`.
pub(crate) fn duration_from_text(s: &str) -> Option<i64> {
    let t = s.trim();
    let (neg, t) = match t.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, t),
    };
    let (days, time) = match t.split_once(':') {
        None => (t.parse::<i64>().ok()?, "0:0"),
        Some(_) => match t.split_once('.') {
            Some((d, rest)) if rest.contains(':') && !d.contains(':') => (d.parse::<i64>().ok()?, rest),
            _ => (0, t),
        },
    };
    let mut parts = time.split(':');
    let h: i64 = parts.next()?.parse().ok()?;
    let mi: i64 = parts.next()?.parse().ok()?;
    let (sec, ms) = match parts.next() {
        None => (0, 0),
        Some(p) => {
            let (w, f) = p.split_once('.').unwrap_or((p, ""));
            let mut ms = 0;
            for i in 0..3 {
                ms = ms * 10
                    + f.as_bytes()
                        .get(i)
                        .map_or(0, |b| i64::from(b.wrapping_sub(b'0')).min(9));
            }
            (w.parse::<i64>().ok()?, ms)
        }
    };
    if h > 23 || mi > 59 || sec > 59 {
        return None;
    }
    let total = days
        .checked_mul(MS_PER_DAY)?
        .checked_add(((h * 60 + mi) * 60 + sec) * 1000 + ms)?;
    Some(if neg { -total } else { total })
}

// ---------------------------------------------------------------------------
// Type conversion.

fn format_err(what: &str, v: &Value, c: &CultureInfo) -> super::value::Flow {
    let shown = match v {
        Value::Text(t) => format!("\"{}\"", cv::truncate(t)),
        other => to_text(other, c).unwrap_or_else(|| other.type_name().into()),
    };
    super::value::Flow::Err(std::rc::Rc::new(super::value::MError {
        reason: "DataFormat.Error".into(),
        message: format!("We couldn't convert to {what}. Details: {shown}"),
        detail: Value::Null,
    }))
}

/// Converts a value to a number (`Number.From`).
pub(crate) fn to_number(v: &Value, c: &CultureInfo) -> Res<Value> {
    Ok(match v {
        Value::Null => Value::Null,
        Value::Number(_) => v.clone(),
        Value::Logical(b) => Value::num(if *b { 1.0 } else { 0.0 }),
        Value::Text(t) => match number_from_text(t, c) {
            Some(n) => Value::Number(n),
            None => return Err(format_err("Number", v, c)),
        },
        Value::Date(d) => Value::num(*d as f64),
        Value::DateTime(ms) => Value::num(*ms as f64 / MS_PER_DAY as f64),
        Value::DateTimeZone(ms, o) => {
            Value::num(ms.saturating_sub(i64::from(*o) * 60_000) as f64 / MS_PER_DAY as f64)
        }
        Value::Time(ms) | Value::Duration(ms) => Value::num(*ms as f64 / MS_PER_DAY as f64),
        _ => return Err(format_err("Number", v, c)),
    })
}

/// Rounds a decimal to `digits` places, half to even (`digits` at most 28).
pub(crate) fn dec_round_even(d: Dec, digits: u32) -> Option<Dec> {
    if d.scale() <= digits {
        return Some(d);
    }
    let p = Dec::pow10(i32::try_from(digits).ok()?)?;
    let scaled = d.mul(p).ok()?;
    let r = if scaled.is_half() {
        let t = scaled.trunc();
        if t.to_i128().is_some_and(|i| i % 2 == 0) {
            t
        } else {
            scaled.away()
        }
    } else {
        scaled.round_nearest()
    };
    r.div(p).ok()
}

fn decimal_range_err<T>() -> Res<T> {
    err("Expression.Error", "The number is out of the range of a decimal.")
}

/// Applies a number facet: integer types round half to even (exactly), currency to 4 places,
/// `Decimal.Type` converts to a decimal, `Double.Type` and `Single.Type` to a double.
pub(crate) fn apply_facet(n: Num, facet: Facet) -> Res<Num> {
    Ok(match facet {
        Facet::Int64 | Facet::Int32 | Facet::Int16 | Facet::Int8 | Facet::Byte => {
            let (lo, hi): (i128, i128) = match facet {
                Facet::Int32 => (i32::MIN.into(), i32::MAX.into()),
                Facet::Int16 => (i16::MIN.into(), i16::MAX.into()),
                Facet::Int8 => (i8::MIN.into(), i8::MAX.into()),
                Facet::Byte => (0, 255),
                _ => (i64::MIN.into(), i64::MAX.into()),
            };
            let r = n.dec().and_then(|d| dec_round_even(d, 0)).and_then(Dec::to_i128);
            match r {
                Some(i) if (lo..=hi).contains(&i) => match Dec::from_i128(i) {
                    Some(d) => Num::D(d),
                    None => Num::F(i as f64),
                },
                _ => {
                    return err(
                        "Expression.Error",
                        format!("The number {} is out of range for the integer type.", n.f()),
                    );
                }
            }
        }
        Facet::Currency => match n.dec().and_then(|d| dec_round_even(d, 4)) {
            Some(d) => Num::D(d),
            None => return decimal_range_err(),
        },
        Facet::Decimal => match n.dec() {
            Some(d) => Num::D(d),
            None => return decimal_range_err(),
        },
        Facet::Double => Num::F(n.f()),
        Facet::Single => Num::F(n.f() as f32 as f64),
        _ => n,
    })
}

/// Converts a value to `t` the way `Table.TransformColumnTypes` does.
/// Unconvertible values are `DataFormat.Error`s.
pub(crate) fn convert_to(v: &Value, t: &MType, c: &CultureInfo) -> Res<Value> {
    if v.is_null() {
        return Ok(Value::Null);
    }
    let bad = || Err(format_err(super::eval::title(t.name()).as_str(), v, c));
    Ok(match t.kind {
        Kind::Any | Kind::AnyNonNull => v.clone(),
        Kind::Text => match v {
            Value::Text(_) => v.clone(),
            Value::Logical(b) => Value::text(if *b { "TRUE" } else { "FALSE" }),
            other => match to_text(other, c) {
                Some(s) => Value::text(s),
                None => return bad(),
            },
        },
        Kind::Number => {
            let n = match to_number(v, c)? {
                Value::Number(n) => n,
                _ => return bad(),
            };
            Value::Number(apply_facet(n, t.facet)?)
        }
        Kind::Logical => match v {
            Value::Logical(_) => v.clone(),
            Value::Number(n) => Value::Logical(n.f() != 0.0),
            Value::Text(s) => match cv::parse_bool(s.trim()) {
                Some(b) => Value::Logical(b),
                None => return bad(),
            },
            _ => return bad(),
        },
        Kind::Date => match v {
            Value::Date(_) => v.clone(),
            Value::DateTime(ms) | Value::DateTimeZone(ms, _) => Value::Date(split_ms(*ms).0),
            Value::Number(n) => match super::eval::whole(n.f().floor()) {
                Some(d) if (-657_434..=2_958_465).contains(&d) => Value::Date(d),
                _ => return bad(),
            },
            // A time part makes the text a datetime, not a date.
            Value::Text(s) if s.contains(':') => return bad(),
            Value::Text(s) => match datetime_from_text(s, c) {
                Some((ms, _)) => Value::Date(split_ms(ms).0),
                None => return bad(),
            },
            _ => return bad(),
        },
        Kind::DateTime => match v {
            Value::DateTime(_) => v.clone(),
            Value::Date(d) => Value::DateTime(super::eval::date_ms(*d)?),
            Value::DateTimeZone(ms, _) => Value::DateTime(*ms),
            Value::Number(n) => match ole_ms(n.f()) {
                Some(ms) => super::eval::in_range(Value::DateTime(ms))?,
                None => return bad(),
            },
            Value::Text(s) => match datetime_from_text(s, c) {
                Some((ms, _)) => Value::DateTime(ms),
                None => return bad(),
            },
            _ => return bad(),
        },
        Kind::DateTimeZone => match v {
            Value::DateTimeZone(..) => v.clone(),
            Value::Text(s) => match datetime_from_text(s, c) {
                Some((ms, Some(o))) => Value::DateTimeZone(ms, o),
                _ => {
                    return super::value::unsupported(
                        "a conversion to datetimezone without an explicit offset (depends on the machine's time zone)",
                    );
                }
            },
            _ => {
                return super::value::unsupported(
                    "a conversion to datetimezone (depends on the machine's time zone)",
                );
            }
        },
        Kind::Time => match v {
            Value::Time(_) => v.clone(),
            Value::DateTime(ms) | Value::DateTimeZone(ms, _) => Value::Time(split_ms(*ms).1),
            Value::Number(n) if (0.0..1.0).contains(&n.f()) => match ole_ms(n.f()) {
                Some(ms) => Value::Time(ms),
                None => return bad(),
            },
            Value::Text(s) => match time_from_text(s) {
                Some(ms) => Value::Time(ms),
                None => match datetime_from_text(s, c) {
                    Some((ms, _)) => Value::Time(split_ms(ms).1),
                    None => return bad(),
                },
            },
            _ => return bad(),
        },
        Kind::Duration => match v {
            Value::Duration(_) => v.clone(),
            Value::Number(n) => match ole_ms(n.f()) {
                Some(ms) => Value::Duration(ms),
                None => return bad(),
            },
            Value::Text(s) => match duration_from_text(s) {
                Some(ms) => Value::Duration(ms),
                None => return bad(),
            },
            _ => return bad(),
        },
        Kind::Binary => match v {
            Value::Binary(_) => v.clone(),
            _ => return bad(),
        },
        k => {
            if v.kind() == k {
                v.clone()
            } else {
                return bad();
            }
        }
    })
}

/// OLE days (fractional) → milliseconds.
pub(crate) fn ole_ms(n: f64) -> Option<i64> {
    let ms = (n * MS_PER_DAY as f64).round();
    (ms.is_finite() && ms.abs() < 9.0e17).then_some(ms as i64)
}
