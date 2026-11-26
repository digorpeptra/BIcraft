//! Keeping perspectives consistent through removals.
//!
//! A perspective names its members by name: `perspectives[].tables[]` (a table) and, inside
//! each, `columns`, `measures` and `hierarchies`. Renames follow them ([`crate::rename_refs`]);
//! [`drop_dangling`] drops the members a removal left behind, since a perspective that names a
//! missing object does not load. Every model edit runs it ([`crate::edit::apply`]).

use std::collections::{HashMap, HashSet};

use bicraft_model::SemanticModel;
use serde_json::{Map, Value, json};

#[derive(Default)]
struct Members {
    columns: HashSet<String>,
    measures: HashSet<String>,
    hierarchies: HashSet<String>,
}

fn lower(s: &str) -> String {
    s.to_ascii_lowercase()
}

fn name_of(o: &Map<String, Value>) -> &str {
    o.get("name").and_then(Value::as_str).unwrap_or_default()
}

fn names(v: Option<&Value>) -> HashSet<String> {
    v.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|e| e.get("name").and_then(Value::as_str))
        .map(lower)
        .collect()
}

/// Removes the entries of `o[key]` whose name is not in `keep`, recording them in `dropped`;
/// an emptied list goes too.
fn retain(
    o: &mut Map<String, Value>,
    key: &str,
    keep: &HashSet<String>,
    what: &str,
    dropped: &mut Vec<String>,
) {
    let Some(list) = o.get_mut(key).and_then(Value::as_array_mut) else {
        return;
    };
    list.retain(|e| match e.as_object() {
        Some(e) => {
            let name = name_of(e);
            let ok = keep.contains(&lower(name));
            if !ok {
                dropped.push(format!("{what} `{name}`"));
            }
            ok
        }
        None => true,
    });
    if list.is_empty() {
        o.remove(key);
    }
}

/// Drops the perspective members that are no longer in the model (after a removal): whole
/// perspective tables for removed tables, and the columns, measures and hierarchies of the
/// tables that remain. Returns `{perspective, objects}` per perspective that lost some.
pub(crate) fn drop_dangling(m: &mut SemanticModel) -> Vec<Value> {
    let has_any = m
        .extra
        .get("perspectives")
        .and_then(Value::as_array)
        .is_some_and(|a| !a.is_empty());
    if !has_any {
        return Vec::new();
    }
    let tables: HashMap<String, Members> = m
        .tables
        .iter()
        .map(|t| {
            (
                lower(&t.name),
                Members {
                    columns: t.columns.iter().map(|c| lower(&c.name)).collect(),
                    measures: t.measures.iter().map(|x| lower(&x.name)).collect(),
                    hierarchies: names(t.extra.get("hierarchies")),
                },
            )
        })
        .collect();
    let mut out = Vec::new();
    for p in m
        .extra
        .get_mut("perspectives")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        let Some(po) = p.as_object_mut() else { continue };
        let perspective = name_of(po).to_string();
        let mut dropped = Vec::new();
        let mut inner = Vec::new();
        if let Some(list) = po.get_mut("tables").and_then(Value::as_array_mut) {
            list.retain_mut(|t| {
                let Some(t) = t.as_object_mut() else {
                    return true;
                };
                let tname = name_of(t).to_string();
                let Some(members) = tables.get(&lower(&tname)) else {
                    dropped.push(format!("table `{tname}`"));
                    return false;
                };
                for (key, keep, what) in [
                    ("columns", &members.columns, "column"),
                    ("measures", &members.measures, "measure"),
                    ("hierarchies", &members.hierarchies, "hierarchy"),
                ] {
                    retain(t, key, keep, &format!("{what} of `{tname}`"), &mut inner);
                }
                true
            });
        }
        dropped.extend(inner);
        if !dropped.is_empty() {
            out.push(json!({"perspective": perspective, "objects": dropped}));
        }
    }
    out
}
