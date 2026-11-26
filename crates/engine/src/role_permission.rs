//! `role.set_permission`: object-level security (OLS) of a role on a table or a column
//! (docs/formats/model-objects.md, "Object-level security"). TMSL keeps it as `roles[].tablePermissions[].metadataPermission`
//! and `…columnPermissions[{name, metadataPermission}]`, with `none` (the object is hidden from
//! the role's members) or `read` (visible); no value is the default, read. Microsoft Learn
//! ("Object-level security", Analysis Services tabular models) states the design-time rule kept
//! here: a table cannot be secured when that breaks a relationship chain (a table related to two
//! other tables).

use serde_json::{Map, Value, json};

use crate::Session;
use crate::edit::{Edit, apply, dax_column, dax_table, table_index};
use crate::params::{allow, opt_str, str_param};

/// The permission given: `Some("none" | "read")`, or `None` to remove it (the default, read).
fn permission(p: &Value) -> Result<Option<&'static str>, String> {
    match p.get("metadata_permission") {
        None => Err(
            "missing parameter `metadata_permission` (\"none\", \"read\", or null for the default)".into(),
        ),
        Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => match s.to_ascii_lowercase().as_str() {
            "none" => Ok(Some("none")),
            "read" => Ok(Some("read")),
            "default" | "" => Ok(None),
            _ => Err(format!(
                "unknown metadata_permission `{s}` (expected none, read, or null for the default)"
            )),
        },
        Some(_) => Err("parameter `metadata_permission` must be a string or null".into()),
    }
}

/// Sets or removes `metadataPermission` in `o`.
fn set(o: &mut Map<String, Value>, value: Option<&str>) {
    match value {
        Some(v) => {
            o.insert("metadataPermission".into(), json!(v));
        }
        None => {
            o.remove("metadataPermission");
        }
    }
}

/// `role.set_permission`.
pub(crate) fn role_set_permission(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["role", "table", "column", "metadata_permission"])?;
    let role = str_param(p, "role")?;
    let table = str_param(p, "table")?;
    let column = opt_str(p, "column")?;
    let value = permission(p)?;
    apply(s, p, |m| {
        let ti = table_index(m, table)?;
        let t = m.tables.get(ti).ok_or("internal: table index out of range")?;
        let table_name = t.name.clone();
        let column_name = match column {
            None => None,
            Some(c) => Some(
                t.columns
                    .iter()
                    .filter(|x| !x.is_row_number())
                    .find(|x| x.name.eq_ignore_ascii_case(c))
                    .map(|x| x.name.clone())
                    .ok_or_else(|| format!("no column `{c}` in table {}", dax_table(&table_name)))?,
            ),
        };
        if column_name.is_none() && value == Some("none") {
            let mut others: Vec<String> = m
                .relationships
                .iter()
                .filter_map(|r| {
                    if r.from_table.eq_ignore_ascii_case(&table_name) {
                        Some(r.to_table.to_lowercase())
                    } else if r.to_table.eq_ignore_ascii_case(&table_name) {
                        Some(r.from_table.to_lowercase())
                    } else {
                        None
                    }
                })
                .filter(|o| *o != table_name.to_lowercase())
                .collect();
            others.sort();
            others.dedup();
            if others.len() >= 2 {
                return Err(format!(
                    "table {} is related to {} other tables: securing it would break their relationship \
                     chain (relate those tables directly, or secure its columns instead)",
                    dax_table(&table_name),
                    others.len()
                ));
            }
        }
        let r = m
            .roles
            .iter_mut()
            .find(|r| r.name.eq_ignore_ascii_case(role))
            .ok_or_else(|| format!("no role named `{role}`"))?;
        let role_name = r.name.clone();
        let i = match r
            .table_permissions
            .iter()
            .position(|tp| tp.table.eq_ignore_ascii_case(&table_name))
        {
            Some(i) => i,
            None => {
                r.table_permissions.push(bicraft_model::TablePermission {
                    table: table_name.clone(),
                    ..Default::default()
                });
                r.table_permissions.len().saturating_sub(1)
            }
        };
        let tp = r
            .table_permissions
            .get_mut(i)
            .ok_or("internal: table permission out of range")?;
        match &column_name {
            None => set(&mut tp.extra, value),
            Some(c) => {
                let list = tp
                    .extra
                    .entry("columnPermissions")
                    .or_insert_with(|| Value::Array(Vec::new()));
                if !list.is_array() {
                    *list = Value::Array(Vec::new());
                }
                if let Some(a) = list.as_array_mut() {
                    let at = a.iter().position(|e| {
                        e.get("name")
                            .and_then(Value::as_str)
                            .is_some_and(|n| n.eq_ignore_ascii_case(c))
                    });
                    match (at, value) {
                        (Some(k), _) => {
                            if let Some(Value::Object(o)) = a.get_mut(k) {
                                o.insert("name".into(), json!(c));
                                set(o, value);
                                // an entry with nothing but its name says nothing
                                if o.keys().all(|k| k == "name") {
                                    a.remove(k);
                                }
                            }
                        }
                        (None, Some(v)) => a.push(json!({"name": c, "metadataPermission": v})),
                        (None, None) => {}
                    }
                    if a.is_empty() {
                        tp.extra.remove("columnPermissions");
                    }
                }
            }
        }
        // a table permission with nothing left goes
        if tp.filter_expression.is_none() && tp.extra.is_empty() {
            r.table_permissions.remove(i);
        }
        let object = match &column_name {
            Some(c) => dax_column(&table_name, c),
            None => dax_table(&table_name),
        };
        Ok(Edit::new(json!({
            "role": role_name,
            "table": table_name,
            "column": column_name,
            "object": object,
            "metadata_permission": value.unwrap_or("default"),
        })))
    })
}
