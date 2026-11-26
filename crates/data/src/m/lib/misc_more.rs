// Ported from DashCraft (MIT OR Apache-2.0, used here under MIT),
// crates/pbi-m/src/stdlib/number.rs, binary.rs, text.rs, value_fns.rs and record.rs@0958fd82b04656ee1797101a2b4df8d114dfebd1
// Copyright (c) 2026 Jack Sweeney and the DashCraft contributors. Licensed under the MIT License.
// See crates/data/LICENSE-MIT-dashcraft and THIRD_PARTY_NOTICES.md at the repository root.

//! More library functions (ROADMAP M11.34): `Number.Bitwise*`, `Binary.From`, `Combine`,
//! `Range`, `Split`, `ApproximateLength`, `Json.FromValue`, `Value.Type`, `ReplaceMetadata`,
//! `NullableEquals`, `Type.Is`, `IsNullable`, `NonNullable`, `TableColumn`,
//! `Record.ReorderFields`, the splitters `Splitter.SplitTextByEachDelimiter`, `ByRanges`,
//! `ByRepeatedLengths`, `ByWhitespace` and `ByCharacterTransition`, and `Time.ToRecord`,
//! `DateTimeZone.ToRecord`. Each follows its Microsoft Learn reference page
//! (`docs/formats/data-sources.md`, *Library*); the logic is DashCraft's where it had one,
//! adapted to `data::m` values.

use std::fmt::Write as _;
use std::rc::Rc;

use super::misc::{base64_encode, binary, binary_from_text, quote_csv, text_to_binary};
use super::{Builtin, arg, b, conv_err, items, names, native, opt_number, record, text};
use crate::m::convert;
use crate::m::eval::{Interp, values_equal};
use crate::m::lib::table::missing_mode;
use crate::m::value::{Kind, MType, Native, Record, Res, Value, expr_err, fatal};

pub(crate) static FUNCS: &[Builtin] = &[
    b("Number.BitwiseAnd", 2, 2, bitwise_and),
    b("Number.BitwiseOr", 2, 2, bitwise_or),
    b("Number.BitwiseXor", 2, 2, bitwise_xor),
    b("Number.BitwiseNot", 1, 1, bitwise_not),
    b("Number.BitwiseShiftLeft", 2, 2, shift_left),
    b("Number.BitwiseShiftRight", 2, 2, shift_right),
    b("Binary.From", 1, 2, binary_from),
    b("Binary.Combine", 1, 1, binary_combine),
    b("Binary.Range", 2, 3, binary_range),
    b("Binary.Split", 2, 2, binary_split),
    b("Binary.ApproximateLength", 1, 1, approximate_length),
    b("Json.FromValue", 1, 2, json_from_value),
    b("Value.Type", 1, 1, value_type),
    b("Value.ReplaceMetadata", 2, 2, replace_metadata),
    b("Value.NullableEquals", 2, 3, nullable_equals),
    b("Type.Is", 2, 2, type_is),
    b("Type.IsNullable", 1, 1, type_is_nullable),
    b("Type.NonNullable", 1, 1, type_non_nullable),
    b("Type.TableColumn", 2, 2, type_table_column),
    b("Record.ReorderFields", 2, 3, reorder_fields),
    b("Splitter.SplitTextByEachDelimiter", 1, 3, split_by_each_delimiter),
    b("Splitter.SplitTextByRanges", 1, 2, split_by_ranges),
    b(
        "Splitter.SplitTextByRepeatedLengths",
        1,
        2,
        split_by_repeated_lengths,
    ),
    b("Splitter.SplitTextByWhitespace", 0, 1, split_by_whitespace),
    b(
        "Splitter.SplitTextByCharacterTransition",
        2,
        2,
        split_by_character_transition,
    ),
    b("Time.ToRecord", 1, 1, time_to_record),
    b("DateTimeZone.ToRecord", 1, 1, dtz_to_record),
];

// ---------------------------------------------------------------------------------------------
// Number.Bitwise*
// ---------------------------------------------------------------------------------------------

const TWO_POW_63: f64 = 9_223_372_036_854_775_808.0;

/// A whole number of the 64-bit integer range.
fn to_int64(n: f64) -> Res<i64> {
    if n.fract() == 0.0 && (-TWO_POW_63..TWO_POW_63).contains(&n) {
        Ok(n as i64)
    } else {
        expr_err("The number is out of range of a 64 bit integer value.")
    }
}

/// A bitwise operation on two 64-bit integers; null gives null.
fn bitwise(a: &[Value], op: impl Fn(i64, i64) -> i64) -> Res<Value> {
    match (opt_number(arg(a, 0))?, opt_number(arg(a, 1))?) {
        (Some(x), Some(y)) => Ok(Value::num(op(to_int64(x)?, to_int64(y)?) as f64)),
        _ => Ok(Value::Null),
    }
}

fn bitwise_and(_: &mut Interp, a: &[Value]) -> Res<Value> {
    bitwise(a, |x, y| x & y)
}

fn bitwise_or(_: &mut Interp, a: &[Value]) -> Res<Value> {
    bitwise(a, |x, y| x | y)
}

fn bitwise_xor(_: &mut Interp, a: &[Value]) -> Res<Value> {
    bitwise(a, |x, y| x ^ y)
}

fn bitwise_not(_: &mut Interp, a: &[Value]) -> Res<Value> {
    match opt_number(arg(a, 0))? {
        None => Ok(Value::Null),
        Some(n) => Ok(Value::num(!to_int64(n)? as f64)),
    }
}

/// Shift counts are taken modulo 64, as for 64-bit integers in .NET.
fn shift_left(_: &mut Interp, a: &[Value]) -> Res<Value> {
    bitwise(a, |x, y| x.wrapping_shl((y & 63) as u32))
}

fn shift_right(_: &mut Interp, a: &[Value]) -> Res<Value> {
    bitwise(a, |x, y| x.wrapping_shr((y & 63) as u32))
}

// ---------------------------------------------------------------------------------------------
// Binary
// ---------------------------------------------------------------------------------------------

fn check_bytes(interp: &Interp, n: usize) -> Res<()> {
    if n > interp.limits.max_bytes {
        return fatal(crate::Error::LimitExceeded(format!(
            "a binary value is larger than {} bytes",
            interp.limits.max_bytes
        )));
    }
    Ok(())
}

fn out_of_range<T>(name: &str) -> Res<T> {
    expr_err(format!("The '{name}' argument is out of range."))
}

/// A non-negative whole argument named `name` in the out-of-range error; values past the
/// addressable range clamp to it.
fn index_arg(v: &Value, name: &str) -> Res<Option<usize>> {
    match opt_number(v)? {
        None => Ok(None),
        Some(n) if n.fract() != 0.0 || n < 0.0 || n.is_nan() => out_of_range(name),
        Some(n) if n >= usize::MAX as f64 => Ok(Some(usize::MAX)),
        Some(n) => Ok(Some(n as usize)),
    }
}

/// `Binary.From(value, encoding)`: null, a binary as it is, or a text decoded as
/// `Binary.FromText` does.
fn binary_from(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    match arg(a, 0) {
        Value::Null => Ok(Value::Null),
        Value::Binary(_) => Ok(arg(a, 0).clone()),
        Value::Text(_) => binary_from_text(interp, a),
        _ => crate::m::value::err("DataFormat.Error", "We couldn't convert to Binary."),
    }
}

fn binary_combine(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let mut out = Vec::new();
    for v in items(arg(a, 0))? {
        let bytes = binary(&v)?;
        interp.tick((bytes.len() / 4096) as u64 + 1)?;
        out.extend_from_slice(&bytes);
        check_bytes(interp, out.len())?;
    }
    Ok(Value::bytes(out))
}

/// `Binary.Range(binary, offset, count)`: the bytes from `offset` (clamped to the end), at most
/// `count` of them.
fn binary_range(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let bytes = binary(arg(a, 0))?;
    let Some(offset) = index_arg(arg(a, 1), "offset")? else {
        return conv_err(&Value::Null, "Number");
    };
    let count = index_arg(arg(a, 2), "count")?;
    let start = offset.min(bytes.len());
    let end = count.map_or(bytes.len(), |c| start.saturating_add(c).min(bytes.len()));
    Ok(Value::bytes(bytes.get(start..end).unwrap_or_default().to_vec()))
}

fn binary_split(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let bytes = binary(arg(a, 0))?;
    let page = match index_arg(arg(a, 1), "pageSize")? {
        Some(p) if p > 0 => p,
        _ => return out_of_range("pageSize"),
    };
    interp.check_rows(bytes.len().div_ceil(page))?;
    Ok(Value::list(
        bytes.chunks(page).map(|c| Value::bytes(c.to_vec())).collect(),
    ))
}

fn approximate_length(_: &mut Interp, a: &[Value]) -> Res<Value> {
    match arg(a, 0) {
        Value::Null => Ok(Value::Null),
        v => Ok(Value::num(binary(v)?.len() as f64)),
    }
}

// ---------------------------------------------------------------------------------------------
// Json.FromValue
// ---------------------------------------------------------------------------------------------

/// `Json.FromValue(value, encoding)`: the JSON text of a value, encoded (UTF-8 by default).
fn json_from_value(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let mut out = String::new();
    let depth = interp.limits.max_eval_depth;
    write_json(interp, arg(a, 0), &mut out, depth)?;
    text_to_binary(interp, &[Value::text(out), arg(a, 1).clone()])
}

fn write_json(interp: &mut Interp, v: &Value, out: &mut String, depth: usize) -> Res<()> {
    if depth == 0 {
        return fatal(crate::Error::LimitExceeded(
            "Json.FromValue: value nested too deep".into(),
        ));
    }
    interp.tick(1)?;
    check_bytes(interp, out.len())?;
    let culture = convert::CultureInfo::from_name("en-US");
    match v {
        Value::Error(_) => {
            v.clone().check()?;
        }
        Value::Null => out.push_str("null"),
        Value::Logical(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) if !n.f().is_finite() => out.push_str("null"),
        Value::Number(n) if n.f() == 0.0 => out.push('0'),
        Value::Number(n) => out.push_str(&convert::number_to_text(*n, &culture)),
        Value::Text(t) => write_json_string(t, out),
        Value::Date(d) => write_json_string(&json_date(*d), out),
        Value::Time(ms) => write_json_string(&json_time(*ms), out),
        Value::DateTime(ms) => {
            let (d, t) = convert::split_ms(*ms);
            write_json_string(&format!("{}T{}", json_date(d), json_time(t)), out);
        }
        Value::DateTimeZone(ms, offset) => {
            let (d, t) = convert::split_ms(*ms);
            let sign = if *offset < 0 { '-' } else { '+' };
            let o = offset.unsigned_abs();
            write_json_string(
                &format!(
                    "{}T{}{sign}{:02}:{:02}",
                    json_date(d),
                    json_time(t),
                    o / 60,
                    o % 60
                ),
                out,
            );
        }
        Value::Duration(ms) => write_json_string(&xml_duration(*ms), out),
        Value::Binary(_) => write_json_string(&base64_encode(&binary(v)?), out),
        Value::List(l) => {
            out.push('[');
            for (i, x) in l.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json(interp, x, out, depth - 1)?;
            }
            out.push(']');
        }
        Value::Record(r) => {
            out.push('{');
            for (i, (n, x)) in r.names.iter().zip(r.values.iter()).enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json_string(n, out);
                out.push(':');
                write_json(interp, x, out, depth - 1)?;
            }
            out.push('}');
        }
        Value::Table(t) => {
            out.push('[');
            for (i, row) in t.rows.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json(interp, &crate::m::eval::row_record(t, row), out, depth - 1)?;
            }
            out.push(']');
        }
        Value::Function(_) | Value::Type(_) | Value::Remote(_) => {
            return expr_err("We can't convert values of this type to JSON.");
        }
    }
    Ok(())
}

/// A JSON string as Power Query writes it: `"`, `\` and `/` escaped, everything outside
/// printable ASCII as `\uXXXX` (lowercase hex, UTF-16 code units).
fn write_json_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '/' => out.push_str("\\/"),
            ' '..='~' => out.push(c),
            _ => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    let _ = write!(out, "\\u{unit:04x}");
                }
            }
        }
    }
    out.push('"');
}

fn json_date(day: i64) -> String {
    let (y, m, d) = convert::ymd(day);
    format!("{y:04}-{m:02}-{d:02}")
}

/// `HH:mm:ss`, with the fraction of a second when it is not zero.
fn json_time(ms: i64) -> String {
    let mut s = format!(
        "{:02}:{:02}:{:02}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60
    );
    let frac = ms % 1000;
    if frac != 0 {
        let digits = format!("{frac:03}");
        s.push('.');
        s.push_str(digits.trim_end_matches('0'));
    }
    s
}

/// An ISO 8601 duration as .NET's `XmlConvert` writes it (`P1DT2H3M4S`, `-PT0.5S`, `PT0S`).
fn xml_duration(ms: i64) -> String {
    let negative = ms < 0;
    let ms = ms.unsigned_abs();
    let days = ms / 86_400_000;
    let hours = ms / 3_600_000 % 24;
    let minutes = ms / 60_000 % 60;
    let seconds = ms / 1000 % 60;
    let frac = ms % 1000;
    let mut s = String::new();
    if negative {
        s.push('-');
    }
    s.push('P');
    if days != 0 {
        let _ = write!(s, "{days}D");
    }
    if hours != 0 || minutes != 0 || seconds != 0 || frac != 0 || days == 0 {
        s.push('T');
        if hours != 0 {
            let _ = write!(s, "{hours}H");
        }
        if minutes != 0 {
            let _ = write!(s, "{minutes}M");
        }
        if seconds != 0 || frac != 0 || (days == 0 && hours == 0 && minutes == 0) {
            let _ = write!(s, "{seconds}");
            if frac != 0 {
                let digits = format!("{frac:03}");
                s.push('.');
                s.push_str(digits.trim_end_matches('0'));
            }
            s.push('S');
        }
    }
    s
}

// ---------------------------------------------------------------------------------------------
// Values and types
// ---------------------------------------------------------------------------------------------

/// `Value.Type`: a table's own type (its columns), else the primitive type of the value.
fn value_type(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let v = arg(a, 0).clone().check()?;
    Ok(Value::ty(match &v {
        Value::Table(t) => t.ty(),
        Value::Type(_) => MType::prim(Kind::Type),
        other => MType::prim(other.kind()),
    }))
}

/// `Value.ReplaceMetadata(value, meta)`: bicraft keeps no metadata (`Value.Metadata` is empty),
/// so the value comes back as it is; `meta` must be a record or null.
fn replace_metadata(_: &mut Interp, a: &[Value]) -> Res<Value> {
    match arg(a, 1) {
        Value::Null | Value::Record(_) => Ok(arg(a, 0).clone()),
        other => conv_err(other, "Record"),
    }
}

/// `Value.NullableEquals`: null when either value is null, else `Value.Equals`.
fn nullable_equals(_: &mut Interp, a: &[Value]) -> Res<Value> {
    if arg(a, 0).is_null() || arg(a, 1).is_null() {
        return Ok(Value::Null);
    }
    Ok(Value::Logical(values_equal(arg(a, 0), arg(a, 1))))
}

/// A required type argument.
fn type_arg(v: &Value) -> Res<MType> {
    match v {
        Value::Type(t) => Ok((**t).clone()),
        other => conv_err(other, "Type"),
    }
}

fn type_is(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let (x, y) = (type_arg(arg(a, 0))?, type_arg(arg(a, 1))?);
    Ok(Value::Logical(super::table_more::type_compatible(&x, &y)))
}

fn type_is_nullable(_: &mut Interp, a: &[Value]) -> Res<Value> {
    Ok(Value::Logical(type_arg(arg(a, 0))?.nullable))
}

/// `Type.NonNullable`: `type nullable number` → `type number`, `any` → `anynonnull`,
/// `null` → `none`.
fn type_non_nullable(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let mut t = type_arg(arg(a, 0))?;
    t.kind = match t.kind {
        Kind::Any => Kind::AnyNonNull,
        Kind::Null => Kind::None,
        k => k,
    };
    t.nullable = false;
    Ok(Value::ty(t))
}

fn type_table_column(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = type_arg(arg(a, 0))?;
    let name = text(arg(a, 1))?;
    match (&t.kind, &t.fields) {
        (Kind::Table, Some(cols)) => match cols.iter().find(|(n, _)| &**n == name) {
            Some((_, ty)) => Ok(Value::ty(ty.clone())),
            None => expr_err(format!("The column '{name}' of the table wasn't found.")),
        },
        (Kind::Table, None) => expr_err(format!("The column '{name}' of the table wasn't found.")),
        _ => conv_err(arg(a, 0), "table type"),
    }
}

/// `Record.ReorderFields(record, order, missingField)`: the listed fields take the places of
/// the listed fields, in the order given; the others stay where they are.
fn reorder_fields(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let r = record(arg(a, 0))?.clone();
    let order = names(arg(a, 1))?;
    let mode = missing_mode(arg(a, 2))?;
    let mut pairs: Vec<(Rc<str>, Value)> = r.names.iter().cloned().zip(r.values.iter().cloned()).collect();
    let mut listed: Vec<Rc<str>> = Vec::with_capacity(order.len());
    for n in order {
        if r.get(&n).is_none() {
            if mode == 1.0 {
                continue;
            }
            if mode != 2.0 {
                return expr_err(format!("The field '{n}' of the record wasn't found."));
            }
            if !pairs.iter().any(|(p, _)| *p == n) {
                pairs.push((n.clone(), Value::Null));
            }
        }
        if listed.contains(&n) {
            return expr_err(format!("The field '{n}' is listed more than once."));
        }
        listed.push(n);
    }
    let mut queue = listed
        .iter()
        .filter_map(|n| pairs.iter().find(|(p, _)| p == n).cloned())
        .collect::<Vec<_>>()
        .into_iter();
    let reordered: Vec<(Rc<str>, Value)> = pairs
        .iter()
        .map(|p| {
            if listed.contains(&p.0) {
                queue.next().unwrap_or_else(|| p.clone())
            } else {
                p.clone()
            }
        })
        .collect();
    Ok(Value::Record(Rc::new(Record::new(reordered))))
}

// ---------------------------------------------------------------------------------------------
// Splitters
// ---------------------------------------------------------------------------------------------

fn opt_bool(v: &Value) -> Res<bool> {
    match v {
        Value::Null => Ok(false),
        Value::Logical(b) => Ok(*b),
        other => conv_err(other, "Logical"),
    }
}

/// A character test of `Splitter.SplitTextByCharacterTransition`: a list of characters or a
/// function of a one-character text.
#[derive(Debug, Clone)]
pub(crate) enum CharTest {
    Chars(Vec<char>),
    Func(Value),
}

impl CharTest {
    fn from_arg(v: &Value) -> Res<CharTest> {
        match v {
            Value::Function(_) => Ok(CharTest::Func(v.clone())),
            Value::List(_) => {
                let mut chars = Vec::new();
                for item in items(v)? {
                    let mut it = match &item {
                        Value::Text(t) => t.chars(),
                        other => return conv_err(other, "Character"),
                    };
                    match (it.next(), it.next()) {
                        (Some(c), None) => chars.push(c),
                        _ => return conv_err(&item, "Character"),
                    }
                }
                Ok(CharTest::Chars(chars))
            }
            other => conv_err(other, "Function"),
        }
    }

    fn test(&self, interp: &mut Interp, c: char) -> Res<bool> {
        match self {
            CharTest::Chars(chars) => Ok(chars.contains(&c)),
            CharTest::Func(f) => match interp.call(f, vec![Value::text(c.to_string())])? {
                Value::Logical(b) => Ok(b),
                other => conv_err(&other, "Logical"),
            },
        }
    }
}

fn check_delimiter(d: &str) -> Res<()> {
    if d.is_empty() {
        return expr_err("A delimiter must be text at least 1 character in length.");
    }
    Ok(())
}

fn split_by_each_delimiter(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let ds = names(arg(a, 0))?;
    for d in &ds {
        check_delimiter(d)?;
    }
    let csv = quote_csv(arg(a, 1))?;
    Ok(native(Native::SplitByEachDelimiter(
        Rc::new(ds),
        csv,
        opt_bool(arg(a, 2))?,
    )))
}

fn split_by_ranges(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let bad = || expr_err("A range must be a list of an offset and a count, both non-negative integers.");
    let mut ranges = Vec::new();
    for range in items(arg(a, 0))? {
        let pair = items(&range)?;
        let bound = |v: Option<&Value>| -> Option<Option<usize>> {
            match v {
                None | Some(Value::Null) => Some(None),
                Some(Value::Number(n)) if n.f().fract() == 0.0 && n.f() >= 0.0 => {
                    Some(Some(if n.f() >= usize::MAX as f64 {
                        usize::MAX
                    } else {
                        n.f() as usize
                    }))
                }
                Some(_) => None,
            }
        };
        let (Some(Some(offset)), Some(count)) = (bound(pair.first()), bound(pair.get(1))) else {
            return bad();
        };
        if pair.len() > 2 {
            return bad();
        }
        ranges.push((offset, count.unwrap_or(usize::MAX)));
    }
    Ok(native(Native::SplitByRanges(
        Rc::new(ranges),
        opt_bool(arg(a, 1))?,
    )))
}

fn split_by_repeated_lengths(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let length = match opt_number(arg(a, 0))? {
        Some(n) if n >= 1.0 && n.fract() == 0.0 => {
            if n >= usize::MAX as f64 {
                usize::MAX
            } else {
                n as usize
            }
        }
        _ => return expr_err("A length must be greater than zero."),
    };
    Ok(native(Native::SplitByRepeatedLengths(
        length,
        opt_bool(arg(a, 1))?,
    )))
}

fn split_by_whitespace(_: &mut Interp, a: &[Value]) -> Res<Value> {
    Ok(native(Native::SplitByWhitespace(quote_csv(arg(a, 0))?)))
}

fn split_by_character_transition(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let before = CharTest::from_arg(arg(a, 0))?;
    let after = CharTest::from_arg(arg(a, 1))?;
    Ok(native(Native::SplitByTransition(Rc::new((before, after)))))
}

/// Splits `text` at the delimiters (`each`: each one once, in order; else any of them at every
/// place) outside quotes. Quotes are removed, `""` inside quotes is one quote, an unclosed
/// quote runs to the end of the text.
fn split_quoted(text: &str, delimiters: &[Rc<str>], csv: bool, each: bool) -> Vec<String> {
    let mut parts = Vec::new();
    let mut part = String::new();
    let mut in_quotes = false;
    let mut next = 0usize;
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        let after = rest.get(c.len_utf8()..).unwrap_or("");
        if in_quotes {
            if c == '"' {
                if let Some(r) = after.strip_prefix('"') {
                    part.push('"');
                    rest = r;
                } else {
                    in_quotes = false;
                    rest = after;
                }
            } else {
                part.push(c);
                rest = after;
            }
            continue;
        }
        let hit = if each {
            delimiters.get(next).filter(|d| rest.starts_with(&***d))
        } else {
            delimiters.iter().find(|d| rest.starts_with(&***d))
        };
        if let Some(d) = hit {
            parts.push(std::mem::take(&mut part));
            next += 1;
            rest = rest.get(d.len()..).unwrap_or("");
            continue;
        }
        if c == '"' && csv {
            in_quotes = true;
            rest = after;
            continue;
        }
        part.push(c);
        rest = after;
    }
    parts.push(part);
    parts
}

/// Splits at runs of white space outside quotes; white space at the start or end gives an empty
/// first or last part.
fn split_whitespace(text: &str, csv: bool) -> Vec<String> {
    let mut parts = Vec::new();
    let mut part = String::new();
    let mut in_quotes = false;
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        let after = rest.get(c.len_utf8()..).unwrap_or("");
        if in_quotes {
            if c == '"' {
                if let Some(r) = after.strip_prefix('"') {
                    part.push('"');
                    rest = r;
                } else {
                    in_quotes = false;
                    rest = after;
                }
            } else {
                part.push(c);
                rest = after;
            }
            continue;
        }
        if c.is_whitespace() {
            parts.push(std::mem::take(&mut part));
            rest = rest.trim_start_matches(char::is_whitespace);
            continue;
        }
        if c == '"' && csv {
            in_quotes = true;
            rest = after;
            continue;
        }
        part.push(c);
        rest = after;
    }
    parts.push(part);
    parts
}

fn rev(s: &str) -> String {
    s.chars().rev().collect()
}

/// `split` on the reversed text when `from_end`, the parts reversed back.
fn from_end(text: &str, from_end: bool, split: impl Fn(&str) -> Vec<String>) -> Vec<String> {
    if !from_end {
        return split(text);
    }
    let mut parts: Vec<String> = split(&rev(text)).iter().map(|p| rev(p)).collect();
    parts.reverse();
    parts
}

/// Runs one of the splitters above on a text.
pub(crate) fn call_splitter(interp: &mut Interp, n: &Native, t: &str) -> Res<Vec<String>> {
    interp.tick((t.len() / 64) as u64 + 1)?;
    Ok(match n {
        Native::SplitByEachDelimiter(ds, csv, end) => {
            let ds_rev: Vec<Rc<str>> = ds.iter().map(|d| Rc::from(rev(d))).collect();
            let ds = if *end { ds_rev } else { ds.to_vec() };
            from_end(t, *end, |s| split_quoted(s, &ds, *csv, true))
        }
        Native::SplitByRanges(ranges, end) => from_end(t, *end, |s| {
            let chars: Vec<char> = s.chars().collect();
            ranges
                .iter()
                .map(|&(offset, count)| {
                    let start = offset.min(chars.len());
                    let stop = start.saturating_add(count).min(chars.len());
                    chars.get(start..stop).unwrap_or_default().iter().collect()
                })
                .collect()
        }),
        Native::SplitByRepeatedLengths(len, end) => from_end(t, *end, |s| {
            let chars: Vec<char> = s.chars().collect();
            chars.chunks((*len).max(1)).map(|c| c.iter().collect()).collect()
        }),
        Native::SplitByWhitespace(csv) => split_whitespace(t, *csv),
        Native::SplitByTransition(tests) => {
            let (before, after) = &**tests;
            let mut parts = Vec::new();
            let mut part = String::new();
            let mut previous: Option<char> = None;
            for c in t.chars() {
                interp.tick(1)?;
                if let Some(p) = previous
                    && before.test(interp, p)?
                    && after.test(interp, c)?
                {
                    parts.push(std::mem::take(&mut part));
                }
                part.push(c);
                previous = Some(c);
            }
            parts.push(part);
            parts
        }
        _ => vec![t.to_string()],
    })
}

// ---------------------------------------------------------------------------------------------
// Records of times
// ---------------------------------------------------------------------------------------------

fn time_fields(ms: i64) -> [(&'static str, Value); 3] {
    [
        ("Hour", Value::num((ms / 3_600_000) as f64)),
        ("Minute", Value::num((ms / 60_000 % 60) as f64)),
        ("Second", Value::num((ms % 60_000) as f64 / 1000.0)),
    ]
}

/// `Time.ToRecord(#time(11, 56, 2))` is `[Hour = 11, Minute = 56, Second = 2]`.
fn time_to_record(_: &mut Interp, a: &[Value]) -> Res<Value> {
    match arg(a, 0) {
        Value::Null => Ok(Value::Null),
        Value::Time(ms) => Ok(Value::record(time_fields(*ms).to_vec())),
        other => conv_err(other, "Time"),
    }
}

/// `DateTimeZone.ToRecord`: `Year`, `Month`, `Day`, `Hour`, `Minute`, `Second`, `ZoneHours`,
/// `ZoneMinutes`.
fn dtz_to_record(_: &mut Interp, a: &[Value]) -> Res<Value> {
    match arg(a, 0) {
        Value::Null => Ok(Value::Null),
        Value::DateTimeZone(ms, offset) => {
            let (d, t) = convert::split_ms(*ms);
            let (y, m, dd) = convert::ymd(d);
            let mut fields = vec![
                ("Year", Value::num(y as f64)),
                ("Month", Value::num(m as f64)),
                ("Day", Value::num(dd as f64)),
            ];
            fields.extend(time_fields(t));
            fields.push(("ZoneHours", Value::num(f64::from(offset / 60))));
            fields.push(("ZoneMinutes", Value::num(f64::from(offset % 60))));
            Ok(Value::record(fields))
        }
        other => conv_err(other, "DateTimeZone"),
    }
}
