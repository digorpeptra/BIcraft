//! Paths inside the legacy document (`config.singleVisual.objects`, `filters[2]`,
//! `config.singleVisual.columnProperties["Sales.Total"]`), the unmapped-property collector, and
//! the JSON-in-a-string slots of the legacy format.

use serde_json::{Map, Value};

pub(crate) type Object = Map<String, Value>;

/// Deepest JSON nesting walked by the recursive helpers of this crate (serde_json itself stops
/// parsing at 128 levels).
pub(crate) const MAX_DEPTH: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Seg {
    Key(String),
    Index(usize),
}

/// A path being built while reading.
#[derive(Debug, Clone, Default)]
pub(crate) struct At(Vec<Seg>);

impl At {
    pub fn root() -> Self {
        At(Vec::new())
    }

    pub fn key(&self, k: &str) -> Self {
        let mut v = self.0.clone();
        v.push(Seg::Key(k.to_string()));
        At(v)
    }

    pub fn index(&self, i: usize) -> Self {
        let mut v = self.0.clone();
        v.push(Seg::Index(i));
        At(v)
    }

    pub fn text(&self) -> String {
        format_path(&self.0)
    }
}

fn plain(k: &str) -> bool {
    !k.is_empty() && !k.contains(['.', '[', ']', '"'])
}

pub(crate) fn format_path(segs: &[Seg]) -> String {
    let mut out = String::new();
    for s in segs {
        match s {
            Seg::Key(k) if plain(k) => {
                if !out.is_empty() {
                    out.push('.');
                }
                out.push_str(k);
            }
            Seg::Key(k) => {
                out.push('[');
                out.push_str(&Value::String(k.clone()).to_string());
                out.push(']');
            }
            Seg::Index(i) => {
                out.push('[');
                out.push_str(&i.to_string());
                out.push(']');
            }
        }
    }
    out
}

/// The inverse of [`format_path`]; `None` for a malformed path.
pub(crate) fn parse_path(text: &str) -> Option<Vec<Seg>> {
    let mut out = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('[') {
            if after.starts_with('"') {
                // A quoted key: the JSON string runs up to the first `"]` that ends it.
                let mut de = serde_json::Deserializer::from_str(after).into_iter::<String>();
                let key = de.next()?.ok()?;
                let used = de.byte_offset();
                rest = after.get(used..)?.strip_prefix(']')?;
                out.push(Seg::Key(key));
            } else {
                let end = after.find(']')?;
                let n: usize = after.get(..end)?.parse().ok()?;
                out.push(Seg::Index(n));
                rest = after.get(end + 1..)?;
            }
        } else {
            let rest2 = if out.is_empty() {
                rest
            } else {
                rest.strip_prefix('.')?
            };
            let end = rest2.find(['.', '[']).unwrap_or(rest2.len());
            let key = rest2.get(..end)?;
            if key.is_empty() {
                return None;
            }
            out.push(Seg::Key(key.to_string()));
            rest = rest2.get(end..)?;
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

/// Puts `value` at `segs` under `root`, creating objects on the way. A final index inserts into
/// the array (an element that had no PBIR form goes back to where it was); an intermediate index
/// past the end appends an object.
pub(crate) fn set_path(root: &mut Value, segs: &[Seg], value: Value) {
    let Some((last, init)) = segs.split_last() else {
        return;
    };
    let mut cur = root;
    for s in init {
        cur = match s {
            Seg::Key(k) => {
                if !cur.is_object() {
                    *cur = Value::Object(Map::new());
                }
                let Value::Object(m) = cur else { return };
                m.entry(k.clone()).or_insert(Value::Null)
            }
            Seg::Index(i) => {
                if !cur.is_array() {
                    *cur = Value::Array(Vec::new());
                }
                let Value::Array(a) = cur else { return };
                if *i >= a.len() {
                    a.push(Value::Object(Map::new()));
                    let n = a.len().saturating_sub(1);
                    match a.get_mut(n) {
                        Some(v) => v,
                        None => return,
                    }
                } else {
                    match a.get_mut(*i) {
                        Some(v) => v,
                        None => return,
                    }
                }
            }
        };
    }
    match last {
        Seg::Key(k) => {
            if !cur.is_object() {
                *cur = Value::Object(Map::new());
            }
            if let Value::Object(m) = cur {
                m.insert(k.clone(), value);
            }
        }
        Seg::Index(i) => {
            if !cur.is_array() {
                *cur = Value::Array(Vec::new());
            }
            if let Value::Array(a) = cur {
                let at = (*i).min(a.len());
                a.insert(at, value);
            }
        }
    }
}

/// Legacy properties without a PBIR form, keyed by path.
#[derive(Debug, Default)]
pub(crate) struct Unmapped(Object);

impl Unmapped {
    pub fn put(&mut self, at: &At, value: Value) {
        self.0.insert(at.text(), value);
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn into_value(self) -> Value {
        Value::Object(self.0)
    }
}

/// Applies an unmapped map (as stored under `UNMAPPED_KEY`) onto a regenerated legacy object.
/// Entries whose path starts with one of `skip` are left out (they belong elsewhere).
pub(crate) fn apply_unmapped(target: &mut Value, unmapped: Option<&Value>, skip: &[&str]) {
    let Some(Value::Object(m)) = unmapped else {
        return;
    };
    for (path, value) in m {
        if skip
            .iter()
            .any(|s| path == s || path.starts_with(&format!("{s}.")) || path.starts_with(&format!("{s}[")))
        {
            continue;
        }
        if let Some(segs) = parse_path(path) {
            set_path(target, &segs, value.clone());
        }
    }
}

/// The entries of an unmapped map under `prefix`, re-rooted (`pod.config.x` → `config.x`).
pub(crate) fn unmapped_under(unmapped: Option<&Value>, prefix: &str) -> Value {
    let mut out = Map::new();
    if let Some(Value::Object(m)) = unmapped {
        for (path, value) in m {
            if let Some(rest) = path.strip_prefix(prefix) {
                let rest = rest.strip_prefix('.').unwrap_or(rest);
                if !rest.is_empty() {
                    out.insert(rest.to_string(), value.clone());
                }
            }
        }
    }
    Value::Object(out)
}

/// A JSON-in-a-string slot of the legacy format (`config`, `filters`, `query`, …): the parsed
/// value, or the value itself when it is not a string. `Err` holds a string that does not parse.
pub(crate) fn slot(v: Option<&Value>) -> Result<Option<Value>, Value> {
    match v {
        None => Ok(None),
        Some(Value::String(s)) => match serde_json::from_str::<Value>(s) {
            Ok(p) => Ok(Some(p)),
            Err(_) => Err(Value::String(s.clone())),
        },
        Some(other) => Ok(Some(other.clone())),
    }
}

/// Turns the value at `key` of `obj` into its JSON text (a slot written back as a string).
pub(crate) fn encode_slot(obj: &mut Object, key: &str) {
    if let Some(v) = obj.get_mut(key)
        && !v.is_string()
    {
        let text = v.to_string();
        *v = Value::String(text);
    }
}

/// A number as Desktop writes it in PBIR: integral values without a fraction.
pub(crate) fn norm_num(v: &Value) -> Value {
    match v.as_f64() {
        Some(f) if v.is_f64() && f.fract() == 0.0 && f.abs() < 9.0e15 => Value::from(f as i64),
        _ => v.clone(),
    }
}

/// A legacy number (`10.0`) for an `f64` the report holds.
pub(crate) fn float(v: f64) -> Value {
    serde_json::Number::from_f64(v).map_or(Value::Null, Value::Number)
}

/// Reorders the keys of `obj` to follow `order`; keys not listed keep their order after them.
pub(crate) fn reorder(obj: &mut Object, order: &[&str]) {
    let mut out = Map::new();
    for k in order {
        if let Some(v) = obj.shift_remove(*k) {
            out.insert((*k).to_string(), v);
        }
    }
    out.append(obj);
    *obj = out;
}

#[cfg(test)]
#[allow(clippy::indexing_slicing)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn paths_round_trip() {
        for p in [
            "config.modelExtensions",
            "filters[3]",
            "config.singleVisual.columnProperties[\"Sales.Total\"].foo",
            "a[0][1].b",
            "[\"\"]",
        ] {
            let segs = parse_path(p).unwrap();
            assert_eq!(format_path(&segs), p, "{p}");
        }
        for bad in ["", ".a", "a..b", "a[", "a[x]", "[\"x\"", "a.[0]"] {
            assert!(parse_path(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn set_path_inserts_and_creates() {
        let mut v = json!({"a": [1, 3]});
        set_path(&mut v, &parse_path("a[1]").unwrap(), json!(2));
        set_path(&mut v, &parse_path("b.c[0].d").unwrap(), json!(true));
        set_path(&mut v, &parse_path("a[9]").unwrap(), json!(4));
        assert_eq!(v, json!({"a": [1, 2, 3, 4], "b": {"c": [{"d": true}]}}));
    }
}
