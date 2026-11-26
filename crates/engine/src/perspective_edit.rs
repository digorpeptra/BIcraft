//! `perspective.add`, `perspective.set`, `perspective.remove`: authoring the model's perspectives
//! (TMSL `perspectives[{name, description?, tables: [{name, columns: [{name}], measures: [{name}],
//! hierarchies: [{name}]}]}]`, TMDL `perspective` blocks; docs/formats/model-objects.md,
//! "Perspectives"). Members are named by name, so renames follow and removals drop them
//! ([`crate::perspectives`]). A table given with `all` lists every column, measure and hierarchy
//! it has now, one by one.

use std::collections::HashSet;

use bicraft_model::SemanticModel;
use serde_json::{Map, Value, json};

use crate::Session;
use crate::edit::{Edit, apply, check_name, dax_table, table_index};
use crate::params::{allow, change_str, opt_str, str_param};

/// Most perspectives a model takes, and most members a table lists.
const MAX_PERSPECTIVES: usize = 500;
const MAX_MEMBERS: usize = 100_000;

fn list(v: &Value) -> Option<&Vec<Value>> {
    v.as_array()
}

fn perspectives(m: &SemanticModel) -> &[Value] {
    m.extra
        .get("perspectives")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn index(m: &SemanticModel, name: &str) -> Option<usize> {
    perspectives(m).iter().position(|p| {
        p.get("name")
            .and_then(Value::as_str)
            .is_some_and(|n| n.eq_ignore_ascii_case(name))
    })
}

/// The names of `t[key]` the model has (canonical case), from a list of strings.
fn resolve(given: Option<&Value>, have: &[String], what: &str, table: &str) -> Result<Vec<String>, String> {
    let Some(given) = given else {
        return Ok(Vec::new());
    };
    let items = list(given).ok_or_else(|| format!("`{what}` of table `{table}` must be a list of names"))?;
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for i in items.iter().take(MAX_MEMBERS) {
        let n = i
            .as_str()
            .ok_or_else(|| format!("`{what}` of table `{table}` must be a list of names"))?;
        let found = have.iter().find(|h| h.eq_ignore_ascii_case(n)).ok_or_else(|| {
            format!(
                "table {} has no {} `{n}`",
                dax_table(table),
                what.trim_end_matches('s')
            )
        })?;
        if seen.insert(found.to_lowercase()) {
            out.push(found.clone());
        }
    }
    Ok(out)
}

fn named(names: &[String]) -> Value {
    Value::Array(names.iter().map(|n| json!({"name": n})).collect())
}

/// The TMSL `tables` of a perspective from the `tables` parameter: `[{table, all?, columns?,
/// measures?, hierarchies?}]` or a table name for the whole table.
fn tables(m: &SemanticModel, v: &Value) -> Result<Vec<Value>, String> {
    let items =
        list(v).ok_or("`tables` must be a list of {table, all?, columns?, measures?, hierarchies?}")?;
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for item in items.iter().take(MAX_MEMBERS) {
        let (name, all, spec) = match item {
            Value::String(s) => (s.as_str(), true, None),
            Value::Object(o) => {
                if let Some(k) = o
                    .keys()
                    .find(|k| !["table", "all", "columns", "measures", "hierarchies"].contains(&k.as_str()))
                {
                    return Err(format!(
                        "unknown key `{k}` in a `tables` entry (expected table, all, columns, measures, hierarchies)"
                    ));
                }
                let name = o
                    .get("table")
                    .and_then(Value::as_str)
                    .ok_or("each `tables` entry needs a string `table`")?;
                let all = match o.get("all") {
                    None | Some(Value::Null) => false,
                    Some(Value::Bool(b)) => *b,
                    Some(_) => return Err("`all` must be true or false".into()),
                };
                (name, all, Some(o))
            }
            _ => return Err("each `tables` entry is a table name or {table, ...}".into()),
        };
        let ti = table_index(m, name)?;
        let t = m.tables.get(ti).ok_or("internal: table index out of range")?;
        if !seen.insert(t.name.to_lowercase()) {
            return Err(format!("table {} is listed twice", dax_table(&t.name)));
        }
        let columns: Vec<String> = t
            .columns
            .iter()
            .filter(|c| !c.is_row_number())
            .map(|c| c.name.clone())
            .collect();
        let measures: Vec<String> = t.measures.iter().map(|x| x.name.clone()).collect();
        let hierarchies: Vec<String> = t
            .extra
            .get("hierarchies")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|h| h.get("name").and_then(Value::as_str).map(str::to_string))
            .collect();
        let (c, me, h) = if all {
            (columns, measures, hierarchies)
        } else {
            let get = |k: &str| spec.and_then(|o| o.get(k));
            (
                resolve(get("columns"), &columns, "columns", &t.name)?,
                resolve(get("measures"), &measures, "measures", &t.name)?,
                resolve(get("hierarchies"), &hierarchies, "hierarchies", &t.name)?,
            )
        };
        let mut o = Map::new();
        o.insert("name".into(), json!(t.name));
        for (k, names) in [("columns", c), ("measures", me), ("hierarchies", h)] {
            if !names.is_empty() {
                o.insert(k.into(), named(&names));
            }
        }
        out.push(Value::Object(o));
    }
    if items.len() > MAX_MEMBERS {
        return Err(format!("`tables` lists more than {MAX_MEMBERS} tables"));
    }
    Ok(out)
}

/// Counts of a perspective's members, for the results.
fn counts(tables: &[Value]) -> Value {
    let n = |k: &str| {
        tables
            .iter()
            .map(|t| t.get(k).and_then(Value::as_array).map_or(0, Vec::len))
            .sum::<usize>()
    };
    json!({"tables": tables.len(), "columns": n("columns"), "measures": n("measures"), "hierarchies": n("hierarchies")})
}

fn list_mut(m: &mut SemanticModel) -> Result<&mut Vec<Value>, String> {
    let v = m
        .extra
        .entry("perspectives")
        .or_insert_with(|| Value::Array(Vec::new()));
    if !v.is_array() {
        *v = Value::Array(Vec::new());
    }
    v.as_array_mut()
        .ok_or_else(|| "internal: perspectives is not a list".to_string())
}

/// `perspective.add`.
pub(crate) fn perspective_add(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["name", "description", "tables"])?;
    let name = str_param(p, "name")?;
    check_name("perspective", name)?;
    let description = opt_str(p, "description")?;
    apply(s, p, |m| {
        if let Some(i) = index(m, name) {
            let have = perspectives(m)
                .get(i)
                .and_then(|x| x.get("name"))
                .and_then(Value::as_str)
                .unwrap_or(name)
                .to_string();
            return Err(format!("a perspective named `{have}` already exists"));
        }
        if perspectives(m).len() >= MAX_PERSPECTIVES {
            return Err(format!(
                "a model takes at most {MAX_PERSPECTIVES} perspectives here"
            ));
        }
        let t = match p.get("tables") {
            None | Some(Value::Null) => Vec::new(),
            Some(v) => tables(m, v)?,
        };
        let mut o = Map::new();
        o.insert("name".into(), json!(name));
        if let Some(d) = description.filter(|d| !d.is_empty()) {
            o.insert("description".into(), json!(d));
        }
        let c = counts(&t);
        o.insert("tables".into(), Value::Array(t));
        list_mut(m)?.push(Value::Object(o));
        Ok(Edit::new(json!({"perspective": name, "members": c})))
    })
}

/// `perspective.set`.
pub(crate) fn perspective_set(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["name", "new_name", "description", "tables"])?;
    let name = str_param(p, "name")?;
    let new_name = opt_str(p, "new_name")?;
    if let Some(n) = new_name {
        check_name("perspective", n)?;
    }
    let description = change_str(p, "description")?;
    if new_name.is_none() && description.is_none() && p.get("tables").is_none() {
        return Err("perspective.set needs at least one of `new_name`, `description`, `tables`".into());
    }
    apply(s, p, |m| {
        let i = index(m, name).ok_or_else(|| format!("no perspective named `{name}`"))?;
        if let Some(n) = new_name
            && let Some(j) = index(m, n)
            && j != i
        {
            return Err(format!("a perspective named `{n}` already exists"));
        }
        let t = match p.get("tables") {
            None => None,
            Some(Value::Null) => Some(Vec::new()),
            Some(v) => Some(tables(m, v)?),
        };
        let persp = list_mut(m)?
            .get_mut(i)
            .and_then(Value::as_object_mut)
            .ok_or("internal: perspective index out of range")?;
        let old = persp
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(name)
            .to_string();
        if let Some(n) = new_name {
            persp.insert("name".into(), json!(n));
        }
        match description {
            Some(Some(d)) => {
                persp.insert("description".into(), json!(d));
            }
            Some(None) => {
                persp.remove("description");
            }
            None => {}
        }
        if let Some(t) = t {
            persp.insert("tables".into(), Value::Array(t));
        }
        let now: Vec<Value> = persp
            .get("tables")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut r = json!({"perspective": new_name.unwrap_or(&old), "members": counts(&now)});
        if new_name.is_some_and(|n| n != old)
            && let Value::Object(o) = &mut r
        {
            o.insert("renamed".into(), json!(old));
        }
        Ok(Edit::new(r))
    })
}

/// `perspective.remove`.
pub(crate) fn perspective_remove(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["name"])?;
    let name = str_param(p, "name")?;
    apply(s, p, |m| {
        let i = index(m, name).ok_or_else(|| format!("no perspective named `{name}`"))?;
        let list = list_mut(m)?;
        let gone = list.remove(i);
        if list.is_empty() {
            m.extra.remove("perspectives");
        }
        let n = gone
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(name)
            .to_string();
        Ok(Edit::new(json!({"removed": n})))
    })
}
