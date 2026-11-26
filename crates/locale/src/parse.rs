//! Text → number in a culture.
//!
//! [`value`] reads text the way DAX `VALUE` (and `FORMAT` of a text) does in the engine of
//! Power BI Desktop: the culture's decimal separator, its group separator anywhere after the
//! first digit (a space-like group separator also accepts a plain space), the culture's
//! currency symbol before or after the number, a sign in front or at the end, parentheses for
//! negatives and an exponent. A percent sign is not accepted.
//!
//! [`strip_currency`] removes a currency symbol for parsers with their own number rules (Power
//! Query's `Number.FromText`, CSV type detection).

use crate::Locale;

/// Longest text read as a number.
const MAX_LEN: usize = 400;

fn space_like(c: char) -> bool {
    matches!(c, ' ' | '\u{a0}' | '\u{202f}')
}

/// The text without the culture's currency symbol (before or after the number, with or without
/// a space) and surrounding blanks; `None` when there was no symbol.
pub fn strip_currency<'a>(text: &'a str, loc: &Locale) -> Option<&'a str> {
    strip_symbol(text, loc.currency.symbol)
}

/// The text without currency symbol `sym` before or after the number (and the blanks around
/// it); `None` when the symbol is not there.
pub fn strip_symbol<'a>(text: &'a str, sym: &str) -> Option<&'a str> {
    if sym.is_empty() {
        return None;
    }
    let t = text.trim_matches(space_like);
    // A sign may come before the symbol: "-$5", "-€ 5".
    if let Some(r) = t.strip_prefix(sym) {
        return Some(r.trim_matches(space_like));
    }
    if let Some(r) = t.strip_suffix(sym) {
        return Some(r.trim_matches(space_like));
    }
    None
}

/// DAX `VALUE(text)` in culture `loc`; `None` when the text is not a number there.
pub fn value(text: &str, loc: &Locale) -> Option<f64> {
    if text.len() > MAX_LEN {
        return None;
    }
    let mut t = text.trim_matches(space_like).to_string();
    let mut negative = false;
    if let Some(inner) = t.strip_prefix('(').and_then(|r| r.strip_suffix(')')) {
        negative = true;
        t = inner.trim_matches(space_like).to_string();
    }
    // Sign before or after a currency symbol, or at the end.
    for _ in 0..2 {
        if let Some(r) = t.strip_prefix('-') {
            negative = !negative;
            t = r.trim_start_matches(space_like).to_string();
        } else if let Some(r) = t.strip_prefix('+') {
            t = r.trim_start_matches(space_like).to_string();
        } else if let Some(r) = t.strip_suffix('-') {
            negative = !negative;
            t = r.trim_end_matches(space_like).to_string();
        }
        if let Some(r) = strip_currency(&t, loc) {
            t = r.to_string();
        }
    }
    let group_space = space_like(loc.group);
    let mut norm = String::with_capacity(t.len());
    let (mut seen_dec, mut seen_exp, mut digits) = (false, false, 0usize);
    for c in t.chars() {
        if c.is_ascii_digit() {
            norm.push(c);
            digits += 1;
        } else if c == loc.decimal && !seen_dec && !seen_exp {
            seen_dec = true;
            norm.push('.');
        } else if (c == loc.group || (group_space && c == ' ')) && !seen_exp && digits > 0 {
            // group separators are dropped wherever they are (en-US "1.234,5" is 1.2345)
        } else if (c == 'e' || c == 'E') && digits > 0 && !seen_exp {
            seen_exp = true;
            norm.push('e');
        } else if (c == '-' || c == '+') && norm.ends_with('e') {
            norm.push(c);
        } else {
            return None;
        }
    }
    if digits == 0 {
        return None;
    }
    let x: f64 = norm.parse().ok()?;
    if !x.is_finite() {
        return None;
    }
    Some(if negative { -x } else { x })
}
