//! JSON helpers shared by the reader, writer and validator.

use serde::Serialize;
use serde_json::{Map, Number, Value};

use crate::Error;

/// Prefix of every PBIR `$schema` URL.
pub(crate) const SCHEMA_PREFIX: &str =
    "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/";

/// Parses a PBIR JSON document: UTF-8 without BOM (Desktop rejects a BOM), any JSON value.
pub(crate) fn parse(path: &str, bytes: &[u8]) -> Result<Value, Error> {
    let invalid = |message: String| Error::Invalid {
        path: path.to_string(),
        message,
    };
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return Err(invalid(
            "starts with a byte order mark; PBIR files are UTF-8 without BOM".into(),
        ));
    }
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        return Err(invalid("UTF-16 text; PBIR files are UTF-8 without BOM".into()));
    }
    serde_json::from_slice(bytes).map_err(|e| invalid(format!("invalid JSON: {e}")))
}

/// Parses a document that must be a JSON object.
pub(crate) fn parse_object(path: &str, bytes: &[u8]) -> Result<Map<String, Value>, Error> {
    match parse(path, bytes)? {
        Value::Object(map) => Ok(map),
        _ => Err(Error::Invalid {
            path: path.to_string(),
            message: "the document is not a JSON object".into(),
        }),
    }
}

/// Desktop's layout: 2-space indentation, `": "` separators, LF, no trailing newline.
pub(crate) fn to_bytes(value: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b"  ");
    let mut ser = serde_json::Serializer::with_formatter(&mut out, formatter);
    // Serializing a `Value` into a Vec cannot fail (map keys are strings).
    if value.serialize(&mut ser).is_err() {
        out.clear();
    }
    out
}

/// A number as Desktop writes it: integral values without a fraction.
pub(crate) fn number(v: f64) -> Value {
    if !v.is_finite() {
        return Value::from(0);
    }
    if v.fract() == 0.0 && v.abs() < 9.0e15 {
        // Exact: |v| < 2^53.
        let i = v as i64;
        return Value::from(i);
    }
    Number::from_f64(v).map(Value::Number).unwrap_or(Value::from(0))
}

/// Builds an object from `typed` (keys this crate models) and `extra` (unmodelled keys in source
/// order). Typed keys that also appear in `extra` take that position; the others are inserted
/// before the first `extra` key that comes later in `order` (the Desktop key order), or at the end.
/// Keys listed in `remove` are dropped from `extra`.
pub(crate) fn merge(
    order: &[&str],
    mut typed: Vec<(&str, Value)>,
    extra: &Map<String, Value>,
    remove: &[&str],
) -> Map<String, Value> {
    let rank = |key: &str| order.iter().position(|k| *k == key);
    let unranked = order.len();
    typed.sort_by_key(|(k, _)| rank(k).unwrap_or(unranked));
    let mut out = Map::new();
    let mut pending = typed;
    // `$schema` always comes first.
    if order.first() == Some(&"$schema") {
        if let Some(i) = pending.iter().position(|(k, _)| *k == "$schema") {
            let (k, v) = pending.remove(i);
            out.insert(k.to_string(), v);
        } else if let Some(v) = extra.get("$schema") {
            out.insert("$schema".into(), v.clone());
        }
    }
    let flush_before = |limit: usize, pending: &mut Vec<(&str, Value)>, out: &mut Map<String, Value>| {
        let mut i = 0;
        while let Some((k, _)) = pending.get(i) {
            if rank(k).unwrap_or(unranked) >= limit {
                break;
            }
            if extra.contains_key(*k) {
                i += 1;
            } else {
                let (k, v) = pending.remove(i);
                out.insert(k.to_string(), v);
            }
        }
    };
    for (key, value) in extra {
        if remove.contains(&key.as_str()) || out.contains_key(key) {
            continue;
        }
        if let Some(r) = rank(key) {
            flush_before(r, &mut pending, &mut out);
        }
        if let Some(i) = pending.iter().position(|(k, _)| *k == key) {
            let (k, v) = pending.remove(i);
            out.insert(k.to_string(), v);
        } else {
            out.insert(key.clone(), value.clone());
        }
    }
    for (k, v) in pending {
        out.insert(k.to_string(), v);
    }
    out
}

/// `key`, version triple of a PBIR `$schema` URL.
pub(crate) fn parse_schema_url(url: &str) -> Option<(&str, (u64, u64, u64))> {
    let rest = url.strip_prefix(SCHEMA_PREFIX)?;
    let mut parts = rest.split('/');
    let key = parts.next()?;
    let version = parse_version(parts.next()?)?;
    if parts.next()? != "schema.json" || parts.next().is_some() || key.is_empty() {
        return None;
    }
    Some((key, version))
}

pub(crate) fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let mut it = s.split('.');
    let v = (
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
    );
    if it.next().is_some() {
        return None;
    }
    Some(v)
}

pub(crate) fn schema_url(key: &str, version: &str) -> String {
    format!("{SCHEMA_PREFIX}{key}/{version}/schema.json")
}

/// JSON equality where numbers compare by value (`1` equals `1.0`).
pub(crate) fn json_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            if let (Some(x), Some(y)) = (x.as_i64(), y.as_i64()) {
                x == y
            } else if let (Some(x), Some(y)) = (x.as_u64(), y.as_u64()) {
                x == y
            } else {
                x.as_f64() == y.as_f64()
            }
        }
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(a, b)| json_eq(a, b))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| json_eq(v, w)))
        }
        _ => a == b,
    }
}

/// Short JSON type name for messages.
pub(crate) fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merge_keeps_extra_order_and_places_typed_keys() {
        let order = [
            "$schema",
            "name",
            "position",
            "visual",
            "annotations",
            "filterConfig",
        ];
        let extra = json!({"$schema": "s", "annotations": [], "zz": 1});
        let typed = vec![
            ("filterConfig", json!({})),
            ("name", json!("n")),
            ("visual", json!({})),
        ];
        let out = merge(&order, typed, extra.as_object().unwrap(), &[]);
        let keys: Vec<&str> = out.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            ["$schema", "name", "visual", "annotations", "zz", "filterConfig"]
        );
    }

    #[test]
    fn merge_typed_override_takes_extra_position() {
        let extra = json!({"a": 1, "isHidden": false, "b": 2});
        let out = merge(
            &["a", "isHidden", "b"],
            vec![("isHidden", json!(true))],
            extra.as_object().unwrap(),
            &[],
        );
        assert_eq!(Value::Object(out), json!({"a": 1, "isHidden": true, "b": 2}));
    }

    #[test]
    fn numbers_and_schema_urls() {
        assert_eq!(number(720.0), json!(720));
        assert_eq!(number(10.5), json!(10.5));
        assert_eq!(number(f64::NAN), json!(0));
        let url = schema_url("page", "2.1.0");
        assert_eq!(parse_schema_url(&url), Some(("page", (2, 1, 0))));
        assert_eq!(
            parse_schema_url("https://example.com/page/2.1.0/schema.json"),
            None
        );
        assert_eq!(
            to_bytes(&json!({"a": [1, {}], "b": []})),
            b"{\n  \"a\": [\n    1,\n    {}\n  ],\n  \"b\": []\n}"
        );
    }
}
