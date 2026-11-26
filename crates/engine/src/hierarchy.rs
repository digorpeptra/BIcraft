//! User hierarchies (`hierarchy.add`, `hierarchy.remove`) and the expression rewrite that removes
//! a column from a calculated table. On a PBIX, `file.save` writes a new hierarchy's `U$` node
//! table (`bicraft_pbix::structural`); on PBIP/PBIT it is an ordinary model edit.

use bicraft_model::{SemanticModel, Table};
use serde_json::{Map, Value, json};

use crate::Session;
use crate::edit::{Edit, apply, check_name, new_guid, table_index};
use crate::params::{allow, opt_bool, opt_str, str_param};

/// Most levels of one hierarchy.
const MAX_LEVELS: usize = 64;

fn hierarchies(t: &Table) -> &[Value] {
    t.extra
        .get("hierarchies")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

/// TOM's `isAvailableInMdx` (legacy spelling `isAvailableInMDX`), true by default.
fn available_in_mdx(c: &bicraft_model::Column) -> bool {
    c.extra
        .get("isAvailableInMdx")
        .or_else(|| c.extra.get("isAvailableInMDX"))
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

fn hierarchy_name(h: &Value) -> &str {
    h.get("name").and_then(Value::as_str).unwrap_or_default()
}

/// `(table, hierarchy, [(level, column)])`.
pub(crate) type Levels = (String, String, Vec<(String, String)>);

/// A user hierarchy's levels in ordinal order: `(table, hierarchy, [(level, column)])` with the
/// model's spellings, found case-insensitively; `None` when `table` has no such hierarchy.
pub(crate) fn levels(model: &SemanticModel, table: &str, hierarchy: &str) -> Option<Levels> {
    let t = model.table(table)?;
    let h = hierarchies(t)
        .iter()
        .find(|h| hierarchy_name(h).eq_ignore_ascii_case(hierarchy))?;
    let mut out: Vec<(i64, String, String)> = h
        .get("levels")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(MAX_LEVELS)
        .enumerate()
        .filter_map(|(i, l)| {
            let name = l.get("name")?.as_str()?.to_string();
            let column = l
                .get("column")
                .and_then(Value::as_str)
                .map_or_else(|| name.clone(), str::to_string);
            let ordinal = l
                .get("ordinal")
                .and_then(Value::as_i64)
                .unwrap_or_else(|| i64::try_from(i).unwrap_or(i64::MAX));
            Some((ordinal, name, column))
        })
        .collect();
    out.sort_by_key(|(o, _, _)| *o);
    Some((
        t.name.clone(),
        hierarchy_name(h).to_string(),
        out.into_iter().map(|(_, n, c)| (n, c)).collect(),
    ))
}

/// The column of level `level` of hierarchy `hierarchy` of `table`.
pub(crate) fn level_column(
    model: &SemanticModel,
    table: &str,
    hierarchy: &str,
    level: &str,
) -> Option<String> {
    levels(model, table, hierarchy)?
        .2
        .into_iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(level))
        .map(|(_, c)| c)
}

/// `hierarchy.add`: a user hierarchy of `table` whose levels are columns of that table.
pub(crate) fn hierarchy_add(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "table",
            "name",
            "levels",
            "hidden",
            "description",
            "display_folder",
        ],
    )?;
    let table = str_param(p, "table")?;
    let name = str_param(p, "name")?.to_string();
    check_name("hierarchy", &name)?;
    let levels = p
        .get("levels")
        .and_then(Value::as_array)
        .filter(|l| !l.is_empty())
        .ok_or("`levels` must be a non-empty array of column names or {name?, column} objects")?;
    if levels.len() > MAX_LEVELS {
        return Err(format!("at most {MAX_LEVELS} levels"));
    }
    let hidden = opt_bool(p, "hidden")?.unwrap_or(false);
    let description = opt_str(p, "description")?.map(str::to_string);
    let folder = opt_str(p, "display_folder")?.map(str::to_string);
    let mut spec: Vec<(String, String)> = Vec::with_capacity(levels.len());
    for l in levels {
        let (level, column) = match l {
            Value::String(c) => (c.clone(), c.clone()),
            Value::Object(o) => {
                let column = o
                    .get("column")
                    .and_then(Value::as_str)
                    .ok_or("each level object needs a `column`")?;
                let level = o.get("name").and_then(Value::as_str).unwrap_or(column);
                (level.to_string(), column.to_string())
            }
            _ => return Err("each level is a column name or a {name?, column} object".into()),
        };
        check_name("level", &level)?;
        spec.push((level, column));
    }
    apply(s, p, |m: &mut SemanticModel| {
        let ti = table_index(m, table)?;
        let t = m.tables.get_mut(ti).ok_or("internal: table index out of range")?;
        if hierarchies(t)
            .iter()
            .any(|h| hierarchy_name(h).eq_ignore_ascii_case(&name))
        {
            return Err(format!(
                "table `{}` already has a hierarchy named `{name}`",
                t.name
            ));
        }
        let mut out_levels = Vec::with_capacity(spec.len());
        for (i, (level, column)) in spec.iter().enumerate() {
            let c = t
                .columns
                .iter()
                .find(|c| !c.is_row_number() && c.name.eq_ignore_ascii_case(column))
                .ok_or_else(|| format!("no column `{column}` in table `{}`", t.name))?;
            if !available_in_mdx(c) {
                return Err(format!(
                    "column `{}` has no attribute hierarchy (isAvailableInMdx false) and cannot be a level",
                    c.name
                ));
            }
            let repeated = spec
                .iter()
                .take(i)
                .any(|(l, col)| l.eq_ignore_ascii_case(level) || col.eq_ignore_ascii_case(column));
            if repeated {
                return Err(format!("level `{level}` repeats a level name or column"));
            }
            out_levels.push(json!({
                "name": level,
                "ordinal": i,
                "column": c.name,
                "lineageTag": new_guid(),
            }));
        }
        let mut h = Map::new();
        h.insert("name".into(), json!(name));
        if let Some(d) = &description {
            h.insert("description".into(), json!(d));
        }
        if hidden {
            h.insert("isHidden".into(), json!(true));
        }
        if let Some(f) = &folder {
            h.insert("displayFolder".into(), json!(f));
        }
        h.insert("lineageTag".into(), json!(new_guid()));
        h.insert("levels".into(), Value::Array(out_levels.clone()));
        match t.extra.get_mut("hierarchies") {
            Some(Value::Array(list)) => list.push(Value::Object(h)),
            _ => {
                t.extra
                    .insert("hierarchies".into(), Value::Array(vec![Value::Object(h)]));
            }
        }
        Ok(Edit::new(json!({
            "table": t.name,
            "hierarchy": name,
            "levels": out_levels,
        })))
    })
}

/// `hierarchy.remove`.
pub(crate) fn hierarchy_remove(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["table", "name"])?;
    let table = str_param(p, "table")?;
    let name = str_param(p, "name")?.to_string();
    apply(s, p, |m: &mut SemanticModel| {
        let ti = table_index(m, table)?;
        let t = m.tables.get_mut(ti).ok_or("internal: table index out of range")?;
        let table_name = t.name.clone();
        let Some(Value::Array(list)) = t.extra.get_mut("hierarchies") else {
            return Err(format!("table `{table_name}` has no hierarchy named `{name}`"));
        };
        let before = list.len();
        list.retain(|h| !hierarchy_name(h).eq_ignore_ascii_case(&name));
        if list.len() == before {
            return Err(format!("table `{table_name}` has no hierarchy named `{name}`"));
        }
        if list.is_empty() {
            t.extra.remove("hierarchies");
        }
        // Perspective memberships go with it (`perspectives::drop_dangling`, in `apply`).
        Ok(Edit::new(json!({"table": table_name, "removed": name})))
    })
}

/// A calculated table's columns are inferred from its expression: removing one of them rewrites
/// the expression to select the others (`SELECTCOLUMNS(<expression>, "A", <source of A>, …)`),
/// else the engine would bring the column back on the next calculation (engine-observed,
/// Desktop 2.158). Returns the new expression, `None` when `t` is not a calculated table or
/// `column` is not one of its inferred columns.
pub(crate) fn calculated_table_without(t: &Table, column: &str) -> Option<String> {
    let [p] = t.partitions.as_slice() else {
        return None;
    };
    if !p.source_type.eq_ignore_ascii_case("calculated") {
        return None;
    }
    let inferred = |c: &bicraft_model::Column| {
        c.extra.get("type").and_then(Value::as_str) == Some("calculatedTableColumn")
    };
    if !t
        .columns
        .iter()
        .any(|c| inferred(c) && c.name.eq_ignore_ascii_case(column))
    {
        return None;
    }
    let expression = p.expression.as_deref()?.trim();
    let mut parts = vec![expression.to_string()];
    for c in t
        .columns
        .iter()
        .filter(|c| inferred(c) && !c.name.eq_ignore_ascii_case(column))
    {
        let source = c
            .source_column
            .as_deref()
            .map(str::trim)
            .filter(|s| s.ends_with(']'))
            .map_or_else(|| format!("[{}]", c.name.replace(']', "]]")), str::to_string);
        parts.push(format!("\"{}\", {source}", c.name.replace('"', "\"\"")));
    }
    if parts.len() < 2 {
        return None;
    }
    // The expression on its own lines: a trailing `//` comment must not swallow the rest.
    let first = parts.remove(0);
    Some(format!(
        "SELECTCOLUMNS(
{first}
, {})",
        parts.join(", ")
    ))
}
