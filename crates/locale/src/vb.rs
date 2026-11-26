//! DAX `FORMAT(value, format_string[, locale])`: the named formats (`General Number`,
//! `Currency`, `Fixed`, `Standard`, `Percent`, `Scientific`, `Yes/No`, `True/False`, `On/Off`,
//! `General Date`, `Long Date`, `Medium Date`, `Short Date`, `Long Time`, `Medium Time`,
//! `Short Time`) and the custom number and date/time format strings, in a culture.
//!
//! Behavior follows what the engine of Power BI Desktop returns (recorded engine results,
//! `docs/formats/locales.md`): in a custom format, `.` and `,` are the culture's decimal and
//! group separators, `/` and `:` its date and time separators; `AM/PM`, `am/pm`, `A/P` and `a/p`
//! print English letters, `AMPM` the culture's designators; month and day names are the
//! culture's (English for custom tokens in ja, zh and ko).

use crate::datefmt::{self, DateParts, Names};
use crate::{Locale, number};

/// A value to format.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value<'a> {
    Blank,
    Text(&'a str),
    Bool(bool),
    Number(f64),
    /// An OLE date serial (days since 1899-12-30).
    Date(f64),
}

/// Longest format string read.
const MAX_FORMAT: usize = 1024;
/// Most characters a result may have.
const MAX_OUT: usize = 8192;

/// `FORMAT(value, f)` in culture `loc`; `None` for a format string this does not cover.
pub fn format(v: Value, f: &str, loc: &Locale) -> Option<String> {
    if f.len() > MAX_FORMAT {
        return None;
    }
    let x = match v {
        Value::Blank => return Some(String::new()),
        Value::Text(t) => match crate::parse::value(t, loc) {
            Some(x) => x,
            None => return Some(t.to_string()),
        },
        Value::Bool(b) => {
            let w = &loc.vb.words;
            let pick = |i: usize| {
                w.get(if b { i } else { i + 1 })
                    .copied()
                    .unwrap_or("")
                    .to_string()
            };
            return Some(match f.to_ascii_lowercase().as_str() {
                "yes/no" => pick(0),
                "on/off" => pick(4),
                "" | "true/false" | "general number" => {
                    if b {
                        "True".into()
                    } else {
                        "False".into()
                    }
                }
                _ => format_number(if b { -1.0 } else { 0.0 }, f, loc)?,
            });
        }
        Value::Number(x) | Value::Date(x) => x,
    };
    let is_date = matches!(v, Value::Date(_));
    let named = f.trim().to_ascii_lowercase();
    let word = |i: usize| -> Option<String> {
        let w = &loc.vb.words;
        Some(w.get(if x != 0.0 { i } else { i + 1 })?.to_string())
    };
    match named.as_str() {
        "" if is_date => return format_date(x, "c", loc),
        "" | "general number" => {
            return x.is_finite().then(|| number::general(x, loc.decimal));
        }
        "general date" => return format_date(x, "c", loc),
        "long date" => return format_date(x, "dddddd", loc),
        "medium date" => return named_date(x, loc.vb.medium_date, loc, true),
        "short date" => return format_date(x, "ddddd", loc),
        "long time" => return format_date(x, "ttttt", loc),
        "medium time" => return named_date(x, loc.vb.medium_time, loc, false),
        "short time" => return named_date(x, "HH:mm", loc, false),
        "currency" => return currency(x, loc),
        "fixed" => return format_number(x, "0.00", loc),
        "standard" => return format_number(x, "#,0.00", loc),
        "percent" => return format_number(x, "0.00%", loc),
        "scientific" => return format_number(x, "0.00E+00", loc),
        "yes/no" => return word(0),
        "true/false" => return word(2),
        "on/off" => return word(4),
        _ => {}
    }
    if is_date_format(f) {
        format_date(x, f, loc)
    } else {
        format_number(x, f, loc)
    }
}

/// The culture's currency format.
fn currency(x: f64, loc: &Locale) -> Option<String> {
    let c = &loc.currency;
    let (int, frac) = number::fixed_digits(x, usize::from(c.digits))?;
    let int = if int.is_empty() { "0".to_string() } else { int };
    let zero = int.bytes().chain(frac.bytes()).all(|b| b == b'0');
    let affix = if x < 0.0 && !zero { c.negative } else { c.positive };
    let mut out = String::from(affix.prefix);
    out.push_str(&number::group(&int, c.group));
    if !frac.is_empty() {
        out.push(c.decimal);
        out.push_str(&frac);
    }
    out.push_str(affix.suffix);
    Some(out)
}

/// A named format given as a .NET pattern (`Medium Date`, `Medium Time`, `Short Time`).
fn named_date(x: f64, pattern: &str, loc: &Locale, genitive: bool) -> Option<String> {
    let p = DateParts::from_serial(x)?;
    let names = loc.vb_names();
    let names = if genitive {
        names
    } else {
        Names {
            months_abbr_genitive: names.months_abbr,
            ..names
        }
    };
    Some(datefmt::format_pattern(&p, pattern, &names))
}

/// Whether a custom format string is a date/time one: it has date or time tokens outside
/// quotes and escapes, and no digit placeholders.
pub fn is_date_format(f: &str) -> bool {
    let lower = f.to_ascii_lowercase();
    if lower.contains("am/pm") || lower.contains("a/p") || lower.contains("ampm") {
        return true;
    }
    let mut quoted = false;
    let mut chars = f.chars();
    let (mut date, mut digits) = (false, false);
    while let Some(c) = chars.next() {
        match c {
            '"' => quoted = !quoted,
            _ if quoted => {}
            '\\' => {
                chars.next();
            }
            'd' | 'D' | 'm' | 'M' | 'y' | 'Y' | 'h' | 'H' | 'n' | 'N' | 's' | 'S' | 'q' | 'Q' | 'w' | 'W'
            | 'c' | 'C' | 't' | 'T' => {
                date = true;
            }
            '0' | '#' => digits = true,
            _ => {}
        }
    }
    date && !digits
}

/// Week of the year with weeks starting on `first` (0 = Sunday). Week 1 is the week of
/// 1 January (`rule` 0), the first full week (1) or the first week with four days in the year
/// (2); days before week 1 belong to the last week of the year before, and with rule 2 the
/// last days of a year can be in week 1 of the next (ISO-like).
fn week_of_year(p: &DateParts, first: u8, rule: u8) -> i64 {
    let day = datefmt::days_from_civil(p.year, p.month, p.day);
    let dow = |d: i64| crate::calendar::weekday_of_serial(d - crate::calendar::SERIAL_EPOCH);
    let week1 = |y: i64| -> i64 {
        let jan1 = datefmt::days_from_civil(y, 1, 1);
        let back = (dow(jan1) - i64::from(first)).rem_euclid(7);
        match rule {
            1 if back == 0 => jan1,
            1 => jan1 + 7 - back,
            2 if back <= 3 => jan1 - back,
            2 => jan1 + 7 - back,
            _ => jan1 - back,
        }
    };
    let mut start = week1(p.year);
    if day < start {
        start = week1(p.year - 1);
    } else if rule == 2 && day >= week1(p.year + 1) {
        // the first four-day week of next year (2024-12-30 is in week 1)
        return 1;
    }
    (day - start).div_euclid(7) + 1
}

/// A date/time format string (VB-style tokens as DAX reads them) of an OLE date serial.
pub fn format_date(x: f64, f: &str, loc: &Locale) -> Option<String> {
    if f.len() > MAX_FORMAT {
        return None;
    }
    let p = DateParts::from_serial(x)?;
    let day_serial = x.floor();
    let time_only = day_serial == 0.0;
    let custom_names = if loc.vb.english_names {
        loc.english_names()
    } else {
        loc.vb_names()
    };
    let vb = loc.vb_names();
    let chars: Vec<char> = f.chars().collect();
    let lower = f.to_ascii_lowercase();
    let has_ampm = lower.contains("am/pm") || lower.contains("a/p") || lower.contains("ampm");
    // Month names after a day number take the genitive form (ru "7 марта").
    let genitive = {
        let mut quoted = false;
        let mut found = false;
        let mut k = 0;
        while let Some(&c) = chars.get(k) {
            match c {
                '"' => quoted = !quoted,
                '\\' => k += 1,
                'd' | 'D' if !quoted => {
                    let n = chars.get(k..).map_or(0, |r| {
                        r.iter().take_while(|x| x.eq_ignore_ascii_case(&'d')).count()
                    });
                    if n <= 2 {
                        found = true;
                    }
                    k += n;
                    continue;
                }
                _ => {}
            }
            k += 1;
        }
        found
    };
    let mut out = String::new();
    let mut i = 0usize;
    // The last time token was an hour: an `m` right after it is minutes.
    let mut after_hour = false;
    let run = |i: usize, c: char| -> usize {
        chars
            .get(i..)
            .map_or(0, |r| r.iter().take_while(|x| x.eq_ignore_ascii_case(&c)).count())
    };
    let month = idx(p.month.saturating_sub(1));
    let wd = idx(p.weekday);
    while let Some(&c) = chars.get(i) {
        if out.len() > MAX_OUT {
            return None;
        }
        match c.to_ascii_lowercase() {
            '"' => {
                let end = chars.get(i + 1..)?.iter().position(|x| *x == '"')?;
                out.extend(chars.get(i + 1..i + 1 + end)?);
                i += end + 2;
                continue;
            }
            '\\' => {
                out.push(*chars.get(i + 1)?);
                i += 2;
                continue;
            }
            'c' => {
                out.push_str(&general_date(&p, time_only, loc));
                i += 1;
                continue;
            }
            'y' => {
                let n = run(i, 'y');
                match n {
                    1 => out.push_str(&p.day_of_year().to_string()),
                    2 | 3 => out.push_str(&format!("{:02}", p.year.rem_euclid(100))),
                    _ => out.push_str(&format!("{:04}", p.year)),
                }
                // `yyyyy`: four digits, then the rest again.
                let used = if n >= 4 { 4 } else { n };
                i += used;
                after_hour = false;
                continue;
            }
            'd' => {
                let n = run(i, 'd');
                let used = n.min(6);
                match used {
                    1 => out.push_str(&p.day.to_string()),
                    2 => out.push_str(&format!("{:02}", p.day)),
                    3 => out.push_str(custom_names.days_abbr.get(wd)?),
                    4 => out.push_str(custom_names.days.get(wd)?),
                    5 => out.push_str(&datefmt::format_pattern(&p, loc.short_date, &vb)),
                    _ => out.push_str(&long_date(&p, loc)),
                }
                i += used;
                after_hour = false;
                continue;
            }
            'm' => {
                let n = run(i, 'm');
                let used = n.min(4);
                // Minutes after an hour token (or before seconds), months otherwise.
                let next_is_s = chars.get(i + used..).is_some_and(|r| {
                    r.iter()
                        .find(|c| c.is_ascii_alphabetic())
                        .is_some_and(|c| c.eq_ignore_ascii_case(&'s'))
                });
                if (after_hour || next_is_s) && used <= 2 {
                    out.push_str(&if used == 1 {
                        p.minute.to_string()
                    } else {
                        format!("{:02}", p.minute)
                    });
                } else {
                    match used {
                        1 => out.push_str(&p.month.to_string()),
                        2 => out.push_str(&format!("{:02}", p.month)),
                        3 => {
                            let list = if genitive {
                                custom_names.months_abbr_genitive
                            } else {
                                custom_names.months_abbr
                            };
                            out.push_str(list.get(month)?);
                        }
                        _ => {
                            let list = if genitive {
                                custom_names.months_genitive
                            } else {
                                custom_names.months
                            };
                            out.push_str(list.get(month)?);
                        }
                    }
                }
                i += used;
                after_hour = false;
                continue;
            }
            'n' => {
                let n = run(i, 'n');
                let used = n.min(2);
                out.push_str(&if used == 1 {
                    p.minute.to_string()
                } else {
                    format!("{:02}", p.minute)
                });
                i += used;
                after_hour = false;
                continue;
            }
            'h' => {
                let n = run(i, 'h');
                let used = n.min(2);
                let h = if has_ampm {
                    match p.hour % 12 {
                        0 => 12,
                        h => h,
                    }
                } else {
                    p.hour
                };
                out.push_str(&if used == 1 {
                    h.to_string()
                } else {
                    format!("{h:02}")
                });
                i += used;
                after_hour = true;
                continue;
            }
            's' => {
                let n = run(i, 's');
                let used = n.min(2);
                out.push_str(&if used == 1 {
                    p.second.to_string()
                } else {
                    format!("{:02}", p.second)
                });
                i += used;
                after_hour = false;
                continue;
            }
            't' => {
                let n = run(i, 't');
                if n >= 5 {
                    out.push_str(&datefmt::format_pattern(&p, loc.long_time, &vb));
                    i += 5;
                    continue;
                }
                out.push(c);
            }
            'q' => {
                out.push_str(&((p.month - 1) / 3 + 1).to_string());
                i += 1;
                continue;
            }
            'w' => {
                let n = run(i, 'w');
                if n >= 2 {
                    out.push_str(&week_of_year(&p, loc.vb.first_day, loc.vb.first_week).to_string());
                    i += 2;
                } else {
                    let w = (i64::from(p.weekday) - i64::from(loc.vb.first_day)).rem_euclid(7) + 1;
                    out.push_str(&w.to_string());
                    i += 1;
                }
                continue;
            }
            'a' => {
                let rest: String = chars.get(i..)?.iter().take(5).collect();
                let pm = p.hour >= 12;
                if rest.eq_ignore_ascii_case("am/pm") {
                    // `am/pm` is lowercase only for a PM time of a value with a date part (observed
                    // on the engine: "pm" for 2024-03-05 14:07, "AM" for 2024-11-25 09:05 and
                    // "PM" for a time alone).
                    let lower_case = rest.starts_with('a') && pm && !time_only;
                    out.push_str(match (pm, lower_case) {
                        (false, false) => "AM",
                        (true, false) => "PM",
                        (false, true) => "am",
                        (true, true) => "pm",
                    });
                    i += 5;
                    continue;
                }
                let rest4: String = chars.get(i..)?.iter().take(4).collect();
                if rest4.eq_ignore_ascii_case("ampm") {
                    out.push_str(if pm { loc.pm } else { loc.am });
                    i += 4;
                    continue;
                }
                let rest3: String = chars.get(i..)?.iter().take(3).collect();
                if rest3.eq_ignore_ascii_case("a/p") {
                    let upper = rest3.starts_with('A');
                    out.push(match (pm, upper) {
                        (false, true) => 'A',
                        (true, true) => 'P',
                        (false, false) => 'a',
                        (true, false) => 'p',
                    });
                    i += 3;
                    continue;
                }
                out.push(c);
            }
            '/' => out.push_str(loc.date_separator),
            ':' => out.push_str(loc.time_separator),
            _ => out.push(c),
        }
        i += 1;
    }
    Some(out)
}

/// `General Date` (`c`): the short date, the long time, or both.
fn general_date(p: &DateParts, time_only: bool, loc: &Locale) -> String {
    let names = loc.vb_names();
    let has_time = p.hour != 0 || p.minute != 0 || p.second != 0;
    if time_only {
        return datefmt::format_pattern(p, loc.long_time, &names);
    }
    let date = datefmt::format_pattern(p, loc.short_date, &names);
    if has_time {
        format!("{date} {}", datefmt::format_pattern(p, loc.long_time, &names))
    } else {
        date
    }
}

/// `Long Date`: the culture's pattern with the nominative month names.
fn long_date(p: &DateParts, loc: &Locale) -> String {
    let names = loc.vb_names();
    let names = Names {
        months_genitive: names.months,
        months_abbr_genitive: names.months_abbr,
        ..names
    };
    datefmt::format_pattern(p, loc.long_date, &names)
}

fn idx(n: u32) -> usize {
    usize::try_from(n).unwrap_or(0)
}

/// One section of a custom number format.
#[derive(Debug)]
struct Section {
    tokens: Vec<Tok>,
    int_zeros: usize,
    frac_zeros: usize,
    frac_hashes: usize,
    grouping: bool,
    percent: bool,
    /// Trailing commas: each divides by 1000.
    scale_commas: usize,
    /// Scientific notation: (sign always shown, minimum exponent digits, upper case).
    exponent: Option<(bool, usize, bool)>,
    /// Literal text between integer placeholders (`# ##0`), with the number of placeholders
    /// to its right: it goes before that many last digits.
    embedded: Vec<(String, usize)>,
}

#[derive(Debug)]
enum Tok {
    Lit(String),
    /// Where the integer digits go.
    Int,
    /// Where the decimal point and fraction go.
    Frac,
    /// Where the exponent goes.
    Exp,
}

fn parse_section(f: &str) -> Option<Section> {
    let chars: Vec<char> = f.chars().collect();
    let mut tokens = Vec::new();
    let (mut int_zeros, mut frac_zeros, mut frac_hashes) = (0, 0, 0);
    let (mut grouping, mut percent, mut in_frac, mut placed_int) = (false, false, false, false);
    let mut scale_commas = 0;
    let mut exponent = None;
    let mut lit = String::new();
    let mut embedded: Vec<(String, usize)> = Vec::new();
    let mut int_places = 0usize;
    let mut i = 0;
    let flush = |lit: &mut String, tokens: &mut Vec<Tok>| {
        if !lit.is_empty() {
            tokens.push(Tok::Lit(std::mem::take(lit)));
        }
    };
    while let Some(&c) = chars.get(i) {
        match c {
            '0' | '#' if exponent.is_none() => {
                if in_frac {
                    if c == '0' {
                        frac_zeros += 1 + frac_hashes;
                        frac_hashes = 0;
                    } else {
                        frac_hashes += 1;
                    }
                } else {
                    if !placed_int {
                        flush(&mut lit, &mut tokens);
                        tokens.push(Tok::Int);
                        placed_int = true;
                    } else if !lit.is_empty() {
                        embedded.push((std::mem::take(&mut lit), int_places));
                    }
                    int_places += 1;
                    if scale_commas > 0 {
                        grouping = true;
                        scale_commas = 0;
                    }
                    if c == '0' {
                        int_zeros += 1;
                    }
                }
            }
            '.' if !in_frac && exponent.is_none() => {
                if !placed_int {
                    flush(&mut lit, &mut tokens);
                    tokens.push(Tok::Int);
                    placed_int = true;
                }
                flush(&mut lit, &mut tokens);
                tokens.push(Tok::Frac);
                in_frac = true;
            }
            ',' if placed_int && !in_frac && exponent.is_none() => scale_commas += 1,
            // commas after the decimal point neither group nor scale
            ',' if in_frac => {}
            '%' => {
                percent = true;
                lit.push('%');
            }
            'e' | 'E' if placed_int && exponent.is_none() => {
                let sign = chars.get(i + 1).copied();
                let always = match sign {
                    Some('+') => true,
                    Some('-') => false,
                    _ => return None,
                };
                let digits = chars
                    .get(i + 2..)
                    .map_or(0, |r| r.iter().take_while(|x| **x == '0' || **x == '#').count());
                if digits == 0 {
                    return None;
                }
                if !in_frac {
                    flush(&mut lit, &mut tokens);
                    tokens.push(Tok::Frac);
                    in_frac = true;
                }
                flush(&mut lit, &mut tokens);
                tokens.push(Tok::Exp);
                exponent = Some((always, digits, c == 'E'));
                i += 2 + digits;
                continue;
            }
            '"' => {
                let end = chars.get(i + 1..)?.iter().position(|x| *x == '"')?;
                lit.extend(chars.get(i + 1..i + 1 + end)?);
                i += end + 2;
                continue;
            }
            '\\' => {
                lit.push(*chars.get(i + 1)?);
                i += 2;
                continue;
            }
            // The reference's date, time and numeric characters cannot be literal; read
            // unquoted in a number format, their meaning is not known here.
            c if "acdehmnpqstwy".contains(c.to_ascii_lowercase()) => return None,
            // Any other letter is literal (`0,K`, `#,0,.0 k€`).
            c => lit.push(c),
        }
        i += 1;
    }
    flush(&mut lit, &mut tokens);
    let embedded = embedded
        .into_iter()
        .map(|(l, before)| (l, int_places.saturating_sub(before)))
        .collect();
    Some(Section {
        embedded,
        tokens,
        int_zeros,
        frac_zeros,
        frac_hashes,
        grouping,
        percent,
        scale_commas,
        exponent,
    })
}

fn render(x: f64, sec: &Section, sign: bool, loc: &Locale) -> Option<String> {
    let mut x = x;
    if sec.percent {
        x *= 100.0;
    }
    for _ in 0..sec.scale_commas {
        x /= 1000.0;
    }
    let places = sec.frac_zeros + sec.frac_hashes;
    let mut exp_text = String::new();
    if let Some((always, digits, upper)) = sec.exponent {
        // Scientific: one integer digit per integer zero (at least one).
        let int_digits = sec.int_zeros.max(1);
        let mut e: i32 = 0;
        if x != 0.0 && x.is_finite() {
            e = x.abs().log10().floor() as i32 - (int_digits as i32 - 1);
            let mut m = x / 10f64.powi(e);
            // Rounding may carry into another digit (9.995 → 10.0).
            if let Some((int, _)) = number::fixed_digits(m, places)
                && int.len() > int_digits
            {
                e += 1;
                m = x / 10f64.powi(e);
            }
            x = m;
        }
        exp_text.push(if upper { 'E' } else { 'e' });
        if e < 0 {
            exp_text.push('-');
        } else if always {
            exp_text.push('+');
        }
        exp_text.push_str(&format!("{:0width$}", e.unsigned_abs(), width = digits));
    }
    let (int, mut frac) = number::fixed_digits(x, places)?;
    while frac.len() > sec.frac_zeros && frac.ends_with('0') {
        frac.pop();
    }
    let mut int = int;
    while int.len() < sec.int_zeros {
        int.insert(0, '0');
    }
    let negative = sign && x < 0.0 && (int.bytes().any(|b| b != b'0') || frac.bytes().any(|b| b != b'0'));
    let int = layout_int(&int, sec, loc);
    let mut out = String::new();
    if negative {
        out.push('-');
    }
    for t in &sec.tokens {
        match t {
            Tok::Lit(l) => out.push_str(l),
            Tok::Int => out.push_str(&int),
            Tok::Frac => {
                // The separator stays even without fraction digits (`#.##` of 3 is `3.`).
                out.push(loc.decimal);
                out.push_str(&frac);
            }
            Tok::Exp => out.push_str(&exp_text),
        }
    }
    Some(out)
}

/// The integer digits with group separators and the literals placed between placeholders.
fn layout_int(int: &str, sec: &Section, loc: &Locale) -> String {
    if sec.embedded.is_empty() {
        return if sec.grouping {
            number::group(int, loc.group)
        } else {
            int.to_string()
        };
    }
    let n = int.chars().count();
    let mut out = String::new();
    // Literals anchored further left than the first digit come first.
    for (l, k) in &sec.embedded {
        if *k >= n {
            out.push_str(l);
        }
    }
    for (i, c) in int.chars().enumerate() {
        let left = n - i;
        if sec.grouping && i > 0 && left.is_multiple_of(3) {
            out.push(loc.group);
        }
        for (l, k) in &sec.embedded {
            if *k == left && *k < n {
                out.push_str(l);
            }
        }
        out.push(c);
    }
    out
}

/// A custom number format: up to three sections (positive; negative; zero).
pub fn format_number(x: f64, f: &str, loc: &Locale) -> Option<String> {
    if !x.is_finite() || f.len() > MAX_FORMAT {
        return None;
    }
    let parts = split_sections(f);
    let sections: Vec<Section> = parts.iter().map(|p| parse_section(p)).collect::<Option<_>>()?;
    match sections.as_slice() {
        [one] => render(x, one, true, loc),
        // A value that rounds to zero takes the zero section, or the positive one unsigned.
        [pos, neg] if x < 0.0 => {
            if rounds_to_zero(-x, neg) {
                render(0.0, pos, true, loc)
            } else {
                render(-x, neg, false, loc)
            }
        }
        [pos, _] => render(x, pos, true, loc),
        [pos, neg, zero] => {
            let (abs, sec) = if x < 0.0 { (-x, neg) } else { (x, pos) };
            if rounds_to_zero(abs, sec) {
                render(0.0, zero, true, loc)
            } else {
                render(abs, sec, x >= 0.0, loc)
            }
        }
        _ => None,
    }
}

/// True when `x` (≥ 0) shows as zero in section `sec`.
fn rounds_to_zero(x: f64, sec: &Section) -> bool {
    if sec.exponent.is_some() {
        return x == 0.0;
    }
    let mut x = x;
    if sec.percent {
        x *= 100.0;
    }
    for _ in 0..sec.scale_commas {
        x /= 1000.0;
    }
    number::fixed_digits(x, sec.frac_zeros + sec.frac_hashes)
        .is_none_or(|(i, f)| i.bytes().chain(f.bytes()).all(|b| b == b'0'))
}

fn split_sections(f: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut quoted = false;
    let mut chars = f.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                if let Some(s) = out.last_mut() {
                    s.push(c);
                }
            }
            '\\' if !quoted => {
                if let Some(s) = out.last_mut() {
                    s.push(c);
                    if let Some(n) = chars.next() {
                        s.push(n);
                    }
                }
            }
            ';' if !quoted => out.push(String::new()),
            c => {
                if let Some(s) = out.last_mut() {
                    s.push(c);
                }
            }
        }
    }
    out
}
