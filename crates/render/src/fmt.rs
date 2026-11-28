//! Numbers and dates, Power BI style: auto display units (K/M/bn, per locale), the parts of a
//! format string that matter for display (%, decimals, grouping, currency), per-culture
//! separators. A port of the live viewer's formatting (`crates/view/src/page.html`), which uses
//! the browser's `Intl`; here the culture data come from `bicraft-locale`.

use serde_json::Value;

const NB: char = '\u{a0}';

/// Separators and unit suffixes of a culture, from `bicraft-locale`'s display data (CLDR, with
/// Power BI's display-unit suffixes).
#[derive(Debug, Clone, PartialEq)]
pub struct Locale {
    pub tag: String,
    pub decimal: char,
    pub group: Option<char>,
    /// Smallest integer part (digits) that gets grouping (es: 5).
    pub min_group_digits: usize,
    /// Text before and after a percentage's number (`%` included): `%` right after the number in
    /// every culture, as Desktop 2.158 draws a `0.0%` format with a French model ("38,0%"); a
    /// format string's own space before `%` is kept ([`FmtParts::pct_sp`]).
    pub pct: (&'static str, &'static str),
    /// Display-unit suffixes: K, M, bn, T in every culture (Desktop 2.158 with a French model and
    /// a French UI: "912K", "0M").
    pub units: [String; 4],
    /// Date order and separator: 'm' month first (US), 'd' day first, 'y' year first.
    pub date_order: char,
    pub date_sep: char,
    pub h12: bool,
    /// The culture's full data (month names, AM/PM designators…).
    pub culture: &'static bicraft_locale::Locale,
}

impl Locale {
    /// The culture `culture` (`fr-FR`, `fr`; empty or unknown: en-US).
    pub fn new(culture: &str) -> Locale {
        let c = if culture.trim().is_empty() {
            bicraft_locale::Locale::en_us()
        } else {
            bicraft_locale::Locale::find(culture)
                .filter(|l| !l.is_invariant())
                .unwrap_or_else(bicraft_locale::Locale::en_us)
        };
        let d = &c.display;
        Locale {
            tag: if culture.trim().is_empty() {
                c.name.to_string()
            } else {
                culture.trim().chars().take(32).collect()
            },
            decimal: d.decimal,
            group: Some(d.group),
            min_group_digits: if d.min_grouping >= 2 { 5 } else { 4 },
            pct: ("", "%"),
            units: bicraft_locale::Locale::en_us().display.units.map(String::from),
            date_order: if c.year_first() {
                'y'
            } else if c.day_first() {
                'd'
            } else {
                'm'
            },
            date_sep: c.date_separator.chars().next().unwrap_or('/'),
            h12: c.hour12(),
            culture: c,
        }
    }

    /// `n` with between `min` and `max` fraction digits (round half away from zero).
    pub fn number(&self, n: f64, min: usize, max: usize, grouping: bool) -> String {
        if !n.is_finite() {
            return if n.is_nan() {
                "NaN".into()
            } else if n > 0.0 {
                "∞".into()
            } else {
                "-∞".into()
            };
        }
        let max = max.min(15);
        let min = min.min(max);
        let digits = round_digits(n.abs(), max);
        let (int, mut frac) = match digits.split_once('.') {
            Some((a, b)) => (a.to_string(), b.to_string()),
            None => (digits.clone(), String::new()),
        };
        while frac.len() > min && frac.ends_with('0') {
            frac.pop();
        }
        let neg = n < 0.0 && (int.chars().any(|c| c != '0') || frac.chars().any(|c| c != '0'));
        let mut out = String::new();
        if neg {
            out.push('-');
        }
        match self.group {
            Some(g) if grouping && int.len() >= self.min_group_digits => {
                let len = int.len();
                for (i, c) in int.chars().enumerate() {
                    if i > 0 && (len - i) % 3 == 0 {
                        out.push(g);
                    }
                    out.push(c);
                }
            }
            _ => out.push_str(&int),
        }
        if !frac.is_empty() {
            out.push(self.decimal);
            out.push_str(&frac);
        }
        out
    }

    pub fn percent(&self, n: f64, min: usize, max: usize) -> String {
        let (pre, suf) = self.pct;
        format!("{pre}{}{suf}", self.number(n * 100.0, min, max, true))
    }
}

/// `x` (≥ 0) with exactly `d` decimals, rounded half away from zero on its decimal value.
fn round_digits(x: f64, d: usize) -> String {
    // format with extra digits first so that 0.125 → "0.125000…" rounds up like Intl does
    let wide = format!("{:.*}", (d + 3).min(20), x);
    let (int, frac) = wide.split_once('.').unwrap_or((wide.as_str(), ""));
    let mut digits: Vec<u8> = int
        .bytes()
        .chain(frac.bytes())
        .map(|b| b.wrapping_sub(b'0'))
        .collect();
    let keep = int.len() + d;
    let round_up = digits.get(keep).is_some_and(|&x| x >= 5);
    digits.truncate(keep);
    if round_up {
        let mut i = digits.len();
        let mut carry = true;
        while carry && i > 0 {
            i -= 1;
            if let Some(dg) = digits.get_mut(i) {
                if *dg == 9 {
                    *dg = 0;
                } else {
                    *dg += 1;
                    carry = false;
                }
            }
        }
        if carry {
            digits.insert(0, 1);
        }
    }
    let int_len = digits.len().saturating_sub(d);
    let mut s: String = digits
        .iter()
        .take(int_len)
        .map(|x| char::from(b'0' + x.min(&9)))
        .collect();
    if s.is_empty() {
        s.push('0');
    }
    if d > 0 {
        s.push('.');
        s.extend(digits.iter().skip(int_len).map(|x| char::from(b'0' + x.min(&9))));
    }
    s
}

/// What a format string says about display.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FmtParts {
    pub pct: bool,
    /// The positive section writes a `+` before the number ("+0.0%;-0.0%").
    pub plus: bool,
    pub group: bool,
    pub min: Option<usize>,
    pub max: Option<usize>,
    pub cur: String,
    pub pre: bool,
    pub sp: bool,
    /// A space before the `%` ("0.0 %"): kept, as a no-break space.
    pub pct_sp: bool,
}

pub fn fmt_parts(f: &str) -> FmtParts {
    let raw = f.split(';').next().unwrap_or("");
    // the section without quoted text and escaped characters
    let mut sec = String::new();
    let mut quoted = false;
    let mut esc = false;
    for c in raw.chars() {
        if esc {
            esc = false;
            continue;
        }
        match c {
            '"' => quoted = !quoted,
            '\\' if !quoted => esc = true,
            _ if !quoted => sec.push(c),
            _ => {}
        }
    }
    let first = sec.find(['0', '#']);
    let dm: Option<String> = sec.find('.').and_then(|i| {
        let tail: String = sec
            .get(i + 1..)?
            .chars()
            .take_while(|c| *c == '0' || *c == '#')
            .collect();
        (!tail.is_empty()).then_some(tail)
    });
    const CURS: [&str; 6] = ["€", "$", "£", "¥", "₹", "CHF"];
    let cm = CURS
        .iter()
        .filter_map(|c| raw.find(c).map(|i| (i, *c)))
        .min_by_key(|(i, _)| *i);
    let fi = raw.find(['0', '#']);
    let group = sec.is_empty() || {
        let b: Vec<char> = sec.chars().collect();
        b.windows(3)
            .any(|w| matches!(w, [a, ',', c] if "#0".contains(*a) && "#0".contains(*c)))
    };
    let (min, max) = match (&dm, first) {
        (Some(d), _) => (Some(d.matches('0').count().min(15)), Some(d.len().min(15))),
        (None, Some(_)) => (Some(0), Some(0)),
        (None, None) => (None, None),
    };
    let (cur, pre, sp) = match cm {
        Some((ci, c)) => {
            let pre = fi.is_none_or(|fi| ci < fi);
            let probe = if pre {
                raw.get(ci + c.len()..).and_then(|s| s.chars().next())
            } else {
                raw.get(..ci).and_then(|s| s.chars().last())
            };
            (
                c.to_string(),
                pre,
                probe.is_some_and(|ch| ch.is_whitespace() || ch == '"'),
            )
        }
        None => (String::new(), false, false),
    };
    FmtParts {
        plus: sec.split(['0', '#']).next().is_some_and(|pre| pre.contains('+')),
        pct: sec.contains('%'),
        pct_sp: sec
            .find('%')
            .and_then(|i| sec.get(..i))
            .and_then(|h| h.chars().last())
            .is_some_and(|c| c.is_whitespace()),
        group,
        min,
        max,
        cur,
        pre,
        sp,
    }
}

/// A percentage with the format string's spacing.
fn pct_text(loc: &Locale, n: f64, min: usize, max: usize, p: &FmtParts) -> String {
    // grouped only when the format groups ("0.0%" of 950.5 is "95050,0%", Desktop 2.158)
    let (pre, suf) = loc.pct;
    let s = format!("{pre}{}{suf}", loc.number(n * 100.0, min, max, p.group));
    if p.pct_sp && !s.contains(&format!("{NB}%")) {
        s.replacen('%', &format!("{NB}%"), 1)
    } else {
        s
    }
}

fn with_cur(s: String, p: &FmtParts) -> String {
    if p.cur.is_empty() {
        return s;
    }
    let sp = if p.sp { NB.to_string() } else { String::new() };
    if p.pre {
        // the sign goes before a leading currency: "-$40.00", as the VB `FORMAT` rules write it
        match s.strip_prefix('-') {
            Some(rest) => format!("-{}{sp}{rest}", p.cur),
            None => format!("{}{sp}{s}", p.cur),
        }
    } else {
        format!("{s}{sp}{}", p.cur)
    }
}

fn unit_of(a: f64) -> usize {
    if a >= 1e12 {
        4
    } else if a >= 1e9 {
        3
    } else if a >= 1e6 {
        2
    } else if a >= 1e3 {
        1
    } else {
        0
    }
}
const DIV: [f64; 5] = [1.0, 1e3, 1e6, 1e9, 1e12];

/// How a column displays (from the model and the data).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ColFmt {
    pub fmt: String,
    pub kind: String,
    pub data_type: String,
}

/// A number, Power BI style (`abbrev`: display units for |n| ≥ 1000).
pub fn fmt_val(n: f64, col: Option<&ColFmt>, loc: &Locale, abbrev: bool) -> String {
    // scaling commas or literal text (`#,0.0,"K"`, `0 "units"`): the whole format string, as the
    // Gauge / KPI+ and Waterfall+ labels write it
    if !abbrev
        && let Some(f) = col.map(|c| c.fmt.as_str()).filter(|f| wants_full_format(f))
        && let Some(s) = crate::chart_pack::gauge::full_format(n, f, loc)
    {
        return s;
    }
    // a negative value under a format with its own negative section (Desktop's dollar currency
    // `\$#,0.00;(\$#,0.00);\$#,0.00` draws -5 as "($5.00)"): the VB `FORMAT` rules
    if n < 0.0
        && !abbrev
        && let Some(f) = col.map(|c| c.fmt.as_str()).filter(|f| has_negative_section(f))
        && let Some(s) = bicraft_locale::vb::format(bicraft_locale::vb::Value::Number(n), f, loc.culture)
    {
        return s;
    }
    let s = fmt_val_unsigned(n, col, loc, abbrev);
    if n > 0.0 && !s.is_empty() && fmt_parts(col.map_or("", |c| c.fmt.as_str())).plus {
        format!("+{s}")
    } else {
        s
    }
}

/// Whether number format string `f` needs the whole format-string rules: a section with scaling
/// commas (right after the last digit placeholder before the decimal point, or after the last
/// decimal) or with literal text other than a currency symbol and spaces (quoted or escaped).
fn wants_full_format(f: &str) -> bool {
    const CURS: [&str; 6] = ["€", "$", "£", "¥", "₹", "CHF"];
    let f: String = f.chars().take(200).collect();
    if f.trim().is_empty() || bicraft_locale::vb::is_date_format(&f) {
        return false;
    }
    let literal = |s: &str| {
        let rest = CURS.iter().fold(s.to_string(), |s, c| s.replace(c, ""));
        rest.chars().any(|c| !c.is_whitespace())
    };
    for sec in crate::chart_pack::gauge::scale_commas(&f).split(';') {
        // the section's characters outside quotes and escapes, and its literal text
        let (mut plain, mut text) = (Vec::new(), String::new());
        let (mut quoted, mut esc) = (false, false);
        for c in sec.chars() {
            if esc {
                esc = false;
                text.push(c);
            } else if c == '"' {
                quoted = !quoted;
            } else if quoted {
                text.push(c);
            } else if c == '\\' {
                esc = true;
            } else {
                plain.push(c);
            }
        }
        if literal(&text) {
            return true;
        }
        let int = plain.iter().position(|c| *c == '.').unwrap_or(plain.len());
        let last = plain
            .get(..int)
            .and_then(|p| p.iter().rposition(|c| *c == '0' || *c == '#'));
        if last.is_some_and(|l| plain.get(l + 1) == Some(&',')) {
            return true;
        }
    }
    false
}

/// Whether format string `f` has a non-empty second (negative) section, outside quoted text.
fn has_negative_section(f: &str) -> bool {
    let (mut quoted, mut esc) = (false, false);
    for (i, c) in f.char_indices() {
        if esc {
            esc = false;
            continue;
        }
        match c {
            '\\' if !quoted => esc = true,
            '"' => quoted = !quoted,
            ';' if !quoted => {
                let rest = f.get(i.saturating_add(1)..).unwrap_or("");
                return !rest.split(';').next().unwrap_or("").trim().is_empty();
            }
            _ => {}
        }
    }
    false
}

/// Whether a format string is one the renderer leaves to the VB `FORMAT` rules: a scientific
/// format (`0.00E+00`) or a named general one (`General`, `General Number`, `Scientific`), as a
/// visual's Data format writes them.
fn vb_number_format(f: &str) -> Option<&str> {
    let t = f.trim();
    match t.to_ascii_lowercase().as_str() {
        // `G`: General as Power BI Desktop 2.159.1231.0's Data format card writes it
        "g" | "general" | "general number" => return Some("General Number"),
        "scientific" => return Some(t),
        _ => {}
    }
    let sec = t.split(';').next().unwrap_or("");
    let (mut quoted, mut prev) = (false, ' ');
    let mut chars = sec.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => quoted = !quoted,
            'E' | 'e' if !quoted && matches!(prev, '0' | '#') && matches!(chars.peek(), Some('+' | '-')) => {
                return Some(t);
            }
            _ => {}
        }
        prev = c;
    }
    None
}

fn fmt_val_unsigned(n: f64, col: Option<&ColFmt>, loc: &Locale, abbrev: bool) -> String {
    if !n.is_finite() {
        return String::new();
    }
    if let Some(f) = vb_number_format(col.map_or("", |c| c.fmt.as_str()))
        && let Some(s) = bicraft_locale::vb::format(bicraft_locale::vb::Value::Number(n), f, loc.culture)
    {
        return s;
    }
    let p = fmt_parts(col.map_or("", |c| c.fmt.as_str()));
    if p.pct {
        let min = p.min.unwrap_or(0);
        let max = p.max.map_or(1, |m| m.max(min));
        return pct_text(loc, n, min, max, &p);
    }
    if let Some(c) = col
        && c.kind == "column"
        && c.fmt.is_empty()
        && c.data_type.to_ascii_lowercase().contains("int")
    {
        return format!("{}", n.round());
    }
    let a = n.abs();
    let s = if abbrev && a >= 1000.0 {
        let u = unit_of(a);
        let d = DIV.get(u).copied().unwrap_or(1.0);
        let suf = loc.units.get(u.saturating_sub(1)).cloned().unwrap_or_default();
        // the format string's decimals apply to the scaled value (Desktop: "#,0" → "910K")
        format!("{}{suf}", loc.number(n / d, 0, p.max.unwrap_or(2), true))
    } else if let Some(max) = p.max {
        let min = p.min.unwrap_or(0);
        loc.number(n, min, min.max(max), p.group)
    } else if col.is_some_and(|c| {
        c.fmt.is_empty()
            && (c.kind == "column" || c.kind == "aggregation")
            && c.data_type.to_ascii_lowercase().contains("double")
    }) {
        // a decimal column (or its Sum, Average…) without a format string: Desktop 2.158 shows
        // two decimals ("3 081,30", "1 839,80")
        loc.number(n, 2, 2, true)
    } else {
        loc.number(n, 0, if n.fract() == 0.0 { 0 } else { 2 }, true)
    };
    with_cur(s, &p)
}

/// A data label: the format string, with the label's display units (`None`/0 auto, 1 none, 1000…
/// that unit) and precision (decimals) when the report sets them.
pub fn fmt_label(
    n: f64,
    col: Option<&ColFmt>,
    loc: &Locale,
    units: Option<f64>,
    precision: Option<usize>,
) -> String {
    let p = fmt_parts(col.map_or("", |c| c.fmt.as_str()));
    let Some(prec) = precision.map(|x| x.min(10)) else {
        return match units {
            Some(u) if (1.0..1000.0).contains(&u) => fmt_val(n, col, loc, false),
            Some(u) if u >= 1000.0 => scaled(n, u, p.max.unwrap_or(0), p.max.unwrap_or(2), &p, loc),
            _ => fmt_val(n, col, loc, true),
        };
    };
    if !n.is_finite() {
        return String::new();
    }
    if p.pct {
        return pct_text(loc, n, prec, prec, &p);
    }
    match units {
        Some(u) if u >= 1000.0 => scaled(n, u, prec, prec, &p, loc),
        Some(u) if u >= 1.0 => with_cur(loc.number(n, prec, prec, true), &p),
        _ if n.abs() >= 1000.0 => {
            let u = DIV.get(unit_of(n.abs())).copied().unwrap_or(1.0);
            scaled(n, u, prec, prec, &p, loc)
        }
        _ => with_cur(loc.number(n, prec, prec, true), &p),
    }
}

/// The literal text before the first and after the last digit placeholder of a format
/// string's first section (quotes and escapes removed, outer spaces trimmed): `$` of
/// `\$#,0.0`, `€` of `#,0.00 €`, `u` of `#,0.00 "u"`.
fn literal_ends(f: &str) -> (String, String) {
    let sec = f.split(';').next().unwrap_or("");
    let mut chars: Vec<(char, bool)> = Vec::new(); // (char, literal)
    let (mut quoted, mut esc) = (false, false);
    for c in sec.chars() {
        if esc {
            chars.push((c, true));
            esc = false;
            continue;
        }
        match c {
            '"' => quoted = !quoted,
            '\\' if !quoted => esc = true,
            _ => chars.push((c, quoted)),
        }
    }
    let is_ph = |(c, lit): &(char, bool)| !*lit && matches!(c, '0' | '#');
    let (Some(a), Some(b)) = (chars.iter().position(is_ph), chars.iter().rposition(is_ph)) else {
        return (String::new(), String::new());
    };
    let text = |s: &[(char, bool)]| s.iter().map(|x| x.0).collect::<String>().trim().to_string();
    (
        text(chars.get(..a).unwrap_or(&[])),
        text(chars.get(b + 1..).unwrap_or(&[])),
    )
}

/// A KPI's indicator value (Desktop 2.158, measured on `oracle-gen kpis`): with automatic
/// display units (`units` `None`/0) a value of 1000 and more is scaled to K, M, bn… only when
/// the format string has at least two decimals (`#,0.00` "78,27K", `#,0.###` "7,827M") or
/// there is none (two decimals, "950,50" below 1000); `#,0`, `0.0`, percentages keep the
/// format ("78 270", "78 270,3"). Set units (1000…) scale whatever the format, with its
/// decimals when it has two or more, else two ("78,27K" for `#,0`); 1 shows none. The precision
/// applies only to a scaled value. A scaled value keeps the format's literal ends without their
/// spaces ("78,27K€", "78,27Ku").
pub fn fmt_kpi(
    n: f64,
    col: Option<&ColFmt>,
    loc: &Locale,
    units: Option<f64>,
    precision: Option<usize>,
) -> String {
    if !n.is_finite() {
        return String::new();
    }
    let fs = col.map_or("", |c| c.fmt.as_str());
    let p = fmt_parts(fs);
    let general = fs.trim().is_empty();
    let unit = if p.pct {
        None
    } else {
        match units.filter(|u| *u >= 1.0) {
            Some(u) if u >= 1000.0 => Some(u),
            Some(_) => None,
            None => (n.abs() >= 1000.0 && (general || p.max.unwrap_or(0) >= 2))
                .then(|| DIV.get(unit_of(n.abs())).copied().unwrap_or(1.0)),
        }
    };
    let Some(u) = unit else {
        if general && n.fract() != 0.0 {
            return loc.number(n, 2, 2, true);
        }
        return fmt_val(n, col, loc, false);
    };
    let (min, max) = match precision.map(|x| x.min(10)) {
        Some(d) => (d, d),
        None if !general && p.max.unwrap_or(0) >= 2 => (p.min.unwrap_or(0), p.max.unwrap_or(2)),
        None => (2, 2),
    };
    let i = unit_of(u).max(1);
    let d = DIV.get(i).copied().unwrap_or(1.0);
    let suf = loc.units.get(i - 1).cloned().unwrap_or_default();
    let (pre, post) = literal_ends(fs);
    let num = loc.number(n / d, min, max.max(min), general || p.group);
    format!("{pre}{num}{suf}{post}")
}

fn scaled(n: f64, u: f64, min: usize, max: usize, p: &FmtParts, loc: &Locale) -> String {
    let i = if u >= 1e12 {
        4
    } else if u >= 1e9 {
        3
    } else if u >= 1e6 {
        2
    } else {
        1
    };
    let d = DIV.get(i).copied().unwrap_or(1.0);
    let suf = loc.units.get(i - 1).cloned().unwrap_or_default();
    with_cur(format!("{}{suf}", loc.number(n / d, min, max.max(min), true)), p)
}

/// The label formatter of an axis with ticks `lo..hi` every `step` (one display unit for all).
#[cfg(test)]
pub fn axis_fmt(
    lo: f64,
    hi: f64,
    step: f64,
    col: Option<&ColFmt>,
    loc: &Locale,
) -> impl Fn(f64) -> String + use<> {
    axis_fmt_units(lo, hi, step, col, loc, None)
}

/// [`axis_fmt`] with the axis' display units (`labelDisplayUnits`: 0 or absent = auto, 1 = none,
/// 1000… = that unit).
pub fn axis_fmt_units(
    lo: f64,
    hi: f64,
    step: f64,
    col: Option<&ColFmt>,
    loc: &Locale,
    units: Option<f64>,
) -> impl Fn(f64) -> String + use<> {
    let p = fmt_parts(col.map_or("", |c| c.fmt.as_str()));
    let loc = loc.clone();
    let m = lo.abs().max(hi.abs());
    let u = match units {
        Some(x) if x >= 1e12 => 4,
        Some(x) if x >= 1e9 => 3,
        Some(x) if x >= 1e6 => 2,
        Some(x) if x >= 1e3 => 1,
        Some(x) if x >= 1.0 => 0,
        // "auto" on an axis (Desktop 2.158): thousands from 10 000, millions from 100 000…
        _ if m >= 1e11 => 4,
        _ if m >= 1e8 => 3,
        _ if m >= 1e5 => 2,
        _ if m >= 1e4 => 1,
        _ => 0,
    };
    let d = DIV.get(u).copied().unwrap_or(1.0);
    let st = step / d;
    let dec = if p.pct {
        if step < 0.01 { 1 } else { 0 }
    } else if st <= 0.0 || !st.is_finite() {
        0
    } else {
        // the fewest decimals that write every tick exactly (at most 3)
        (0..=3usize)
            .find(|d| {
                let k = st * 10f64.powi(*d as i32);
                (k - k.round()).abs() < 1e-6 * k.abs().max(1.0)
            })
            .unwrap_or(3)
    };
    let suf = if u > 0 {
        loc.units.get(u - 1).cloned().unwrap_or_default()
    } else {
        String::new()
    };
    move |x: f64| {
        if p.pct {
            pct_text(&loc, x, 0, dec, &p)
        } else {
            with_cur(format!("{}{suf}", loc.number(x / d, dec, dec, true)), &p)
        }
    }
}

/// A date "YYYY-MM-DD[THH:MM]" as the culture writes it.
pub fn fmt_date(s: &str, loc: &Locale) -> Option<String> {
    let b = s.as_bytes();
    let num = |a: usize, n: usize| -> Option<u32> {
        let t = s.get(a..a + n)?;
        if t.bytes().all(|c| c.is_ascii_digit()) {
            t.parse().ok()
        } else {
            None
        }
    };
    let (y, m, d) = (num(0, 4)?, num(5, 2)?, num(8, 2)?);
    if b.get(4) != Some(&b'-') || b.get(7) != Some(&b'-') || !(1..=12).contains(&m) || !(1..=31).contains(&d)
    {
        return None;
    }
    let (hh, mm) = if matches!(b.get(10), Some(b'T' | b' ')) {
        (num(11, 2).unwrap_or(0), num(14, 2).unwrap_or(0))
    } else {
        (0, 0)
    };
    let sep = loc.date_sep;
    let date = match loc.date_order {
        'm' => format!("{m:02}{sep}{d:02}{sep}{y}"),
        'y' => format!("{y}{sep}{m:02}{sep}{d:02}"),
        _ => format!("{d:02}{sep}{m:02}{sep}{y}"),
    };
    if hh == 0 && mm == 0 {
        return Some(date);
    }
    Some(if loc.h12 {
        let am = Some(loc.culture.am).filter(|s| !s.is_empty()).unwrap_or("AM");
        let pm = Some(loc.culture.pm).filter(|s| !s.is_empty()).unwrap_or("PM");
        let (h, ap) = match hh {
            0 => (12, am),
            1..=11 => (hh, am),
            12 => (12, pm),
            _ => (hh - 12, pm),
        };
        format!("{date}, {h:02}:{mm:02} {ap}")
    } else {
        format!("{date} {hh:02}:{mm:02}")
    })
}

/// Whether `s` starts like an ISO date.
pub fn is_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 10
        && b.get(4) == Some(&b'-')
        && b.get(7) == Some(&b'-')
        && [0, 1, 2, 3, 5, 6, 8, 9]
            .iter()
            .all(|i| b.get(*i).is_some_and(u8::is_ascii_digit))
}

/// The OLE serial (days since 1899-12-30, time as the fraction) of an ISO date or date-time.
pub(crate) fn iso_serial(s: &str) -> Option<f64> {
    let days = iso_days(s)?;
    let b = s.as_bytes();
    let part = |a: usize| -> f64 {
        s.get(a..a + 2)
            .filter(|t| t.bytes().all(|c| c.is_ascii_digit()))
            .and_then(|t| t.parse::<f64>().ok())
            .unwrap_or(0.0)
    };
    let secs = if matches!(b.get(10), Some(b'T' | b' ')) {
        part(11) * 3600.0 + part(14) * 60.0 + part(17)
    } else {
        0.0
    };
    Some(days + 25_569.0 + secs / 86_400.0)
}

/// Days since 1970-01-01 of an ISO date (for slider positions).
pub fn iso_days(s: &str) -> Option<f64> {
    if !is_iso_date(s) {
        return None;
    }
    let y: i64 = s.get(0..4)?.parse().ok()?;
    let m: i64 = s.get(5..7)?.parse().ok()?;
    let d: i64 = s.get(8..10)?.parse().ok()?;
    Some(bicraft_locale::calendar::days_from_civil(y, m, d) as f64)
}

/// A data cell as text.
pub fn fmt_cell(x: &Value, col: Option<&ColFmt>, loc: &Locale, abbrev: bool) -> String {
    match x {
        Value::Null => String::new(),
        Value::Number(n) => n
            .as_f64()
            .map(|f| fmt_val(f, col, loc, abbrev))
            .unwrap_or_default(),
        Value::Bool(b) => {
            if *b {
                "True".into()
            } else {
                "False".into()
            }
        }
        Value::String(s) => {
            let typ = col.map_or("", |c| c.data_type.as_str()).to_ascii_lowercase();
            // a named date format (`Long Date`, `General Date`…), as the report's visuals write it
            if let Some(f) = col.map(|c| c.fmt.as_str())
                && let Some(x) = iso_serial(s)
                && let Some(d) = named_date(x, f, loc)
            {
                return d;
            }
            // a date/time with a date format string (a measure's `dd/MM/yyyy HH:mm`): that format
            // in the culture, as Desktop draws it
            if let Some(f) = col
                .map(|c| c.fmt.as_str())
                .filter(|f| bicraft_locale::vb::is_date_format(f))
                && let Some(x) = iso_serial(s)
                && let Some(d) = bicraft_locale::vb::format_date(x, f, loc.culture)
            {
                return d;
            }
            if is_iso_date(s)
                && (typ.is_empty() || typ.contains("date") || typ.contains("time"))
                && let Some(d) = fmt_date(s, loc)
            {
                return d;
            }
            s.chars().take(200).collect()
        }
        _ => String::new(),
    }
}

/// A named date / time format string as Desktop's visuals draw it: `General Date` is the short
/// date and the long time, midnight included ("01/12/2025 00:00:00" in fr-FR, Desktop 2.158),
/// unlike DAX `FORMAT`, which drops a midnight time.
/// Power BI Desktop 2.159.1231.0's Data format card writes .NET standard letters: `d` for Short
/// Date, `D`, `G`, `T`, `t` (docs/formats/sparklines.md).
fn named_date(x: f64, f: &str, loc: &Locale) -> Option<String> {
    let f = match f.trim() {
        "d" => "Short Date",
        "D" => "Long Date",
        "G" => "General Date",
        "T" => "Long Time",
        "t" => "Short Time",
        other => other,
    };
    let pattern = match f.to_ascii_lowercase().as_str() {
        "general date" => "ddddd ttttt",
        "long date" | "medium date" | "short date" | "long time" | "medium time" | "short time" => {
            return bicraft_locale::vb::format(bicraft_locale::vb::Value::Date(x), f, loc.culture);
        }
        _ => return None,
    };
    bicraft_locale::vb::format_date(x, pattern, loc.culture)
}

/// An OLE serial written with a named (`Short Date`, `d`…) or custom date format string, as the
/// report's visuals draw it; `None` when `f` is no date format.
pub(crate) fn serial_text(x: f64, f: &str, loc: &Locale) -> Option<String> {
    named_date(x, f, loc).or_else(|| {
        bicraft_locale::vb::is_date_format(f)
            .then(|| bicraft_locale::vb::format_date(x, f, loc.culture))
            .flatten()
    })
}

/// Whether a date format string writes a time of day (`General Date`, `Long Time`, `HH:mm`…).
pub(crate) fn has_time(f: &str) -> bool {
    let l = f.trim().to_ascii_lowercase();
    matches!(
        l.as_str(),
        "general date" | "g" | "long time" | "medium time" | "short time" | "t"
    ) || (!l.ends_with(" date") && (l.contains('h') || l.contains(':')))
}

/// Whether a UI language tag is French (`fr`, `fr-FR`, `fr_CA`), as BIcraft's visual runtime
/// reads `host.locale` (`langOf` in `crates/engine/src/visual_runtime/strings.ts`).
pub fn is_fr(lang: &str) -> bool {
    let t = lang.trim().to_ascii_lowercase();
    t == "fr" || t.starts_with("fr-") || t.starts_with("fr_")
}

/// A date label of BIcraft's custom visuals: milliseconds since 1970-01-01 (the value's own
/// clock) written with its column's format string in English (en-US) or French (fr-FR), a named
/// format as the report's visuals draw it, else a date/time format string by the DAX `FORMAT`
/// rules. `None` when the format is empty or not a date one, or the date is outside years
/// 100–9999 (the layout keeps its own text). The TypeScript twin is `dateLabel` in
/// `crates/engine/src/visual_runtime/format.ts`; the engine's tests hold both equal.
pub fn date_label(ms: f64, format: &str, lang: &str) -> Option<String> {
    let f = format.trim();
    if f.is_empty() || !ms.is_finite() {
        return None;
    }
    let loc = Locale::new(if is_fr(lang) { "fr-FR" } else { "en-US" });
    let x = ms / 86_400_000.0 + 25_569.0;
    bicraft_locale::datefmt::DateParts::from_serial(x)?;
    named_date(x, f, &loc).or_else(|| {
        bicraft_locale::vb::is_date_format(f)
            .then(|| bicraft_locale::vb::format_date(x, f, loc.culture))
            .flatten()
    })
}

/// Short month name (fake data only).
pub fn month_name(i: usize, loc: &Locale) -> &'static str {
    loc.culture.months_abbr.get(i % 12).copied().unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(fmt: &str) -> ColFmt {
        ColFmt {
            fmt: fmt.into(),
            kind: "measure".into(),
            data_type: "double".into(),
        }
    }

    #[test]
    fn kpi_indicator_units_follow_desktop() {
        // Desktop 2.158 on `oracle-gen kpis` (fr-FR); U+00A0 groups as the renderer writes them
        let fr = Locale::new("fr-FR");
        let k = |f: &str, n: f64, u: Option<f64>, p: Option<usize>| {
            fmt_kpi(n, Some(&col(f)), &fr, u, p).replace('\u{a0}', " ")
        };
        // auto units only with two decimals or more, or no format string
        assert_eq!(k("#,0", 78_270.25, None, None), "78 270");
        assert_eq!(k("#,0.0", 7_827_025.0, None, None), "7 827 025,0");
        assert_eq!(k("#,0.00", 78_270.25, None, None), "78,27K");
        assert_eq!(k("#,0.00", 950.5, None, None), "950,50");
        assert_eq!(k("#,0.000", 7_827_025.0, None, None), "7,827M");
        assert_eq!(k("#,0.###", 78_270.25, None, None), "78,27K");
        assert_eq!(k("", 950.5, None, None), "950,50");
        assert_eq!(k("", 7_827.4, None, None), "7,83K");
        assert_eq!(k("0.0%", 950.5, None, None), "95050,0%");
        // literal ends lose their spaces when scaled
        assert_eq!(k("#,0.00 €", 78_270.25, None, None), "78,27K€");
        assert_eq!(k("#,0.00 \"u\"", 7_827_025.0, None, None), "7,83Mu");
        // set units and precision
        assert_eq!(k("#,0", 78_270.25, Some(1000.0), None), "78,27K");
        assert_eq!(k("#,0", 78_270.25, Some(0.0), None), "78 270");
        assert_eq!(k("#,0", 78_270.25, None, Some(2)), "78 270");
        assert_eq!(k("#,0.00", 78_270.25, Some(1.0), None), "78 270,25");
        assert_eq!(k("#,0.00", 78_270.25, None, Some(0)), "78K");
        assert_eq!(k("#,0.00", 7_827_025.0, None, Some(1)), "7,8M");
        assert_eq!(k("#,0", 7_827_025.0, Some(1000.0), Some(1)), "7 827,0K");
        assert!(fmt_kpi(f64::NAN, None, &fr, None, None).is_empty());
    }

    #[test]
    fn a_negative_amount_puts_its_sign_before_a_leading_currency() {
        // the VB `FORMAT` rules (and Power BI's visuals) write -40 under `$#,0.00` as "-$40.00",
        // with the sign before the currency symbol, not "$-40.00"
        let en = Locale::new("en-US");
        assert_eq!(fmt_val(-40.0, Some(&col("\\$#,0.00")), &en, false), "-$40.00");
        assert_eq!(fmt_val(40.0, Some(&col("\\$#,0.00")), &en, false), "$40.00");
        assert_eq!(fmt_val(-1_500.0, Some(&col("\\$#,0")), &en, true), "-$2K");
        assert_eq!(
            fmt_label(-40.0, Some(&col("\\$#,0.00")), &en, Some(1.0), Some(1)),
            "-$40.0"
        );
        // a trailing currency keeps the sign at the front already
        let fr = Locale::new("fr-FR");
        assert_eq!(
            fmt_val(-40.0, Some(&col("#,0.00 €")), &fr, false).replace('\u{a0}', " "),
            "-40,00 €"
        );
    }

    #[test]
    fn numbers_by_culture() {
        let en = Locale::new("en-US");
        assert_eq!(en.number(1234567.891, 0, 2, true), "1,234,567.89");
        assert_eq!(en.number(0.125, 0, 2, true), "0.13");
        assert_eq!(en.number(-0.001, 0, 2, true), "0");
        assert_eq!(en.number(999.996, 0, 2, true), "1,000");
        let fr = Locale::new("fr-FR");
        // Desktop's visuals group French numbers with U+00A0 (its DOM text)
        assert_eq!(fr.number(1234.5, 2, 2, true), "1\u{a0}234,50");
        // percent sign and display units as Desktop 2.158 draws them for a French model
        assert_eq!(fr.percent(0.256, 1, 1), "25,6%");
        assert_eq!(fr.units[1], "M");
        let pc = |f: &str, n: f64| {
            let c = ColFmt {
                fmt: f.into(),
                ..ColFmt::default()
            };
            fmt_val(n, Some(&c), &fr, true)
        };
        assert_eq!(pc("0.0%", 0.38), "38,0%");
        assert_eq!(pc("+#,0.0 %;-#,0.0 %;0.0 %", 0.304), "+30,4\u{a0}%");
        assert_eq!(pc("#,0", 912_000.0), "912K");
        // a date-time measure with its date format string (pea, item a)
        let dt = ColFmt {
            fmt: "dd/MM/yyyy HH:mm".into(),
            kind: "measure".into(),
            data_type: "string".into(),
        };
        assert_eq!(
            fmt_cell(&Value::String("2026-09-25T16:31:22".into()), Some(&dt), &fr, true),
            "25/09/2026 16:31"
        );
        let de = Locale::new("de-DE");
        assert_eq!(de.units[1], "M");
        assert_eq!(month_name(2, &de), Locale::new("de").culture.months_abbr[2]);
        // unknown and empty cultures are en-US, never the invariant one
        assert_eq!(Locale::new("").culture.name, "en-US");
        assert_eq!(Locale::new("xx-YY").number(1234.5, 1, 1, true), "1,234.5");
        assert_eq!(Locale::new("invariant").culture.name, "en-US");
        assert_eq!(Locale::new(&"x".repeat(10_000)).culture.name, "en-US");
        let es = Locale::new("es-ES");
        assert_eq!(es.number(1234.0, 0, 0, true), "1234");
        assert_eq!(es.number(12345.0, 0, 0, true), "12.345");
    }

    #[test]
    fn values_and_axes() {
        let en = Locale::new("en-US");
        assert_eq!(fmt_val(1234.0, None, &en, true), "1.23K");
        assert_eq!(fmt_val(1234.0, None, &en, false), "1,234");
        assert_eq!(fmt_val(0.256, Some(&col("0.0%")), &en, false), "25.6%");
        assert_eq!(fmt_val(12.5, Some(&col("\\$#,0.00")), &en, false), "$12.50");
        assert_eq!(fmt_val(0.064, Some(&col("+0.0%;-0.0%")), &en, false), "+6.4%");
        assert_eq!(
            fmt_label(417_736.0, Some(&col("#,0")), &en, Some(1.0), None),
            "417,736"
        );
        assert_eq!(
            fmt_label(417_736.0, Some(&col("#,0")), &en, Some(1e6), Some(2)),
            "0.42M"
        );
        assert_eq!(fmt_label(417_736.0, Some(&col("#,0")), &en, None, None), "418K");
        assert_eq!(fmt_val(-0.037, Some(&col("+0.0%;-0.0%")), &en, false), "-3.7%");
        // axis display units: thousands from 10 000, millions from 100 000, uniform decimals
        assert_eq!(axis_fmt(0.0, 300_000.0, 100_000.0, None, &en)(0.0), "0.0M");
        assert_eq!(axis_fmt(0.0, 80_000.0, 10_000.0, None, &en)(50_000.0), "50K");
        assert_eq!(
            fmt_val(12.5, Some(&col("#,0.00 €")), &Locale::new("fr-FR"), false),
            "12,50\u{a0}€"
        );
        let f = axis_fmt(0.0, 40000.0, 10000.0, None, &en);
        assert_eq!(f(20000.0), "20K");
        let f = axis_fmt(0.0, 1.0, 0.25, None, &en);
        assert_eq!(f(0.25), "0.25");
    }

    #[test]
    fn dates() {
        assert_eq!(
            fmt_date("2025-01-31", &Locale::new("en-US")).as_deref(),
            Some("01/31/2025")
        );
        assert_eq!(
            fmt_date("2025-01-31T14:05:00", &Locale::new("fr-FR")).as_deref(),
            Some("31/01/2025 14:05")
        );
        assert_eq!(fmt_date("2025-13-01", &Locale::new("en-US")), None);
        assert_eq!(
            fmt_date("2025-01-31", &Locale::new("ja-JP")).as_deref(),
            Some("2025/01/31")
        );
        assert_eq!(
            fmt_date("2025-01-31T14:05", &Locale::new("en-US")).as_deref(),
            Some("01/31/2025, 02:05 PM")
        );
        assert_eq!(iso_days("1970-01-02"), Some(1.0));
        assert_eq!(
            fmt_cell(&Value::from("2025-02-03"), None, &Locale::new("de-DE"), false),
            "03.02.2025"
        );
    }

    /// The Data format card's scientific and General formats go through the VB `FORMAT` rules;
    /// the others keep the renderer's own formatting.
    #[test]
    fn scientific_and_general_formats() {
        let en = Locale::new("en-US");
        assert_eq!(fmt_val(12345.0, Some(&col("0.00E+00")), &en, false), "1.23E+04");
        assert_eq!(fmt_val(-0.5, Some(&col("0.0E+00")), &en, false), "-5.0E-01");
        assert_eq!(fmt_val(1234.5, Some(&col("General")), &en, false), "1234.5");
        assert_eq!(vb_number_format("\"E+\"0"), None);
        assert_eq!(vb_number_format("#,0.00"), None);
        assert_eq!(fmt_val(1234.5, Some(&col("#,0.0")), &en, false), "1,234.5");
    }

    /// A cell or tooltip value under a format string with scaling commas (before the decimal
    /// point or right after the last decimal) or literal text takes the whole format string.
    #[test]
    fn cells_take_scaling_commas_and_literal_text() {
        let en = Locale::new("en-US");
        let fr = Locale::new("fr-FR");
        let cell = |n: f64, f: &str, l: &Locale| fmt_cell(&json_num(n), Some(&col(f)), l, false);
        assert_eq!(cell(1_234_567.0, "#,0.0,\"K\"", &en), "1,234.6K");
        assert_eq!(cell(1_234_567.0, "#,##0,\"K\"", &en), "1,235K");
        assert_eq!(cell(7_500_000.0, "0,,\"M\"", &en), "8M");
        assert_eq!(cell(-7_433_915.0, "$#,0.00,,\"M\"", &en), "-$7.43M");
        assert_eq!(cell(1_234_567.0, "#,0.0,\"K\"", &fr), "1\u{a0}234,6K");
        assert_eq!(cell(1234.0, "#,0 \"units\"", &en), "1,234 units");
        assert_eq!(cell(12.0, "0\\h", &en), "12h");
        // a currency or a grouping comma alone keeps the renderer's own path
        assert_eq!(cell(1234.5, "#,0.00", &en), "1,234.50");
        assert_eq!(cell(12.5, "#,0.00 \"€\"", &fr), "12,50\u{a0}€");
        assert_eq!(cell(-40.0, "\\$#,0.00", &en), "-$40.00");
    }

    fn json_num(n: f64) -> Value {
        serde_json::Number::from_f64(n).map_or(Value::Null, Value::Number)
    }

    /// The format strings Power BI Desktop 2.159.1231.0's Data format card writes
    /// (docs/formats/sparklines.md): `G` (General), `0.00E+000` (Scientific), `d` (a date's Short
    /// Date), and three-section currencies.
    #[test]
    fn desktop_data_format_strings() {
        let en = Locale::new("en-US");
        let fr = Locale::new("fr-FR");
        assert_eq!(fmt_val(1234.5, Some(&col("G")), &en, false), "1234.5");
        assert_eq!(fmt_val(12345.0, Some(&col("0.00E+000")), &en, false), "1.23E+004");
        let usd = col("\\$#,0.00;(\\$#,0.00);\\$#,0.00");
        assert_eq!(fmt_val(1234.5, Some(&usd), &en, false), "$1,234.50");
        assert_eq!(fmt_val(-5.0, Some(&usd), &en, false), "($5.00)");
        let eur = col("#,0.0 \"€\";-#,0.0 \"€\";#,0.0 \"€\"");
        // the renderer keeps the space before a symbol unbreakable
        assert_eq!(fmt_val(10.0, Some(&eur), &fr, false), "10,0\u{a0}€");
        let day = |f: &str, l: &Locale| {
            let mut c = col(f);
            c.data_type = "dateTime".into();
            fmt_cell(&Value::from("2025-01-15T00:00:00"), Some(&c), l, false)
        };
        assert_eq!(day("d", &fr), "15/01/2025");
        assert_eq!(day("d", &en), "1/15/2025");
    }
}
