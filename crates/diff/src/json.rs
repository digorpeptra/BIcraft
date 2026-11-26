//! Generic, order-insensitive comparison of JSON values (the `extra` maps where formats keep
//! what bicraft does not model), with caps on depth, count and value size.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value, json};

use crate::Prop;

/// Deepest level compared key by key; deeper values are compared whole.
pub const MAX_DEPTH: usize = 48;
/// Most properties reported for one object; the rest are counted in a last entry.
pub const MAX_PROPS: usize = 100;
/// Longest JSON value copied into a change (bytes, serialized); longer ones are summarised.
pub const MAX_VALUE_BYTES: usize = 4096;

/// Keys never compared: format, version and time churn.
pub const NOISE: &[&str] = &[
    "$schema",
    "modifiedTime",
    "structureModifiedTime",
    "refreshedTime",
    "lastUpdate",
    "lastSchemaUpdate",
    "lastProcessed",
    "createdTimestamp",
    "reportVersionAtImport",
];

/// A value small enough to copy, or a summary of it.
pub fn clip(v: &Value) -> Value {
    match v {
        Value::String(s) if s.len() <= MAX_VALUE_BYTES => v.clone(),
        Value::String(s) => json!(format!("<{} characters>", s.chars().count())),
        Value::Null | Value::Bool(_) | Value::Number(_) => v.clone(),
        _ => {
            let n = serde_json::to_string(v).map(|s| s.len()).unwrap_or(usize::MAX);
            if n <= MAX_VALUE_BYTES {
                v.clone()
            } else {
                json!(format!("<{n} bytes of JSON>"))
            }
        }
    }
}

/// Collects the differences between `a` and `b` under `prefix` into `out`.
pub struct Differ<'o> {
    pub out: &'o mut Vec<Prop>,
    /// Differences beyond [`MAX_PROPS`].
    pub dropped: usize,
    pub ignore: &'o [&'o str],
}

impl<'o> Differ<'o> {
    pub fn new(out: &'o mut Vec<Prop>, ignore: &'o [&'o str]) -> Differ<'o> {
        Differ {
            out,
            dropped: 0,
            ignore,
        }
    }

    fn push(&mut self, path: &str, a: Option<&Value>, b: Option<&Value>) {
        if self.out.len() >= MAX_PROPS {
            self.dropped = self.dropped.saturating_add(1);
            return;
        }
        self.out.push(Prop::new(path, a.map(clip), b.map(clip)));
    }

    /// Ends the comparison: a last entry counts what [`MAX_PROPS`] left out.
    pub fn finish(self) {
        if self.dropped > 0 {
            self.out.push(Prop::new(
                "…",
                None,
                Some(json!(format!("{} more differences", self.dropped))),
            ));
        }
    }

    fn skip(&self, key: &str) -> bool {
        NOISE.contains(&key) || self.ignore.contains(&key)
    }

    /// Compares two maps key by key (keys in `ignore` and [`NOISE`] skipped).
    pub fn maps(&mut self, prefix: &str, a: &Map<String, Value>, b: &Map<String, Value>) {
        self.maps_at(prefix, a, b, 0);
    }

    fn maps_at(&mut self, prefix: &str, a: &Map<String, Value>, b: &Map<String, Value>, depth: usize) {
        let keys: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
        for k in keys {
            if self.skip(k) {
                continue;
            }
            let path = join(prefix, k);
            self.values_at(
                &path,
                a.get(k.as_str()),
                b.get(k.as_str()),
                depth.saturating_add(1),
            );
        }
    }

    /// Compares two optional values.
    pub fn values(&mut self, path: &str, a: Option<&Value>, b: Option<&Value>) {
        self.values_at(path, a, b, 0);
    }

    fn values_at(&mut self, path: &str, a: Option<&Value>, b: Option<&Value>, depth: usize) {
        if a == b
            || (a.is_none() && b.is_some_and(Value::is_null))
            || (b.is_none() && a.is_some_and(Value::is_null))
        {
            return;
        }
        let (Some(va), Some(vb)) = (a, b) else {
            self.push(path, a, b);
            return;
        };
        if depth >= MAX_DEPTH {
            self.push(path, a, b);
            return;
        }
        match (va, vb) {
            (Value::Object(ma), Value::Object(mb)) => self.maps_at(path, ma, mb, depth),
            (Value::Array(xa), Value::Array(xb)) => self.arrays(path, xa, xb, depth),
            _ => self.push(path, a, b),
        }
    }

    fn arrays(&mut self, path: &str, a: &[Value], b: &[Value], depth: usize) {
        if let (Some(ka), Some(kb)) = (keyed(a), keyed(b)) {
            let names: BTreeSet<&str> = ka.keys().chain(kb.keys()).copied().collect();
            for n in names {
                let p = format!("{path}[{n}]");
                self.values_at(
                    &p,
                    ka.get(n).copied(),
                    kb.get(n).copied(),
                    depth.saturating_add(1),
                );
            }
            return;
        }
        if a.len() == b.len() {
            for (i, (x, y)) in a.iter().zip(b).enumerate() {
                self.values_at(&format!("{path}[{i}]"), Some(x), Some(y), depth.saturating_add(1));
            }
            return;
        }
        self.push(
            path,
            Some(&Value::Array(a.to_vec())),
            Some(&Value::Array(b.to_vec())),
        );
    }
}

/// `prefix.key`, or `key` at the top.
fn join(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_string()
    } else {
        format!("{prefix}.{key}")
    }
}

/// The name an array element is known by: `name`, else `memberName` (role members).
pub fn element_name(v: &Value) -> Option<&str> {
    v.get("name")
        .or_else(|| v.get("memberName"))
        .and_then(Value::as_str)
}

/// An array of objects with unique names, by name; `None` otherwise.
pub fn keyed(items: &[Value]) -> Option<BTreeMap<&str, &Value>> {
    let mut m = BTreeMap::new();
    for v in items {
        let n = element_name(v)?;
        if m.insert(n, v).is_some() {
            return None;
        }
    }
    Some(m)
}

/// Formatting objects (`{card: [{properties: {prop: expr}, selector?}]}`) flattened to
/// `card.prop` (with `[selector]` when the entry has one) → value, literals unwrapped.
pub fn flatten_objects(objects: Option<&Value>) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    let Some(Value::Object(cards)) = objects else {
        return out;
    };
    for (card, entries) in cards {
        let entries: Vec<&Value> = match entries {
            Value::Array(v) => v.iter().collect(),
            other => vec![other],
        };
        for entry in entries {
            let selector = entry.get("selector").map(selector_label);
            let Some(Value::Object(props)) = entry.get("properties") else {
                continue;
            };
            for (prop, value) in props {
                let mut key = format!("{card}.{prop}");
                if let Some(s) = &selector {
                    key.push_str(&format!("[{s}]"));
                }
                out.insert(key, clip(&literal(value)));
            }
        }
    }
    out
}

/// `{expr: {Literal: {Value: v}}}` → `v`; anything else as is.
fn literal(v: &Value) -> Value {
    v.pointer("/expr/Literal/Value")
        .or_else(|| v.pointer("/solid/color/expr/Literal/Value"))
        .cloned()
        .unwrap_or_else(|| v.clone())
}

fn selector_label(v: &Value) -> String {
    if let Some(m) = v.get("metadata").and_then(Value::as_str) {
        return m.to_string();
    }
    if let Some(id) = v.get("id").and_then(Value::as_str) {
        return id.to_string();
    }
    let s = serde_json::to_string(v).unwrap_or_default();
    if s.len() > 120 {
        let cut: String = s.chars().take(120).collect();
        format!("{cut}…")
    } else {
        s
    }
}

/// Compares two flattened formatting maps.
pub fn formatting(a: &BTreeMap<String, Value>, b: &BTreeMap<String, Value>, out: &mut Vec<Prop>) {
    let keys: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
    let mut dropped = 0usize;
    for k in keys {
        let (x, y) = (a.get(k), b.get(k));
        if x == y {
            continue;
        }
        if out.len() >= MAX_PROPS {
            dropped += 1;
            continue;
        }
        out.push(Prop::new(k.clone(), x.cloned(), y.cloned()));
    }
    if dropped > 0 {
        out.push(Prop::new(
            "…",
            None,
            Some(json!(format!("{dropped} more formatting changes"))),
        ));
    }
}

/// `Entity[Property]` of a PBIR field expression (`{Column: {Expression: {SourceRef:
/// {Entity}}, Property}}`, measures, aggregations, hierarchy levels), searched to a bounded
/// depth; `None` when it has none.
pub fn field_label(v: &Value) -> Option<String> {
    fn find<'v>(v: &'v Value, key: &str, depth: usize) -> Option<&'v str> {
        if depth > 12 {
            return None;
        }
        match v {
            Value::Object(m) => {
                if let Some(s) = m.get(key).and_then(Value::as_str) {
                    return Some(s);
                }
                m.values().find_map(|x| find(x, key, depth + 1))
            }
            Value::Array(a) => a.iter().take(16).find_map(|x| find(x, key, depth + 1)),
            _ => None,
        }
    }
    let entity = find(v, "Entity", 0)?;
    match find(v, "Property", 0).or_else(|| find(v, "Level", 0)) {
        Some(p) => Some(format!("{entity}[{p}]")),
        None => Some(entity.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(a: Value, b: Value) -> Vec<Prop> {
        let mut out = Vec::new();
        let mut d = Differ::new(&mut out, &[]);
        d.values("x", Some(&a), Some(&b));
        d.finish();
        out
    }

    #[test]
    fn key_order_and_named_arrays_are_not_changes() {
        assert!(
            run(
                json!({"a": 1, "b": [{"name": "p", "v": 1}, {"name": "q"}]}),
                json!({"b": [{"name": "q"}, {"name": "p", "v": 1}], "a": 1})
            )
            .is_empty()
        );
        let d = run(
            json!({"b": [{"name": "p", "v": 1}]}),
            json!({"b": [{"name": "p", "v": 2}]}),
        );
        assert_eq!(d, [Prop::new("x.b[p].v", Some(json!(1)), Some(json!(2)))]);
        assert!(
            run(
                json!({"$schema": "u1", "modifiedTime": 1}),
                json!({"$schema": "u2"})
            )
            .is_empty()
        );
    }

    #[test]
    fn caps_hold_on_hostile_values() {
        let mut deep = json!(1);
        for _ in 0..200 {
            deep = json!({"k": deep});
        }
        assert_eq!(run(deep.clone(), json!({"k": 2})).len(), 1);
        let wide_a: Map<String, Value> = (0..1000).map(|i| (format!("k{i:04}"), json!(i))).collect();
        let d = run(Value::Object(wide_a), json!({}));
        assert_eq!(d.len(), MAX_PROPS + 1);
        assert_eq!(d[MAX_PROPS].after, Some(json!("900 more differences")));
        let big = json!("x".repeat(10_000));
        assert_eq!(clip(&big), json!("<10000 characters>"));
    }

    #[test]
    fn formatting_is_flattened() {
        let objects = json!({"legend": [{"properties": {"show": {"expr": {"Literal": {"Value": "true"}}}}}],
            "dataPoint": [{"properties": {"fill": {"solid": {"color": {"expr": {"Literal": {"Value": "'#FF0000'"}}}}}}, "selector": {"metadata": "Sales.Amount"}}]});
        let f = flatten_objects(Some(&objects));
        assert_eq!(f.get("legend.show"), Some(&json!("true")));
        assert_eq!(f.get("dataPoint.fill[Sales.Amount]"), Some(&json!("'#FF0000'")));
    }

    #[test]
    fn field_labels() {
        let f = json!({"Column": {"Expression": {"SourceRef": {"Entity": "Sales"}}, "Property": "Region"}});
        assert_eq!(field_label(&f).as_deref(), Some("Sales[Region]"));
        assert_eq!(field_label(&json!({"x": 1})), None);
    }
}
