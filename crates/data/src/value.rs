//! Value helpers shared by every loader: OLE dates, culture-aware number and
//! date parsing, invariant formatting, type inference and coercion.

use bicraft_locale::calendar::{self, days_in_month};
use bicraft_model::DataType;
use bicraft_vertipaq::CellValue;

/// Milliseconds in a day.
pub(crate) const MS_PER_DAY: i64 = 86_400_000;

/// Day number of 1899-12-30 (the OLE Automation epoch) in [`days_from_civil`] units.
pub(crate) const OLE_EPOCH: i64 = calendar::SERIAL_EPOCH;

/// Order of the day, month and year parts in a culture's short date.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateOrder {
    /// `31/12/2026` (fr-FR, en-GB, de-DE, …).
    Dmy,
    /// `12/31/2026` (en-US, invariant).
    Mdy,
    /// `2026/12/31` (ja-JP, zh-CN, …).
    Ymd,
}

/// The few culture settings the text parsers need.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Culture {
    /// Decimal separator: `.` or `,`. Thousands separators are never accepted.
    pub decimal_separator: char,
    /// Order of the parts in non-ISO dates.
    pub date_order: DateOrder,
    /// Currency symbol accepted before or after a number (`$2.675`, `2,675 €`); empty for
    /// none.
    pub currency_symbol: &'static str,
}

impl Default for Culture {
    fn default() -> Self {
        Self::INVARIANT
    }
}

impl Culture {
    /// The invariant culture (same rules as en-US): `.` decimals, month/day/year dates.
    pub const INVARIANT: Self = Self {
        decimal_separator: '.',
        date_order: DateOrder::Mdy,
        currency_symbol: "",
    };

    /// en-US: `1.5`, `12/31/2026`, `$2.675`.
    pub fn en_us() -> Self {
        Self {
            currency_symbol: "$",
            ..Self::INVARIANT
        }
    }

    /// fr-FR: `1,5`, `31/12/2026`, `2,675 €`.
    pub fn fr_fr() -> Self {
        Self {
            decimal_separator: ',',
            date_order: DateOrder::Dmy,
            currency_symbol: "€",
        }
    }

    /// The settings of a culture name (`fr-FR`, `en-GB`, `de`, …) from `bicraft-locale`:
    /// its decimal separator, the order of its short date and Power Query's currency symbol.
    /// A language bicraft has no data for, or the invariant culture, is
    /// [`Culture::INVARIANT`]. Year-first cultures read day/month dates month first, as
    /// Desktop does, except that the order of their own short date comes first.
    pub fn from_name(name: &str) -> Self {
        let Some(loc) = bicraft_locale::Locale::find(name).filter(|l| !l.is_invariant()) else {
            return Self::INVARIANT;
        };
        let date_order = if loc.year_first() {
            DateOrder::Ymd
        } else if loc.day_first() {
            DateOrder::Dmy
        } else {
            DateOrder::Mdy
        };
        Self {
            decimal_separator: loc.decimal,
            date_order,
            currency_symbol: loc.pq.currency.symbol,
        }
    }

    /// `s` without this culture's currency symbol (and the blanks next to it), when it has one
    /// (`$2.675`, `-$5`, `2,675 €`).
    pub(crate) fn without_currency(&self, s: &str) -> Option<String> {
        let t = s.trim();
        let (neg, body) = match t.strip_prefix('-') {
            Some(r) => (true, r.trim_start()),
            None => (false, t),
        };
        let inner = bicraft_locale::parse::strip_symbol(body, self.currency_symbol)?;
        Some(if neg {
            format!("-{inner}")
        } else {
            inner.to_string()
        })
    }
}

// ---------------------------------------------------------------------------
// Calendar arithmetic (proleptic Gregorian).

/// Days since 1970-01-01 for a civil date (`bicraft_locale::calendar`, through jiff).
pub(crate) fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    calendar::days_from_civil(y, m, d)
}

/// Inverse of [`days_from_civil`].
pub(crate) fn civil_from_days(z: i64) -> (i64, i64, i64) {
    calendar::civil_from_days(z)
}

/// Validated date → OLE day number (days since 1899-12-30). Years 1..=9999.
pub(crate) fn ole_date(y: i64, m: i64, d: i64) -> Result<i64, String> {
    if !(1..=9999).contains(&y) {
        return Err(format!("year {y} is outside 1..9999"));
    }
    if !(1..=12).contains(&m) {
        return Err(format!("month {m} is outside 1..12"));
    }
    if d < 1 || d > days_in_month(y, m) {
        return Err(format!("day {d} is not valid for {y:04}-{m:02}"));
    }
    Ok(days_from_civil(y, m, d) - OLE_EPOCH)
}

/// Validated time of day → milliseconds. `24:00:00` is accepted (end of day).
pub(crate) fn time_ms(h: i64, mi: i64, s: i64, ms: i64) -> Result<i64, String> {
    let ok =
        (0..=23).contains(&h) && (0..=59).contains(&mi) && (0..=59).contains(&s) && (0..=999).contains(&ms);
    let end_of_day = h == 24 && mi == 0 && s == 0 && ms == 0;
    if !ok && !end_of_day {
        return Err(format!("time {h:02}:{mi:02}:{s:02}.{ms:03} is not valid"));
    }
    Ok(((h * 60 + mi) * 60 + s) * 1000 + ms)
}

/// OLE day number + milliseconds of day → the `CellValue::DateTime` payload.
/// The representation is continuous (`-1.25` is 1899-12-28 18:00), not the
/// sign-split legacy OLE form.
pub(crate) fn ole_value(days: i64, ms: i64) -> f64 {
    days as f64 + ms as f64 / MS_PER_DAY as f64
}

/// Splits a DateTime payload into (OLE day, ms of day), rounding to the millisecond.
pub(crate) fn split_ole(v: f64) -> Option<(i64, i64)> {
    if !v.is_finite() || v.abs() > 4.0e6 {
        return None;
    }
    let day = v.floor();
    let mut ms = ((v - day) * MS_PER_DAY as f64).round() as i64;
    let mut day = day as i64;
    if ms >= MS_PER_DAY {
        day += 1;
        ms -= MS_PER_DAY;
    }
    Some((day, ms))
}

// ---------------------------------------------------------------------------
// Formatting.

/// Formats a fixed decimal (value × 10 000) with as few digits as needed.
pub(crate) fn format_decimal(n: i64, sep: char) -> String {
    let neg = n < 0;
    let abs = n.unsigned_abs();
    let int = abs / 10_000;
    let frac = abs % 10_000;
    let mut s = String::new();
    if neg {
        s.push('-');
    }
    s.push_str(&int.to_string());
    if frac != 0 {
        let f = format!("{frac:04}");
        s.push(sep);
        s.push_str(f.trim_end_matches('0'));
    }
    s
}

/// Formats a double so that it parses back to the same bits (`1.0`, `1e300`, `NaN`, `inf`).
pub(crate) fn format_double(r: f64, sep: char) -> String {
    let s = format!("{r:?}");
    if sep == '.' {
        s
    } else {
        s.replace('.', &sep.to_string())
    }
}

/// ISO 8601 text for a DateTime payload: `2026-01-05` at midnight, else
/// `2026-01-05T13:45:00` (with `.fff` when there are milliseconds).
pub(crate) fn format_datetime(v: f64) -> String {
    let Some((day, ms)) = split_ole(v) else {
        return format_double(v, '.');
    };
    let (y, m, d) = civil_from_days(day + OLE_EPOCH);
    let date = format!("{y:04}-{m:02}-{d:02}");
    if ms == 0 {
        return date;
    }
    let (h, rest) = (ms / 3_600_000, ms % 3_600_000);
    let (mi, rest) = (rest / 60_000, rest % 60_000);
    let (s, milli) = (rest / 1000, rest % 1000);
    if milli == 0 {
        format!("{date}T{h:02}:{mi:02}:{s:02}")
    } else {
        format!("{date}T{h:02}:{mi:02}:{s:02}.{milli:03}")
    }
}

/// Invariant text for any value (used when a non-text value lands in a text column).
pub(crate) fn format_invariant(v: &CellValue) -> Option<String> {
    Some(match v {
        CellValue::Null => return None,
        CellValue::Int(n) => n.to_string(),
        CellValue::Real(r) => {
            if r.is_finite() && r.fract() == 0.0 && r.abs() < 1e15 {
                format!("{}", *r as i64)
            } else {
                format!("{r}")
            }
        }
        CellValue::Text(t) => t.clone(),
        CellValue::Bool(b) => if *b { "true" } else { "false" }.to_string(),
        CellValue::DateTime(d) => format_datetime(*d),
        CellValue::Decimal(n) => format_decimal(*n, '.'),
    })
}

// ---------------------------------------------------------------------------
// Parsing.

/// Parses an integer: optional sign then ASCII digits only.
pub(crate) fn parse_int(s: &str) -> Option<i64> {
    let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// Validates a culture number (`-1,5e3` with `,` decimals) and returns its
/// normalized ASCII form, or `None`.
fn normalize_number(s: &str, sep: char) -> Option<String> {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    if let Some(&c) = chars.peek()
        && (c == '+' || c == '-')
    {
        out.push(c);
        chars.next();
    }
    let mut mantissa_digits = 0usize;
    let mut seen_sep = false;
    while let Some(&c) = chars.peek() {
        if c.is_ascii_digit() {
            mantissa_digits += 1;
            out.push(c);
        } else if c == sep && !seen_sep {
            seen_sep = true;
            out.push('.');
        } else {
            break;
        }
        chars.next();
    }
    if mantissa_digits == 0 {
        return None;
    }
    if let Some(&c) = chars.peek()
        && (c == 'e' || c == 'E')
    {
        out.push('e');
        chars.next();
        if let Some(&c) = chars.peek()
            && (c == '+' || c == '-')
        {
            out.push(c);
            chars.next();
        }
        let mut exp_digits = 0usize;
        while let Some(&c) = chars.peek() {
            if !c.is_ascii_digit() {
                break;
            }
            exp_digits += 1;
            out.push(c);
            chars.next();
        }
        if exp_digits == 0 {
            return None;
        }
    }
    if chars.next().is_some() { None } else { Some(out) }
}

/// Parses a finite double in the culture's notation (no thousands separators).
pub(crate) fn parse_double(s: &str, sep: char) -> Option<f64> {
    let n = normalize_number(s, sep)?;
    n.parse::<f64>().ok().filter(|r| r.is_finite())
}

/// Like [`parse_double`] but also accepts `NaN`, `inf`, `-inf`, `Infinity`.
pub(crate) fn parse_double_lenient(s: &str, sep: char) -> Option<f64> {
    match s.to_ascii_lowercase().as_str() {
        "nan" => Some(f64::NAN),
        "inf" | "+inf" | "infinity" | "+infinity" => Some(f64::INFINITY),
        "-inf" | "-infinity" => Some(f64::NEG_INFINITY),
        _ => parse_double(s, sep),
    }
}

/// Parses a fixed decimal (4 places, half away from zero) to value × 10 000.
pub(crate) fn parse_decimal(s: &str, sep: char) -> Option<i64> {
    let n = normalize_number(s, sep)?;
    if n.contains('e') {
        let r: f64 = n.parse().ok()?;
        return decimal_from_f64(r);
    }
    let (neg, body) = match n.strip_prefix('-') {
        Some(b) => (true, b),
        None => (false, n.strip_prefix('+').unwrap_or(&n)),
    };
    let (int, frac) = body.split_once('.').unwrap_or((body, ""));
    if int.len() > 20 {
        return None;
    }
    let int_val: i128 = if int.is_empty() { 0 } else { int.parse().ok()? };
    let mut scaled = int_val * 10_000;
    let fb = frac.as_bytes();
    let mut place = 1000i128;
    for i in 0..4 {
        let digit = fb.get(i).map_or(0, |b| i128::from(b - b'0'));
        scaled += digit * place;
        place /= 10;
    }
    if fb.get(4).is_some_and(|b| *b >= b'5') {
        scaled += 1;
    }
    i64::try_from(if neg { -scaled } else { scaled }).ok()
}

/// Rounds a double to a fixed decimal, `None` when out of range or not finite.
pub(crate) fn decimal_from_f64(r: f64) -> Option<i64> {
    let scaled = (r * 10_000.0).round();
    if scaled.is_finite() && scaled.abs() < 9.2e18 {
        Some(scaled as i64)
    } else {
        None
    }
}

/// Parses `true`/`false` (any case).
pub(crate) fn parse_bool(s: &str) -> Option<bool> {
    if s.eq_ignore_ascii_case("true") {
        Some(true)
    } else if s.eq_ignore_ascii_case("false") {
        Some(false)
    } else {
        None
    }
}

fn digits(s: &str, min: usize, max: usize) -> Option<i64> {
    if s.len() < min || s.len() > max || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// Parses `H:MM[:SS[.fff…]]` → ms of day. Digits beyond the millisecond are truncated.
fn parse_time(s: &str) -> Option<i64> {
    let mut parts = s.split(':');
    let h = digits(parts.next()?, 1, 2)?;
    let mi = digits(parts.next()?, 2, 2)?;
    let (sec, ms) = match parts.next() {
        None => (0, 0),
        Some(p) => {
            let (whole, frac) = p.split_once(['.', ',']).unwrap_or((p, ""));
            let sec = digits(whole, 2, 2)?;
            if !frac.is_empty() && !frac.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let mut ms = 0;
            for i in 0..3 {
                ms = ms * 10 + frac.as_bytes().get(i).map_or(0, |b| i64::from(b - b'0'));
            }
            (sec, ms)
        }
    };
    if parts.next().is_some() {
        return None;
    }
    time_ms(h, mi, sec, ms).ok()
}

/// Parses `±HH:MM`, `±HHMM` or `±HH` → offset in ms.
fn parse_offset(s: &str) -> Option<i64> {
    let (sign, rest) = match s.as_bytes().first()? {
        b'+' => (1, s.get(1..)?),
        b'-' => (-1, s.get(1..)?),
        _ => return None,
    };
    let (h, m) = if let Some((h, m)) = rest.split_once(':') {
        (digits(h, 2, 2)?, digits(m, 2, 2)?)
    } else if rest.len() == 4 {
        (digits(rest.get(..2)?, 2, 2)?, digits(rest.get(2..)?, 2, 2)?)
    } else {
        (digits(rest, 2, 2)?, 0)
    };
    if h > 14 || m > 59 {
        return None;
    }
    Some(sign * (h * 60 + m) * 60_000)
}

/// Combines a day number and a ms-of-day (possibly shifted by a UTC offset).
fn combine(day: i64, ms: i64) -> f64 {
    let total = day * MS_PER_DAY + ms;
    ole_value(total.div_euclid(MS_PER_DAY), total.rem_euclid(MS_PER_DAY))
}

/// Parses ISO 8601 (`2026-01-05`, `2026-01-05T13:45[:00[.123]]`, optional `Z`
/// or offset, which is converted to UTC; a space may replace `T`).
pub(crate) fn parse_iso_datetime(s: &str) -> Option<f64> {
    let date = s.get(..10)?;
    let b = date.as_bytes();
    if b.get(4) != Some(&b'-') || b.get(7) != Some(&b'-') {
        return None;
    }
    let y = digits(date.get(..4)?, 4, 4)?;
    let m = digits(date.get(5..7)?, 2, 2)?;
    let d = digits(date.get(8..10)?, 2, 2)?;
    let day = ole_date(y, m, d).ok()?;
    let rest = s.get(10..)?;
    if rest.is_empty() {
        return Some(ole_value(day, 0));
    }
    let rest = rest.strip_prefix(['T', 't', ' '])?;
    let (time, offset) = if let Some(t) = rest.strip_suffix(['Z', 'z']) {
        (t, 0)
    } else if let Some(pos) = rest.rfind(['+', '-']) {
        (rest.get(..pos)?, -parse_offset(rest.get(pos..)?)?)
    } else {
        (rest, 0)
    };
    let ms = parse_time(time)?;
    Some(combine(day, ms + offset))
}

/// Parses a culture short date (`31/12/2026`, `12-31-2026`, `31.12.2026`,
/// `2026/12/31`) with an optional time (`13:45[:00]`, or `1:45 PM`).
pub(crate) fn parse_culture_datetime(s: &str, order: DateOrder) -> Option<f64> {
    let (date, time) = match s.split_once(' ') {
        Some((d, t)) => (d, Some(t.trim())),
        None => (s, None),
    };
    let sep = date.chars().find(|c| matches!(c, '/' | '-' | '.'))?;
    let parts: Vec<&str> = date.split(sep).collect();
    let [a, b, c] = parts.as_slice() else {
        return None;
    };
    let (y, m, d) = match order {
        DateOrder::Dmy => (digits(c, 4, 4)?, digits(b, 1, 2)?, digits(a, 1, 2)?),
        DateOrder::Mdy => (digits(c, 4, 4)?, digits(a, 1, 2)?, digits(b, 1, 2)?),
        DateOrder::Ymd => (digits(a, 4, 4)?, digits(b, 1, 2)?, digits(c, 1, 2)?),
    };
    let day = ole_date(y, m, d).ok()?;
    let ms = match time {
        None => 0,
        Some(t) => {
            let upper = t.to_ascii_uppercase();
            let (clock, pm) = if let Some(c) = upper.strip_suffix("PM") {
                (c.trim_end().to_string(), Some(true))
            } else if let Some(c) = upper.strip_suffix("AM") {
                (c.trim_end().to_string(), Some(false))
            } else {
                (upper, None)
            };
            let ms = parse_time(&clock)?;
            match pm {
                None => ms,
                Some(pm) => {
                    let h = ms / 3_600_000;
                    if !(1..=12).contains(&h) {
                        return None;
                    }
                    let base = ms % (12 * 3_600_000);
                    if pm { base + 12 * 3_600_000 } else { base }
                }
            }
        }
    };
    Some(ole_value(day, ms))
}

/// ISO first, then the culture's short date.
pub(crate) fn parse_datetime(s: &str, culture: Culture) -> Option<f64> {
    parse_iso_datetime(s).or_else(|| parse_culture_datetime(s, culture.date_order))
}

/// Parses `text` as a value of type `to` under `culture`.
pub(crate) fn parse_text_as(text: &str, to: DataType, culture: Culture) -> Result<CellValue, String> {
    // Numbers may carry the culture's currency symbol.
    let unsigned;
    let t = match to {
        DataType::Int64 | DataType::Double | DataType::Decimal => match culture.without_currency(text) {
            Some(n) => {
                unsigned = n;
                unsigned.as_str()
            }
            None => text.trim(),
        },
        _ => text.trim(),
    };
    let sep = culture.decimal_separator;
    let fail = |what: &str| Err(format!("cannot read `{}` as {what}", truncate(text)));
    match to {
        DataType::String | DataType::Unknown | DataType::Variant => Ok(CellValue::Text(text.to_string())),
        _ if t.is_empty() => Ok(CellValue::Null),
        DataType::Int64 => match parse_int(t) {
            Some(n) => Ok(CellValue::Int(n)),
            None => match parse_double(t, sep).and_then(int_from_f64) {
                Some(n) => Ok(CellValue::Int(n)),
                None => fail("an integer"),
            },
        },
        DataType::Double => parse_double_lenient(t, sep)
            .map(CellValue::Real)
            .map_or_else(|| fail("a number"), Ok),
        DataType::Decimal => parse_decimal(t, sep)
            .map(CellValue::Decimal)
            .map_or_else(|| fail("a fixed decimal"), Ok),
        DataType::DateTime => parse_datetime(t, culture)
            .map(CellValue::DateTime)
            .map_or_else(|| fail("a date/time"), Ok),
        DataType::Boolean => parse_bool(t)
            .map(CellValue::Bool)
            .map_or_else(|| fail("true/false"), Ok),
        DataType::Binary => Err("binary columns cannot be loaded from text".into()),
    }
}

// ---------------------------------------------------------------------------
// Fast forms of the common cell texts (the CSV fast paths). Each returns `None` for anything
// but its one narrow shape, which then goes through the general parser; on its shape it gives
// what the general parser gives (checked by the differential tests in `csv.rs` and
// `m/fastcsv.rs`).

/// `-?[0-9]{1,18}`: the integer.
pub(crate) fn fast_int(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    let (neg, digits) = match b.split_first() {
        Some((b'-', rest)) => (true, rest),
        _ => (false, b),
    };
    if digits.is_empty() || digits.len() > 18 {
        return None;
    }
    let mut v: i64 = 0;
    for &c in digits {
        if !c.is_ascii_digit() {
            return None;
        }
        v = v * 10 + i64::from(c - b'0');
    }
    Some(if neg { -v } else { v })
}

/// `-?[0-9]{1,14}(D[0-9]{1,4})?` with `D` the ASCII decimal separator `sep`: the value × 10 000.
pub(crate) fn fast_fixed(s: &str, sep: u8) -> Option<i64> {
    let b = s.as_bytes();
    let (neg, body) = match b.split_first() {
        Some((b'-', rest)) => (true, rest),
        _ => (false, b),
    };
    let (int, frac) = match body.iter().position(|&c| c == sep) {
        Some(p) => (body.get(..p)?, Some(body.get(p + 1..)?)),
        None => (body, None),
    };
    if int.is_empty() || int.len() > 14 || !int.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let mut v: i64 = 0;
    for &c in int {
        v = v * 10 + i64::from(c - b'0');
    }
    v *= 10_000;
    if let Some(frac) = frac {
        if frac.is_empty() || frac.len() > 4 || !frac.iter().all(u8::is_ascii_digit) {
            return None;
        }
        let mut place = 1000i64;
        for &c in frac {
            v += i64::from(c - b'0') * place;
            place /= 10;
        }
    }
    Some(if neg { -v } else { v })
}

/// `-?[0-9]+(D[0-9]+)?` with `D` the ASCII decimal separator `sep`: the text with `D` written
/// `.` (in `buf` when it changes), ready for `f64` or decimal parsing.
pub(crate) fn fast_plain_number<'a>(s: &'a str, sep: u8, buf: &'a mut String) -> Option<&'a str> {
    let b = s.as_bytes();
    let body = match b.split_first() {
        Some((b'-', rest)) => rest,
        _ => b,
    };
    let mut seen_sep = false;
    let mut digits_before = 0usize;
    let mut digits_after = 0usize;
    for &c in body {
        if c.is_ascii_digit() {
            if seen_sep {
                digits_after += 1;
            } else {
                digits_before += 1;
            }
        } else if c == sep && !seen_sep {
            seen_sep = true;
        } else {
            return None;
        }
    }
    if digits_before == 0 || (seen_sep && digits_after == 0) {
        return None;
    }
    if sep == b'.' || !seen_sep {
        return Some(s);
    }
    buf.clear();
    buf.extend(s.chars().map(|c| if c == char::from(sep) { '.' } else { c }));
    Some(buf.as_str())
}

/// `yyyy-mm-dd` exactly: the OLE day number of a valid date.
pub(crate) fn fast_iso_day(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() != 10 || b.get(4) != Some(&b'-') || b.get(7) != Some(&b'-') {
        return None;
    }
    let num = |r: std::ops::Range<usize>| -> Option<i64> {
        let mut v = 0i64;
        for &c in b.get(r)? {
            if !c.is_ascii_digit() {
                return None;
            }
            v = v * 10 + i64::from(c - b'0');
        }
        Some(v)
    };
    ole_date(num(0..4)?, num(5..7)?, num(8..10)?).ok()
}

/// [`parse_text_as`] with the fast forms first (same results).
pub(crate) fn parse_text_as_fast(text: &str, to: DataType, culture: Culture) -> Result<CellValue, String> {
    let sep = u8::try_from(culture.decimal_separator).ok().filter(u8::is_ascii);
    let fast = match (to, sep) {
        (DataType::Int64, _) => fast_int(text).map(CellValue::Int),
        (DataType::Decimal, Some(sep)) => fast_fixed(text, sep).map(CellValue::Decimal),
        (DataType::Double, Some(sep)) => {
            let mut buf = String::new();
            fast_plain_number(text, sep, &mut buf)
                .and_then(|n| n.parse::<f64>().ok())
                .filter(|r| r.is_finite())
                .map(CellValue::Real)
        }
        (DataType::DateTime, _) => fast_iso_day(text).map(|d| CellValue::DateTime(ole_value(d, 0))),
        _ => None,
    };
    match fast {
        Some(v) => Ok(v),
        None => parse_text_as(text, to, culture),
    }
}

/// Shortens long input for error messages.
pub(crate) fn truncate(s: &str) -> String {
    const MAX: usize = 40;
    if s.chars().count() <= MAX {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(MAX).collect();
        t.push('…');
        t
    }
}

/// An integral double that fits in i64.
pub(crate) fn int_from_f64(r: f64) -> Option<i64> {
    (r.is_finite() && r.fract() == 0.0 && r.abs() < 9.2e18).then_some(r as i64)
}

// ---------------------------------------------------------------------------
// Inference and coercion.

/// Whether a value can live in a column of the given type without conversion.
pub(crate) fn fits(v: &CellValue, t: DataType) -> bool {
    matches!(
        (t, v),
        (_, CellValue::Null)
            | (DataType::Unknown | DataType::Variant, _)
            | (DataType::String, CellValue::Text(_))
            | (DataType::Int64, CellValue::Int(_))
            | (DataType::Double, CellValue::Real(_))
            | (DataType::Decimal, CellValue::Decimal(_))
            | (DataType::DateTime, CellValue::DateTime(_))
            | (DataType::Boolean, CellValue::Bool(_))
    )
}

/// The natural column type of one value.
fn kind(v: &CellValue) -> Option<DataType> {
    Some(match v {
        CellValue::Null => return None,
        CellValue::Int(_) => DataType::Int64,
        CellValue::Real(_) => DataType::Double,
        CellValue::Decimal(_) => DataType::Decimal,
        CellValue::Text(_) => DataType::String,
        CellValue::Bool(_) => DataType::Boolean,
        CellValue::DateTime(_) => DataType::DateTime,
    })
}

/// Column type for untyped literal values: one kind → that kind; only numbers
/// → `Double`; any other mix (or all blank) → `String`.
pub(crate) fn infer_from_values(values: &[CellValue]) -> DataType {
    let mut acc: Option<DataType> = None;
    for v in values {
        let Some(k) = kind(v) else { continue };
        acc = Some(match acc {
            None => k,
            Some(a) if a == k => a,
            Some(a) if is_numeric(a) && is_numeric(k) => DataType::Double,
            Some(_) => return DataType::String,
        });
    }
    acc.unwrap_or(DataType::String)
}

fn is_numeric(t: DataType) -> bool {
    matches!(t, DataType::Int64 | DataType::Double | DataType::Decimal)
}

/// What a coercion may do besides lossless numeric widening.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Coercion {
    /// Text may be parsed (invariant culture) into a non-text type.
    pub parse_text: bool,
    /// Non-text values may be formatted (invariant) into a text column.
    pub format_text: bool,
}

/// Converts `v` to column type `to` (rules in `docs/formats/data-sources.md`).
pub(crate) fn coerce(v: CellValue, to: DataType, mode: Coercion) -> Result<CellValue, String> {
    if fits(&v, to) {
        return Ok(v);
    }
    let shown = || format_invariant(&v).map(|s| truncate(&s)).unwrap_or_default();
    let mismatch = |want: &str| Err(format!("value `{}` cannot be converted to {want}", shown()));
    match (to, &v) {
        (DataType::Double, CellValue::Int(n)) => Ok(CellValue::Real(*n as f64)),
        (DataType::Double, CellValue::Decimal(n)) => Ok(CellValue::Real(*n as f64 / 10_000.0)),
        (DataType::Int64, CellValue::Real(r)) => int_from_f64(*r)
            .map(CellValue::Int)
            .map_or_else(|| mismatch("int64"), Ok),
        (DataType::Int64, CellValue::Decimal(n)) if n % 10_000 == 0 => Ok(CellValue::Int(n / 10_000)),
        (DataType::Decimal, CellValue::Int(n)) => n
            .checked_mul(10_000)
            .map(CellValue::Decimal)
            .map_or_else(|| mismatch("decimal"), Ok),
        (DataType::Decimal, CellValue::Real(r)) => decimal_from_f64(*r)
            .map(CellValue::Decimal)
            .map_or_else(|| mismatch("decimal"), Ok),
        (DataType::String, _) if mode.format_text => {
            Ok(format_invariant(&v).map_or(CellValue::Null, CellValue::Text))
        }
        (_, CellValue::Text(t)) if mode.parse_text => parse_text_as(t, to, Culture::INVARIANT),
        (DataType::Binary, _) => Err("binary columns can only hold blanks".into()),
        _ => mismatch(to.as_tmsl().unwrap_or("unknown")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ole_epoch_and_known_dates() {
        assert_eq!(days_from_civil(1899, 12, 30), OLE_EPOCH);
        assert_eq!(ole_date(1899, 12, 30), Ok(0));
        assert_eq!(ole_date(1900, 1, 1), Ok(2));
        assert_eq!(ole_date(2026, 1, 5), Ok(46027));
        assert!(ole_date(2025, 2, 29).is_err());
        assert!(ole_date(2024, 2, 29).is_ok());
        assert!(ole_date(0, 1, 1).is_err());
        for day in [-700_000i64, -1, 0, 1, 46_027, 2_958_465] {
            let (y, m, d) = civil_from_days(day + OLE_EPOCH);
            assert_eq!(days_from_civil(y, m, d) - OLE_EPOCH, day);
        }
    }

    #[test]
    fn datetime_split_round_trips() {
        for (day, ms) in [
            (46_027, 0),
            (46_027, 1),
            (46_027, MS_PER_DAY - 1),
            (-5, 43_200_000),
            (2_958_465, 86_399_999),
        ] {
            let v = ole_value(day, ms);
            assert_eq!(split_ole(v), Some((day, ms)));
        }
        assert_eq!(split_ole(f64::NAN), None);
    }

    #[test]
    fn iso_and_culture_dates() {
        let d = ole_value(46_027, 0);
        assert_eq!(parse_iso_datetime("2026-01-05"), Some(d));
        assert_eq!(parse_iso_datetime("2026-01-05T12:00"), Some(d + 0.5));
        assert_eq!(parse_iso_datetime("2026-01-05 12:00:00.000"), Some(d + 0.5));
        assert_eq!(parse_iso_datetime("2026-01-05T13:00:00+01:00"), Some(d + 0.5));
        assert_eq!(parse_iso_datetime("2026-01-05T12:00:00Z"), Some(d + 0.5));
        assert_eq!(
            parse_iso_datetime("2026-01-05T00:30:00+01:00"),
            Some(ole_value(46_026, 84_600_000))
        );
        assert_eq!(parse_iso_datetime("2026-13-05"), None);
        assert_eq!(parse_iso_datetime("2026-01-05X"), None);
        assert_eq!(parse_iso_datetime("2026-01-05T25:00"), None);
        assert_eq!(parse_iso_datetime("é"), None);
        assert_eq!(parse_culture_datetime("05/01/2026", DateOrder::Dmy), Some(d));
        assert_eq!(parse_culture_datetime("1/5/2026", DateOrder::Mdy), Some(d));
        assert_eq!(
            parse_culture_datetime("5.1.2026 12:00", DateOrder::Dmy),
            Some(d + 0.5)
        );
        assert_eq!(
            parse_culture_datetime("1/5/2026 12:00 PM", DateOrder::Mdy),
            Some(d + 0.5)
        );
        assert_eq!(
            parse_culture_datetime("1/5/2026 12:00 AM", DateOrder::Mdy),
            Some(d)
        );
        assert_eq!(parse_culture_datetime("2026/01/05", DateOrder::Ymd), Some(d));
        assert_eq!(parse_culture_datetime("13/13/2026", DateOrder::Dmy), None);
        assert_eq!(parse_culture_datetime("1/5/26", DateOrder::Mdy), None);
    }

    #[test]
    fn numbers() {
        assert_eq!(parse_int("-42"), Some(-42));
        assert_eq!(parse_int("+7"), Some(7));
        assert_eq!(parse_int("1.0"), None);
        assert_eq!(parse_int("99999999999999999999"), None);
        assert_eq!(parse_double("1,5", ','), Some(1.5));
        assert_eq!(parse_double("1.5", ','), None);
        assert_eq!(parse_double("-1.5e3", '.'), Some(-1500.0));
        assert_eq!(parse_double("1e", '.'), None);
        assert_eq!(parse_double("inf", '.'), None);
        assert!(parse_double_lenient("NaN", '.').is_some_and(f64::is_nan));
        assert_eq!(parse_decimal("12.34565", '.'), Some(123_457));
        assert_eq!(parse_decimal("-0,5", ','), Some(-5000));
        assert_eq!(parse_decimal("1e2", '.'), Some(1_000_000));
        assert_eq!(parse_decimal("9999999999999999999", '.'), None);
        assert_eq!(parse_decimal("-922337203685477.5808", '.'), Some(i64::MIN));
        assert_eq!(parse_decimal("922337203685477.5808", '.'), None);
        assert_eq!(format_decimal(-5000, '.'), "-0.5");
        assert_eq!(format_decimal(123_400, ','), "12,34");
        assert_eq!(format_decimal(i64::MIN, '.'), "-922337203685477.5808");
        assert_eq!(format_double(1.0, '.'), "1.0");
        assert_eq!(format_double(1.5, ','), "1,5");
    }

    #[test]
    fn coercions() {
        let strict = Coercion {
            parse_text: false,
            format_text: false,
        };
        let lenient = Coercion {
            parse_text: true,
            format_text: true,
        };
        assert_eq!(
            coerce(CellValue::Int(2), DataType::Double, strict),
            Ok(CellValue::Real(2.0))
        );
        assert_eq!(
            coerce(CellValue::Real(2.0), DataType::Int64, strict),
            Ok(CellValue::Int(2))
        );
        assert!(coerce(CellValue::Real(2.5), DataType::Int64, strict).is_err());
        assert_eq!(
            coerce(CellValue::Real(2.5), DataType::Decimal, strict),
            Ok(CellValue::Decimal(25_000))
        );
        assert!(coerce(CellValue::Int(i64::MAX), DataType::Decimal, strict).is_err());
        assert!(coerce(CellValue::Text("1".into()), DataType::Int64, strict).is_err());
        assert_eq!(
            coerce(CellValue::Text("1".into()), DataType::Int64, lenient),
            Ok(CellValue::Int(1))
        );
        assert_eq!(
            coerce(CellValue::Int(1), DataType::String, lenient),
            Ok(CellValue::Text("1".into()))
        );
        assert!(coerce(CellValue::Bool(true), DataType::Int64, lenient).is_err());
        assert_eq!(
            coerce(CellValue::Text("2026-01-05".into()), DataType::DateTime, lenient),
            Ok(CellValue::DateTime(46_027.0))
        );
        assert_eq!(
            coerce(CellValue::Null, DataType::Int64, strict),
            Ok(CellValue::Null)
        );
    }

    #[test]
    fn inference_from_values() {
        use CellValue::*;
        assert_eq!(infer_from_values(&[Int(1), Null, Int(2)]), DataType::Int64);
        assert_eq!(infer_from_values(&[Int(1), Real(1.5)]), DataType::Double);
        assert_eq!(infer_from_values(&[Int(1), Text("a".into())]), DataType::String);
        assert_eq!(infer_from_values(&[Null]), DataType::String);
        assert_eq!(infer_from_values(&[Bool(true)]), DataType::Boolean);
    }

    #[test]
    fn cultures() {
        assert_eq!(Culture::from_name("fr-FR"), Culture::fr_fr());
        assert_eq!(Culture::from_name("en-US"), Culture::en_us());
        assert_eq!(Culture::from_name("en-GB").date_order, DateOrder::Dmy);
        assert_eq!(Culture::from_name("ja-JP").date_order, DateOrder::Ymd);
        assert_eq!(Culture::from_name("xx"), Culture::INVARIANT);
    }
}
