//! `Record.*`, `Value.*`, `Binary.*`, `Text.FromBinary`, `Lines.*`,
//! `Splitter.*`, `Combiner.*`, `Replacer.*`, `Comparer.*`, `Error.Record`.

use std::rc::Rc;

use super::{
    Builtin, arg, b, conv_err, count, culture_arg, decimal_precision, func, items, list, mtype, names,
    native, opt_number, opt_text, record, text,
};
use crate::m::ast::BinOp;
use crate::m::convert;
use crate::m::eval::{Interp, conforms, values_equal};
use crate::m::lib::table::make;
use crate::m::value::{
    Column, Kind, MError, MType, Native, Num, Record, Res, Value, err, expr_err, fatal, unsupported,
};

pub(crate) static FUNCS: &[Builtin] = &[
    b("Record.Field", 2, 2, field),
    b("Record.FieldOrDefault", 2, 3, field_or_default),
    b("Record.FieldNames", 1, 1, field_names),
    b("Record.FieldValues", 1, 1, field_values),
    b("Record.ToList", 1, 1, field_values),
    b("Record.FieldCount", 1, 1, field_count),
    b("Record.HasFields", 2, 2, has_fields),
    b("Record.AddField", 3, 4, add_field),
    b("Record.RemoveFields", 2, 3, remove_fields),
    b("Record.RenameFields", 2, 3, rename_fields),
    b("Record.SelectFields", 2, 3, select_fields),
    b("Record.TransformFields", 2, 3, transform_fields),
    b("Record.Combine", 1, 1, record_combine),
    b("Record.ToTable", 1, 1, record_to_table),
    b("Record.FromList", 2, 2, record_from_list),
    b("Record.FromTable", 1, 1, record_from_table),
    b("Value.Is", 2, 2, value_is),
    b("Value.As", 2, 2, value_as),
    b("Value.ReplaceType", 2, 2, replace_type),
    b("Value.Metadata", 1, 1, metadata),
    b("Value.RemoveMetadata", 1, 2, first_arg),
    b("Value.FromText", 1, 2, value_from_text),
    b("Value.Add", 2, 3, value_add),
    b("Value.Subtract", 2, 3, value_subtract),
    b("Value.Multiply", 2, 3, value_multiply),
    b("Value.Divide", 2, 3, value_divide),
    b("Diagnostics.Trace", 3, 4, trace),
    b("Error.Record", 1, 3, error_record),
    b("Function.Invoke", 2, 2, invoke),
    b("Expression.Evaluate", 1, 2, evaluate),
    b("Comparer.FromCulture", 1, 2, comparer_from_culture),
    b("Comparer.Equals", 3, 3, comparer_equals),
    b("Binary.FromText", 1, 2, binary_from_text),
    b("Binary.ToText", 1, 2, binary_to_text),
    b("Binary.FromList", 1, 1, binary_from_list),
    b("Binary.ToList", 1, 1, binary_to_list),
    b("Binary.Length", 1, 1, binary_length),
    b("Binary.Buffer", 1, 1, first_arg),
    b("Binary.Decompress", 2, 2, decompress),
    b("#binary", 1, 1, hash_binary),
    b("Text.FromBinary", 1, 2, text_from_binary),
    b("Text.ToBinary", 1, 3, text_to_binary),
    b("Lines.FromText", 1, 3, lines_from_text),
    b("Lines.FromBinary", 1, 4, lines_from_binary),
    b("Lines.ToText", 1, 2, lines_to_text),
    b("Splitter.SplitTextByDelimiter", 1, 2, split_by_delimiter),
    b("Splitter.SplitTextByAnyDelimiter", 1, 3, split_by_any_delimiter),
    b("Splitter.SplitTextByPositions", 1, 2, split_by_positions),
    b("Splitter.SplitTextByLengths", 1, 2, split_by_lengths),
    b("Splitter.SplitByNothing", 0, 0, split_by_nothing),
    b("Combiner.CombineTextByDelimiter", 1, 2, combine_by_delimiter),
    b("Replacer.ReplaceText", 3, 3, replace_text),
    b("Replacer.ReplaceValue", 3, 3, replace_value),
];

fn first_arg(_: &mut Interp, a: &[Value]) -> Res<Value> {
    Ok(arg(a, 0).clone())
}

// ---------------------------------------------------------------------------
// Records.

fn field(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let r = arg(a, 0).clone();
    interp.field(&r, text(arg(a, 1))?, false)
}

fn field_or_default(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let r = record(arg(a, 0))?;
    match r.get(text(arg(a, 1))?) {
        Some(v) => v.clone().check(),
        None => Ok(arg(a, 2).clone()),
    }
}

fn field_names(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let r = record(arg(a, 0))?;
    Ok(Value::list(
        r.names.iter().map(|n| Value::Text(n.clone())).collect(),
    ))
}

fn field_values(_: &mut Interp, a: &[Value]) -> Res<Value> {
    Ok(Value::list(record(arg(a, 0))?.values.clone()))
}

fn field_count(_: &mut Interp, a: &[Value]) -> Res<Value> {
    Ok(Value::num(record(arg(a, 0))?.names.len() as f64))
}

fn has_fields(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let r = record(arg(a, 0))?;
    Ok(Value::Logical(
        names(arg(a, 1))?.iter().all(|n| r.index(n).is_some()),
    ))
}

fn fields_of(r: &Record) -> Vec<(Rc<str>, Value)> {
    r.names.iter().cloned().zip(r.values.iter().cloned()).collect()
}

fn add_field(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let r = record(arg(a, 0))?;
    let n = text(arg(a, 1))?;
    if r.index(n).is_some() {
        return expr_err(format!("The field '{n}' already exists in the record."));
    }
    let mut f = fields_of(r);
    f.push((n.into(), arg(a, 2).clone()));
    Ok(Value::Record(Rc::new(Record::new(f))))
}

fn missing(v: &Value) -> Res<f64> {
    Ok(opt_number(v)?.unwrap_or(0.0))
}

fn remove_fields(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let r = record(arg(a, 0))?;
    let drop = names(arg(a, 1))?;
    if missing(arg(a, 2))? == 0.0
        && let Some(n) = drop.iter().find(|n| r.index(n).is_none())
    {
        return expr_err(format!("The field '{n}' of the record wasn't found."));
    }
    let f = fields_of(r)
        .into_iter()
        .filter(|(n, _)| !drop.contains(n))
        .collect();
    Ok(Value::Record(Rc::new(Record::new(f))))
}

fn rename_fields(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let r = record(arg(a, 0))?;
    let mut f = fields_of(r);
    let pairs = items(arg(a, 1))?;
    let pairs: Vec<Vec<Value>> = if pairs.first().is_some_and(|p| matches!(p, Value::List(_))) {
        pairs.iter().map(items).collect::<Res<_>>()?
    } else {
        vec![pairs]
    };
    let strict = missing(arg(a, 2))? == 0.0;
    for p in pairs {
        let old = text(p.first().unwrap_or(&Value::Null))?;
        let new = text(p.get(1).unwrap_or(&Value::Null))?;
        match f.iter_mut().find(|(n, _)| &**n == old) {
            Some(slot) => slot.0 = new.into(),
            None if strict => return expr_err(format!("The field '{old}' of the record wasn't found.")),
            None => {}
        }
    }
    Ok(Value::Record(Rc::new(Record::new(f))))
}

fn select_fields(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let r = record(arg(a, 0))?;
    let mode = missing(arg(a, 2))?;
    let mut out = Vec::new();
    for n in names(arg(a, 1))? {
        match r.get(&n) {
            Some(v) => out.push((n, v.clone())),
            None if mode == 2.0 => out.push((n, Value::Null)),
            None if mode == 1.0 => {}
            None => return expr_err(format!("The field '{n}' of the record wasn't found.")),
        }
    }
    Ok(Value::Record(Rc::new(Record::new(out))))
}

fn transform_fields(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let r = record(arg(a, 0))?.clone();
    let mut f = fields_of(&r);
    let ops = items(arg(a, 1))?;
    let ops: Vec<Vec<Value>> = if ops.first().is_some_and(|p| matches!(p, Value::List(_))) {
        ops.iter().map(items).collect::<Res<_>>()?
    } else {
        vec![ops]
    };
    for p in ops {
        let n = text(p.first().unwrap_or(&Value::Null))?;
        let fun = func(p.get(1).unwrap_or(&Value::Null))?.clone();
        match f.iter_mut().find(|(m, _)| &**m == n) {
            Some(slot) => {
                let v = slot.1.clone().check()?;
                slot.1 = Value::store(interp.call(&fun, vec![v]))?;
            }
            None => return expr_err(format!("The field '{n}' of the record wasn't found.")),
        }
    }
    Ok(Value::Record(Rc::new(Record::new(f))))
}

fn record_combine(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let mut acc = Value::Record(Rc::new(Record::new(Vec::new())));
    for r in items(arg(a, 0))? {
        acc = interp.binary(BinOp::Concat, acc, r)?;
    }
    Ok(acc)
}

fn record_to_table(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let r = record(arg(a, 0))?;
    let rows = r
        .names
        .iter()
        .zip(r.values.iter())
        .map(|(n, v)| vec![Value::Text(n.clone()), v.clone()])
        .collect();
    let cols = vec![
        Column {
            name: "Name".into(),
            ty: MType::prim(Kind::Text),
        },
        Column {
            name: "Value".into(),
            ty: MType::any(),
        },
    ];
    make(interp, cols, rows)
}

fn record_from_list(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let vals = list(arg(a, 0))?.clone();
    let ns = match arg(a, 1) {
        Value::Type(t) => t
            .fields
            .as_ref()
            .map(|f| f.iter().map(|(n, _)| n.clone()).collect())
            .unwrap_or_default(),
        v => names(v)?,
    };
    if ns.len() != vals.len() {
        return expr_err("The number of field names and values must match.");
    }
    Ok(Value::Record(Rc::new(Record::new(
        ns.into_iter().zip(vals.iter().cloned()).collect(),
    ))))
}

fn record_from_table(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = super::tbl(arg(a, 0))?;
    let (Some(n), Some(v)) = (t.index("Name"), t.index("Value")) else {
        return expr_err("The table must have Name and Value columns.");
    };
    let mut f = Vec::with_capacity(t.rows.len());
    for r in &t.rows {
        let name = text(r.get(n).unwrap_or(&Value::Null))?;
        f.push((Rc::from(name), r.get(v).cloned().unwrap_or(Value::Null)));
    }
    Ok(Value::Record(Rc::new(Record::new(f))))
}

// ---------------------------------------------------------------------------
// Values.

fn value_is(_: &mut Interp, a: &[Value]) -> Res<Value> {
    Ok(Value::Logical(conforms(arg(a, 0), &mtype(arg(a, 1))?)))
}

fn value_as(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = mtype(arg(a, 1))?;
    if conforms(arg(a, 0), &t) {
        Ok(arg(a, 0).clone())
    } else {
        conv_err(arg(a, 0), &crate::m::eval::title(t.name()))
    }
}

/// `Value.ReplaceType`: for tables, the new type's columns rename and retype by position.
fn replace_type(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = mtype(arg(a, 1))?;
    match (arg(a, 0), &t.fields) {
        (Value::Table(tb), Some(f)) if t.kind == Kind::Table => {
            if f.len() != tb.columns.len() {
                return expr_err("The number of columns in the type doesn't match the table.");
            }
            let cols = f
                .iter()
                .map(|(n, ty)| Column {
                    name: n.clone(),
                    ty: ty.clone(),
                })
                .collect();
            make(interp, cols, tb.rows.clone())
        }
        (v, _) => Ok(v.clone()),
    }
}

fn metadata(_: &mut Interp, _: &[Value]) -> Res<Value> {
    Ok(Value::Record(Rc::new(Record::new(Vec::new()))))
}

fn value_from_text(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let c = culture_arg(interp, arg(a, 1))?;
    let Some(t) = opt_text(arg(a, 0))? else {
        return Ok(arg(a, 0).clone());
    };
    if let Some(n) = convert::number_from_text(t, &c)
        && !t.trim().ends_with('%')
    {
        return Ok(Value::Number(n));
    }
    if let Some(b) = crate::value::parse_bool(t.trim()) {
        return Ok(Value::Logical(b));
    }
    if t.trim().is_empty() {
        return Ok(Value::Null);
    }
    if let Some((ms, off)) = convert::datetime_from_text(t, &c) {
        return Ok(match off {
            Some(o) => Value::DateTimeZone(ms, o),
            None if ms.rem_euclid(crate::m::eval::MS_PER_DAY) == 0 && !t.contains(':') => {
                Value::Date(convert::split_ms(ms).0)
            }
            None => Value::DateTime(ms),
        });
    }
    if let Some(ms) = convert::time_from_text(t) {
        return Ok(Value::Time(ms));
    }
    Ok(Value::text(t))
}

/// `Value.Add` and the like: with `Precision.Decimal`, numbers compute as decimals (doubles
/// converted to 15 digits; an overflow gives an infinity, as Desktop does); otherwise the
/// operator.
fn value_arith(interp: &mut Interp, a: &[Value], op: BinOp) -> Res<Value> {
    if let (Value::Number(x), Value::Number(y)) = (arg(a, 0), arg(a, 1))
        && decimal_precision(arg(a, 2))?
    {
        return decimal_op(*x, *y, op);
    }
    interp.binary(op, arg(a, 0).clone(), arg(a, 1).clone())
}

/// One decimal operation on two numbers (see [`value_arith`]).
pub(crate) fn decimal_op(x: Num, y: Num, op: BinOp) -> Res<Value> {
    use crate::m::decimal::DecError;
    let (Some(dx), Some(dy)) = (x.dec(), y.dec()) else {
        return expr_err("The number is out of the range of a decimal.");
    };
    let r = match op {
        BinOp::Add => dx.add(dy),
        BinOp::Sub => dx.sub(dy),
        BinOp::Mul => dx.mul(dy),
        _ => dx.div(dy),
    };
    Ok(match r {
        Ok(d) => Value::dec(d),
        Err(DecError::DivideByZero) => Value::num(dx.to_f64() / 0.0),
        Err(DecError::Overflow) => {
            let positive = match op {
                BinOp::Mul | BinOp::Div => dx.is_negative() == dy.is_negative(),
                BinOp::Sub => !dx.is_negative(),
                _ => !dx.is_negative(),
            };
            Value::num(if positive {
                f64::INFINITY
            } else {
                f64::NEG_INFINITY
            })
        }
    })
}

fn value_add(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    value_arith(interp, a, BinOp::Add)
}

fn value_subtract(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    value_arith(interp, a, BinOp::Sub)
}

fn value_multiply(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    value_arith(interp, a, BinOp::Mul)
}

fn value_divide(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    value_arith(interp, a, BinOp::Div)
}

fn trace(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let v = arg(a, 2).clone();
    // With a delayed value (a function), the trace returns its result.
    if matches!(arg(a, 3), Value::Logical(true)) {
        return interp.call(&v, vec![]);
    }
    Ok(v)
}

fn error_record(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let reason = text(arg(a, 0))?;
    let message = opt_text(arg(a, 1))?.unwrap_or("");
    Ok(crate::m::eval::error_record(&MError {
        reason: reason.into(),
        message: message.into(),
        detail: arg(a, 2).clone(),
    }))
}

fn invoke(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let f = func(arg(a, 0))?.clone();
    let args = items(arg(a, 1))?;
    interp.call(&f, args)
}

fn evaluate(_: &mut Interp, _: &[Value]) -> Res<Value> {
    unsupported("`Expression.Evaluate`")
}

fn comparer_from_culture(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let ci = matches!(arg(a, 1), Value::Logical(true));
    Ok(native(Native::Comparer { ignore_case: ci }))
}

fn comparer_equals(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let r = interp.call(arg(a, 0), vec![arg(a, 1).clone(), arg(a, 2).clone()])?;
    Ok(Value::Logical(matches!(r, Value::Number(n) if n.f() == 0.0)))
}

// ---------------------------------------------------------------------------
// Binary.

pub(crate) fn binary(v: &Value) -> Res<Rc<Vec<u8>>> {
    match v {
        Value::Binary(b) => b.get(),
        other => conv_err(other, "Binary"),
    }
}

fn b64_val(c: u8) -> Option<u32> {
    Some(u32::from(match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' | b'-' => 62,
        b'/' | b'_' => 63,
        _ => return None,
    }))
}

pub(crate) fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let mut acc = 0u32;
    let mut bits = 0u32;
    for c in s.bytes() {
        if c.is_ascii_whitespace() {
            continue;
        }
        if c == b'=' {
            break;
        }
        acc = (acc << 6) | b64_val(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

pub(crate) fn base64_encode(b: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(b.len().div_ceil(3) * 4);
    for chunk in b.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, x)| acc | (u32::from(*x) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                let idx = ((n >> (18 - 6 * i)) & 63) as usize;
                s.push(char::from(T.get(idx).copied().unwrap_or(b'A')));
            } else {
                s.push('=');
            }
        }
    }
    s
}

pub(crate) fn binary_from_text(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let Some(t) = opt_text(arg(a, 0))? else {
        return Ok(Value::Null);
    };
    interp.tick((t.len() / 64) as u64)?;
    let hex = opt_number(arg(a, 1))? == Some(1.0);
    let bytes = if hex {
        let clean: Vec<u8> = t.bytes().filter(|c| !c.is_ascii_whitespace()).collect();
        if !clean.len().is_multiple_of(2) {
            return err("DataFormat.Error", "The hex text has an odd length.");
        }
        let mut out = Vec::with_capacity(clean.len() / 2);
        for pair in clean.chunks(2) {
            let s = std::str::from_utf8(pair).unwrap_or("");
            match u8::from_str_radix(s, 16) {
                Ok(b) => out.push(b),
                Err(_) => return err("DataFormat.Error", "The text is not valid hex."),
            }
        }
        out
    } else {
        match base64_decode(t) {
            Some(b) => b,
            None => return err("DataFormat.Error", "The text is not valid base64."),
        }
    };
    Ok(Value::bytes(bytes))
}

fn binary_to_text(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let b = binary(arg(a, 0))?;
    if opt_number(arg(a, 1))? == Some(1.0) {
        return Ok(Value::text(
            b.iter().map(|x| format!("{x:02x}")).collect::<String>(),
        ));
    }
    Ok(Value::text(base64_encode(&b)))
}

fn binary_from_list(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let mut out = Vec::new();
    for v in items(arg(a, 0))? {
        let n = super::int(&v)?;
        match u8::try_from(n) {
            Ok(b) => out.push(b),
            Err(_) => return expr_err(format!("{n} is not a byte value.")),
        }
    }
    Ok(Value::bytes(out))
}

fn binary_to_list(_: &mut Interp, a: &[Value]) -> Res<Value> {
    Ok(Value::list(
        binary(arg(a, 0))?
            .iter()
            .map(|b| Value::num(f64::from(*b)))
            .collect(),
    ))
}

fn binary_length(_: &mut Interp, a: &[Value]) -> Res<Value> {
    Ok(Value::num(binary(arg(a, 0))?.len() as f64))
}

fn hash_binary(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    match arg(a, 0) {
        Value::Text(_) => binary_from_text(interp, &[arg(a, 0).clone(), Value::Null]),
        Value::List(_) => binary_from_list(interp, a),
        other => conv_err(other, "List"),
    }
}

fn decompress(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let data = match arg(a, 0) {
        Value::Null => return Ok(Value::Null),
        v => binary(v)?,
    };
    let data: &[u8] = &data;
    let kind = opt_number(arg(a, 1))?.unwrap_or(-1.0);
    let max = interp.limits.max_bytes;
    let raw: &[u8] = if kind == 0.0 {
        // GZip: skip the header (RFC 1952) and the 8-byte trailer.
        gzip_body(data).map_or_else(|| err("DataFormat.Error", "The data is not valid gzip."), Ok)?
    } else if kind == 1.0 {
        data
    } else {
        return unsupported("a `Binary.Decompress` compression other than Deflate or GZip");
    };
    interp.tick((data.len() / 64) as u64)?;
    match miniz_oxide::inflate::decompress_to_vec_with_limit(raw, max) {
        Ok(v) => Ok(Value::bytes(v)),
        Err(e) if matches!(e.status, miniz_oxide::inflate::TINFLStatus::HasMoreOutput) => fatal(
            crate::Error::LimitExceeded(format!("decompressed data is larger than {max} bytes")),
        ),
        Err(_) => err("DataFormat.Error", "The data could not be decompressed."),
    }
}

fn gzip_body(d: &[u8]) -> Option<&[u8]> {
    if d.get(0..3)? != [0x1f, 0x8b, 8] {
        return None;
    }
    let flags = *d.get(3)?;
    let mut p = 10usize;
    if flags & 4 != 0 {
        let xlen = usize::from(u16::from_le_bytes([*d.get(p)?, *d.get(p + 1)?]));
        p = p.checked_add(2 + xlen)?;
    }
    for bit in [8u8, 16] {
        if flags & bit != 0 {
            while *d.get(p)? != 0 {
                p += 1;
            }
            p += 1;
        }
    }
    if flags & 2 != 0 {
        p += 2;
    }
    d.get(p..d.len().checked_sub(8)?)
}

/// Decodes bytes with a code page (65001 UTF-8, 1200/1201 UTF-16, 1252, 28591, 20127);
/// a BOM wins over the code page.
pub(crate) fn decode(bytes: &[u8], code_page: Option<f64>) -> Res<String> {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return Ok(String::from_utf8_lossy(rest).into_owned());
    }
    let utf16 = |b: &[u8], le: bool| -> String {
        let units: Vec<u16> = b
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| {
                if le {
                    u16::from_le_bytes(*pair)
                } else {
                    u16::from_be_bytes(*pair)
                }
            })
            .collect();
        String::from_utf16_lossy(&units)
    };
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return Ok(utf16(rest, true));
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return Ok(utf16(rest, false));
    }
    match code_page.map(|c| c as i64) {
        None | Some(65001) => Ok(String::from_utf8_lossy(bytes).into_owned()),
        Some(1200) => Ok(utf16(bytes, true)),
        Some(1201) => Ok(utf16(bytes, false)),
        Some(1252) => Ok(bytes.iter().map(|&b| cp1252(b)).collect()),
        Some(28591) => Ok(bytes.iter().map(|&b| char::from(b)).collect()),
        Some(20127) => Ok(bytes
            .iter()
            .map(|&b| if b < 128 { char::from(b) } else { '?' })
            .collect()),
        Some(other) => unsupported(format!("text encoding (code page) {other}")),
    }
}

fn cp1252(b: u8) -> char {
    const HIGH: [u16; 32] = [
        0x20AC, 0x81, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152,
        0x8D, 0x017D, 0x8F, 0x90, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122,
        0x0161, 0x203A, 0x0153, 0x9D, 0x017E, 0x0178,
    ];
    if (0x80..0xA0).contains(&b) {
        HIGH.get(usize::from(b - 0x80))
            .and_then(|&u| char::from_u32(u32::from(u)))
            .unwrap_or('?')
    } else {
        char::from(b)
    }
}

fn text_from_binary(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    match arg(a, 0) {
        Value::Null => Ok(Value::Null),
        v => {
            let b = binary(v)?;
            let s = decode(&b, opt_number(arg(a, 1))?)?;
            interp.check_text(s.len())?;
            Ok(Value::text(s))
        }
    }
}

pub(crate) fn text_to_binary(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let Some(t) = opt_text(arg(a, 0))? else {
        return Ok(Value::Null);
    };
    let bytes = match opt_number(arg(a, 1))?.map(|c| c as i64) {
        None | Some(65001) => t.as_bytes().to_vec(),
        Some(1200) => t.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        Some(1201) => t.encode_utf16().flat_map(u16::to_be_bytes).collect(),
        Some(other) => return unsupported(format!("text encoding (code page) {other}")),
    };
    Ok(Value::bytes(bytes))
}

fn split_lines(t: &str) -> Vec<Value> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = t.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push(Value::text(std::mem::take(&mut cur)));
            }
            '\n' => out.push(Value::text(std::mem::take(&mut cur))),
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(Value::text(cur));
    }
    out
}

fn lines_from_text(_: &mut Interp, a: &[Value]) -> Res<Value> {
    Ok(Value::list(split_lines(text(arg(a, 0))?)))
}

fn lines_from_binary(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let s = decode(&binary(arg(a, 0))?, opt_number(arg(a, 3))?)?;
    Ok(Value::list(split_lines(&s)))
}

fn lines_to_text(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let sep = opt_text(arg(a, 1))?.unwrap_or("\r\n");
    let mut out = String::new();
    for l in items(arg(a, 0))? {
        out.push_str(text(&l)?);
        out.push_str(sep);
    }
    Ok(Value::text(out))
}

// ---------------------------------------------------------------------------
// Splitters, combiners, replacers.

pub(crate) fn quote_csv(v: &Value) -> Res<bool> {
    Ok(opt_number(v)?.unwrap_or(1.0) == 1.0)
}

fn split_by_delimiter(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let d = text(arg(a, 0))?;
    Ok(native(Native::SplitByDelimiter(d.into(), quote_csv(arg(a, 1))?)))
}

fn split_by_any_delimiter(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let ds = names(arg(a, 0))?;
    Ok(native(Native::SplitByAnyDelimiter(
        Rc::new(ds),
        quote_csv(arg(a, 1))?,
    )))
}

fn positions(v: &Value) -> Res<Vec<usize>> {
    items(v)?.iter().map(count).collect()
}

fn split_by_positions(_: &mut Interp, a: &[Value]) -> Res<Value> {
    Ok(native(Native::SplitByPositions(Rc::new(positions(arg(a, 0))?))))
}

fn split_by_lengths(_: &mut Interp, a: &[Value]) -> Res<Value> {
    Ok(native(Native::SplitByLengths(Rc::new(positions(arg(a, 0))?))))
}

fn split_by_nothing(_: &mut Interp, _: &[Value]) -> Res<Value> {
    Ok(native(Native::SplitByNothing))
}

fn combine_by_delimiter(_: &mut Interp, a: &[Value]) -> Res<Value> {
    Ok(native(Native::CombineByDelimiter(text(arg(a, 0))?.into())))
}

fn replace_text(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    match arg(a, 0) {
        Value::Null => Ok(Value::Null),
        Value::Text(t) => {
            let old = text(arg(a, 1))?;
            let new = text(arg(a, 2))?;
            super::text::replace_text(interp, t, old, new)
        }
        other => conv_err(other, "Text"),
    }
}

fn replace_value(_: &mut Interp, a: &[Value]) -> Res<Value> {
    if values_equal(arg(a, 0), arg(a, 1)) {
        Ok(arg(a, 2).clone())
    } else {
        Ok(arg(a, 0).clone())
    }
}

/// Splits on any of `delims`; with `csv`, a field starting with `"` runs to its
/// closing quote (`""` is a quote).
pub(crate) fn split_text(t: &str, delims: &[&str], csv: bool) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut rest = t;
    let mut at_field_start = true;
    while !rest.is_empty() {
        if csv && at_field_start && rest.starts_with('"') {
            let mut body = rest.get(1..).unwrap_or("");
            loop {
                match body.find('"') {
                    None => {
                        cur.push_str(body);
                        body = "";
                        break;
                    }
                    Some(i) => {
                        cur.push_str(body.get(..i).unwrap_or(""));
                        let after = body.get(i + 1..).unwrap_or("");
                        if let Some(r) = after.strip_prefix('"') {
                            cur.push('"');
                            body = r;
                        } else {
                            body = after;
                            break;
                        }
                    }
                }
            }
            rest = body;
            at_field_start = false;
            continue;
        }
        if let Some(d) = delims.iter().find(|d| !d.is_empty() && rest.starts_with(**d)) {
            out.push(std::mem::take(&mut cur));
            rest = rest.get(d.len()..).unwrap_or("");
            at_field_start = true;
            continue;
        }
        let mut chars = rest.chars();
        if let Some(c) = chars.next() {
            cur.push(c);
        }
        rest = chars.as_str();
        at_field_start = false;
    }
    out.push(cur);
    out
}

pub(crate) fn call_native(interp: &mut Interp, n: &Native, args: Vec<Value>) -> Res<Value> {
    let x = args.into_iter().next().unwrap_or(Value::Null);
    match n {
        Native::Comparer { ignore_case } => {
            // As a two-argument comparer: (x, y) → -1, 0, 1.
            unsupported(format!(
                "calling a comparer function directly (ignore case: {ignore_case})"
            ))
        }
        Native::SplitByNothing => Ok(Value::list(vec![x])),
        Native::CombineByDelimiter(d) => {
            let mut out = String::new();
            for (i, v) in items(&x)?.iter().enumerate() {
                if i > 0 {
                    out.push_str(d);
                }
                match v {
                    Value::Null => {}
                    Value::Text(t) => out.push_str(t),
                    other => return conv_err(other, "Text"),
                }
                interp.check_text(out.len())?;
            }
            Ok(Value::text(out))
        }
        _ => {
            let Value::Text(t) = &x else {
                return match x {
                    Value::Null => Ok(Value::list(vec![Value::Null])),
                    other => conv_err(&other, "Text"),
                };
            };
            let parts: Vec<String> = match n {
                Native::SplitByDelimiter(d, csv) => split_text(t, &[d], *csv),
                Native::SplitByAnyDelimiter(ds, csv) => {
                    let ds: Vec<&str> = ds.iter().map(|d| &**d).collect();
                    split_text(t, &ds, *csv)
                }
                Native::SplitByPositions(ps) => {
                    let c: Vec<char> = t.chars().collect();
                    let mut out = Vec::new();
                    for (i, &p) in ps.iter().enumerate() {
                        let end = ps.get(i + 1).copied().unwrap_or(c.len()).min(c.len());
                        let start = p.min(end);
                        out.push(c.get(start..end).unwrap_or(&[]).iter().collect());
                    }
                    out
                }
                Native::SplitByLengths(ls) => {
                    let c: Vec<char> = t.chars().collect();
                    let mut out = Vec::new();
                    let mut p = 0usize;
                    for &l in ls.iter() {
                        let end = p.saturating_add(l).min(c.len());
                        out.push(c.get(p.min(end)..end).unwrap_or(&[]).iter().collect());
                        p = end;
                    }
                    out
                }
                Native::SplitByEachDelimiter(..)
                | Native::SplitByRanges(..)
                | Native::SplitByRepeatedLengths(..)
                | Native::SplitByWhitespace(_)
                | Native::SplitByTransition(_) => super::misc_more::call_splitter(interp, n, t)?,
                _ => vec![t.to_string()],
            };
            interp.check_rows(parts.len())?;
            interp.tick(parts.len() as u64)?;
            Ok(Value::list(parts.into_iter().map(Value::text).collect()))
        }
    }
}
