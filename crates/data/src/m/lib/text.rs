//! `Text.*` and `Character.*`. Positions and lengths count Unicode scalar values.

use super::{Builtin, arg, b, conv_err, count, culture_arg, int, items, opt_number, opt_text, text};
use crate::m::convert;
use crate::m::eval::Interp;
use crate::m::value::{Func, Native, Res, Value, expr_err, unsupported};

pub(crate) static FUNCS: &[Builtin] = &[
    b("Text.From", 1, 2, from),
    b("Text.Upper", 1, 2, upper),
    b("Text.Lower", 1, 2, lower),
    b("Text.Proper", 1, 2, proper),
    b("Text.Trim", 1, 2, trim),
    b("Text.TrimStart", 1, 2, trim_start),
    b("Text.TrimEnd", 1, 2, trim_end),
    b("Text.Clean", 1, 1, clean),
    b("Text.Length", 1, 1, length),
    b("Text.Start", 2, 2, start),
    b("Text.End", 2, 2, end),
    b("Text.Middle", 2, 3, middle),
    b("Text.Range", 2, 3, range),
    b("Text.At", 2, 2, at),
    b("Text.Contains", 2, 3, contains),
    b("Text.StartsWith", 2, 3, starts_with),
    b("Text.EndsWith", 2, 3, ends_with),
    b("Text.PositionOf", 2, 4, position_of),
    b("Text.PositionOfAny", 2, 3, position_of_any),
    b("Text.Replace", 3, 3, replace),
    b("Text.ReplaceRange", 4, 4, replace_range),
    b("Text.Insert", 3, 3, insert),
    b("Text.Remove", 2, 2, remove),
    b("Text.RemoveRange", 2, 3, remove_range),
    b("Text.Select", 2, 2, select),
    b("Text.Split", 2, 2, split),
    b("Text.SplitAny", 2, 2, split_any),
    b("Text.Combine", 1, 2, combine),
    b("Text.PadStart", 2, 3, pad_start),
    b("Text.PadEnd", 2, 3, pad_end),
    b("Text.Repeat", 2, 2, repeat),
    b("Text.Reverse", 1, 1, reverse),
    b("Text.ToList", 1, 1, to_list),
    b("Text.BeforeDelimiter", 2, 3, before_delimiter),
    b("Text.AfterDelimiter", 2, 3, after_delimiter),
    b("Text.BetweenDelimiters", 3, 5, between_delimiters),
    b("Text.Format", 2, 3, format),
    b("Text.NewGuid", 0, 0, new_guid),
    b("Character.FromNumber", 1, 1, char_from_number),
    b("Character.ToNumber", 1, 1, char_to_number),
];

fn text_or_null(v: &Value) -> Res<Option<String>> {
    Ok(opt_text(v)?.map(str::to_string))
}

fn map_text(a: &[Value], f: impl FnOnce(&str) -> String) -> Res<Value> {
    Ok(match opt_text(arg(a, 0))? {
        None => Value::Null,
        Some(t) => Value::text(f(t)),
    })
}

fn chars(s: &str) -> Vec<char> {
    s.chars().collect()
}

fn from(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let c = culture_arg(interp, arg(a, 1))?;
    let v = arg(a, 0);
    match v {
        Value::Null => Ok(Value::Null),
        Value::List(_)
        | Value::Record(_)
        | Value::Table(_)
        | Value::Function(_)
        | Value::Type(_)
        | Value::Binary(_) => conv_err(v, "Text"),
        other => Ok(convert::to_text(other, &c).map_or(Value::Null, Value::text)),
    }
}

fn upper(_: &mut Interp, a: &[Value]) -> Res<Value> {
    map_text(a, str::to_uppercase)
}

fn lower(_: &mut Interp, a: &[Value]) -> Res<Value> {
    map_text(a, str::to_lowercase)
}

fn proper(_: &mut Interp, a: &[Value]) -> Res<Value> {
    map_text(a, |t| {
        // A letter that does not follow another letter starts a word (`O'Neil`, `2Nd`).
        let mut out = String::with_capacity(t.len());
        let mut start = true;
        for c in t.chars() {
            if c.is_alphabetic() {
                if start {
                    out.extend(c.to_uppercase());
                } else {
                    out.extend(c.to_lowercase());
                }
                start = false;
            } else {
                out.push(c);
                start = true;
            }
        }
        out
    })
}

fn trim_set(v: &Value) -> Res<Option<Vec<char>>> {
    Ok(match v {
        Value::Null => None,
        Value::Text(t) => Some(t.chars().collect()),
        Value::List(l) => {
            let mut out = Vec::new();
            for x in l.iter() {
                out.extend(text(x)?.chars());
            }
            Some(out)
        }
        other => return conv_err(other, "Text"),
    })
}

fn trim_with(a: &[Value], start: bool, end: bool) -> Res<Value> {
    let set = trim_set(arg(a, 1))?;
    let is_trim = |c: char| match &set {
        None => c.is_whitespace(),
        Some(s) => s.contains(&c),
    };
    map_text(a, |t| {
        let mut s = t;
        if start {
            s = s.trim_start_matches(is_trim);
        }
        if end {
            s = s.trim_end_matches(is_trim);
        }
        s.to_string()
    })
}

fn trim(_: &mut Interp, a: &[Value]) -> Res<Value> {
    trim_with(a, true, true)
}

fn trim_start(_: &mut Interp, a: &[Value]) -> Res<Value> {
    trim_with(a, true, false)
}

fn trim_end(_: &mut Interp, a: &[Value]) -> Res<Value> {
    trim_with(a, false, true)
}

fn clean(_: &mut Interp, a: &[Value]) -> Res<Value> {
    map_text(a, |t| t.chars().filter(|c| !c.is_control()).collect())
}

fn length(_: &mut Interp, a: &[Value]) -> Res<Value> {
    Ok(match opt_text(arg(a, 0))? {
        None => Value::Null,
        Some(t) => Value::num(t.chars().count() as f64),
    })
}

fn start(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let n = count(arg(a, 1))?;
    map_text(a, |t| t.chars().take(n).collect())
}

fn end(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let n = count(arg(a, 1))?;
    map_text(a, |t| {
        let c = chars(t);
        c.iter().skip(c.len().saturating_sub(n)).collect()
    })
}

fn middle(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let Some(t) = text_or_null(arg(a, 0))? else {
        return Ok(Value::Null);
    };
    let c = chars(&t);
    // A start beyond the end gives "" (as Desktop does), not an error.
    let s = count(arg(a, 1))?.min(c.len());
    let n = match arg(a, 2) {
        Value::Null => c.len() - s,
        v => count(v)?,
    };
    Ok(Value::text(c.iter().skip(s).take(n).collect::<String>()))
}

fn range(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let Some(t) = text_or_null(arg(a, 0))? else {
        return Ok(Value::Null);
    };
    let c = chars(&t);
    let s = count(arg(a, 1))?;
    let n = match arg(a, 2) {
        Value::Null => c.len().saturating_sub(s),
        v => count(v)?,
    };
    if s > c.len() || s.saturating_add(n) > c.len() {
        return expr_err("The 'offset' or 'count' argument is out of range.");
    }
    Ok(Value::text(c.iter().skip(s).take(n).collect::<String>()))
}

fn at(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = text(arg(a, 0))?;
    let i = count(arg(a, 1))?;
    match t.chars().nth(i) {
        Some(c) => Ok(Value::text(c.to_string())),
        None => expr_err("The index is out of range."),
    }
}

fn ignore_case(v: &Value) -> bool {
    matches!(v, Value::Function(f) if matches!(&**f, Func::Native(Native::Comparer { ignore_case: true })))
}

fn fold(s: &str, ci: bool) -> String {
    if ci { s.to_uppercase() } else { s.to_string() }
}

fn contains(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let Some(t) = opt_text(arg(a, 0))? else {
        return Ok(Value::Null);
    };
    let ci = ignore_case(arg(a, 2));
    let s = text(arg(a, 1))?;
    Ok(Value::Logical(fold(t, ci).contains(&fold(s, ci))))
}

fn starts_with(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let Some(t) = opt_text(arg(a, 0))? else {
        return Ok(Value::Null);
    };
    let ci = ignore_case(arg(a, 2));
    let s = text(arg(a, 1))?;
    Ok(Value::Logical(fold(t, ci).starts_with(&fold(s, ci))))
}

fn ends_with(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let Some(t) = opt_text(arg(a, 0))? else {
        return Ok(Value::Null);
    };
    let ci = ignore_case(arg(a, 2));
    let s = text(arg(a, 1))?;
    Ok(Value::Logical(fold(t, ci).ends_with(&fold(s, ci))))
}

/// Char positions of every occurrence of `needle` in `hay` (non-overlapping).
fn occurrences(hay: &str, needle: &str) -> Vec<usize> {
    if needle.is_empty() {
        return vec![0];
    }
    hay.match_indices(needle)
        .map(|(byte, _)| hay.get(..byte).map_or(0, |p| p.chars().count()))
        .collect()
}

fn position_of(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = text(arg(a, 0))?;
    let s = text(arg(a, 1))?;
    let ci = ignore_case(arg(a, 3));
    let hits = occurrences(&fold(t, ci), &fold(s, ci));
    let occ = opt_number(arg(a, 2))?.unwrap_or(0.0);
    Ok(match occ as i64 {
        2 => Value::list(hits.into_iter().map(|i| Value::num(i as f64)).collect()),
        1 => Value::num(hits.last().map_or(-1.0, |&i| i as f64)),
        _ => Value::num(hits.first().map_or(-1.0, |&i| i as f64)),
    })
}

fn position_of_any(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = text(arg(a, 0))?;
    let set: Vec<char> = items(arg(a, 1))?
        .iter()
        .map(|v| text(v).map(|s| s.chars().collect::<Vec<_>>()))
        .collect::<Res<Vec<_>>>()?
        .concat();
    let hits: Vec<usize> = t
        .chars()
        .enumerate()
        .filter(|(_, c)| set.contains(c))
        .map(|(i, _)| i)
        .collect();
    let occ = opt_number(arg(a, 2))?.unwrap_or(0.0);
    Ok(match occ as i64 {
        2 => Value::list(hits.into_iter().map(|i| Value::num(i as f64)).collect()),
        1 => Value::num(hits.last().map_or(-1.0, |&i| i as f64)),
        _ => Value::num(hits.first().map_or(-1.0, |&i| i as f64)),
    })
}

pub(crate) fn replace_text(interp: &Interp, t: &str, old: &str, new: &str) -> Res<Value> {
    if old.is_empty() {
        return Ok(Value::text(t));
    }
    let n = t.matches(old).count();
    let len = t.len().saturating_add(n.saturating_mul(new.len()));
    interp.check_text(len)?;
    Ok(Value::text(t.replace(old, new)))
}

fn replace(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let Some(t) = opt_text(arg(a, 0))? else {
        return Ok(Value::Null);
    };
    let old = text(arg(a, 1))?;
    let new = text(arg(a, 2))?;
    replace_text(interp, t, old, new)
}

fn replace_range(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = chars(text(arg(a, 0))?);
    let off = count(arg(a, 1))?;
    let n = count(arg(a, 2))?;
    let new = text(arg(a, 3))?;
    if off.saturating_add(n) > t.len() {
        return expr_err("The 'offset' or 'count' argument is out of range.");
    }
    let mut s: String = t.iter().take(off).collect();
    s.push_str(new);
    s.extend(t.iter().skip(off + n));
    Ok(Value::text(s))
}

fn insert(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = chars(text(arg(a, 0))?);
    let off = count(arg(a, 1))?;
    let new = text(arg(a, 2))?;
    if off > t.len() {
        return expr_err("The 'offset' argument is out of range.");
    }
    let mut s: String = t.iter().take(off).collect();
    s.push_str(new);
    s.extend(t.iter().skip(off));
    Ok(Value::text(s))
}

fn remove(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let set = trim_set(arg(a, 1))?.unwrap_or_default();
    map_text(a, |t| t.chars().filter(|c| !set.contains(c)).collect())
}

fn remove_range(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = chars(text(arg(a, 0))?);
    let off = count(arg(a, 1))?;
    let n = match arg(a, 2) {
        Value::Null => 1,
        v => count(v)?,
    };
    if off.saturating_add(n) > t.len() {
        return expr_err("The 'offset' or 'count' argument is out of range.");
    }
    Ok(Value::text(
        t.iter()
            .take(off)
            .chain(t.iter().skip(off + n))
            .collect::<String>(),
    ))
}

fn select(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let set = trim_set(arg(a, 1))?.unwrap_or_default();
    map_text(a, |t| t.chars().filter(|c| set.contains(c)).collect())
}

fn split(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = text(arg(a, 0))?;
    let sep = text(arg(a, 1))?;
    if sep.is_empty() {
        return Ok(Value::list(vec![Value::text(t)]));
    }
    Ok(Value::list(t.split(sep).map(Value::text).collect()))
}

fn split_any(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = text(arg(a, 0))?;
    let seps: Vec<char> = text(arg(a, 1))?.chars().collect();
    Ok(Value::list(
        t.split(|c| seps.contains(&c)).map(Value::text).collect(),
    ))
}

fn combine(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let l = items(arg(a, 0))?;
    let sep = opt_text(arg(a, 1))?.unwrap_or("");
    let mut out = String::new();
    let mut first = true;
    for v in &l {
        let Some(s) = opt_text(v)? else { continue };
        if !first {
            out.push_str(sep);
        }
        first = false;
        out.push_str(s);
        interp.check_text(out.len())?;
    }
    Ok(Value::text(out))
}

fn pad(interp: &mut Interp, a: &[Value], at_start: bool) -> Res<Value> {
    let Some(t) = text_or_null(arg(a, 0))? else {
        return Ok(Value::Null);
    };
    let n = count(arg(a, 1))?;
    interp.check_text(n.saturating_mul(4))?;
    let ch = match opt_text(arg(a, 2))? {
        None => ' ',
        Some(s) => {
            let mut it = s.chars();
            match (it.next(), it.next()) {
                (Some(c), None) => c,
                _ => return expr_err("The padding must be a single character."),
            }
        }
    };
    let len = t.chars().count();
    if len >= n {
        return Ok(Value::text(t));
    }
    let fill: String = std::iter::repeat_n(ch, n - len).collect();
    Ok(Value::text(if at_start {
        format!("{fill}{t}")
    } else {
        format!("{t}{fill}")
    }))
}

fn pad_start(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    pad(interp, a, true)
}

fn pad_end(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    pad(interp, a, false)
}

fn repeat(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let Some(t) = text_or_null(arg(a, 0))? else {
        return Ok(Value::Null);
    };
    let n = count(arg(a, 1))?;
    interp.check_text(t.len().saturating_mul(n))?;
    Ok(Value::text(t.repeat(n)))
}

fn reverse(_: &mut Interp, a: &[Value]) -> Res<Value> {
    map_text(a, |t| t.chars().rev().collect())
}

fn to_list(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = text(arg(a, 0))?;
    Ok(Value::list(
        t.chars().map(|c| Value::text(c.to_string())).collect(),
    ))
}

/// `index` argument of the delimiter functions: a number, or `{n, RelativePosition.FromEnd}`.
fn delim_index(v: &Value) -> Res<(usize, bool)> {
    Ok(match v {
        Value::Null => (0, false),
        Value::Number(_) => (count(v)?, false),
        Value::List(l) => {
            let n = count(l.first().unwrap_or(&Value::Null))?;
            let from_end = matches!(l.get(1), Some(Value::Number(x)) if x.f() == 1.0);
            (n, from_end)
        }
        other => return conv_err(other, "Number"),
    })
}

/// Byte offset of the n-th occurrence of `d` (from the start or the end).
fn nth_delim(t: &str, d: &str, n: usize, from_end: bool) -> Option<usize> {
    if d.is_empty() {
        return None;
    }
    let hits: Vec<usize> = t.match_indices(d).map(|(i, _)| i).collect();
    if from_end {
        hits.len().checked_sub(n + 1).and_then(|i| hits.get(i).copied())
    } else {
        hits.get(n).copied()
    }
}

fn before_delimiter(_: &mut Interp, a: &[Value]) -> Res<Value> {
    // null reads as "" (Desktop returns "" here, not null).
    let t = opt_text(arg(a, 0))?.unwrap_or("");
    let d = text(arg(a, 1))?;
    let (n, end) = delim_index(arg(a, 2))?;
    Ok(Value::text(match nth_delim(t, d, n, end) {
        Some(i) => t.get(..i).unwrap_or(t),
        None => t,
    }))
}

fn after_delimiter(_: &mut Interp, a: &[Value]) -> Res<Value> {
    // null reads as "" (Desktop returns "" here, not null).
    let t = opt_text(arg(a, 0))?.unwrap_or("");
    let d = text(arg(a, 1))?;
    let (n, end) = delim_index(arg(a, 2))?;
    Ok(Value::text(match nth_delim(t, d, n, end) {
        Some(i) => t.get(i + d.len()..).unwrap_or(""),
        None => "",
    }))
}

fn between_delimiters(_: &mut Interp, a: &[Value]) -> Res<Value> {
    // null reads as "" (Desktop returns "" here, not null).
    let t = opt_text(arg(a, 0))?.unwrap_or("");
    let s = text(arg(a, 1))?;
    let e = text(arg(a, 2))?;
    let (n, from_end) = delim_index(arg(a, 3))?;
    let (m, _) = delim_index(arg(a, 4))?;
    let Some(i) = nth_delim(t, s, n, from_end) else {
        return Ok(Value::text(""));
    };
    let rest = t.get(i + s.len()..).unwrap_or("");
    Ok(Value::text(match nth_delim(rest, e, m, false) {
        Some(j) => rest.get(..j).unwrap_or(rest),
        None => rest,
    }))
}

fn format(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let fmt = text(arg(a, 0))?;
    let c = culture_arg(interp, arg(a, 2))?;
    let args = arg(a, 1).clone();
    let mut out = String::new();
    let mut rest = fmt;
    while let Some(p) = rest.find('#') {
        out.push_str(rest.get(..p).unwrap_or(""));
        let after = rest.get(p + 1..).unwrap_or("");
        let (open, close) = match after.chars().next() {
            Some('{') => ('{', '}'),
            Some('[') => ('[', ']'),
            _ => {
                out.push('#');
                rest = after;
                continue;
            }
        };
        let Some(end) = after.find(close) else {
            out.push('#');
            rest = after;
            continue;
        };
        let key = after.get(1..end).unwrap_or("");
        let v = if open == '{' {
            let i: usize = key
                .trim()
                .parse()
                .map_err(|_| ())
                .or_else(|_| expr_err("Bad Text.Format index."))?;
            match &args {
                Value::List(l) => l.get(i).cloned().unwrap_or(Value::Null).check()?,
                other => return conv_err(other, "List"),
            }
        } else {
            interp.field(&args, key, false)?
        };
        out.push_str(&convert::to_text(&v, &c).unwrap_or_default());
        interp.check_text(out.len())?;
        rest = after.get(end + 1..).unwrap_or("");
    }
    out.push_str(rest);
    Ok(Value::text(out))
}

fn new_guid(_: &mut Interp, _: &[Value]) -> Res<Value> {
    unsupported("`Text.NewGuid` (non-deterministic)")
}

fn char_from_number(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let n = int(arg(a, 0))?;
    match u32::try_from(n).ok().and_then(char::from_u32) {
        Some(c) => Ok(Value::text(c.to_string())),
        None => expr_err(format!("{n} is not a valid character code.")),
    }
}

fn char_to_number(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = text(arg(a, 0))?;
    match t.chars().next() {
        Some(c) => Ok(Value::num(f64::from(u32::from(c)))),
        None => expr_err("The text is empty."),
    }
}
