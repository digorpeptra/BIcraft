//! The supported subset of the M standard library.
//!
//! Every function is pure: no network, no environment, no clock. File access
//! (`File.Contents`, `Folder.*`) is limited to the allow-list in `SourceOptions`.

use std::collections::HashMap;
use std::rc::Rc;

use super::eval::{Interp, title, whole};
use super::value::{Facet, Func, Kind, MType, Native, Num, Record, Res, Table, Value, expr_err};

pub(crate) mod datetime;
pub(crate) mod files;
pub(crate) mod list;
pub(crate) mod list_more;
pub(crate) mod misc;
pub(crate) mod misc_more;
pub(crate) mod number;
pub(crate) mod source;
pub(crate) mod table;
pub(crate) mod table_more;
pub(crate) mod text;

/// A library function implemented in Rust.
pub(crate) struct Builtin {
    pub name: &'static str,
    pub min: usize,
    pub max: usize,
    /// Called with exactly `max` arguments (missing optional ones are null).
    pub f: fn(&mut Interp, &[Value]) -> Res<Value>,
}

impl std::fmt::Debug for Builtin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name)
    }
}

/// Shorthand for the function tables.
pub(crate) const fn b(
    name: &'static str,
    min: usize,
    max: usize,
    f: fn(&mut Interp, &[Value]) -> Res<Value>,
) -> Builtin {
    Builtin { name, min, max, f }
}

pub(crate) fn table() -> HashMap<&'static str, &'static Builtin> {
    let mut m = HashMap::new();
    for group in [
        table::FUNCS,
        list::FUNCS,
        text::FUNCS,
        number::FUNCS,
        datetime::FUNCS,
        misc::FUNCS,
        source::FUNCS,
        files::FUNCS,
        list_more::FUNCS,
        table_more::FUNCS,
        misc_more::FUNCS,
    ] {
        for f in group {
            m.insert(f.name, f);
        }
    }
    m
}

fn num(n: f64) -> Value {
    Value::num(n)
}

/// Library constants: enumerations and type names.
pub(crate) fn constant(name: &str, interp: &Interp) -> Option<Value> {
    let ty = |k: Kind| Some(Value::ty(MType::prim(k)));
    let facet = |f: Facet| Some(Value::ty(MType::facet(Kind::Number, f)));
    match name {
        "Order.Ascending"
        | "JoinKind.Inner"
        | "MissingField.Error"
        | "GroupKind.Local"
        | "QuoteStyle.None"
        | "Compression.GZip"
        | "BinaryEncoding.Base64"
        | "Occurrence.First"
        | "RoundingMode.Up"
        | "Precision.Double"
        | "Day.Sunday"
        | "ExtraValues.List"
        | "JoinAlgorithm.Dynamic"
        | "CsvStyle.QuoteAfterDelimiter" => Some(num(0.0)),
        "Order.Descending"
        | "JoinKind.LeftOuter"
        | "MissingField.Ignore"
        | "GroupKind.Global"
        | "QuoteStyle.Csv"
        | "Compression.Deflate"
        | "BinaryEncoding.Hex"
        | "Occurrence.Last"
        | "RoundingMode.Down"
        | "Precision.Decimal"
        | "Day.Monday"
        | "ExtraValues.Error"
        | "CsvStyle.QuoteAlways" => Some(num(1.0)),
        "JoinKind.RightOuter"
        | "MissingField.UseNull"
        | "Occurrence.All"
        | "RoundingMode.AwayFromZero"
        | "Day.Tuesday"
        | "ExtraValues.Ignore"
        | "Compression.Snappy" => Some(num(2.0)),
        "JoinKind.FullOuter" | "RoundingMode.TowardZero" | "Day.Wednesday" | "Compression.Brotli" => {
            Some(num(3.0))
        }
        "JoinKind.LeftAnti" | "RoundingMode.ToEven" | "Day.Thursday" | "Compression.LZ4" => Some(num(4.0)),
        "JoinKind.RightAnti" | "Day.Friday" | "Compression.Zstandard" => Some(num(5.0)),
        "Day.Saturday" => Some(num(6.0)),
        "RankKind.Competition" => Some(num(0.0)),
        "RankKind.Dense" | "PercentileMode.ExcelInc" => Some(num(1.0)),
        "RankKind.Ordinal" | "PercentileMode.ExcelExc" => Some(num(2.0)),
        "PercentileMode.SqlDisc" => Some(num(3.0)),
        "PercentileMode.SqlCont" => Some(num(4.0)),
        "RankKind.Type" | "PercentileMode.Type" => ty(Kind::Number),
        "Number.PI" => Some(num(std::f64::consts::PI)),
        "Number.E" => Some(num(std::f64::consts::E)),
        "Number.Epsilon" => Some(num(f64::from_bits(1))),
        "Number.PositiveInfinity" => Some(num(f64::INFINITY)),
        "Number.NegativeInfinity" => Some(num(f64::NEG_INFINITY)),
        "Number.NaN" => Some(num(f64::NAN)),
        "TextEncoding.Utf8" => Some(num(65001.0)),
        "TextEncoding.Utf16" | "TextEncoding.Unicode" => Some(num(1200.0)),
        "TextEncoding.BigEndianUnicode" => Some(num(1201.0)),
        "TextEncoding.Windows" => Some(num(1252.0)),
        "TextEncoding.Ascii" => Some(num(20127.0)),
        "Culture.Current" => Some(Value::text(interp.culture.name.as_str())),
        "Comparer.Ordinal" => Some(native(Native::Comparer { ignore_case: false })),
        "Comparer.OrdinalIgnoreCase" => Some(native(Native::Comparer { ignore_case: true })),
        "Any.Type" => ty(Kind::Any),
        "None.Type" => ty(Kind::None),
        "Null.Type" => ty(Kind::Null),
        "Logical.Type" => ty(Kind::Logical),
        "Number.Type" => ty(Kind::Number),
        "Text.Type" => ty(Kind::Text),
        "Date.Type" => ty(Kind::Date),
        "DateTime.Type" => ty(Kind::DateTime),
        "DateTimeZone.Type" => ty(Kind::DateTimeZone),
        "Time.Type" => ty(Kind::Time),
        "Duration.Type" => ty(Kind::Duration),
        "Binary.Type" => ty(Kind::Binary),
        "List.Type" => ty(Kind::List),
        "Record.Type" => ty(Kind::Record),
        "Table.Type" => ty(Kind::Table),
        "Function.Type" => ty(Kind::Function),
        "Type.Type" => ty(Kind::Type),
        "Int64.Type" => facet(Facet::Int64),
        "Int32.Type" => facet(Facet::Int32),
        "Int16.Type" => facet(Facet::Int16),
        "Int8.Type" => facet(Facet::Int8),
        "Byte.Type" => facet(Facet::Byte),
        "Double.Type" => facet(Facet::Double),
        "Single.Type" => facet(Facet::Single),
        "Decimal.Type" => facet(Facet::Decimal),
        "Currency.Type" => facet(Facet::Currency),
        "Percentage.Type" => facet(Facet::Percentage),
        _ => None,
    }
}

pub(crate) fn native(n: Native) -> Value {
    Value::Function(Rc::new(Func::Native(n)))
}

pub(crate) fn call_native(interp: &mut Interp, n: &Native, args: Vec<Value>) -> Res<Value> {
    misc::call_native(interp, n, args)
}

// ---------------------------------------------------------------------------
// Argument helpers.

pub(crate) fn arg(args: &[Value], i: usize) -> &Value {
    args.get(i).unwrap_or(&Value::Null)
}

pub(crate) fn conv_err<T>(v: &Value, to: &str) -> Res<T> {
    let shown = match v {
        Value::Text(t) => format!("\"{}\"", crate::value::truncate(t)),
        Value::Number(n) => format!("{}", n.f()),
        Value::Null => "null".into(),
        Value::Logical(b) => b.to_string(),
        other => format!("of type {}", title(other.type_name())),
    };
    expr_err(format!("We cannot convert the value {shown} to type {to}."))
}

pub(crate) fn text(v: &Value) -> Res<&str> {
    match v {
        Value::Text(t) => Ok(t),
        other => conv_err(other, "Text"),
    }
}

pub(crate) fn opt_text(v: &Value) -> Res<Option<&str>> {
    match v {
        Value::Null => Ok(None),
        other => text(other).map(Some),
    }
}

pub(crate) fn number(v: &Value) -> Res<f64> {
    match v {
        Value::Number(n) => Ok(n.f()),
        other => conv_err(other, "Number"),
    }
}

/// A number argument as it is (double or decimal).
pub(crate) fn opt_num(v: &Value) -> Res<Option<Num>> {
    match v {
        Value::Null => Ok(None),
        Value::Number(n) => Ok(Some(*n)),
        other => conv_err(other, "Number"),
    }
}

/// `Precision.Decimal` (1) in an optional precision argument.
pub(crate) fn decimal_precision(v: &Value) -> Res<bool> {
    Ok(opt_number(v)? == Some(1.0))
}

pub(crate) fn opt_number(v: &Value) -> Res<Option<f64>> {
    match v {
        Value::Null => Ok(None),
        other => number(other).map(Some),
    }
}

/// A whole, non-negative count.
pub(crate) fn count(v: &Value) -> Res<usize> {
    let n = number(v)?;
    match whole(n) {
        Some(i) if i >= 0 => Ok(usize::try_from(i).unwrap_or(usize::MAX)),
        _ => expr_err(format!("The count {n} must be a non-negative whole number.")),
    }
}

pub(crate) fn int(v: &Value) -> Res<i64> {
    let n = number(v)?;
    match whole(n) {
        Some(i) => Ok(i),
        None => expr_err(format!("The number {n} must be a whole number.")),
    }
}

pub(crate) fn list(v: &Value) -> Res<&Rc<Vec<Value>>> {
    match v {
        Value::List(l) => Ok(l),
        other => conv_err(other, "List"),
    }
}

pub(crate) fn tbl(v: &Value) -> Res<&Rc<Table>> {
    match v {
        Value::Table(t) => Ok(t),
        other => conv_err(other, "Table"),
    }
}

pub(crate) fn record(v: &Value) -> Res<&Rc<Record>> {
    match v {
        Value::Record(r) => Ok(r),
        other => conv_err(other, "Record"),
    }
}

pub(crate) fn func(v: &Value) -> Res<&Value> {
    match v {
        Value::Function(_) => Ok(v),
        other => conv_err(other, "Function"),
    }
}

pub(crate) fn mtype(v: &Value) -> Res<MType> {
    match v {
        Value::Type(t) => Ok((**t).clone()),
        Value::Null => Ok(MType::any()),
        other => conv_err(other, "Type"),
    }
}

/// A list of texts, or one text treated as a one-item list.
pub(crate) fn names(v: &Value) -> Res<Vec<Rc<str>>> {
    match v {
        Value::Text(t) => Ok(vec![t.clone()]),
        Value::List(l) => l
            .iter()
            .map(|x| match x {
                Value::Text(t) => Ok(t.clone()),
                other => conv_err(other, "Text"),
            })
            .collect(),
        other => conv_err(other, "List"),
    }
}

/// The items of a list, raising any stored error.
pub(crate) fn items(v: &Value) -> Res<Vec<Value>> {
    list(v)?.iter().map(|x| x.clone().check()).collect()
}

/// A culture argument: text, or an options record with a `Culture` field.
pub(crate) fn culture_arg(interp: &Interp, v: &Value) -> Res<super::convert::CultureInfo> {
    Ok(match v {
        Value::Null => interp.culture.clone(),
        Value::Text(t) => super::convert::CultureInfo::from_name(t),
        Value::Record(r) => match r.get("Culture") {
            Some(Value::Text(t)) => super::convert::CultureInfo::from_name(t),
            _ => interp.culture.clone(),
        },
        other => return conv_err(other, "Text"),
    })
}
