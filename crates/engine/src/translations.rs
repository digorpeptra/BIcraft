//! Model translations (M10.4): cultures and their translated captions, descriptions and display
//! folders (`docs/formats/translations.md`).
//!
//! A culture is a TMSL `cultures[]` entry kept in the model's `extra`: `{name, translations?:
//! {model: {name, translatedCaption?, …, tables: [{name, …, columns, measures, hierarchies:
//! [{…, levels}]}], perspectives, roles, expressions}}, linguisticMetadata?}`. The commands here
//! edit that tree; linguistic metadata is never touched. Entries are kept in model order with
//! nothing empty left behind, which is how the TMDL and metadata readers return them, so a
//! save → reopen round trip is stable.
//!
//! Reads apply a culture through [`Captions`]: the translated caption (or the name), and the
//! translated description and display folder where there is one.

use std::collections::{HashMap, HashSet};

use bicraft_model::SemanticModel;
use bicraft_report::{FieldRef, Visual};
use serde_json::{Map, Value, json};

use crate::Session;
use crate::commands::model;
use crate::edit::{Edit, apply};
use crate::params::{allow, opt_str, str_param};

type Obj = Map<String, Value>;

/// Translatable properties: (parameter / read name, TMSL key).
pub(crate) const PROPERTIES: [(&str, &str); 3] = [
    ("caption", "translatedCaption"),
    ("description", "translatedDescription"),
    ("display_folder", "translatedDisplayFolder"),
];

/// Child collections of translation entries, in the order the readers write them.
const CHILDREN: [&str; 8] = [
    "tables",
    "columns",
    "measures",
    "hierarchies",
    "levels",
    "perspectives",
    "roles",
    "expressions",
];

/// Object kinds of a translation path, with the collection that holds each.
const KINDS: [(&str, &str); 8] = [
    ("table", "tables"),
    ("column", "columns"),
    ("measure", "measures"),
    ("hierarchy", "hierarchies"),
    ("level", "levels"),
    ("perspective", "perspectives"),
    ("role", "roles"),
    ("expression", "expressions"),
];

/// Deepest translation tree walked (model → table → hierarchy → level is 4).
const MAX_DEPTH: usize = 8;

fn collection_of(kind: &str) -> &'static str {
    KINDS
        .iter()
        .find(|(k, _)| *k == kind)
        .map_or("tables", |(_, c)| c)
}

fn kind_of(collection: &str) -> Option<&'static str> {
    KINDS.iter().find(|(_, c)| *c == collection).map(|(k, _)| *k)
}

fn eq(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn name_of(v: &Value) -> &str {
    v.get("name").and_then(Value::as_str).unwrap_or_default()
}

fn names(list: Option<&Value>) -> Vec<String> {
    list.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|v| name_of(v).to_string())
        .collect()
}

fn cultures(m: &SemanticModel) -> &[Value] {
    m.extra
        .get("cultures")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn culture_index(m: &SemanticModel, name: &str) -> Option<usize> {
    cultures(m).iter().position(|c| eq(name_of(c), name))
}

fn culture_names(m: &SemanticModel) -> String {
    let list: Vec<&str> = cultures(m).iter().map(name_of).collect();
    if list.is_empty() {
        "none".into()
    } else {
        list.join(", ")
    }
}

fn no_culture(m: &SemanticModel, name: &str) -> String {
    format!(
        "the model has no culture `{name}` (cultures: {}; add one with culture.add)",
        culture_names(m)
    )
}

// ---------------------------------------------------------------------------------------------
// Addressing objects
// ---------------------------------------------------------------------------------------------

/// One step of a translation path: the object kind, its name as the model spells it, and the
/// model-order names of its siblings (where a new entry goes).
#[derive(Debug, Clone)]
struct Step {
    kind: &'static str,
    name: String,
    siblings: Vec<String>,
}

fn find<'a>(list: &'a [String], name: &str) -> Option<&'a String> {
    list.iter().find(|n| eq(n, name))
}

fn hierarchies_of(t: &bicraft_model::Table) -> Option<&Value> {
    t.extra.get("hierarchies")
}

/// Resolves `object` (`{}` for the model, `{table}`, `{table, column|measure|hierarchy}`,
/// `{table, hierarchy, level}`, `{measure}`, `{perspective}`, `{role}`, `{expression}`) to the
/// path of model objects it names.
fn resolve(m: &SemanticModel, object: Option<&Value>) -> Result<Vec<Step>, String> {
    let empty = Obj::new();
    let o = match object {
        None | Some(Value::Null) => &empty,
        Some(Value::Object(o)) => o,
        Some(_) => {
            return Err(
                "`object` must be an object such as {\"table\": \"Sales\", \"column\": \"Amount\"}".into(),
            );
        }
    };
    let mut given: HashMap<&str, &str> = HashMap::new();
    for (k, v) in o {
        let Some((kind, _)) = KINDS.iter().find(|(kind, _)| *kind == k) else {
            return Err(format!(
                "unknown key `{k}` in `object` (expected: table, column, measure, hierarchy, level, perspective, role, expression)"
            ));
        };
        let s = v
            .as_str()
            .ok_or_else(|| format!("`object.{k}` must be a string"))?;
        given.insert(kind, s);
    }
    let has = |k: &str| given.contains_key(k);
    let get = |k: &str| given.get(k).copied().unwrap_or_default();
    for lone in ["perspective", "role", "expression"] {
        if has(lone) {
            if given.len() > 1 {
                return Err(format!("`object.{lone}` cannot be combined with other keys"));
            }
            let (list, what): (Vec<String>, &str) = match lone {
                "perspective" => (names(m.extra.get("perspectives")), "perspective"),
                "role" => (m.roles.iter().map(|r| r.name.clone()).collect(), "role"),
                _ => (
                    m.expressions.iter().map(|e| e.name.clone()).collect(),
                    "shared expression",
                ),
            };
            let name = find(&list, get(lone))
                .ok_or_else(|| format!("the model has no {what} named `{}`", get(lone)))?
                .clone();
            return Ok(vec![Step {
                kind: lone,
                name,
                siblings: list,
            }]);
        }
    }
    let leaves = ["column", "measure", "hierarchy"]
        .iter()
        .filter(|k| has(k))
        .count();
    if leaves > 1 {
        return Err("`object` names one of column, measure or hierarchy".into());
    }
    if has("level") && !has("hierarchy") {
        return Err("`object.level` needs `object.hierarchy` (and `object.table`)".into());
    }
    let table_name = if has("table") {
        get("table").to_string()
    } else if has("measure") && given.len() == 1 {
        m.measure(get("measure"))
            .map(|(t, _)| t.name.clone())
            .ok_or_else(|| format!("the model has no measure named `{}`", get("measure")))?
    } else if given.is_empty() {
        return Ok(Vec::new());
    } else {
        return Err("`object` needs `table` (only a measure can be named without its table)".into());
    };
    let t = m
        .table(&table_name)
        .ok_or_else(|| format!("the model has no table named `{table_name}`"))?;
    let tables: Vec<String> = m.tables.iter().map(|t| t.name.clone()).collect();
    let mut path = vec![Step {
        kind: "table",
        name: t.name.clone(),
        siblings: tables,
    }];
    if has("column") {
        let list: Vec<String> = t
            .columns
            .iter()
            .filter(|c| !c.is_row_number())
            .map(|c| c.name.clone())
            .collect();
        let name = find(&list, get("column"))
            .ok_or_else(|| format!("no column '{}'[{}]", t.name, get("column")))?
            .clone();
        path.push(Step {
            kind: "column",
            name,
            siblings: list,
        });
    } else if has("measure") {
        let list: Vec<String> = t.measures.iter().map(|x| x.name.clone()).collect();
        let name = find(&list, get("measure"))
            .ok_or_else(|| format!("no measure '{}'[{}]", t.name, get("measure")))?
            .clone();
        path.push(Step {
            kind: "measure",
            name,
            siblings: list,
        });
    } else if has("hierarchy") {
        let list = names(hierarchies_of(t));
        let name = find(&list, get("hierarchy"))
            .ok_or_else(|| format!("table `{}` has no hierarchy named `{}`", t.name, get("hierarchy")))?
            .clone();
        path.push(Step {
            kind: "hierarchy",
            name: name.clone(),
            siblings: list,
        });
        if has("level") {
            let h = hierarchies_of(t)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .find(|h| eq(name_of(h), &name));
            let levels = names(h.and_then(|h| h.get("levels")));
            let level = find(&levels, get("level"))
                .ok_or_else(|| format!("hierarchy `{name}` has no level named `{}`", get("level")))?
                .clone();
            path.push(Step {
                kind: "level",
                name: level,
                siblings: levels,
            });
        }
    }
    Ok(path)
}

/// The `object` of a path, as translation.set takes it.
fn object_json(path: &[(&'static str, String)]) -> Value {
    let o: Obj = path.iter().map(|(k, n)| ((*k).to_string(), json!(n))).collect();
    Value::Object(o)
}

// ---------------------------------------------------------------------------------------------
// Editing the tree
// ---------------------------------------------------------------------------------------------

/// Puts `name` first and the translated properties next, as the readers do.
fn reorder(o: &mut Obj) {
    let mut out = Obj::new();
    if let Some(n) = o.remove("name") {
        out.insert("name".into(), n);
    }
    for (_, key) in PROPERTIES {
        if let Some(v) = o.remove(key) {
            out.insert(key.into(), v);
        }
    }
    let rest = std::mem::take(o);
    let (children, others): (Vec<_>, Vec<_>) = rest
        .into_iter()
        .partition(|(k, _)| CHILDREN.contains(&k.as_str()));
    out.extend(others);
    for key in CHILDREN {
        if let Some((k, v)) = children.iter().find(|(k, _)| k == key) {
            out.insert(k.clone(), v.clone());
        }
    }
    *o = out;
}

/// The entry of `step` under `parent`, created in model order when `create`.
fn child<'a>(parent: &'a mut Obj, step: &Step, create: bool) -> Result<Option<&'a mut Obj>, String> {
    let key = collection_of(step.kind);
    if !parent.contains_key(key) {
        if !create {
            return Ok(None);
        }
        parent.insert(key.into(), Value::Array(Vec::new()));
        reorder(parent);
    }
    let list = parent
        .get_mut(key)
        .and_then(Value::as_array_mut)
        .ok_or_else(|| format!("translations: `{key}` is not an array"))?;
    let pos = match list.iter().position(|e| eq(name_of(e), &step.name)) {
        Some(i) => i,
        None if !create => return Ok(None),
        None => {
            let order = |n: &str| step.siblings.iter().position(|s| eq(s, n));
            let mine = order(&step.name).unwrap_or(usize::MAX);
            let at = list
                .iter()
                .position(|e| order(name_of(e)).is_none_or(|o| o > mine))
                .unwrap_or(list.len());
            list.insert(at, json!({"name": step.name}));
            at
        }
    };
    list.get_mut(pos)
        .and_then(Value::as_object_mut)
        .map(Some)
        .ok_or_else(|| format!("translations: an entry of `{key}` is not an object"))
}

/// Drops entries that translate nothing; whether `o` still holds something besides its name.
fn prune_empty(o: &mut Obj, depth: usize) -> bool {
    if depth > MAX_DEPTH {
        return true;
    }
    for key in CHILDREN {
        let Some(list) = o.get_mut(key).and_then(Value::as_array_mut) else {
            continue;
        };
        list.retain_mut(|e| match e.as_object_mut() {
            Some(e) => prune_empty(e, depth.saturating_add(1)),
            None => true,
        });
        if list.is_empty() {
            o.remove(key);
        }
    }
    o.keys().any(|k| k != "name")
}

/// Drops the culture's empty translation entries, and `translations` itself when it is empty.
fn tidy_culture(c: &mut Obj) {
    let Some(tr) = c.get_mut("translations").and_then(Value::as_object_mut) else {
        return;
    };
    let keep_model = match tr.get_mut("model").and_then(Value::as_object_mut) {
        Some(model) => prune_empty(model, 0),
        None => true,
    };
    if !keep_model {
        tr.remove("model");
    }
    if tr.is_empty() {
        c.remove("translations");
    }
}

/// Sets (or with `None` removes) one translated property; whether the model changed.
fn set_translation(
    m: &mut SemanticModel,
    ci: usize,
    path: &[Step],
    key: &str,
    value: Option<&str>,
) -> Result<bool, String> {
    let model_name = m
        .extra
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("Model")
        .to_string();
    let culture = m
        .extra
        .get_mut("cultures")
        .and_then(Value::as_array_mut)
        .and_then(|l| l.get_mut(ci))
        .and_then(Value::as_object_mut)
        .ok_or("internal: culture index out of range")?;
    let create = value.is_some();
    if !culture.contains_key("translations") {
        if !create {
            return Ok(false);
        }
        culture.insert("translations".into(), json!({}));
    }
    let changed = {
        let tr = culture
            .get_mut("translations")
            .and_then(Value::as_object_mut)
            .ok_or("translations: not an object")?;
        if !tr.contains_key("model") {
            if !create {
                return Ok(false);
            }
            tr.insert("model".into(), json!({"name": model_name}));
        }
        let model = tr
            .get_mut("model")
            .and_then(Value::as_object_mut)
            .ok_or("translations.model: not an object")?;
        descend(model, path, key, value)?
    };
    tidy_culture(culture);
    Ok(changed)
}

/// Walks `path` down from `cur` (creating entries when setting) and sets or removes `key`.
fn descend(cur: &mut Obj, path: &[Step], key: &str, value: Option<&str>) -> Result<bool, String> {
    match path.split_first() {
        Some((step, rest)) => match child(cur, step, value.is_some())? {
            Some(next) => descend(next, rest, key, value),
            None => Ok(false),
        },
        None => Ok(match value {
            Some(v) => {
                let changed = cur.get(key).and_then(Value::as_str) != Some(v);
                cur.insert(key.into(), json!(v));
                reorder(cur);
                changed
            }
            None => cur.remove(key).is_some(),
        }),
    }
}

// ---------------------------------------------------------------------------------------------
// Walking the tree
// ---------------------------------------------------------------------------------------------

/// One translated property: the object path, the TMSL key and the value.
struct Entry {
    path: Vec<(&'static str, String)>,
    key: String,
    value: String,
}

fn walk(o: &Obj, path: &mut Vec<(&'static str, String)>, out: &mut Vec<Entry>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    for (_, key) in PROPERTIES {
        if let Some(v) = o.get(key).and_then(Value::as_str) {
            out.push(Entry {
                path: path.clone(),
                key: key.to_string(),
                value: v.to_string(),
            });
        }
    }
    for key in CHILDREN {
        let Some(kind) = kind_of(key) else { continue };
        for e in o.get(key).and_then(Value::as_array).into_iter().flatten() {
            let Some(e) = e.as_object() else { continue };
            path.push((kind, name_str(e).to_string()));
            walk(e, path, out, depth.saturating_add(1));
            path.pop();
        }
    }
}

fn entries(culture: &Value) -> Vec<Entry> {
    let mut out = Vec::new();
    if let Some(model) = culture
        .get("translations")
        .and_then(|t| t.get("model"))
        .and_then(Value::as_object)
    {
        walk(model, &mut Vec::new(), &mut out, 0);
    }
    out
}

fn param_name(key: &str) -> &str {
    PROPERTIES.iter().find(|(_, k)| *k == key).map_or(key, |(p, _)| p)
}

// ---------------------------------------------------------------------------------------------
// Keeping translations attached (renames) and dropping them (removals)
// ---------------------------------------------------------------------------------------------

/// The names of the model objects translations can name (lowercased).
#[derive(Default)]
struct Names {
    tables: HashMap<String, TableNames>,
    perspectives: HashSet<String>,
    roles: HashSet<String>,
    expressions: HashSet<String>,
}

#[derive(Default)]
struct TableNames {
    columns: HashSet<String>,
    measures: HashSet<String>,
    hierarchies: HashMap<String, HashSet<String>>,
}

fn lower(s: &str) -> String {
    s.to_ascii_lowercase()
}

impl Names {
    fn of(m: &SemanticModel) -> Names {
        let mut n = Names::default();
        for t in &m.tables {
            let mut tn = TableNames {
                columns: t.columns.iter().map(|c| lower(&c.name)).collect(),
                measures: t.measures.iter().map(|x| lower(&x.name)).collect(),
                hierarchies: HashMap::new(),
            };
            for h in hierarchies_of(t).and_then(Value::as_array).into_iter().flatten() {
                tn.hierarchies.insert(
                    lower(name_of(h)),
                    names(h.get("levels")).iter().map(|l| lower(l)).collect(),
                );
            }
            n.tables.insert(lower(&t.name), tn);
        }
        n.perspectives = names(m.extra.get("perspectives"))
            .iter()
            .map(|p| lower(p))
            .collect();
        n.roles = m.roles.iter().map(|r| lower(&r.name)).collect();
        n.expressions = m.expressions.iter().map(|e| lower(&e.name)).collect();
        n
    }
}

/// Removes the entries of `o[key]` that `keep` rejects, recording them in `dropped`.
fn retain(
    o: &mut Obj,
    key: &str,
    mut keep: impl FnMut(&mut Obj) -> bool,
    dropped: &mut Vec<String>,
    what: &str,
) {
    let Some(list) = o.get_mut(key).and_then(Value::as_array_mut) else {
        return;
    };
    list.retain_mut(|e| match e.as_object_mut() {
        Some(e) => {
            let ok = keep(e);
            if !ok {
                dropped.push(format!("{what} `{}`", name_str(e)));
            }
            ok
        }
        None => true,
    });
}

/// Drops the translations of objects that are no longer in the model (after a removal).
/// Returns `{culture, objects}` per culture that lost some.
pub(crate) fn drop_dangling(m: &mut SemanticModel) -> Vec<Value> {
    if cultures(m).is_empty() {
        return Vec::new();
    }
    let n = Names::of(m);
    let mut out = Vec::new();
    for c in m
        .extra
        .get_mut("cultures")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        let culture = name_of(c).to_string();
        let Some(co) = c.as_object_mut() else { continue };
        let Some(model) = co
            .get_mut("translations")
            .and_then(|t| t.get_mut("model"))
            .and_then(Value::as_object_mut)
        else {
            continue;
        };
        let mut dropped = Vec::new();
        let mut inner = Vec::new();
        let mut levels_dropped = Vec::new();
        retain(
            model,
            "tables",
            |t| {
                let tname = t
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let Some(tn) = n.tables.get(&lower(&tname)) else {
                    return false;
                };
                retain(
                    t,
                    "columns",
                    |c| tn.columns.contains(&lower(name_str(c))),
                    &mut inner,
                    &format!("column of `{tname}`"),
                );
                retain(
                    t,
                    "measures",
                    |x| tn.measures.contains(&lower(name_str(x))),
                    &mut inner,
                    &format!("measure of `{tname}`"),
                );
                retain(
                    t,
                    "hierarchies",
                    |h| {
                        let hname = name_str(h).to_string();
                        let Some(levels) = tn.hierarchies.get(&lower(&hname)) else {
                            return false;
                        };
                        retain(
                            h,
                            "levels",
                            |l| levels.contains(&lower(name_str(l))),
                            &mut levels_dropped,
                            &format!("level of `{tname}`[{hname}]"),
                        );
                        true
                    },
                    &mut inner,
                    &format!("hierarchy of `{tname}`"),
                );
                true
            },
            &mut dropped,
            "table",
        );
        retain(
            model,
            "perspectives",
            |p| n.perspectives.contains(&lower(name_str(p))),
            &mut dropped,
            "perspective",
        );
        retain(
            model,
            "roles",
            |r| n.roles.contains(&lower(name_str(r))),
            &mut dropped,
            "role",
        );
        retain(
            model,
            "expressions",
            |e| n.expressions.contains(&lower(name_str(e))),
            &mut dropped,
            "expression",
        );
        dropped.extend(inner);
        dropped.extend(levels_dropped);
        if !dropped.is_empty() {
            tidy_culture(co);
            out.push(json!({"culture": culture, "objects": dropped}));
        }
    }
    out
}

fn name_str(o: &Obj) -> &str {
    o.get("name").and_then(Value::as_str).unwrap_or_default()
}

/// Renames entries of the translation tables of every culture. `select` picks, from one
/// culture's `translations.model`, the arrays whose entries are renamed by `map` (old → new,
/// matched case-insensitively). Returns the cultures changed.
fn rename_in_cultures(
    m: &mut SemanticModel,
    select: impl Fn(&mut Obj) -> Vec<&mut Vec<Value>>,
    map: &[(String, String)],
) -> Vec<Value> {
    let mut out = Vec::new();
    for c in m
        .extra
        .get_mut("cultures")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        let culture = name_of(c).to_string();
        let Some(model) = c
            .get_mut("translations")
            .and_then(|t| t.get_mut("model"))
            .and_then(Value::as_object_mut)
        else {
            continue;
        };
        let mut changed = false;
        for list in select(model) {
            for e in list.iter_mut() {
                let Some(o) = e.as_object_mut() else { continue };
                if let Some((_, new)) = map.iter().find(|(old, _)| eq(old, name_str(o)))
                    && name_str(o) != new
                {
                    o.insert("name".into(), json!(new));
                    changed = true;
                }
            }
        }
        if changed {
            out.push(json!({"kind": "translations", "culture": culture}));
        }
    }
    out
}

fn array_mut<'a>(o: &'a mut Obj, key: &str) -> Option<&'a mut Vec<Value>> {
    o.get_mut(key).and_then(Value::as_array_mut)
}

/// Keeps the translations of a renamed shared expression attached.
pub(crate) fn rename_expression(m: &mut SemanticModel, old: &str, new: &str) -> Vec<Value> {
    let map = [(old.to_string(), new.to_string())];
    rename_in_cultures(
        m,
        |model| array_mut(model, "expressions").into_iter().collect(),
        &map,
    )
}

/// Keeps the translations of renamed levels of `table`.`hierarchy` attached (`map`: old → new
/// level names, applied at once so swaps work).
pub(crate) fn rename_levels(
    m: &mut SemanticModel,
    table: &str,
    hierarchy: &str,
    map: &[(String, String)],
) -> Vec<Value> {
    if map.is_empty() {
        return Vec::new();
    }
    rename_in_cultures(
        m,
        |model| {
            let mut out = Vec::new();
            for t in array_mut(model, "tables").into_iter().flatten() {
                let Some(t) = t.as_object_mut() else { continue };
                if !eq(name_str(t), table) {
                    continue;
                }
                for h in array_mut(t, "hierarchies").into_iter().flatten() {
                    let Some(h) = h.as_object_mut() else { continue };
                    if eq(name_str(h), hierarchy)
                        && let Some(l) = array_mut(h, "levels")
                    {
                        out.push(l);
                    }
                }
            }
            out
        },
        map,
    )
}

// ---------------------------------------------------------------------------------------------
// Reads in a culture
// ---------------------------------------------------------------------------------------------

/// The translations of one culture, by object path (lowercased `kind:name` steps).
#[derive(Debug, Default)]
pub(crate) struct Captions {
    pub culture: String,
    map: HashMap<String, HashMap<&'static str, String>>,
}

fn path_key(path: &[(&str, &str)]) -> String {
    let mut k = String::new();
    for (kind, name) in path {
        k.push_str(kind);
        k.push(':');
        k.push_str(&lower(name));
        k.push('\u{0}');
    }
    k
}

impl Captions {
    /// The translations of `culture` (case-insensitive). The model's own culture without a
    /// `cultures` entry gives no translations; any other unknown culture is an error.
    pub(crate) fn of(m: &SemanticModel, culture: &str) -> Result<Captions, String> {
        let mut out = Captions {
            culture: culture.to_string(),
            map: HashMap::new(),
        };
        let Some(ci) = culture_index(m, culture) else {
            if m.culture.as_deref().is_some_and(|c| eq(c, culture)) {
                return Ok(out);
            }
            return Err(no_culture(m, culture));
        };
        let Some(c) = cultures(m).get(ci) else {
            return Ok(out);
        };
        out.culture = name_of(c).to_string();
        for e in entries(c) {
            let path: Vec<(&str, &str)> = e.path.iter().map(|(k, n)| (*k, n.as_str())).collect();
            let Some((prop, _)) = PROPERTIES.iter().find(|(_, k)| *k == e.key) else {
                continue;
            };
            out.map.entry(path_key(&path)).or_default().insert(prop, e.value);
        }
        Ok(out)
    }

    /// The translated `prop` (`caption`, `description`, `display_folder`) of the object at
    /// `path` (`[]` is the model).
    pub(crate) fn get(&self, path: &[(&str, &str)], prop: &str) -> Option<&str> {
        self.map.get(&path_key(path))?.get(prop).map(String::as_str)
    }

    /// Annotates the read object `o` at `path`: `caption` (translated, else the name),
    /// `translation: {caption, description, display_folder}` (null where none), and the
    /// translated description / display folder in place of the stored ones.
    pub(crate) fn apply(&self, o: &mut Value, path: &[(&str, &str)]) {
        let Some(obj) = o.as_object_mut() else { return };
        let name = obj
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let mut tr = Obj::new();
        for (prop, _) in PROPERTIES {
            tr.insert(prop.into(), json!(self.get(path, prop)));
        }
        obj.insert(
            "caption".into(),
            json!(self.get(path, "caption").unwrap_or(&name)),
        );
        for prop in ["description", "display_folder"] {
            if let Some(v) = self.get(path, prop)
                && obj.contains_key(prop)
            {
                obj.insert(prop.into(), json!(v));
            }
        }
        obj.insert("translation".into(), Value::Object(tr));
    }

    /// [`Captions::apply`] on a `table.get` object and its columns, measures, hierarchies and
    /// levels.
    pub(crate) fn apply_table(&self, t: &mut Value) {
        let tname = t
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        self.apply(t, &[("table", &tname)]);
        for (key, kind) in [("columns", "column"), ("measures", "measure")] {
            for x in t.get_mut(key).and_then(Value::as_array_mut).into_iter().flatten() {
                let n = name_of(x).to_string();
                self.apply(x, &[("table", &tname), (kind, &n)]);
            }
        }
        for h in t
            .get_mut("hierarchies")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            let hn = name_of(h).to_string();
            self.apply(h, &[("table", &tname), ("hierarchy", &hn)]);
            for l in h
                .get_mut("levels")
                .and_then(Value::as_array_mut)
                .into_iter()
                .flatten()
            {
                let ln = name_of(l).to_string();
                self.apply(l, &[("table", &tname), ("hierarchy", &hn), ("level", &ln)]);
            }
        }
    }

    /// A copy of `v` whose bound columns and measures carry their translated captions as names,
    /// for building labels and the auto title; `None` when no field is translated.
    pub(crate) fn translate_visual(&self, v: &Visual) -> Option<Visual> {
        let mut out: Option<Visual> = None;
        for (i, b) in v.fields.iter().enumerate() {
            let caption = |table: &str, kind: &str, name: &str| {
                self.get(&[("table", table), (kind, name)], "caption")
                    .map(str::to_string)
            };
            let field = match &b.field {
                FieldRef::Column { table, column } => {
                    caption(table, "column", column).map(|c| FieldRef::Column {
                        table: table.clone(),
                        column: c,
                    })
                }
                FieldRef::Measure { table, measure } => {
                    caption(table, "measure", measure).map(|c| FieldRef::Measure {
                        table: table.clone(),
                        measure: c,
                    })
                }
                FieldRef::Aggregation {
                    table,
                    column,
                    function,
                } => caption(table, "column", column).map(|c| FieldRef::Aggregation {
                    table: table.clone(),
                    column: c,
                    function: function.clone(),
                }),
                FieldRef::Other { .. } => None,
            };
            if let Some(f) = field
                && let Some(slot) = out.get_or_insert_with(|| v.clone()).fields.get_mut(i)
            {
                slot.field = f;
            }
        }
        out
    }

    /// The translated caption of a visual field's object, when there is one.
    fn field_caption(&self, f: &Value) -> Option<&str> {
        let s = |k: &str| f.get(k).and_then(Value::as_str);
        let (table, name) = (s("table")?, s("name")?);
        let kind = match s("kind")? {
            "measure" => "measure",
            "column" | "aggregation" => "column",
            _ => return None,
        };
        self.get(&[("table", table), (kind, name)], "caption")
    }

    /// Adds the translations to a viewer snapshot: `caption` on translated tables, columns,
    /// measures and visual fields (only where a translation exists, so a renderer can fall back
    /// to `name`), and the snapshot's `display_culture`.
    pub(crate) fn apply_snapshot(&self, snap: &mut Value) {
        if let Some(o) = snap.as_object_mut() {
            o.insert("display_culture".into(), json!(self.culture));
        }
        let put = |o: &mut Value, caption: Option<&str>| {
            if let (Some(c), Some(o)) = (caption, o.as_object_mut()) {
                o.insert("caption".into(), json!(c));
            }
        };
        for t in snap
            .pointer_mut("/model/tables")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            let tname = name_of(t).to_string();
            put(t, self.get(&[("table", &tname)], "caption"));
            for (key, kind) in [("columns", "column"), ("measures", "measure")] {
                for x in t.get_mut(key).and_then(Value::as_array_mut).into_iter().flatten() {
                    let n = name_of(x).to_string();
                    put(x, self.get(&[("table", &tname), (kind, &n)], "caption"));
                    if kind == "measure"
                        && let Some(f) = self.get(&[("table", &tname), (kind, &n)], "display_folder")
                        && let Some(o) = x.as_object_mut()
                    {
                        o.insert("display_folder".into(), json!(f));
                    }
                }
            }
        }
        for p in snap
            .pointer_mut("/report/pages")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            for v in p
                .get_mut("visuals")
                .and_then(Value::as_array_mut)
                .into_iter()
                .flatten()
            {
                // (role, name, caption) of the translated fields.
                let mut translated: Vec<(String, String, String)> = Vec::new();
                for f in v
                    .get_mut("fields")
                    .and_then(Value::as_array_mut)
                    .into_iter()
                    .flatten()
                {
                    let c = self.field_caption(f).map(str::to_string);
                    if let Some(c) = &c {
                        let s = |k: &str| f.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
                        translated.push((s("role"), s("name"), c.clone()));
                    }
                    put(f, c.as_deref());
                }
                if translated.is_empty() {
                    continue;
                }
                // Evaluated data columns are named after their field ("City", "Sum of Amount"):
                // the same name with the caption in place of the field's name.
                for c in v
                    .pointer_mut("/data/columns")
                    .and_then(Value::as_array_mut)
                    .into_iter()
                    .flatten()
                {
                    let (role, name) = (
                        c.get("role").and_then(Value::as_str).unwrap_or_default(),
                        c.get("name").and_then(Value::as_str).unwrap_or_default(),
                    );
                    let caption = translated
                        .iter()
                        .filter(|(r, _, _)| r == role)
                        .find_map(|(_, n, cap)| {
                            let prefix = name.strip_suffix(n.as_str())?;
                            (prefix.is_empty() || prefix.ends_with(' ')).then(|| format!("{prefix}{cap}"))
                        });
                    put(c, caption.as_deref());
                }
            }
        }
    }
}

/// The `culture` parameter of a read, resolved against the model.
pub(crate) fn captions_param(m: &SemanticModel, p: &Value) -> Result<Option<Captions>, String> {
    opt_str(p, "culture")?.map(|c| Captions::of(m, c)).transpose()
}

/// What `culture` does not do, said in every render warning or refusal about it.
const CULTURE_HINT: &str = "`culture` only picks the model's translated captions; use \
    `format_culture` for number and date formatting and `ui_language` for the generated text \
    (auto titles, (Blank), …)";

/// A well-formed culture name (`en`, `en-US`, `zh-Hans-CN`, `en_US`): letter-first segments of
/// 1 to 8 letters or digits, joined by `-` or `_`.
fn culture_name_shape(name: &str) -> bool {
    let mut parts = name.split(['-', '_']);
    let first_ok = parts
        .next()
        .is_some_and(|p| (2..=8).contains(&p.len()) && p.chars().all(|c| c.is_ascii_alphabetic()));
    name.len() <= 64
        && first_ok
        && parts.all(|p| (1..=8).contains(&p.len()) && p.chars().all(|c| c.is_ascii_alphanumeric()))
}

/// The `culture` of `report.render`: the culture whose translations to apply (`None`: the base
/// captions) and a warning. A well-formed name that is the model's own culture or has no
/// translation in the model renders the base captions, as Desktop shows untranslated captions;
/// only a malformed name is refused.
pub(crate) fn render_culture(
    m: &SemanticModel,
    name: &str,
) -> Result<(Option<String>, Option<String>), String> {
    let name = name.trim();
    if culture_index(m, name).is_some() {
        return Ok((Some(name.to_string()), None));
    }
    if !culture_name_shape(name) {
        return Err(format!(
            "`culture` `{name}` is not a culture name such as \"fr-FR\"; {CULTURE_HINT}"
        ));
    }
    let warning = if m.culture.as_deref().is_some_and(|c| eq(c, name)) {
        format!(
            "`{name}` is the model's own culture: the captions are the base names (translation \
             cultures: {}); {CULTURE_HINT}",
            culture_names(m)
        )
    } else {
        format!(
            "`{name}` is not a translation culture of this model (translation cultures: {}): \
             captions are shown untranslated; {CULTURE_HINT}",
            culture_names(m)
        )
    };
    Ok((None, Some(warning)))
}

// ---------------------------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------------------------

pub(crate) fn culture_add(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["name"])?;
    let name = str_param(p, "name")?;
    crate::model_props::check_culture("name", name)?;
    apply(s, p, |m| {
        if let Some(i) = culture_index(m, name) {
            let existing = cultures(m).get(i).map(name_of).unwrap_or_default();
            return Err(format!("the model already has the culture `{existing}`"));
        }
        let list = m
            .extra
            .entry("cultures")
            .or_insert_with(|| Value::Array(Vec::new()));
        let list = list
            .as_array_mut()
            .ok_or("the model's `cultures` is not an array")?;
        list.push(json!({"name": name}));
        Ok(Edit::new(json!({"culture": name, "cultures": list.len()})))
    })
}

pub(crate) fn culture_remove(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["name"])?;
    let name = str_param(p, "name")?;
    apply(s, p, |m| {
        let ci = culture_index(m, name).ok_or_else(|| no_culture(m, name))?;
        let model_culture = m.culture.clone();
        let list = m
            .extra
            .get_mut("cultures")
            .and_then(Value::as_array_mut)
            .ok_or("internal: no cultures")?;
        let removed = list.remove(ci);
        let translations = entries(&removed).len();
        let linguistic = removed.get("linguisticMetadata").is_some();
        if list.is_empty() {
            m.extra.remove("cultures");
        }
        let removed_name = name_of(&removed).to_string();
        let mut edit = Edit::new(json!({
            "removed": removed_name,
            "translations_removed": translations,
            "linguistic_metadata_removed": linguistic,
        }));
        if model_culture.as_deref().is_some_and(|c| eq(c, &removed_name)) && linguistic {
            edit.notes.push(format!(
                "`{removed_name}` is the model's culture: its Q&A linguistic schema was removed with it"
            ));
        }
        Ok(edit)
    })
}

pub(crate) fn translation_set(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["culture", "object", "property", "value"])?;
    let culture = str_param(p, "culture")?;
    let property = str_param(p, "property")?;
    let (_, key) = PROPERTIES.iter().find(|(n, _)| *n == property).ok_or_else(|| {
        format!("`property` must be caption, description or display_folder, got `{property}`")
    })?;
    let value = match p.get("value") {
        None => return Err("missing parameter `value` (a string, or null to remove the translation)".into()),
        Some(Value::Null) => None,
        Some(Value::String(v)) if v.is_empty() => None,
        Some(Value::String(v)) => Some(v.as_str()),
        Some(_) => return Err("`value` must be a string or null".into()),
    };
    apply(s, p, |m| {
        let ci = culture_index(m, culture).ok_or_else(|| no_culture(m, culture))?;
        let path = resolve(m, p.get("object"))?;
        let kind = path.last().map_or("model", |s| s.kind);
        if property == "display_folder" && !matches!(kind, "column" | "measure" | "hierarchy") {
            return Err(format!(
                "a {kind} has no display folder (display_folder translates columns, measures and hierarchies)"
            ));
        }
        let changed = set_translation(m, ci, &path, key, value)?;
        let culture_name = cultures(m).get(ci).map(name_of).unwrap_or(culture).to_string();
        let object: Vec<(&'static str, String)> = path.iter().map(|s| (s.kind, s.name.clone())).collect();
        Ok(Edit::new(json!({
            "culture": culture_name,
            "object": object_json(&object),
            "property": property,
            "value": value,
            "changed": changed,
        })))
    })
}

pub(crate) fn translation_list(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &["culture"])?;
    let m = model(s)?;
    let only = opt_str(p, "culture")?;
    if let Some(c) = only
        && culture_index(m, c).is_none()
    {
        return Err(no_culture(m, c));
    }
    let mut list = Vec::new();
    let mut summary = Vec::new();
    for c in cultures(m) {
        let name = name_of(c);
        let found = entries(c);
        summary.push(json!({
            "name": name,
            "model_culture": m.culture.as_deref().is_some_and(|x| eq(x, name)),
            "translations": found.len(),
            "linguistic_metadata": c.get("linguisticMetadata").is_some(),
        }));
        if only.is_some_and(|o| !eq(o, name)) {
            continue;
        }
        for e in found {
            list.push(json!({
                "culture": name,
                "object": object_json(&e.path),
                "property": param_name(&e.key),
                "value": e.value,
            }));
        }
    }
    Ok(json!({"cultures": summary, "count": list.len(), "translations": list}))
}

/// `view.set_culture`: the culture the live viewer and `report.render` show captions in.
pub(crate) fn view_set_culture(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["culture"])?;
    let culture = opt_str(p, "culture")?.filter(|c| !c.is_empty());
    if let Some(c) = culture {
        Captions::of(model(s)?, c)?;
    }
    s.display_culture = culture.map(str::to_string);
    Ok(json!({"display_culture": s.display_culture}))
}
