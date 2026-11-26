//! Numbers as Desktop's visuals show them (data labels, cards, axes, tables): the culture's
//! CLDR separators and percent layout and Power BI's display units (`1,2 M` in fr-FR,
//! `1.2M` in en-US, `1,2 Mio.` in de-DE).
//!
//! The viewer and the PNG renderer call these with the model culture; DAX `FORMAT` and Power
//! Query use [`crate::vb`] and [`crate::datefmt`] instead.

use crate::{Locale, number};

/// Display units (`labelDisplayUnits`): none, thousands, millions, billions, trillions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    None,
    Thousands,
    Millions,
    Billions,
    Trillions,
}

impl Unit {
    /// The unit of a `labelDisplayUnits` value (1 none, 1000 …, 0 or anything else: `None`,
    /// for the caller's auto rule).
    pub fn from_property(v: f64) -> Option<Unit> {
        [
            (1.0, Unit::None),
            (1e3, Unit::Thousands),
            (1e6, Unit::Millions),
            (1e9, Unit::Billions),
            (1e12, Unit::Trillions),
        ]
        .into_iter()
        .find(|(d, _)| *d == v)
        .map(|(_, u)| u)
    }

    /// Auto: the largest unit not above `|x|`.
    pub fn auto(x: f64) -> Unit {
        let a = x.abs();
        if a >= 1e12 {
            Unit::Trillions
        } else if a >= 1e9 {
            Unit::Billions
        } else if a >= 1e6 {
            Unit::Millions
        } else if a >= 1e3 {
            Unit::Thousands
        } else {
            Unit::None
        }
    }

    pub fn divisor(self) -> f64 {
        match self {
            Unit::None => 1.0,
            Unit::Thousands => 1e3,
            Unit::Millions => 1e6,
            Unit::Billions => 1e9,
            Unit::Trillions => 1e12,
        }
    }

    /// The culture's suffix (with the space it needs), empty for `None`.
    pub fn suffix(self, loc: &Locale) -> &'static str {
        let u = &loc.display.units;
        match self {
            Unit::None => "",
            Unit::Thousands => u[0],
            Unit::Millions => u[1],
            Unit::Billions => u[2],
            Unit::Trillions => u[3],
        }
    }
}

/// `x` with between `min` and `max` decimals (rounded half away from zero), the culture's
/// separators, grouping when asked (not for 4-digit integer parts in cultures whose minimum
/// grouping is 2).
pub fn number(x: f64, min: usize, max: usize, grouping: bool, loc: &Locale) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "∞".into() } else { "-∞".into() };
    }
    let max = max.min(15);
    let min = min.min(max);
    let Some((int, mut frac)) = number::fixed_digits(x, max) else {
        return String::new();
    };
    while frac.len() > min && frac.ends_with('0') {
        frac.pop();
    }
    let int = if int.is_empty() { "0".to_string() } else { int };
    let neg = x < 0.0 && (int.bytes().any(|b| b != b'0') || frac.bytes().any(|b| b != b'0'));
    let d = &loc.display;
    let mut out = String::new();
    if neg {
        out.push('-');
    }
    let min_len = if d.min_grouping >= 2 { 5 } else { 4 };
    if grouping && int.len() >= min_len {
        out.push_str(&number::group(&int, d.group));
    } else {
        out.push_str(&int);
    }
    if !frac.is_empty() {
        out.push(d.decimal);
        out.push_str(&frac);
    }
    out
}

/// `x` as a percentage (`0.256` → `25.6%`, fr-FR `25,6 %`).
pub fn percent(x: f64, min: usize, max: usize, loc: &Locale) -> String {
    let p = loc.display.percent;
    format!(
        "{}{}{}",
        p.prefix,
        number(x * 100.0, min, max, true, loc),
        p.suffix
    )
}

/// `x` in display unit `unit` with at most `max` decimals (`1234567` → `1.23M`).
pub fn with_unit(x: f64, unit: Unit, max: usize, loc: &Locale) -> String {
    let mut s = number(x / unit.divisor(), 0, max, true, loc);
    s.push_str(unit.suffix(loc));
    s
}
