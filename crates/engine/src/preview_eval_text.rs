//! Text, conversion and date-part functions of the preview evaluator ([`super`]), used by
//! calculated tables and columns (gap G21): `UPPER`, `LOWER`, `LEN`, `LEFT`, `RIGHT`, `MID`,
//! `TRIM`, `SUBSTITUTE`, `REPLACE`, `REPT`, `CONCATENATE`, `EXACT`, `FIND`, `SEARCH`,
//! `CONTAINSSTRING`, `CONTAINSSTRINGEXACT`, `UNICHAR`, `UNICODE`, `VALUE`, `FORMAT`, `WEEKDAY`,
//! `WEEKNUM`, `HOUR`, `MINUTE`, `SECOND`, `TRUNC`, `ROUNDUP`, `ROUNDDOWN`, `SQRT`, `POWER`, `MOD`.
//!
//! Semantics follow the public DAX reference; what it leaves open was checked against the
//! Analysis Services engine of Power BI Desktop 2.158 (`tests/calc_tables/expected.json`):
//! - a blank text argument is the empty string; text functions return text, never blank;
//! - positions are 1-based and count UTF-16 code units, as .NET strings do;
//! - `FIND` is case-sensitive, `SEARCH` and `CONTAINSSTRING` are not; in those two, `?` matches
//!   any one character (UTF-16 unit), `*` any run of characters (none included) and `~` before
//!   `?`, `*` or `~` makes it literal (a `~` before anything else is a literal `~`, unconfirmed);
//!   `SEARCH` returns the first position from which the pattern matches (a leading `*` matches
//!   at the start position);
//! - `FORMAT` covers the named formats and the custom number and date/time format strings in the
//!   model's culture or the culture argument (`bicraft_locale::vb`, checked against the engine
//!   in 59 cultures); a culture whose language bicraft has no data for is refused, not guessed;
//! - `VALUE` reads numbers with the model culture's separators and currency symbol.

use bicraft_dax::ast::Expr;
use bicraft_locale::{Locale, parse, vb};

use super::super::Fail;
use super::{Engine, Scope, V, arg, currency, dax_error, need, num, text_of, unsupported, ymd};

/// Longest text a function builds.
pub(super) const MAX_TEXT: usize = 1 << 20;

/// Functions this module evaluates.
pub(super) fn handles(up: &str) -> bool {
    matches!(
        up,
        "UPPER"
            | "LOWER"
            | "LEN"
            | "LEFT"
            | "RIGHT"
            | "MID"
            | "TRIM"
            | "SUBSTITUTE"
            | "REPLACE"
            | "REPT"
            | "CONCATENATE"
            | "EXACT"
            | "FIND"
            | "SEARCH"
            | "CONTAINSSTRING"
            | "CONTAINSSTRINGEXACT"
            | "UNICHAR"
            | "UNICODE"
            | "VALUE"
            | "FORMAT"
            | "WEEKDAY"
            | "WEEKNUM"
            | "HOUR"
            | "MINUTE"
            | "SECOND"
            | "TRUNC"
            | "ROUNDUP"
            | "ROUNDDOWN"
            | "SQRT"
            | "POWER"
            | "MOD"
    )
}

fn units(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn from_units(u: &[u16]) -> String {
    String::from_utf16_lossy(u)
}

fn capped(s: String) -> Result<V, Fail> {
    if s.len() > MAX_TEXT {
        return dax_error(format!("a text value longer than {MAX_TEXT} bytes"));
    }
    Ok(V::Text(s))
}

/// One element of a `SEARCH`/`CONTAINSSTRING` pattern.
#[derive(Clone, Copy, PartialEq)]
enum Pat {
    Unit(u16),
    One,
    Any,
}

/// The pattern of `find`: `?` and `*` are wildcards, `~` makes the next `?`, `*` or `~` literal.
fn pattern(find: &[u16]) -> Vec<Pat> {
    let (q, star, tilde) = (u16::from(b'?'), u16::from(b'*'), u16::from(b'~'));
    let mut out = Vec::with_capacity(find.len());
    let mut i = 0usize;
    while let Some(&c) = find.get(i) {
        i = i.saturating_add(1);
        if c == tilde
            && let Some(&n) = find.get(i)
            && (n == q || n == star || n == tilde)
        {
            out.push(Pat::Unit(n));
            i = i.saturating_add(1);
        } else if c == q {
            out.push(Pat::One);
        } else if c == star {
            out.push(Pat::Any);
        } else {
            out.push(Pat::Unit(c));
        }
    }
    out
}

fn fold_unit(c: u16, fold: bool) -> u16 {
    if !fold {
        return c;
    }
    char::from_u32(u32::from(c))
        .map(|ch| {
            let mut l = ch.to_lowercase();
            match (l.next(), l.next()) {
                (Some(x), None) if (x as u32) < 0x1_0000 => x as u32 as u16,
                _ => c,
            }
        })
        .unwrap_or(c)
}

/// Whether the segment `seg` (no `*`) matches `within` at `at`.
fn segment_at(within: &[u16], seg: &[Pat], at: usize, fold: bool) -> bool {
    within.get(at..at.saturating_add(seg.len())).is_some_and(|w| {
        w.len() == seg.len()
            && w.iter().zip(seg).all(|(a, p)| match p {
                Pat::Unit(b) => fold_unit(*a, fold) == fold_unit(*b, fold),
                Pat::One => true,
                Pat::Any => false,
            })
    })
}

/// The first position of `seg` in `within` from `from`.
fn segment_from(within: &[u16], seg: &[Pat], from: usize, fold: bool) -> Option<usize> {
    let last = within.len().checked_sub(seg.len())?;
    (from..=last).find(|&i| segment_at(within, seg, i, fold))
}

/// Position (0-based, in UTF-16 units) from which the wildcard pattern of `find` matches
/// `within`, searching from `start`. The pattern is segments between `*`s: the first one must
/// match at the position, each later one anywhere after the previous one (leftmost is enough);
/// when the later segments do not fit after the first possible position they fit after no later
/// one either, so the search is linear in the number of positions tried.
fn wild_position(within: &[u16], find: &[u16], start: usize, fold: bool) -> Option<usize> {
    if start > within.len() {
        return None;
    }
    let pat = pattern(find);
    let mut segs = pat.split(|p| *p == Pat::Any);
    let first = segs.next().unwrap_or_default();
    let rest: Vec<&[Pat]> = segs.collect();
    let at = if rest.is_empty() {
        // no `*`: the segment itself
        return segment_from(within, first, start, fold);
    } else if first.is_empty() {
        start
    } else {
        segment_from(within, first, start, fold)?
    };
    let mut pos = at.saturating_add(first.len());
    for seg in rest {
        if seg.is_empty() {
            continue;
        }
        let found = segment_from(within, seg, pos, fold)?;
        pos = found.saturating_add(seg.len());
    }
    Some(at)
}

/// Position (0-based, in UTF-16 units) of `find` in `within` from `start`.
fn position(within: &[u16], find: &[u16], start: usize, fold: bool) -> Option<usize> {
    if find.is_empty() {
        return (start <= within.len()).then_some(start);
    }
    let last = within.len().checked_sub(find.len())?;
    (start..=last).find(|&i| {
        within.get(i..i.saturating_add(find.len())).is_some_and(|w| {
            w.iter()
                .zip(find)
                .all(|(a, b)| fold_unit(*a, fold) == fold_unit(*b, fold))
        })
    })
}

/// Seconds of the day of a date/time serial, rounded to the second.
fn seconds_of_day(x: f64) -> i64 {
    let frac = x - x.floor();
    ((frac * 86_400.0).round() as i64).clamp(0, 86_399)
}

/// Weekday index of a day serial, 0 = Sunday (1899-12-30 was a Saturday).
fn weekday0(day: i64) -> i64 {
    bicraft_locale::calendar::weekday_of_serial(day)
}

impl Engine<'_, '_> {
    /// The model's culture (`en-US` when unset); `None` for a language bicraft has no data for.
    pub(super) fn culture(&self, name: Option<&str>) -> Result<&'static Locale, Fail> {
        let name = name.or(self.model.culture.as_deref()).unwrap_or("en-US");
        match Locale::find(name) {
            Some(l) => Ok(l),
            None => unsupported(format!("the culture {name}")),
        }
    }

    fn text_arg(&mut self, e: &Expr, s: &Scope) -> Result<String, Fail> {
        Ok(text_of(&self.scalar(e, s)?))
    }

    fn count_arg(&mut self, args: &[Option<Expr>], i: usize, default: i64, s: &Scope) -> Result<i64, Fail> {
        let Some(e) = arg(args, i) else {
            return Ok(default);
        };
        let x = num(&self.scalar(e, s)?)?.unwrap_or(0.0);
        if !x.is_finite() || x.abs() > 1.0e9 {
            return dax_error("a number out of range");
        }
        Ok(x.trunc() as i64)
    }

    /// Evaluates a function [`handles`] accepts.
    pub(super) fn text_function(&mut self, up: &str, args: &[Option<Expr>], s: &Scope) -> Result<V, Fail> {
        // A blank text gives blank, not "" (Desktop 2.158); "" stays "".
        if matches!(
            up,
            "UPPER" | "LOWER" | "LEN" | "LEFT" | "RIGHT" | "MID" | "TRIM" | "SUBSTITUTE" | "REPT"
        ) && let Some(e) = arg(args, 0)
            && matches!(self.scalar(e, s)?, V::Blank)
        {
            return Ok(V::Blank);
        }
        match up {
            "UPPER" | "LOWER" => {
                let t = self.text_arg(need(args, 0, up)?, s)?;
                Ok(V::Text(if up == "UPPER" {
                    t.to_uppercase()
                } else {
                    t.to_lowercase()
                }))
            }
            "LEN" => {
                let t = self.text_arg(need(args, 0, up)?, s)?;
                Ok(V::Num(t.encode_utf16().count() as f64))
            }
            "LEFT" | "RIGHT" => {
                let t = units(&self.text_arg(need(args, 0, up)?, s)?);
                let n = self.count_arg(args, 1, 1, s)?;
                let Ok(n) = usize::try_from(n) else {
                    return dax_error(format!("{up}(): the number of characters must not be negative"));
                };
                let n = n.min(t.len());
                let part = if up == "LEFT" {
                    t.get(..n)
                } else {
                    t.get(t.len().saturating_sub(n)..)
                };
                Ok(V::Text(from_units(part.unwrap_or(&[]))))
            }
            "MID" => {
                let t = units(&self.text_arg(need(args, 0, up)?, s)?);
                let start = self.count_arg(args, 1, 1, s)?;
                let n = self.count_arg(args, 2, 0, s)?;
                if start < 1 || n < 0 {
                    return dax_error("MID(): the start must be at least 1 and the length not negative");
                }
                let from = usize::try_from(start.saturating_sub(1))
                    .unwrap_or(usize::MAX)
                    .min(t.len());
                let to = from.saturating_add(usize::try_from(n).unwrap_or(0)).min(t.len());
                Ok(V::Text(from_units(t.get(from..to).unwrap_or(&[]))))
            }
            "TRIM" => {
                let t = self.text_arg(need(args, 0, up)?, s)?;
                Ok(V::Text(
                    t.split(' ')
                        .filter(|w| !w.is_empty())
                        .collect::<Vec<_>>()
                        .join(" "),
                ))
            }
            "SUBSTITUTE" => {
                let t = self.text_arg(need(args, 0, up)?, s)?;
                let old = self.text_arg(need(args, 1, up)?, s)?;
                let new = self.text_arg(need(args, 2, up)?, s)?;
                if old.is_empty() {
                    return Ok(V::Text(t));
                }
                match arg(args, 3) {
                    None => capped(t.replace(&old, &new)),
                    Some(_) => {
                        let k = self.count_arg(args, 3, 1, s)?;
                        if k < 1 {
                            return dax_error("SUBSTITUTE(): the instance must be at least 1");
                        }
                        let Some((at, _)) = t.match_indices(&old).nth(usize::try_from(k - 1).unwrap_or(0))
                        else {
                            return Ok(V::Text(t));
                        };
                        let mut out = String::with_capacity(t.len());
                        out.push_str(t.get(..at).unwrap_or(""));
                        out.push_str(&new);
                        out.push_str(t.get(at.saturating_add(old.len())..).unwrap_or(""));
                        capped(out)
                    }
                }
            }
            "REPLACE" => {
                let t = units(&self.text_arg(need(args, 0, up)?, s)?);
                let start = self.count_arg(args, 1, 1, s)?;
                let n = self.count_arg(args, 2, 0, s)?;
                let new = units(&self.text_arg(need(args, 3, up)?, s)?);
                if start < 1 || n < 0 {
                    return dax_error("REPLACE(): the start must be at least 1 and the length not negative");
                }
                let from = usize::try_from(start - 1).unwrap_or(usize::MAX).min(t.len());
                let to = from.saturating_add(usize::try_from(n).unwrap_or(0)).min(t.len());
                let mut out: Vec<u16> = t.get(..from).unwrap_or(&[]).to_vec();
                out.extend_from_slice(&new);
                out.extend_from_slice(t.get(to..).unwrap_or(&[]));
                capped(from_units(&out))
            }
            "REPT" => {
                let t = self.text_arg(need(args, 0, up)?, s)?;
                let n = self.count_arg(args, 1, 0, s)?;
                let Ok(n) = usize::try_from(n) else {
                    return dax_error("REPT(): the count must not be negative");
                };
                if t.len().saturating_mul(n) > MAX_TEXT {
                    return dax_error(format!("a text value longer than {MAX_TEXT} bytes"));
                }
                Ok(V::Text(t.repeat(n)))
            }
            "CONCATENATE" => {
                let a = self.scalar(need(args, 0, up)?, s)?;
                let b = self.scalar(need(args, 1, up)?, s)?;
                if matches!((&a, &b), (V::Blank, V::Blank)) {
                    return Ok(V::Blank);
                }
                capped(text_of(&a) + &text_of(&b))
            }
            "EXACT" => {
                let a = self.text_arg(need(args, 0, up)?, s)?;
                let b = self.text_arg(need(args, 1, up)?, s)?;
                Ok(V::Bool(a == b))
            }
            "FIND" | "SEARCH" => {
                let find = self.text_arg(need(args, 0, up)?, s)?;
                let within = self.text_arg(need(args, 1, up)?, s)?;
                let fold = up == "SEARCH";
                let start = self.count_arg(args, 2, 1, s)?;
                let w = units(&within);
                let found = usize::try_from(start.saturating_sub(1))
                    .ok()
                    .filter(|_| start >= 1)
                    .and_then(|st| {
                        if fold {
                            wild_position(&w, &units(&find), st, true)
                        } else {
                            position(&w, &units(&find), st, false)
                        }
                    });
                match found {
                    Some(i) => Ok(V::Num(i.saturating_add(1) as f64)),
                    None => match arg(args, 3) {
                        Some(e) => self.scalar(e, s),
                        None => dax_error(format!("{up}(): the text was not found")),
                    },
                }
            }
            "CONTAINSSTRING" | "CONTAINSSTRINGEXACT" => {
                let within = self.text_arg(need(args, 0, up)?, s)?;
                let find = self.text_arg(need(args, 1, up)?, s)?;
                let (w, f) = (units(&within), units(&find));
                Ok(V::Bool(if up == "CONTAINSSTRING" {
                    wild_position(&w, &f, 0, true).is_some()
                } else {
                    position(&w, &f, 0, false).is_some()
                }))
            }
            "UNICHAR" => {
                let n = self.count_arg(args, 0, 0, s)?;
                match u32::try_from(n).ok().filter(|n| *n > 0).and_then(char::from_u32) {
                    Some(c) => Ok(V::Text(c.to_string())),
                    None => dax_error("UNICHAR(): not a character code"),
                }
            }
            "UNICODE" => {
                let t = self.text_arg(need(args, 0, up)?, s)?;
                match t.chars().next() {
                    Some(c) => Ok(V::Num(f64::from(u32::from(c)))),
                    None => dax_error("UNICODE(): the text is empty"),
                }
            }
            "VALUE" => {
                let v = self.scalar(need(args, 0, up)?, s)?;
                match v {
                    V::Blank => Ok(V::Blank),
                    // a number, else a date (`VALUE("2024-03-05")` is 45356; Desktop 2.159)
                    V::Text(t) => match parse::value(&t, self.culture(None)?)
                        .or_else(|| super::math::text_date(&t, self.culture(None).ok()?))
                    {
                        Some(x) => Ok(V::Num(x)),
                        None => dax_error(format!("cannot convert value '{t}' of type Text to type Number")),
                    },
                    other => Ok(V::Num(num(&other)?.unwrap_or(0.0))),
                }
            }
            "FORMAT" => {
                let v = self.scalar(need(args, 0, up)?, s)?;
                let f = self.text_arg(need(args, 1, up)?, s)?;
                let locale = match arg(args, 2) {
                    Some(e) => Some(self.text_arg(e, s)?),
                    None => None,
                };
                let loc = self.culture(locale.as_deref())?;
                match format_value(&v, &f, loc) {
                    Some(t) => capped(t),
                    None => unsupported(format!("FORMAT() with the format string \"{f}\"")),
                }
            }
            "WEEKDAY" | "WEEKNUM" => {
                let Some(x) = num(&self.scalar(need(args, 0, up)?, s)?)? else {
                    return Ok(V::Blank);
                };
                if !x.is_finite() || x.abs() > 3.0e6 {
                    return dax_error("date out of range");
                }
                let day = x.floor() as i64;
                let kind = self.count_arg(args, 1, 1, s)?;
                if up == "WEEKDAY" {
                    let w = weekday0(day);
                    return Ok(V::Num(match kind {
                        1 => w + 1,
                        2 => (w + 6).rem_euclid(7) + 1,
                        3 => (w + 6).rem_euclid(7),
                        _ => return dax_error("WEEKDAY(): the return type must be 1, 2 or 3"),
                    } as f64));
                }
                Ok(V::Num(weeknum(day, kind)? as f64))
            }
            "HOUR" | "MINUTE" | "SECOND" => {
                let Some(x) = num(&self.scalar(need(args, 0, up)?, s)?)? else {
                    return Ok(V::Blank);
                };
                let secs = seconds_of_day(x);
                Ok(V::Num(match up {
                    "HOUR" => secs / 3600,
                    "MINUTE" => secs / 60 % 60,
                    _ => secs % 60,
                } as f64))
            }
            "TRUNC" | "ROUNDUP" | "ROUNDDOWN" => {
                let x = num(&self.scalar(need(args, 0, up)?, s)?)?.unwrap_or(0.0);
                let d = self.count_arg(args, 1, 0, s)?.clamp(-15, 15) as i32;
                let p = 10f64.powi(d);
                let y = x * p;
                let r = match up {
                    "ROUNDUP" => {
                        if y >= 0.0 {
                            y.ceil()
                        } else {
                            y.floor()
                        }
                    }
                    _ => y.trunc(),
                };
                Ok(V::Num(r / p))
            }
            "SQRT" => match num(&self.scalar(need(args, 0, up)?, s)?)?.unwrap_or(0.0) {
                x if x < 0.0 => dax_error("SQRT() of a negative number"),
                x => Ok(V::Num(x.sqrt())),
            },
            "POWER" => {
                let av = self.scalar(need(args, 0, up)?, s)?;
                let a = num(&av)?.unwrap_or(0.0);
                let b = num(&self.scalar(need(args, 1, up)?, s)?)?.unwrap_or(0.0);
                let r = a.powf(b);
                // POWER(BLANK(), 2) is blank (Desktop 2.159)
                if av.is_blank() && r == 0.0 {
                    return Ok(V::Blank);
                }
                if r.is_finite() {
                    Ok(V::Num(r))
                } else {
                    dax_error("POWER(): the result is not a number")
                }
            }
            "MOD" => {
                let a = num(&self.scalar(need(args, 0, up)?, s)?)?.unwrap_or(0.0);
                let b = num(&self.scalar(need(args, 1, up)?, s)?)?.unwrap_or(0.0);
                if b == 0.0 {
                    return dax_error("MOD(): division by zero");
                }
                // The result has the sign of the divisor.
                Ok(V::Num(a - b * (a / b).floor()))
            }
            _ => unsupported(format!("{up}() is not evaluated by the preview")),
        }
    }
}

/// `WEEKNUM` return types: 1 (weeks start on Sunday), 2 and 11 (Monday), 12–17 (Tuesday …
/// Sunday), 21 (ISO 8601).
fn weeknum(day: i64, kind: i64) -> Result<i64, Fail> {
    let (y, _, _) = ymd(day);
    if kind == 21 {
        // ISO: the week holding the year's first Thursday is week 1.
        return Ok(bicraft_locale::calendar::iso_week(day));
    }
    let first = match kind {
        1 | 17 => 0,
        2 | 11 => 1,
        12..=16 => kind - 10,
        _ => return dax_error("WEEKNUM(): unsupported return type"),
    };
    let jan1 = super::serial(y, 1, 1);
    let offset = (weekday0(jan1) - first).rem_euclid(7);
    Ok((day - jan1 + offset) / 7 + 1)
}

/// `FORMAT(value, format)` in culture `loc`; `None` for a format string this does not cover.
pub(super) fn format_value(v: &V, f: &str, loc: &Locale) -> Option<String> {
    let value = match v {
        V::Blank => vb::Value::Blank,
        V::Text(t) => vb::Value::Text(t),
        V::Bool(b) => vb::Value::Bool(*b),
        V::Cur(n) => vb::Value::Number(currency::to_f64(*n)),
        V::Num(x) => vb::Value::Number(*x),
        V::Date(x) => vb::Value::Date(*x),
    };
    vb::format(value, f, loc)
}

#[cfg(test)]
mod tests {
    use super::{Locale, V, format_value, weeknum};

    fn f(v: V, s: &str) -> String {
        format_value(&v, s, Locale::get("en-US")).unwrap_or_else(|| "<none>".into())
    }

    #[test]
    fn number_formats() {
        assert_eq!(f(V::Num(1234.5678), "0.00"), "1234.57");
        assert_eq!(f(V::Num(1234.5678), "#,0.00"), "1,234.57");
        assert_eq!(f(V::Num(1234.5), "#,##0"), "1,235");
        assert_eq!(f(V::Num(0.125), "0.00"), "0.13");
        assert_eq!(f(V::Num(-0.001), "0.00"), "0.00");
        assert_eq!(f(V::Num(0.256), "0.0%"), "25.6%");
        assert_eq!(f(V::Num(-5.0), "0;(0)"), "(5)");
        assert_eq!(f(V::Num(0.0), "0;(0);\"zero\""), "zero");
        assert_eq!(f(V::Num(1234.5), "Currency"), "$1,234.50");
        assert_eq!(f(V::Num(-1234.5), "Currency"), "($1,234.50)");
        // the engine keeps the decimal point of `#.##`
        assert_eq!(f(V::Num(3.0), "#.##"), "3.");
        assert_eq!(f(V::Num(1_500_000.0), "#,0,,\"M\""), "2M");
        assert_eq!(f(V::Blank, "0.00"), "");
        assert_eq!(f(V::Num(12.0), ""), "12");
        assert_eq!(f(V::Cur(12_345), "0.00"), "1.23");
    }

    #[test]
    fn date_formats() {
        // 2024-03-05 14:07:09
        let x = 45_356.0 + (14.0 * 3600.0 + 7.0 * 60.0 + 9.0) / 86_400.0;
        assert_eq!(f(V::Date(x), "yyyy-mm-dd"), "2024-03-05");
        assert_eq!(f(V::Date(x), "yyyy-MM"), "2024-03");
        assert_eq!(f(V::Date(x), "MMMM"), "March");
        assert_eq!(f(V::Date(x), "mmm yy"), "Mar 24");
        assert_eq!(f(V::Date(x), "dddd d"), "Tuesday 5");
        assert_eq!(f(V::Date(x), "hh:mm:ss"), "14:07:09");
        assert_eq!(f(V::Date(x), "h:nn AM/PM"), "2:07 PM");
        assert_eq!(f(V::Date(x), "\"Q\"q yyyy"), "Q1 2024");
        assert_eq!(f(V::Date(x), "Short Date"), "3/5/2024");
        assert_eq!(f(V::Date(x), "Long Date"), "Tuesday, March 5, 2024");
        let fr = Locale::get("fr-FR");
        assert_eq!(
            format_value(&V::Date(x), "Long Date", fr).as_deref(),
            Some("mardi 5 mars 2024")
        );
        assert_eq!(
            format_value(&V::Num(1234.5), "Currency", fr).as_deref(),
            Some("1\u{202f}234,50 €")
        );
    }

    #[test]
    fn weeks() {
        // 2024-01-01 is a Monday.
        let jan1 = 45_292;
        assert_eq!(weeknum(jan1, 1).ok(), Some(1));
        assert_eq!(weeknum(jan1 + 6, 1).ok(), Some(2)); // Sunday the 7th
        assert_eq!(weeknum(jan1 + 6, 2).ok(), Some(1));
        assert_eq!(weeknum(45_291, 21).ok(), Some(52)); // 2023-12-31
    }
}
