//! Format painter (`visual.copy_format`): one visual's formatting copied onto other visuals, in
//! one undo step (docs/formats/format-painter.md).
//!
//! What is copied follows Microsoft's description of Desktop's Format Painter (the Power BI blog,
//! December 2015 Desktop update): every formatting object, title options but the title text,
//! data labels but display units and decimal places, axes but their start and end, the legend
//! but its title text, data colours (a colour assigned to one value only when the target uses the
//! same field), borders, backgrounds, slicer options; options the target visual type lacks are
//! dropped. Fields, filters, size and position are not formatting and stay the target's.

use std::collections::HashSet;

use bicraft_report::{FieldRef, Visual};
use serde_json::{Map, Value, json};

use crate::Session;
use crate::params::{allow, str_param};
use crate::report::{page_index, parts, visual_index};

/// At most this many targets in one call.
const MAX_TARGETS: usize = 1_000;

/// Container objects that are not formatting: size and position, alt text (`general`), the
/// visual's action (`visualLink`), the aspect lock (a sizing rule).
const NOT_COPIED: &[&str] = &["general", "visualLink", "lockAspect"];

/// Properties a painter leaves to the target: `(object, property)`, `*` for any object.
const KEPT: &[(&str, &str)] = &[
    ("title", "text"),
    ("subTitle", "text"),
    ("labels", "labelDisplayUnits"),
    ("labels", "labelPrecision"),
    ("categoryAxis", "start"),
    ("categoryAxis", "end"),
    ("valueAxis", "start"),
    ("valueAxis", "end"),
    ("valueAxis", "secStart"),
    ("valueAxis", "secEnd"),
    ("legend", "titleText"),
];

fn kept(object: &str, property: &str) -> bool {
    KEPT.iter()
        .any(|(o, p)| (*o == object || *o == "*") && *p == property)
}

/// The `Table.Field` names (lower case) a visual binds.
fn bound_fields(v: &Visual) -> HashSet<String> {
    v.fields
        .iter()
        .filter_map(|f| match &f.field {
            FieldRef::Column { table, column } | FieldRef::Aggregation { table, column, .. } => {
                Some(format!("{table}.{column}").to_lowercase())
            }
            FieldRef::Measure { table, measure } => Some(format!("{table}.{measure}").to_lowercase()),
            FieldRef::Other { .. } => None,
        })
        .collect()
}

/// The `Entity.Property` pairs a selector's `data` names (bounded depth).
fn selector_fields(v: &Value, depth: usize, out: &mut Vec<String>) {
    if depth > 24 {
        return;
    }
    match v {
        Value::Object(o) => {
            let entity = o
                .get("Expression")
                .and_then(|e| e.get("SourceRef"))
                .and_then(|s| s.get("Entity"))
                .and_then(Value::as_str);
            if let (Some(e), Some(p)) = (entity, o.get("Property").and_then(Value::as_str)) {
                out.push(format!("{e}.{p}").to_lowercase());
            }
            for x in o.values() {
                selector_fields(x, depth + 1, out);
            }
        }
        Value::Array(a) => {
            for x in a {
                selector_fields(x, depth + 1, out);
            }
        }
        _ => {}
    }
}

/// The field a `metadata` selector names (`Sum(T.C)` or `T.C`), lower case.
fn metadata_field(m: &str) -> String {
    let inner = match (m.find('('), m.rfind(')')) {
        (Some(a), Some(b)) if a < b => m.get(a + 1..b).unwrap_or(m),
        _ => m,
    };
    inner.trim().to_lowercase()
}

/// Whether an entry's selector names data (fields or values) rather than a state or nothing.
fn field_scoped(entry: &Value) -> bool {
    entry
        .get("selector")
        .and_then(Value::as_object)
        .is_some_and(|s| s.contains_key("data") || s.contains_key("metadata"))
}

/// Whether a field-scoped entry fits the target: every field its selector names is bound there.
fn fits(entry: &Value, fields: &HashSet<String>) -> bool {
    let Some(sel) = entry.get("selector") else {
        return true;
    };
    let mut names = Vec::new();
    if let Some(d) = sel.get("data") {
        selector_fields(d, 0, &mut names);
    }
    if let Some(m) = sel.get("metadata").and_then(Value::as_str) {
        names.push(metadata_field(m));
    }
    !names.is_empty() && names.iter().all(|n| fields.contains(n))
}

/// Whether the target type has object `name` (a container object, or a visual object of its
/// catalogued type; an uncatalogued type takes container objects only).
fn target_has(ty: Option<&str>, name: &str, container: bool) -> bool {
    match ty {
        Some(t) if bicraft_visuals::visual(t).is_some() => {
            bicraft_visuals::object_in(t, name, container).is_some()
        }
        _ => {
            container
                && bicraft_visuals::container_objects()
                    .iter()
                    .any(|(n, _)| *n == name)
        }
    }
}

/// Whether the target type has property `prop` of object `name`.
fn target_prop(ty: Option<&str>, name: &str, container: bool, prop: &str) -> bool {
    match ty {
        Some(t) if bicraft_visuals::visual(t).is_some() => bicraft_visuals::object_in(t, name, container)
            .is_some_and(|o| o.properties.iter().any(|p| p.name == prop)),
        _ => {
            container
                && bicraft_visuals::container_objects()
                    .iter()
                    .any(|(n, ps)| *n == name && ps.contains(&prop))
        }
    }
}

/// `entry` with only the properties `keep` accepts; `None` when none is left.
fn filter_entry(entry: &Value, keep: impl Fn(&str) -> bool) -> Option<Value> {
    let o = entry.as_object()?;
    let props = o.get("properties").and_then(Value::as_object)?;
    let kept: Map<String, Value> = props
        .iter()
        .filter(|(k, _)| keep(k))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    if kept.is_empty() {
        return None;
    }
    let mut out = o.clone();
    out.insert("properties".into(), Value::Object(kept));
    Some(Value::Object(out))
}

/// Adds `entry` to `list`, merging its properties into an entry with the same selector.
fn push_entry(list: &mut Vec<Value>, entry: Value) {
    let sel = entry.get("selector").cloned();
    let same = list.iter_mut().find(|e| e.get("selector").cloned() == sel);
    match (same, entry.get("properties").and_then(Value::as_object)) {
        (Some(e), Some(props)) => {
            if let Some(dst) = e.get_mut("properties").and_then(Value::as_object_mut) {
                for (k, v) in props {
                    dst.insert(k.clone(), v.clone());
                }
            }
        }
        _ => list.push(entry),
    }
}

/// The target's `key` objects (`objects` or `visualContainerObjects`) once painted with the
/// source's, and the names of the objects that changed.
fn paint_group(src: &Visual, dst: &Visual, key: &str) -> (Option<Map<String, Value>>, Vec<String>) {
    let container = key == "visualContainerObjects";
    let ty = dst.visual_type.as_deref();
    let empty = Map::new();
    let s = src
        .visual_extra
        .get(key)
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let d = dst
        .visual_extra
        .get(key)
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let fields = bound_fields(dst);
    let mut out = d.clone();
    let mut changed = Vec::new();
    let names: Vec<String> = s.keys().chain(d.keys()).cloned().collect::<Vec<_>>();
    let mut seen = HashSet::new();
    for name in names {
        if !seen.insert(name.clone()) || (container && NOT_COPIED.contains(&name.as_str())) {
            continue;
        }
        if !target_has(ty, &name, container) {
            continue;
        }
        let entries = |m: &Map<String, Value>| -> Vec<Value> {
            m.get(&name)
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        };
        let mut list: Vec<Value> = Vec::new();
        // the target keeps the properties a painter leaves alone, and its colours and labels
        // for its own fields unless the source has an entry for the same values
        for e in entries(d) {
            if field_scoped(&e) {
                list.push(e);
            } else if let Some(k) = filter_entry(&e, |p| kept(&name, p)) {
                list.push(k);
            }
        }
        for e in entries(s) {
            if field_scoped(&e) && !fits(&e, &fields) {
                continue;
            }
            if field_scoped(&e) {
                let sel = e.get("selector").cloned();
                list.retain(|x| x.get("selector").cloned() != sel);
            }
            if let Some(c) = filter_entry(&e, |p| !kept(&name, p) && target_prop(ty, &name, container, p)) {
                push_entry(&mut list, c);
            }
        }
        let before = d.get(&name).cloned();
        let after = if list.is_empty() {
            None
        } else {
            Some(Value::Array(list))
        };
        if before != after {
            changed.push(name.clone());
            match after {
                Some(a) => {
                    out.insert(name.clone(), a);
                }
                None => {
                    out.shift_remove(&name);
                }
            }
        }
    }
    if changed.is_empty() {
        return (None, changed);
    }
    (Some(out), changed)
}

/// `visual.copy_format {page, from, to, to_page?}`.
pub(crate) fn visual_copy_format(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "from", "to", "to_page"])?;
    let page_key = str_param(p, "page")?;
    let from_key = str_param(p, "from")?;
    let targets: Vec<String> = match p.get("to") {
        Some(Value::String(t)) => vec![t.clone()],
        Some(Value::Array(a)) if !a.is_empty() => a
            .iter()
            .map(|x| {
                x.as_str()
                    .map(str::to_string)
                    .ok_or("`to` must list visual names")
            })
            .collect::<Result<_, _>>()?,
        _ => return Err("`to` must be a visual name or a non-empty list of them".into()),
    };
    if targets.len() > MAX_TARGETS {
        return Err(format!("at most {MAX_TARGETS} targets"));
    }
    let to_page = match p.get("to_page") {
        None | Some(Value::Null) => None,
        Some(Value::String(t)) => Some(t.as_str()),
        Some(_) => return Err("`to_page` must be a page name".into()),
    };
    let (_, r) = parts(s)?;
    let pi = page_index(r, page_key)?;
    let page = r.pages.get(pi).ok_or("internal: page index out of range")?;
    let src = page
        .visuals
        .get(visual_index(page, from_key)?)
        .ok_or("internal: visual index out of range")?
        .clone();
    if src.visual_type.is_none() {
        return Err(format!(
            "`{}` is a group: pick a visual to copy the format from",
            src.name
        ));
    }
    let ti = match to_page {
        Some(t) => page_index(r, t)?,
        None => pi,
    };
    let tpage = r.pages.get_mut(ti).ok_or("internal: page index out of range")?;
    let page_name = tpage.name.clone();
    // check every target first: a failure changes nothing
    let mut idx = Vec::new();
    for t in &targets {
        let i = visual_index(tpage, t)?;
        let v = tpage
            .visuals
            .get(i)
            .ok_or("internal: visual index out of range")?;
        if v.visual_type.is_none() {
            return Err(format!(
                "`{}` is a group: the format painter applies to visuals",
                v.name
            ));
        }
        if !idx.contains(&i) && !(ti == pi && v.name == src.name) {
            idx.push(i);
        }
    }
    let mut changes = Vec::new();
    for i in idx {
        let Some(v) = tpage.visuals.get_mut(i) else {
            continue;
        };
        let mut objects = Vec::new();
        for key in ["objects", "visualContainerObjects"] {
            let (painted, names) = paint_group(&src, v, key);
            if let Some(m) = painted {
                if m.is_empty() {
                    v.visual_extra.shift_remove(key);
                } else {
                    v.visual_extra.insert(key.into(), Value::Object(m));
                }
            }
            objects.extend(names);
        }
        if !objects.is_empty() {
            changes.push(json!({"visual": v.name, "objects": objects}));
        }
    }
    let changed = !changes.is_empty();
    if changed {
        s.report_dirty = true;
    }
    Ok(json!({
        "page": page_name,
        "from": src.name,
        "changed": changed,
        "changes": changes,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_names_its_field() {
        assert_eq!(metadata_field("Sum(Sales.Amount)"), "sales.amount");
        assert_eq!(metadata_field("Sales.Amount"), "sales.amount");
        assert_eq!(metadata_field(")("), ")(");
    }

    #[test]
    fn selector_fields_are_bounded() {
        let mut v = json!({"Property": "x"});
        for _ in 0..100 {
            v = json!([v]);
        }
        let mut out = Vec::new();
        selector_fields(&v, 0, &mut out);
        assert!(out.is_empty());
    }
}
