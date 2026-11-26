//! `hierarchy.set`: rename a user hierarchy or its levels, reorder or replace its levels, hide it,
//! change its description or display folder. A renamed hierarchy is followed where the model
//! names it (perspectives, culture translations, date variations' default hierarchy).
//!
//! On a PBIX, display properties are metadata; a new name, renamed levels or a new level layout
//! rebuild the hierarchy's stored node table (`U$`) through the structural writer, like
//! `hierarchy.add`.

use bicraft_model::SemanticModel;
use serde_json::{Value, json};

use crate::Session;
use crate::edit::{Edit, apply, check_name, new_guid, table_index};
use crate::params::{allow, change_str, opt_bool, opt_str, str_param};

/// Most levels of one hierarchy (as `hierarchy.add`).
const MAX_LEVELS: usize = 64;

/// TOM's `isAvailableInMdx` (legacy spelling `isAvailableInMDX`), true by default.
fn available_in_mdx(c: &bicraft_model::Column) -> bool {
    c.extra
        .get("isAvailableInMdx")
        .or_else(|| c.extra.get("isAvailableInMDX"))
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

fn text<'v>(v: &'v Value, key: &str) -> &'v str {
    v.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn same(v: &Value, key: &str, name: &str) -> bool {
    text(v, key).eq_ignore_ascii_case(name)
}

/// Renames hierarchy `old` of `table` where the model names it: perspectives, culture
/// translations and date variations' default hierarchy. Returns what was updated.
fn follow_rename(m: &mut SemanticModel, table: &str, old: &str, new: &str) -> Vec<Value> {
    let mut updated = Vec::new();
    let mut rename_in = |tables: Option<&mut Value>, what: &str, owner: &str| {
        for t in tables.and_then(Value::as_array_mut).into_iter().flatten() {
            if !same(t, "name", table) {
                continue;
            }
            for h in t
                .get_mut("hierarchies")
                .and_then(Value::as_array_mut)
                .into_iter()
                .flatten()
            {
                if same(h, "name", old)
                    && let Some(o) = h.as_object_mut()
                {
                    o.insert("name".into(), json!(new));
                    updated.push(json!({"kind": what, "name": owner}));
                }
            }
        }
    };
    for p in m
        .extra
        .get_mut("perspectives")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        let owner = text(p, "name").to_string();
        rename_in(p.get_mut("tables"), "perspective", &owner);
    }
    for c in m
        .extra
        .get_mut("cultures")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        let owner = text(c, "name").to_string();
        let tables = c
            .get_mut("translations")
            .and_then(|t| t.get_mut("model"))
            .and_then(|t| t.get_mut("tables"));
        rename_in(tables, "culture", &owner);
    }
    for t in &mut m.tables {
        for c in &mut t.columns {
            for v in c
                .extra
                .get_mut("variations")
                .and_then(Value::as_array_mut)
                .into_iter()
                .flatten()
            {
                if let Some(d) = v.get_mut("defaultHierarchy")
                    && same(d, "table", table)
                    && same(d, "hierarchy", old)
                    && let Some(o) = d.as_object_mut()
                {
                    o.insert("hierarchy".into(), json!(new));
                    updated.push(json!({"kind": "variation", "name": format!("{}[{}]", t.name, c.name)}));
                }
            }
        }
    }
    updated
}

/// `levels` of `hierarchy.set`: (level name, column) in order.
type LevelSpec = Vec<(Option<String>, String)>;

fn parse_levels(p: &Value) -> Result<Option<LevelSpec>, String> {
    match p.get("levels") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(list)) if !list.is_empty() && list.len() <= MAX_LEVELS => list
            .iter()
            .map(|l| match l {
                Value::String(c) => Ok((None, c.clone())),
                Value::Object(o) => {
                    let column = o
                        .get("column")
                        .and_then(Value::as_str)
                        .ok_or("each level object needs a `column`")?;
                    let name = o.get("name").and_then(Value::as_str).map(str::to_string);
                    if let Some(n) = &name {
                        check_name("level", n)?;
                    }
                    Ok((name, column.to_string()))
                }
                _ => Err("each level is a column name or a {name?, column} object".to_string()),
            })
            .collect::<Result<_, String>>()
            .map(Some),
        Some(_) => Err(format!(
            "`levels` must be a non-empty array of at most {MAX_LEVELS} column names or {{name?, column}} objects"
        )),
    }
}

fn parse_renames(p: &Value) -> Result<Vec<(String, String)>, String> {
    match p.get("rename_levels") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Object(o)) => o
            .iter()
            .map(|(k, v)| {
                let n = v
                    .as_str()
                    .ok_or("`rename_levels` maps old level names to new names (strings)")?;
                check_name("level", n)?;
                Ok((k.clone(), n.to_string()))
            })
            .collect(),
        Some(_) => Err("`rename_levels` must be an object {old level name: new name}".into()),
    }
}

/// `hierarchy.set`.
pub(crate) fn hierarchy_set(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "table",
            "name",
            "new_name",
            "levels",
            "rename_levels",
            "hidden",
            "description",
            "display_folder",
        ],
    )?;
    let table = str_param(p, "table")?;
    let name = str_param(p, "name")?.to_string();
    let new_name = opt_str(p, "new_name")?.map(str::to_string);
    if let Some(n) = &new_name {
        check_name("hierarchy", n)?;
    }
    let hidden = opt_bool(p, "hidden")?;
    let description = change_str(p, "description")?;
    let folder = change_str(p, "display_folder")?;
    let levels = parse_levels(p)?;
    let renames = parse_renames(p)?;
    if new_name.is_none()
        && levels.is_none()
        && renames.is_empty()
        && hidden.is_none()
        && description.is_none()
        && folder.is_none()
    {
        return Err(
            "hierarchy.set needs at least one of `new_name`, `levels`, `rename_levels`, `hidden`, \
             `description`, `display_folder`"
                .into(),
        );
    }
    apply(s, p, |m: &mut SemanticModel| {
        let ti = table_index(m, table)?;
        let t = m.tables.get_mut(ti).ok_or("internal: table index out of range")?;
        let table_name = t.name.clone();
        let columns: Vec<(String, bool)> = t
            .columns
            .iter()
            .filter(|c| !c.is_row_number())
            .map(|c| (c.name.clone(), available_in_mdx(c)))
            .collect();
        let missing = || format!("table `{table_name}` has no hierarchy named `{name}`");
        let list = t
            .extra
            .get_mut("hierarchies")
            .and_then(Value::as_array_mut)
            .ok_or_else(missing)?;
        let hi = list
            .iter()
            .position(|h| same(h, "name", &name))
            .ok_or_else(missing)?;
        if let Some(n) = &new_name
            && list
                .iter()
                .enumerate()
                .any(|(j, h)| j != hi && same(h, "name", n))
        {
            return Err(format!(
                "table `{table_name}` already has a hierarchy named `{n}`"
            ));
        }
        let h = list
            .get_mut(hi)
            .and_then(Value::as_object_mut)
            .ok_or("internal: hierarchy is not an object")?;
        let old_name = h
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let old_levels: Vec<Value> = h
            .get("levels")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut sorted = old_levels.clone();
        sorted.sort_by_key(|l| l.get("ordinal").and_then(Value::as_i64).unwrap_or(i64::MAX));
        // 1. The level layout: kept levels (matched by column) keep their lineage tag and
        //    properties.
        let mut new_levels: Vec<Value> = match &levels {
            None => sorted,
            Some(spec) => {
                let mut out = Vec::with_capacity(spec.len());
                for (lname, column) in spec {
                    let (col, mdx) = columns
                        .iter()
                        .find(|(c, _)| c.eq_ignore_ascii_case(column))
                        .ok_or_else(|| format!("no column `{column}` in table `{table_name}`"))?;
                    if !mdx {
                        return Err(format!(
                            "column `{col}` has no attribute hierarchy (isAvailableInMdx false) and cannot be a level"
                        ));
                    }
                    let mut level = sorted
                        .iter()
                        .find(|l| same(l, "column", col))
                        .cloned()
                        .unwrap_or_else(|| json!({"name": col, "column": col, "lineageTag": new_guid()}));
                    if let Some(o) = level.as_object_mut() {
                        o.insert("column".into(), json!(col));
                        if let Some(n) = lname {
                            o.insert("name".into(), json!(n));
                        }
                    }
                    out.push(level);
                }
                out
            }
        };
        // 2. Level renames.
        for (old, new) in &renames {
            let l = new_levels
                .iter_mut()
                .find(|l| same(l, "name", old))
                .ok_or_else(|| format!("hierarchy `{old_name}` has no level named `{old}`"))?;
            if let Some(o) = l.as_object_mut() {
                o.insert("name".into(), json!(new));
            }
        }
        for (i, l) in new_levels.iter().enumerate() {
            let repeated = new_levels
                .iter()
                .take(i)
                .any(|x| same(x, "name", text(l, "name")) || same(x, "column", text(l, "column")));
            if repeated {
                return Err(format!(
                    "level `{}` repeats a level name or column",
                    text(l, "name")
                ));
            }
        }
        for (i, l) in new_levels.iter_mut().enumerate() {
            if let Some(o) = l.as_object_mut() {
                o.insert("ordinal".into(), json!(i));
            }
        }
        let levels_changed = new_levels != old_levels;
        h.insert("levels".into(), Value::Array(new_levels.clone()));
        if let Some(n) = &new_name {
            h.insert("name".into(), json!(n));
        }
        if let Some(v) = hidden {
            if v {
                h.insert("isHidden".into(), json!(true));
            } else {
                h.remove("isHidden");
            }
        }
        for (key, value) in [("description", &description), ("displayFolder", &folder)] {
            match value {
                None => {}
                Some(None) => {
                    h.remove(key);
                }
                Some(Some(v)) => {
                    h.insert(key.into(), json!(v));
                }
            }
        }
        let current = h
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let mut followed = if current != old_name {
            follow_rename(m, &table_name, &old_name, &current)
        } else {
            Vec::new()
        };
        // Renamed levels (matched by column) keep their translations (M10.4).
        let level_renames: Vec<(String, String)> = old_levels
            .iter()
            .filter_map(|o| {
                let n = new_levels.iter().find(|l| same(l, "column", text(o, "column")))?;
                (text(o, "name") != text(n, "name"))
                    .then(|| (text(o, "name").to_string(), text(n, "name").to_string()))
            })
            .collect();
        followed.extend(crate::translations::rename_levels(
            m,
            &table_name,
            &current,
            &level_renames,
        ));
        let mut edit = Edit::new(json!({
            "table": table_name,
            "hierarchy": current,
            "levels": new_levels,
            "levels_changed": levels_changed,
            "metadata_updated": followed,
        }));
        if current != old_name || levels_changed {
            edit.notes
                .push("report visuals bound to the hierarchy or its levels by name are not rewritten".into());
        }
        Ok(edit)
    })
}
