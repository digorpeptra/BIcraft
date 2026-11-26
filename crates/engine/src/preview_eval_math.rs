//! Math, trigonometric, information, bit, conversion, time and path functions of the DAX
//! evaluator ([`super`]): `PI`, `SIN`, `COS`, `TAN`, `ASIN`, `ACOS`, `ATAN`, `SINH`, `COSH`,
//! `TANH`, `ASINH`, `ACOSH`, `ATANH`, `COT`, `COTH`, `DEGREES`, `RADIANS`, `EXP`, `LN`, `LOG`,
//! `LOG10`, `SQRTPI`, `FACT`, `COMBIN`, `COMBINA`, `PERMUT`, `GCD`, `LCM`, `EVEN`, `ODD`,
//! `QUOTIENT`, `CEILING`, `FLOOR`, `ISO.CEILING`, `MROUND`, `BITAND`, `BITOR`, `BITXOR`,
//! `BITLSHIFT`, `BITRSHIFT`, `ISEVEN`, `ISODD`, `ISNUMBER`, `ISTEXT`, `ISNONTEXT`, `ISLOGICAL`,
//! `ISERROR`, `IFERROR`, `CONVERT`, `TIME`, `TIMEVALUE`, `DATEVALUE`, `YEARFRAC`, `NETWORKDAYS`,
//! `FIXED`, `PATHITEM`, `PATHITEMREVERSE`, `PATHLENGTH`, `PATHCONTAINS`, `COMBINEVALUES`.
//!
//! Semantics follow the public DAX reference; each function was checked against the Analysis
//! Services engine of Power BI Desktop 2.159 (`tests/dax_coverage/expected.json`), blank
//! arguments and errors included:
//! - a blank number argument gives the result for 0, or blank when that result is 0
//!   (`SIN(BLANK())` and `CEILING(BLANK(), 1)` are blank, `COS(BLANK())` is 1), without the
//!   domain check (`LN(BLANK())` is -Infinity, `LN(0)` an error); a result that is not a finite
//!   number is an error (`EXP(1000)`);
//! - whole-number arguments are rounded half away from zero (`GCD(24.9, 36.2)` is 1,
//!   `ISEVEN(3.9)` is TRUE, `TIME(1.9, 30.9, 0)` is 02:31), except the one of `FACT`, which is
//!   truncated;
//! - rounding to a multiple (`CEILING`, `FLOOR`, `ISO.CEILING`, `MROUND`) works on the
//!   quotient by the significance; a number and a significance of opposite signs are an error
//!   for `FLOOR`, `CEILING` (positive number) and `MROUND`;
//! - `ISERROR` and `IFERROR` catch the evaluation errors DAX catches (conversions, division of
//!   Currency by zero, a table of several values used as a value, `ERROR`), never a construct
//!   bicraft does not evaluate: that stays `unsupported`.

use bicraft_dax::ast::Expr;
use bicraft_locale::{Locale, vb};

use super::super::Fail;
use super::text::MAX_TEXT;
use super::{
    Engine, Scope, V, arg, currency, dax_error, days_in_month, keyword, need, num, serial, text_of, truth,
    unsupported, ymd,
};

/// Functions this module evaluates.
pub(super) fn handles(up: &str) -> bool {
    matches!(
        up,
        "PI" | "SIN"
            | "COS"
            | "TAN"
            | "ASIN"
            | "ACOS"
            | "ATAN"
            | "SINH"
            | "COSH"
            | "TANH"
            | "ASINH"
            | "ACOSH"
            | "ATANH"
            | "COT"
            | "COTH"
            | "DEGREES"
            | "RADIANS"
            | "EXP"
            | "LN"
            | "LOG"
            | "LOG10"
            | "SQRTPI"
            | "FACT"
            | "COMBIN"
            | "COMBINA"
            | "PERMUT"
            | "GCD"
            | "LCM"
            | "EVEN"
            | "ODD"
            | "QUOTIENT"
            | "CEILING"
            | "FLOOR"
            | "ISO.CEILING"
            | "MROUND"
            | "BITAND"
            | "BITOR"
            | "BITXOR"
            | "BITLSHIFT"
            | "BITRSHIFT"
            | "ISEVEN"
            | "ISODD"
            | "ISNUMBER"
            | "ISTEXT"
            | "ISNONTEXT"
            | "ISLOGICAL"
            | "ISERROR"
            | "IFERROR"
            | "CONVERT"
            | "TIME"
            | "TIMEVALUE"
            | "DATEVALUE"
            | "YEARFRAC"
            | "NETWORKDAYS"
            | "FIXED"
            | "PATHITEM"
            | "PATHITEMREVERSE"
            | "PATHLENGTH"
            | "PATHCONTAINS"
            | "COMBINEVALUES"
    )
}

/// Largest argument of `FACT`, `COMBIN` and `PERMUT` (170! is the largest finite double).
const MAX_FACT: f64 = 170.0;

/// `n!` for `0 <= n <= 170`.
fn factorial(n: f64) -> f64 {
    let mut r = 1.0f64;
    let mut k = 2.0f64;
    while k <= n {
        r *= k;
        k += 1.0;
    }
    r
}

/// `C(n, k)` with integer `0 <= k <= n`, by the multiplicative formula (exact for small values).
fn combinations(n: f64, k: f64) -> f64 {
    let k = k.min(n - k);
    let mut r = 1.0f64;
    let mut i = 1.0f64;
    while i <= k {
        r = r * (n - k + i) / i;
        i += 1.0;
    }
    r.round()
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a
}

/// The result of a function of a number given a blank: blank when it is 0.
fn of_blank(r: f64) -> V {
    if r == 0.0 { V::Blank } else { V::Num(r) }
}

/// A finite result, else the DAX error for an argument outside the function's domain.
fn finite(f: &str, x: f64) -> Result<V, Fail> {
    if x.is_finite() {
        Ok(V::Num(x))
    } else {
        dax_error(format!("{f}(): an argument is outside the function's domain"))
    }
}

/// `x` rounded half away from zero (the rounding of `MROUND`, `EVEN`, `CONVERT`…).
fn round_half_away(x: f64) -> f64 {
    x.round()
}

/// The month (1–12) a word names in `loc` or in English, full or abbreviated.
fn month_word(w: &str, loc: &Locale) -> Option<i64> {
    let w = w.trim_end_matches('.');
    if w.len() < 3 {
        return None;
    }
    let en = Locale::en_us();
    for names in [&loc.months, &loc.months_abbr, &en.months, &en.months_abbr] {
        if let Some(i) = names
            .iter()
            .position(|m| m.trim_end_matches('.').eq_ignore_ascii_case(w))
        {
            return i64::try_from(i).ok().map(|i| i + 1);
        }
    }
    None
}

/// A day number of a text date, as DAX converts text to a date in culture `loc`: `YYYY-MM-DD`,
/// `MM/DD/YYYY` (`DD/MM/YYYY` in a day-first culture), a month name (`March 5, 2024`,
/// `5 March 2024`, `March 2024`: the first of the month), each with an optional time; or a time
/// alone (`14:30`, `2:30 PM`: a fraction of day 0). An ISO `T` separator is refused, as by
/// `DATEVALUE`.
pub(super) fn text_date(t: &str, loc: &Locale) -> Option<f64> {
    let t = t.trim();
    if t.is_empty() {
        return None;
    }
    // the time: from the first token holding a ':' (with a following AM/PM)
    let tokens: Vec<&str> = t.split([' ', ',']).filter(|s| !s.is_empty()).collect();
    let time_at = tokens.iter().position(|s| s.contains(':'));
    let (date_part, time_part) = match time_at {
        Some(i) => (tokens.get(..i).unwrap_or(&[]), tokens.get(i..).unwrap_or(&[])),
        None => (&tokens[..], &[][..]),
    };
    let mut day = 0.0f64;
    if !date_part.is_empty() {
        let (y, m, d) = match date_part {
            [one] => {
                let parts: Vec<&str> = one.split(['-', '/', '.']).collect();
                let [a, b, c] = parts.as_slice() else {
                    return None;
                };
                let (a, b, c): (i64, i64, i64) = (a.parse().ok()?, b.parse().ok()?, c.parse().ok()?);
                if a > 31 {
                    (a, b, c)
                } else if loc.day_first() {
                    (c, b, a)
                } else {
                    (c, a, b)
                }
            }
            words => {
                let mut month = None;
                let mut nums = Vec::new();
                for w in words {
                    match w.parse::<i64>() {
                        Ok(n) => nums.push(n),
                        Err(_) => match month_word(w, loc) {
                            Some(m) if month.is_none() => month = Some(m),
                            _ => return None,
                        },
                    }
                }
                let m = month?;
                match nums.as_slice() {
                    [y] => (*y, m, 1),
                    [a, b] if *a > 31 => (*a, m, *b),
                    [d, y] => (*y, m, *d),
                    _ => return None,
                }
            }
        };
        if !(1..=12).contains(&m) || !(1..=9999).contains(&y) || d < 1 || d > days_in_month(y, m) {
            return None;
        }
        day = serial(y, m, d) as f64;
    }
    if !time_part.is_empty() {
        day += time_text(&time_part.join(" "))?;
    }
    Some(day)
}

/// A number as DAX writes it in text: whole numbers in full, others with 15 significant digits,
/// in scientific notation from 1E+15 (`1E+20`), plain below 1 (`0.00001`); infinities as `inf`
/// (Desktop 2.159).
pub(super) fn number_text(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "inf".into() } else { "-inf".into() };
    }
    if x.fract() == 0.0 && x.abs() < 9.0e15 {
        return format!("{}", x as i64);
    }
    let g = bicraft_locale::number::general(x, '.');
    let Some((mant, exp)) = g.split_once("E-") else {
        return g;
    };
    // small numbers: plain digits instead of `1E-05`
    let neg = mant.starts_with('-');
    let digits: String = mant.chars().filter(char::is_ascii_digit).collect();
    let zeros = exp.parse::<usize>().unwrap_or(0).saturating_sub(1);
    format!("{}0.{}{digits}", if neg { "-" } else { "" }, "0".repeat(zeros))
}

/// The fraction of a day of `hh:mm[:ss]` with an optional `AM`/`PM`.
fn time_text(t: &str) -> Option<f64> {
    let up = t.trim().to_ascii_uppercase();
    let (body, pm) = match (up.strip_suffix("PM"), up.strip_suffix("AM")) {
        (Some(b), _) => (b.trim().to_string(), Some(true)),
        (_, Some(b)) => (b.trim().to_string(), Some(false)),
        _ => (up, None),
    };
    let mut q = body.split(':');
    let mut h: f64 = q.next()?.trim().parse().ok()?;
    let mi: f64 = q.next().map_or(Some(0.0), |s| s.trim().parse().ok())?;
    let se: f64 = q.next().map_or(Some(0.0), |s| s.trim().parse().ok())?;
    if q.next().is_some() || !(0.0..60.0).contains(&mi) || !(0.0..60.0).contains(&se) {
        return None;
    }
    match pm {
        Some(true) if h < 12.0 => h += 12.0,
        Some(false) if h == 12.0 => h = 0.0,
        _ => {}
    }
    if !(0.0..24.0).contains(&h) {
        return None;
    }
    Some((h * 3600.0 + mi * 60.0 + se) / 86_400.0)
}

/// Excel's `YEARFRAC` for bases 0 (US 30/360), 1 (actual/actual), 2 (actual/360), 3
/// (actual/365) and 4 (European 30/360); `a <= b` are day numbers.
fn year_frac(a: i64, b: i64, basis: i64) -> Option<f64> {
    let (y1, m1, d1) = ymd(a);
    let (y2, m2, d2) = ymd(b);
    let days360 = |mut d1: i64, mut d2: i64, us: bool| -> f64 {
        if us {
            let last_feb = |y: i64, m: i64, d: i64| m == 2 && d == days_in_month(y, 2);
            if last_feb(y1, m1, d1) && last_feb(y2, m2, d2) {
                d2 = 30;
            }
            if last_feb(y1, m1, d1) {
                d1 = 30;
            }
            if d2 == 31 && d1 >= 30 {
                d2 = 30;
            }
            if d1 == 31 {
                d1 = 30;
            }
        } else {
            d1 = d1.min(30);
            d2 = d2.min(30);
        }
        ((y2 - y1) * 360 + (m2 - m1) * 30 + (d2 - d1)) as f64 / 360.0
    };
    Some(match basis {
        0 => days360(d1, d2, true),
        4 => days360(d1, d2, false),
        2 => (b - a) as f64 / 360.0,
        3 => (b - a) as f64 / 365.0,
        1 => {
            let leap = |y: i64| days_in_month(y, 2) == 29;
            let days = (b - a) as f64;
            if y1 == y2 {
                days / if leap(y1) { 366.0 } else { 365.0 }
            } else if y2 == y1 + 1 && (m2 < m1 || (m2 == m1 && d2 <= d1)) {
                // less than a year apart: 366 if a leap day falls inside the interval
                let feb29 = |y: i64| leap(y).then(|| serial(y, 2, 29));
                let inside = [feb29(y1), feb29(y2)]
                    .into_iter()
                    .flatten()
                    .any(|f| a <= f && f <= b);
                days / if inside { 366.0 } else { 365.0 }
            } else {
                let years = (y2 - y1 + 1) as f64;
                let total = (serial(y2.saturating_add(1), 1, 1) - serial(y1, 1, 1)) as f64;
                days / (total / years)
            }
        }
        _ => return None,
    })
}

/// `x` rounded to a multiple of `m` by `CEILING`, `FLOOR`, `ISO.CEILING` or `MROUND`.
fn multiple(up: &str, x: f64, m: f64) -> Result<f64, Fail> {
    if m == 0.0 {
        return Ok(0.0);
    }
    let opposite = (x > 0.0 && m < 0.0) || (x < 0.0 && m > 0.0);
    Ok(match up {
        "ISO.CEILING" => (x / m.abs()).ceil() * m.abs(),
        "MROUND" if opposite => {
            return dax_error("MROUND(): the number and the multiple have different signs");
        }
        "MROUND" => round_half_away(x / m) * m,
        _ if x > 0.0 && m < 0.0 => {
            return dax_error(format!(
                "{up}(): the number and the significance have different signs"
            ));
        }
        "CEILING" => (x / m).ceil() * m,
        _ => (x / m).floor() * m,
    })
}

/// Weekend days (0 = Sunday … 6 = Saturday) of a `NETWORKDAYS` weekend code.
fn weekend_days(code: i64) -> Option<[bool; 7]> {
    let mut w = [false; 7];
    match code {
        1..=7 => {
            // 1: Saturday-Sunday, 2: Sunday-Monday, … 7: Friday-Saturday
            let first = (code + 5).rem_euclid(7);
            let first = usize::try_from(first).ok()?;
            *w.get_mut(first)? = true;
            *w.get_mut((first + 1) % 7)? = true;
        }
        11..=17 => {
            // 11: Sunday only, 12: Monday only, … 17: Saturday only
            *w.get_mut(usize::try_from(code - 11).ok()?)? = true;
        }
        _ => return None,
    }
    Some(w)
}

impl Engine<'_, '_> {
    /// A number argument, blank as 0.
    fn num_arg(&mut self, e: &Expr, s: &Scope) -> Result<f64, Fail> {
        Ok(num(&self.scalar(e, s)?)?.unwrap_or(0.0))
    }

    /// An optional number argument.
    fn num_opt(&mut self, args: &[Option<Expr>], i: usize, default: f64, s: &Scope) -> Result<f64, Fail> {
        match arg(args, i) {
            Some(e) => self.num_arg(e, s),
            None => Ok(default),
        }
    }

    /// A whole-number argument (rounded half away from zero), in the i64 range.
    fn whole_arg(&mut self, e: &Expr, s: &Scope, f: &str) -> Result<i64, Fail> {
        let x = round_half_away(self.num_arg(e, s)?);
        if !x.is_finite() || x.abs() > 9.0e18 {
            return dax_error(format!("{f}(): a number is too large"));
        }
        Ok(x as i64)
    }

    /// Evaluates `e`; an evaluation error (not an unsupported construct) is `None`.
    fn caught(&mut self, e: &Expr, s: &Scope) -> Result<Option<V>, Fail> {
        match self.scalar(e, s) {
            Ok(v) => Ok(Some(v)),
            Err(Fail::Miss(super::Miss::Error(_))) => Ok(None),
            Err(other) => Err(other),
        }
    }

    /// `f` of the first argument (see the module notes for blanks and domains).
    fn unary(&mut self, up: &str, args: &[Option<Expr>], s: &Scope, f: fn(f64) -> f64) -> Result<V, Fail> {
        match num(&self.scalar(need(args, 0, up)?, s)?)? {
            None => Ok(of_blank(f(0.0))),
            Some(x) => finite(up, f(x)),
        }
    }

    pub(super) fn math_function(&mut self, up: &str, args: &[Option<Expr>], s: &Scope) -> Result<V, Fail> {
        let one = |eng: &mut Self| -> Result<f64, Fail> { eng.num_arg(need(args, 0, up)?, s) };
        let first_blank = |eng: &mut Self| -> Result<bool, Fail> {
            Ok(match arg(args, 0) {
                Some(e) => eng.scalar(e, s)?.is_blank(),
                None => false,
            })
        };
        match up {
            "PI" => Ok(V::Num(std::f64::consts::PI)),
            "SIN" => self.unary(up, args, s, f64::sin),
            "COS" => self.unary(up, args, s, f64::cos),
            "TAN" => self.unary(up, args, s, f64::tan),
            "ASIN" => self.unary(up, args, s, f64::asin),
            "ACOS" => self.unary(up, args, s, f64::acos),
            "ATAN" => self.unary(up, args, s, f64::atan),
            "SINH" => self.unary(up, args, s, f64::sinh),
            "COSH" => self.unary(up, args, s, f64::cosh),
            "TANH" => self.unary(up, args, s, f64::tanh),
            "ASINH" => self.unary(up, args, s, f64::asinh),
            "ACOSH" => self.unary(up, args, s, f64::acosh),
            "ATANH" => self.unary(up, args, s, f64::atanh),
            "COT" | "COTH" => {
                let x = one(self)?;
                if x == 0.0 {
                    return dax_error(format!("{up}() of 0"));
                }
                finite(up, 1.0 / if up == "COT" { x.tan() } else { x.tanh() })
            }
            "DEGREES" => self.unary(up, args, s, f64::to_degrees),
            "RADIANS" => self.unary(up, args, s, f64::to_radians),
            "EXP" => self.unary(up, args, s, f64::exp),
            "SQRTPI" => self.unary(up, args, s, |x| (x * std::f64::consts::PI).sqrt()),
            "LN" | "LOG10" | "LOG" => {
                let v = self.scalar(need(args, 0, up)?, s)?;
                let Some(x) = num(&v)? else {
                    // the logarithm of 0, without the domain check (Desktop 2.159)
                    return Ok(V::Num(f64::NEG_INFINITY));
                };
                let base = if up == "LOG" {
                    self.num_opt(args, 1, 10.0, s)?
                } else if up == "LOG10" {
                    10.0
                } else {
                    std::f64::consts::E
                };
                if x <= 0.0 || base <= 0.0 || base == 1.0 {
                    return dax_error(format!("{up}() of a number that is not positive"));
                }
                finite(
                    up,
                    match up {
                        "LN" => x.ln(),
                        "LOG10" => x.log10(),
                        _ => x.ln() / base.ln(),
                    },
                )
            }
            "FACT" => {
                let n = one(self)?.trunc();
                if !(0.0..=MAX_FACT).contains(&n) {
                    return dax_error("FACT() of a negative or too large number");
                }
                Ok(V::Num(factorial(n)))
            }
            "COMBIN" | "COMBINA" | "PERMUT" => {
                let n = round_half_away(one(self)?);
                let k = round_half_away(self.num_arg(need(args, 1, up)?, s)?);
                let n = if up == "COMBINA" && k > 0.0 {
                    n + k - 1.0
                } else {
                    n
                };
                if n < 0.0 || k < 0.0 || k > n || n > 1.0e9 {
                    return dax_error(format!("{up}(): the numbers are out of range"));
                }
                if up == "PERMUT" {
                    if n > MAX_FACT {
                        return dax_error("PERMUT(): the numbers are out of range");
                    }
                    return finite(up, (factorial(n) / factorial(n - k)).round());
                }
                finite(up, combinations(n, k))
            }
            "GCD" | "LCM" => {
                let a = round_half_away(one(self)?);
                let b = round_half_away(self.num_arg(need(args, 1, up)?, s)?);
                if a < 0.0 || b < 0.0 || a >= 9.0e15 || b >= 9.0e15 {
                    return dax_error(format!("{up}(): the numbers must be positive"));
                }
                let (x, y) = (a as u64, b as u64);
                let g = gcd(x, y);
                Ok(V::Num(if up == "GCD" {
                    g as f64
                } else {
                    x.checked_div(g).map_or(0.0, |q| q as f64 * y as f64)
                }))
            }
            "EVEN" | "ODD" => {
                let blank = first_blank(self)?;
                let x = one(self)?;
                let a = x.abs().ceil();
                let r = if up == "EVEN" {
                    if a % 2.0 == 0.0 { a } else { a + 1.0 }
                } else if a % 2.0 == 1.0 {
                    a
                } else {
                    a + 1.0
                };
                let r = if x < 0.0 { -r } else { r };
                Ok(if blank { of_blank(r) } else { V::Num(r) })
            }
            "QUOTIENT" => {
                let blank = first_blank(self)?;
                let a = one(self)?;
                let b = self.num_arg(need(args, 1, up)?, s)?;
                if b == 0.0 {
                    return dax_error("QUOTIENT(): division by zero");
                }
                let r = (a / b).trunc();
                if blank { Ok(of_blank(r)) } else { finite(up, r) }
            }
            "CEILING" | "FLOOR" | "ISO.CEILING" | "MROUND" => {
                let xv = self.scalar(need(args, 0, up)?, s)?;
                let mv = match (up, arg(args, 1)) {
                    ("ISO.CEILING", None) => V::Num(1.0),
                    _ => self.scalar(need(args, 1, up)?, s)?,
                };
                let (x, m) = (num(&xv)?.unwrap_or(0.0), num(&mv)?.unwrap_or(0.0));
                let cur = matches!(xv, V::Cur(_)) || matches!(mv, V::Cur(_));
                let r = multiple(up, x, m)?;
                if xv.is_blank() {
                    return Ok(of_blank(r));
                }
                // A Currency number or significance gives a Currency (Desktop 2.159).
                if cur {
                    return match currency::from_f64(r) {
                        Some(n) => Ok(V::Cur(n)),
                        None => dax_error(format!("{up}(): the result is too large for a Currency")),
                    };
                }
                finite(up, r)
            }
            "BITAND" | "BITOR" | "BITXOR" | "BITLSHIFT" | "BITRSHIFT" => {
                let a = self.whole_arg(need(args, 0, up)?, s, up)?;
                let b = self.whole_arg(need(args, 1, up)?, s, up)?;
                Ok(V::Num(match up {
                    "BITAND" => a & b,
                    "BITOR" => a | b,
                    "BITXOR" => a ^ b,
                    _ => {
                        let left = (up == "BITLSHIFT") == (b >= 0);
                        let k = u32::try_from(b.unsigned_abs().min(63)).unwrap_or(63);
                        if left {
                            a.checked_shl(k).unwrap_or(0)
                        } else {
                            a >> k
                        }
                    }
                } as f64))
            }
            "ISEVEN" | "ISODD" => {
                let n = round_half_away(one(self)?);
                let even = n % 2.0 == 0.0;
                Ok(V::Bool(even == (up == "ISEVEN")))
            }
            "ISNUMBER" | "ISTEXT" | "ISNONTEXT" | "ISLOGICAL" => {
                let v = self.scalar(need(args, 0, up)?, s)?;
                Ok(V::Bool(match up {
                    "ISNUMBER" => matches!(v, V::Num(_) | V::Cur(_) | V::Date(_)),
                    "ISTEXT" => matches!(v, V::Text(_)),
                    "ISNONTEXT" => !matches!(v, V::Text(_)),
                    _ => matches!(v, V::Bool(_)),
                }))
            }
            "ISERROR" => Ok(V::Bool(self.caught(need(args, 0, up)?, s)?.is_none())),
            "IFERROR" => match self.caught(need(args, 0, up)?, s)? {
                Some(v) => Ok(v),
                None => self.scalar(need(args, 1, up)?, s),
            },
            "CONVERT" => {
                let v = self.scalar(need(args, 0, up)?, s)?;
                let Some(ty) = arg(args, 1).and_then(keyword) else {
                    return unsupported("CONVERT() to a type that is not a keyword");
                };
                if v.is_blank() {
                    return Ok(V::Blank);
                }
                match ty.as_str() {
                    "INTEGER" => {
                        let x = round_half_away(num(&v)?.unwrap_or(0.0));
                        if !x.is_finite() || x.abs() > 9.2e18 {
                            return dax_error("CONVERT(): the number is too large for an integer");
                        }
                        Ok(V::Num(x))
                    }
                    "DOUBLE" => Ok(V::Num(num(&v)?.unwrap_or(0.0))),
                    "CURRENCY" => match v {
                        V::Cur(n) => Ok(V::Cur(n)),
                        other => match currency::from_f64(num(&other)?.unwrap_or(0.0)) {
                            Some(n) => Ok(V::Cur(n)),
                            None => dax_error("CONVERT(): the number is too large for a Currency"),
                        },
                    },
                    "STRING" => Ok(V::Text(text_of(&v))),
                    "BOOLEAN" => Ok(V::Bool(truth(&v)?)),
                    "DATETIME" => match v {
                        V::Text(t) => match text_date(&t, self.culture(None)?) {
                            Some(x) => Ok(V::Date(x)),
                            None => {
                                dax_error(format!("cannot convert value '{t}' of type Text to type Date"))
                            }
                        },
                        other => Ok(V::Date(num(&other)?.unwrap_or(0.0))),
                    },
                    other => unsupported(format!("CONVERT() to {other}")),
                }
            }
            "TIME" => {
                let h = self.whole_arg(need(args, 0, up)?, s, up)?;
                let m = self.whole_arg(need(args, 1, up)?, s, up)?;
                let sec = self.whole_arg(need(args, 2, up)?, s, up)?;
                let total = h
                    .checked_mul(3600)
                    .and_then(|x| x.checked_add(m.checked_mul(60)?))
                    .and_then(|x| x.checked_add(sec));
                match total {
                    Some(t) if t >= 0 && h <= 32_767 && m <= 32_767 && sec <= 32_767 => {
                        Ok(V::Date(t.rem_euclid(86_400) as f64 / 86_400.0))
                    }
                    _ => dax_error("TIME(): an argument is out of range"),
                }
            }
            "TIMEVALUE" | "DATEVALUE" => {
                let v = self.scalar(need(args, 0, up)?, s)?;
                let x = match v {
                    V::Blank => return Ok(V::Blank),
                    V::Text(t) => match text_date(&t, self.culture(None)?) {
                        Some(x) => x,
                        None => {
                            return dax_error(format!(
                                "cannot convert value '{t}' of type Text to type Date"
                            ));
                        }
                    },
                    other => num(&other)?.unwrap_or(0.0),
                };
                Ok(V::Date(if up == "TIMEVALUE" {
                    x - x.floor()
                } else {
                    x.floor()
                }))
            }
            "YEARFRAC" => {
                let a = self.whole_day(need(args, 0, up)?, s)?;
                let b = self.whole_day(need(args, 1, up)?, s)?;
                let basis = self.num_opt(args, 2, 0.0, s)?.trunc() as i64;
                let (a, b) = if a <= b { (a, b) } else { (b, a) };
                match year_frac(a, b, basis) {
                    Some(x) => Ok(V::Num(x)),
                    None => dax_error("YEARFRAC(): the basis must be 0 to 4"),
                }
            }
            "NETWORKDAYS" => self.networkdays(args, s),
            "FIXED" => {
                let x = one(self)?;
                let d = self.num_opt(args, 1, 2.0, s)?.trunc().clamp(-15.0, 15.0) as i32;
                let no_commas = match arg(args, 2) {
                    Some(e) => truth(&self.scalar(e, s)?)?,
                    None => false,
                };
                let p = 10f64.powi(d);
                let r = (x * p).round() / p;
                let places = usize::try_from(d.max(0)).unwrap_or(0);
                let mut f = if no_commas {
                    "0".to_string()
                } else {
                    "#,0".to_string()
                };
                if places > 0 {
                    f.push('.');
                    f.push_str(&"0".repeat(places));
                }
                let loc = self.culture(None)?;
                match vb::format_number(r, &f, loc) {
                    Some(t) => Ok(V::Text(t)),
                    None => unsupported("FIXED() of this number"),
                }
            }
            "PATHLENGTH" | "PATHITEM" | "PATHITEMREVERSE" | "PATHCONTAINS" => {
                let path = text_of(&self.scalar(need(args, 0, up)?, s)?);
                // An empty path has one (empty) item; a blank one has none (Desktop 2.159).
                if up == "PATHLENGTH" && self.scalar(need(args, 0, up)?, s)?.is_blank() {
                    return Ok(V::Blank);
                }
                let items: Vec<&str> = path.split('|').collect();
                match up {
                    "PATHLENGTH" => Ok(V::Num(items.len() as f64)),
                    "PATHCONTAINS" => {
                        let item = text_of(&self.scalar(need(args, 1, up)?, s)?);
                        Ok(V::Bool(items.iter().any(|i| i.eq_ignore_ascii_case(&item))))
                    }
                    _ => {
                        let pos = self.whole_arg(need(args, 1, up)?, s, up)?;
                        let len = i64::try_from(items.len()).unwrap_or(i64::MAX);
                        let at = if up == "PATHITEM" { pos - 1 } else { len - pos };
                        let found = usize::try_from(at)
                            .ok()
                            .filter(|_| pos >= 1)
                            .and_then(|i| items.get(i));
                        let Some(item) = found else {
                            return Ok(V::Blank);
                        };
                        let integer = match arg(args, 2) {
                            None => false,
                            Some(e) => match keyword(e).as_deref() {
                                Some("INTEGER") => true,
                                Some("TEXT") => false,
                                _ => num(&self.scalar(e, s)?)?.unwrap_or(0.0) == 1.0,
                            },
                        };
                        if integer {
                            match item.trim().parse::<f64>() {
                                Ok(x) => Ok(V::Num(x.trunc())),
                                Err(_) => dax_error(format!(
                                    "cannot convert value '{item}' of type Text to type Integer"
                                )),
                            }
                        } else {
                            Ok(V::Text((*item).to_string()))
                        }
                    }
                }
            }
            "COMBINEVALUES" => {
                let delim = text_of(&self.scalar(need(args, 0, up)?, s)?);
                let mut out = String::new();
                for (i, a) in args.iter().skip(1).enumerate() {
                    let Some(a) = a else {
                        return unsupported("COMBINEVALUES() with a missing argument");
                    };
                    if i > 0 {
                        out.push_str(&delim);
                    }
                    out.push_str(&text_of(&self.scalar(a, s)?));
                    if out.len() > MAX_TEXT {
                        return dax_error(format!("a text value longer than {MAX_TEXT} bytes"));
                    }
                }
                Ok(V::Text(out))
            }
            _ => unsupported(format!("{up}() is not evaluated by the preview")),
        }
    }

    /// A date argument as a whole day number (blank: day 0, 1899-12-30).
    fn whole_day(&mut self, e: &Expr, s: &Scope) -> Result<i64, Fail> {
        let x = match self.scalar(e, s)? {
            V::Text(t) => match text_date(&t, self.culture(None)?) {
                Some(x) => x,
                None => return dax_error(format!("cannot convert value '{t}' of type Text to type Date")),
            },
            other => num(&other)?.unwrap_or(0.0),
        };
        if !x.is_finite() || x.abs() > 3.0e6 {
            return dax_error("date out of range");
        }
        Ok(x.floor() as i64)
    }

    fn networkdays(&mut self, args: &[Option<Expr>], s: &Scope) -> Result<V, Fail> {
        let a = self.whole_day(need(args, 0, "NETWORKDAYS")?, s)?;
        let b = self.whole_day(need(args, 1, "NETWORKDAYS")?, s)?;
        let code = self.num_opt(args, 2, 1.0, s)?.trunc() as i64;
        let Some(weekend) = weekend_days(code) else {
            return dax_error("NETWORKDAYS(): the weekend code is not valid");
        };
        let mut holidays = Vec::new();
        if let Some(h) = arg(args, 3) {
            let t = self.table(h, s)?;
            for r in self.tab_values(&t)?.iter() {
                if let Some(x) = r.first().map(num).transpose()?.flatten() {
                    holidays.push(x.floor() as i64);
                }
            }
        }
        let (lo, hi, sign) = if a <= b { (a, b, 1.0) } else { (b, a, -1.0) };
        self.tick(usize::try_from(hi - lo).unwrap_or(usize::MAX))?;
        let mut n = 0i64;
        for d in lo..=hi {
            let w = usize::try_from((d + 6).rem_euclid(7)).unwrap_or(0);
            if !weekend.get(w).copied().unwrap_or(false) && !holidays.contains(&d) {
                n += 1;
            }
        }
        Ok(V::Num(n as f64 * sign))
    }
}

#[cfg(test)]
mod tests {
    use super::super::serial;
    use super::{Locale, number_text, text_date, year_frac};

    #[test]
    fn year_fractions() {
        let d = |y, m, dd| serial(y, m, dd);
        assert_eq!(year_frac(d(2024, 1, 1), d(2024, 7, 1), 0), Some(0.5));
        let x = year_frac(d(2024, 1, 1), d(2024, 7, 1), 1).unwrap();
        assert!((x - 182.0 / 366.0).abs() < 1e-12);
        assert_eq!(year_frac(d(2024, 1, 1), d(2024, 7, 1), 9), None);
    }

    #[test]
    fn text_dates() {
        let en = Locale::get("en-US");
        let fr = Locale::get("fr-FR");
        assert_eq!(text_date("2024-02-29", en), Some(serial(2024, 2, 29) as f64));
        assert_eq!(text_date("3/5/2024", en), Some(serial(2024, 3, 5) as f64));
        assert_eq!(text_date("3/5/2024", fr), Some(serial(2024, 5, 3) as f64));
        assert_eq!(text_date("March 5, 2024", en), Some(serial(2024, 3, 5) as f64));
        assert_eq!(text_date("March 2024", en), Some(serial(2024, 3, 1) as f64));
        assert_eq!(text_date("5 mars 2024", fr), Some(serial(2024, 3, 5) as f64));
        assert_eq!(text_date("5 mars 2024", en), None);
        assert_eq!(text_date("14:30", en), Some(14.5 / 24.0));
        assert_eq!(text_date("2:30 PM", en), Some(14.5 / 24.0));
        assert_eq!(
            text_date("2024-03-05 14:30", en),
            Some(serial(2024, 3, 5) as f64 + 14.5 / 24.0)
        );
        assert_eq!(text_date("2024-02-30", en), None);
        assert_eq!(text_date("abc", en), None);
        assert_eq!(text_date("2024-03-05T10:00:00", en), None);
    }

    #[test]
    fn numbers_as_text() {
        assert_eq!(number_text(1.5), "1.5");
        assert_eq!(number_text(0.00001), "0.00001");
        assert_eq!(number_text(1e-12), "0.000000000001");
        assert_eq!(number_text(1.0 / 3.0), "0.333333333333333");
        assert_eq!(number_text(123_456_789_012_345.6), "123456789012346");
        assert_eq!(number_text(1e20), "1E+20");
        assert_eq!(number_text(-0.5), "-0.5");
        assert_eq!(number_text(f64::INFINITY), "inf");
    }
}
