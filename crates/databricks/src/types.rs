//! Databricks SQL types → the M types the connector gives a column, and the `JSON_ARRAY` values
//! of the Statement Execution API → [`RemoteValue`]s.
//!
//! The API's manifest names each column's type (`type_name`: `STRING`, `INT`, `DECIMAL`, …, and
//! the full `type_text`, `decimal(18,2)`, `string collate UTF8_LCASE`); with the `JSON_ARRAY`
//! format every value is a JSON string or `null` (Databricks, "Statement Execution API",
//! *Result data format*).

use bicraft_data::connect::{RemoteColumn, RemoteType, RemoteValue};
use bicraft_locale::calendar::{civil_from_days, days_from_civil};
use serde_json::Value;

/// The M type of a Databricks column, and whether SQL compares its values exactly as M does.
pub(crate) fn column(name: &str, type_name: &str, type_text: &str) -> RemoteColumn {
    let upper = type_name.trim().to_ascii_uppercase();
    let text = type_text.trim().to_ascii_lowercase();
    let (ty, comparable) = match upper.as_str() {
        "STRING" | "CHAR" | "VARCHAR" => {
            // A collation other than the binary default (UTF8_LCASE, unicode_ci, …) changes
            // what `=` means.
            let binary = !text.contains("collate") || text.contains("utf8_binary");
            (RemoteType::Text, binary)
        }
        "BOOLEAN" => (RemoteType::Logical, true),
        "BYTE" | "TINYINT" => (RemoteType::Int8, true),
        "SHORT" | "SMALLINT" => (RemoteType::Int16, true),
        "INT" | "INTEGER" => (RemoteType::Int32, true),
        "LONG" | "BIGINT" => (RemoteType::Int64, true),
        // SQL compares a FLOAT with an integer in single precision; M in double.
        "FLOAT" | "REAL" => (RemoteType::Single, false),
        "DOUBLE" => (RemoteType::Double, true),
        "DECIMAL" | "DEC" | "NUMERIC" => (RemoteType::Decimal, true),
        "DATE" => (RemoteType::Date, true),
        "TIMESTAMP" | "TIMESTAMP_NTZ" | "TIMESTAMP_LTZ" => (RemoteType::DateTime, false),
        "BINARY" => (RemoteType::Binary, false),
        "NULL" | "VOID" => (RemoteType::Any, false),
        // INTERVAL, ARRAY, MAP, STRUCT, VARIANT, …: their text (JSON for the complex types).
        _ => (RemoteType::Text, false),
    };
    RemoteColumn {
        name: name.to_string(),
        ty,
        comparable,
    }
}

/// A cell of the API's `data_array`.
pub(crate) fn value(cell: &Value, ty: RemoteType) -> Result<RemoteValue, String> {
    let text: std::borrow::Cow<'_, str> = match cell {
        Value::Null => return Ok(RemoteValue::Null),
        Value::String(s) => s.as_str().into(),
        Value::Bool(b) => if *b { "true" } else { "false" }.into(),
        Value::Number(n) => n.to_string().into(),
        // Complex values (arrays, maps, structs) as their JSON text.
        other => other.to_string().into(),
    };
    let t = text.as_ref();
    let bad = || format!("unexpected {ty:?} value `{}`", short(t));
    Ok(match ty {
        RemoteType::Text | RemoteType::Any | RemoteType::Record | RemoteType::Table => {
            RemoteValue::Text(t.to_string())
        }
        RemoteType::Logical => match t.trim().to_ascii_lowercase().as_str() {
            "true" => RemoteValue::Logical(true),
            "false" => RemoteValue::Logical(false),
            _ => return Err(bad()),
        },
        RemoteType::Int8 | RemoteType::Int16 | RemoteType::Int32 | RemoteType::Int64 => {
            RemoteValue::Int(t.trim().parse::<i64>().map_err(|_| bad())?)
        }
        RemoteType::Single | RemoteType::Double => RemoteValue::Double(float(t).ok_or_else(bad)?),
        RemoteType::Decimal => {
            let s = t.trim();
            let digits = s.strip_prefix(['-', '+']).unwrap_or(s);
            let ok = !digits.is_empty()
                && digits.bytes().filter(|b| *b == b'.').count() <= 1
                && digits.bytes().all(|b| b.is_ascii_digit() || b == b'.')
                && digits.bytes().any(|b| b.is_ascii_digit());
            if !ok {
                return Err(bad());
            }
            RemoteValue::Decimal(s.to_string())
        }
        RemoteType::Date => {
            let (year, month, day) = date(t.trim()).ok_or_else(bad)?;
            RemoteValue::Date { year, month, day }
        }
        RemoteType::DateTime => timestamp(t.trim()).ok_or_else(bad)?,
        RemoteType::DateTimeZone | RemoteType::Time => return Err(bad()),
        RemoteType::Binary => RemoteValue::Binary(base64(t.trim()).ok_or_else(bad)?),
    })
}

fn short(s: &str) -> String {
    match s.char_indices().nth(40) {
        Some((i, _)) => format!("{}…", s.get(..i).unwrap_or(s)),
        None => s.to_string(),
    }
}

fn float(t: &str) -> Option<f64> {
    match t.trim() {
        "NaN" | "nan" => Some(f64::NAN),
        "Infinity" | "inf" | "+Infinity" => Some(f64::INFINITY),
        "-Infinity" | "-inf" => Some(f64::NEG_INFINITY),
        s => s.parse::<f64>().ok().filter(|x| x.is_finite()),
    }
}

fn num<T: std::str::FromStr>(s: &str, len: usize) -> Option<T> {
    if s.len() != len || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// `YYYY-MM-DD` (years 1–9999; the range is checked again when the value becomes M).
fn date(s: &str) -> Option<(i32, u8, u8)> {
    let mut it = s.split('-');
    let y: i32 = num(it.next()?, 4)?;
    let m: u8 = num(it.next()?, 2)?;
    let d: u8 = num(it.next()?, 2)?;
    if it.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some((y, m, d))
}

const MS_PER_DAY: i64 = 86_400_000;

/// `YYYY-MM-DD[T| ]HH:MM[:SS[.fff…]][Z|±HH[:MM]]`: an offset is converted to UTC, fractions
/// beyond milliseconds are truncated.
fn timestamp(s: &str) -> Option<RemoteValue> {
    let (d, rest) = match s.find(['T', ' ']) {
        Some(i) => (s.get(..i)?, s.get(i + 1..)?),
        None => (s, ""),
    };
    let (y, mo, da) = date(d)?;
    let (clock, offset_min) = if let Some(c) = rest.strip_suffix(['Z', 'z']) {
        (c, 0i64)
    } else if let Some(i) = rest.rfind(['+', '-']).filter(|i| *i >= 5) {
        let (c, off) = (rest.get(..i)?, rest.get(i..)?);
        let sign = if off.starts_with('-') { -1 } else { 1 };
        let off = off.get(1..)?.replace(':', "");
        let (h, m) = match off.len() {
            2 => (num::<i64>(&off, 2)?, 0),
            4 => (num::<i64>(off.get(..2)?, 2)?, num::<i64>(off.get(2..)?, 2)?),
            _ => return None,
        };
        (c, sign * (h * 60 + m))
    } else {
        (rest, 0)
    };
    let mut ms_of_day = 0i64;
    if !clock.is_empty() {
        let (hms, frac) = match clock.split_once('.') {
            Some((a, b)) => (a, b),
            None => (clock, ""),
        };
        let mut it = hms.split(':');
        let h: i64 = num(it.next()?, 2)?;
        let mi: i64 = num(it.next()?, 2)?;
        let sec: i64 = match it.next() {
            Some(x) => num(x, 2)?,
            None => 0,
        };
        if it.next().is_some() || h > 23 || mi > 59 || sec > 59 {
            return None;
        }
        if !frac.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let mut f = frac
            .bytes()
            .take(3)
            .map(|b| i64::from(b - b'0'))
            .collect::<Vec<_>>();
        f.resize(3, 0);
        let millis = f.iter().fold(0i64, |a, d| a * 10 + d);
        ms_of_day = ((h * 60 + mi) * 60 + sec) * 1000 + millis;
    }
    let total = days_from_civil(i64::from(y), i64::from(mo), i64::from(da))
        .checked_mul(MS_PER_DAY)?
        .checked_add(ms_of_day)?
        .checked_sub(offset_min.checked_mul(60_000)?)?;
    let (y, mo, da) = civil_from_days(total.div_euclid(MS_PER_DAY));
    Some(RemoteValue::DateTime {
        year: i32::try_from(y).ok()?,
        month: u8::try_from(mo).ok()?,
        day: u8::try_from(da).ok()?,
        millis: u32::try_from(total.rem_euclid(MS_PER_DAY)).ok()?,
    })
}

/// Standard base64 (with or without padding).
fn base64(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => u32::from(c - b'A'),
            b'a'..=b'z' => u32::from(c - b'a') + 26,
            b'0'..=b'9' => u32::from(c - b'0') + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        })
    };
    let bytes: Vec<u8> = s.bytes().filter(|b| *b != b'=').collect();
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3 + 3);
    for chunk in bytes.chunks(4) {
        let mut acc = 0u32;
        for (i, c) in chunk.iter().enumerate() {
            acc |= val(*c)? << (18 - 6 * i as u32);
        }
        let n = match chunk.len() {
            4 => 3,
            3 => 2,
            2 => 1,
            _ => return None,
        };
        for i in 0..n {
            out.push((acc >> (16 - 8 * i)) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn dt(year: i32, month: u8, day: u8, millis: u32) -> RemoteValue {
        RemoteValue::DateTime {
            year,
            month,
            day,
            millis,
        }
    }

    #[test]
    fn column_types() {
        let c = |n: &str, t: &str| {
            let c = column("c", n, t);
            (c.ty, c.comparable)
        };
        assert_eq!(c("STRING", "string"), (RemoteType::Text, true));
        assert_eq!(
            c("STRING", "string collate UTF8_LCASE"),
            (RemoteType::Text, false)
        );
        assert_eq!(
            c("STRING", "string COLLATE UTF8_BINARY"),
            (RemoteType::Text, true)
        );
        assert_eq!(c("LONG", "bigint"), (RemoteType::Int64, true));
        assert_eq!(c("INT", "int"), (RemoteType::Int32, true));
        assert_eq!(c("DECIMAL", "decimal(18,2)"), (RemoteType::Decimal, true));
        assert_eq!(c("FLOAT", "float"), (RemoteType::Single, false));
        assert_eq!(c("TIMESTAMP", "timestamp"), (RemoteType::DateTime, false));
        assert_eq!(c("ARRAY", "array<int>"), (RemoteType::Text, false));
        assert_eq!(c("NULL", "void"), (RemoteType::Any, false));
    }

    #[test]
    fn values() {
        let v = |c: Value, t| value(&c, t).unwrap();
        assert_eq!(v(json!(null), RemoteType::Int64), RemoteValue::Null);
        assert_eq!(v(json!("42"), RemoteType::Int64), RemoteValue::Int(42));
        assert_eq!(v(json!(42), RemoteType::Int32), RemoteValue::Int(42));
        assert_eq!(v(json!("true"), RemoteType::Logical), RemoteValue::Logical(true));
        assert_eq!(
            v(json!("-12.5000"), RemoteType::Decimal),
            RemoteValue::Decimal("-12.5000".into())
        );
        assert_eq!(
            v(json!("1.5E10"), RemoteType::Double),
            RemoteValue::Double(1.5e10)
        );
        assert!(matches!(v(json!("NaN"), RemoteType::Double), RemoteValue::Double(x) if x.is_nan()));
        assert_eq!(
            v(json!("2026-01-05"), RemoteType::Date),
            RemoteValue::Date {
                year: 2026,
                month: 1,
                day: 5
            }
        );
        assert_eq!(
            v(json!("2026-01-05T13:45:10.123456Z"), RemoteType::DateTime),
            dt(2026, 1, 5, 49_510_123)
        );
        assert_eq!(
            v(json!("2026-01-05 13:45:10"), RemoteType::DateTime),
            dt(2026, 1, 5, 49_510_000)
        );
        // offsets are converted to UTC, across midnight
        assert_eq!(
            v(json!("2026-01-01T00:30:00+01:00"), RemoteType::DateTime),
            dt(2025, 12, 31, 84_600_000)
        );
        assert_eq!(
            v(json!("2026-01-05T23:00:00-0200"), RemoteType::DateTime),
            dt(2026, 1, 6, 3_600_000)
        );
        assert_eq!(
            v(json!("aGVsbG8="), RemoteType::Binary),
            RemoteValue::Binary(b"hello".to_vec())
        );
        assert_eq!(
            v(json!("aGk"), RemoteType::Binary),
            RemoteValue::Binary(b"hi".to_vec())
        );
        assert_eq!(
            v(json!([1, 2]), RemoteType::Text),
            RemoteValue::Text("[1,2]".into())
        );
        for (c, t) in [
            (json!("x"), RemoteType::Int64),
            (json!("1e3"), RemoteType::Decimal),
            (json!("2026-13-01"), RemoteType::Date),
            (json!("2026-01-01T25:00:00"), RemoteType::DateTime),
            (json!("yes"), RemoteType::Logical),
            (json!("!!"), RemoteType::Binary),
        ] {
            assert!(value(&c, t).is_err(), "{c} as {t:?}");
        }
    }
}
