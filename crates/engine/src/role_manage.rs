//! `role.rename` and `role.check_filter`, the Manage security roles dialog's rename and its check
//! (✓) of a row filter (docs/formats/model-objects.md, "Roles"). Microsoft Learn ("Row-level
//! security (RLS) with Power BI", Define roles) describes the dialog: a role name cannot contain a
//! comma, and the DAX editor's check mark validates the expression without saving it.

use bicraft_dax::{Context, ExprKind, Severity};
use serde_json::{Value, json};

use crate::Session;
use crate::commands::model;
use crate::edit::{Edit, apply, check_name, table_index};
use crate::params::{allow, change_str, opt_str, str_param};

/// Longest row filter checked (bytes).
const MAX_FILTER_BYTES: usize = 1 << 20;

/// A role name: [`check_name`] plus Desktop's rule that a role name has no comma.
pub(crate) fn check_role_name(name: &str) -> Result<(), String> {
    check_name("role", name)?;
    if name.contains(',') {
        return Err(format!("role name `{name}` must not contain a comma"));
    }
    Ok(())
}

/// `role.rename {name, new_name}`.
pub(crate) fn role_rename(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["name", "new_name"])?;
    let name = str_param(p, "name")?;
    let new_name = str_param(p, "new_name")?;
    check_role_name(new_name)?;
    let mut old = String::new();
    let out = apply(s, p, |m| {
        let i = m
            .roles
            .iter()
            .position(|r| r.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| format!("no role named `{name}`"))?;
        if let Some(r) = m
            .roles
            .iter()
            .enumerate()
            .find(|(j, r)| *j != i && r.name.eq_ignore_ascii_case(new_name))
        {
            return Err(format!("a role named `{}` already exists", r.1.name));
        }
        let r = m.roles.get_mut(i).ok_or("internal: role index out of range")?;
        old = std::mem::replace(&mut r.name, new_name.to_string());
        Ok(Edit::new(json!({"role": new_name, "old_name": old})))
    })?;
    // View as follows the rename (it names roles as the model spells them)
    s.interactions.rename_view_as_role(&old, new_name);
    Ok(out)
}

/// Longest role description (characters).
const MAX_DESCRIPTION: usize = 10_000;

/// `role.set {name, description?, model_permission?}`: the Properties pane's fields of a role
/// (TMSL `description` and `modelPermission`). One undo step.
pub(crate) fn role_set(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["name", "description", "model_permission"])?;
    let name = str_param(p, "name")?;
    let description = change_str(p, "description")?;
    if description
        .as_ref()
        .and_then(Option::as_deref)
        .is_some_and(|d| d.chars().count() > MAX_DESCRIPTION)
    {
        return Err(format!("a description has at most {MAX_DESCRIPTION} characters"));
    }
    let permission = opt_str(p, "model_permission")?
        .map(crate::edit::parse_model_permission)
        .transpose()?;
    if description.is_none() && permission.is_none() {
        return Err("role.set needs `description` or `model_permission`".into());
    }
    apply(s, p, |m| {
        let r = m
            .roles
            .iter_mut()
            .find(|r| r.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| format!("no role named `{name}`"))?;
        if let Some(d) = &description {
            match d {
                Some(d) => {
                    r.extra.insert("description".into(), json!(d));
                }
                None => {
                    r.extra.remove("description");
                }
            }
        }
        if let Some(mp) = &permission {
            r.model_permission = Some(mp.clone());
        }
        Ok(Edit::new(json!({
            "role": r.name,
            "description": r.extra.get("description"),
            "model_permission": r.model_permission,
        })))
    })
}

/// `role.check_filter {role?, table, filter_expression}`: the row filter validated as a
/// row-level security filter of `table` against the open model, nothing changed.
pub(crate) fn role_check_filter(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &["role", "table", "filter_expression"])?;
    let role = opt_str(p, "role")?;
    let table = str_param(p, "table")?;
    let filter = match p.get("filter_expression") {
        None | Some(Value::Null) => "",
        Some(Value::String(f)) => f.as_str(),
        Some(_) => return Err("parameter `filter_expression` must be a string or null".into()),
    };
    if filter.len() > MAX_FILTER_BYTES {
        return Err(format!("the filter is longer than {MAX_FILTER_BYTES} bytes"));
    }
    let m = model(s)?;
    let ti = table_index(m, table)?;
    let table_name = m.tables.get(ti).map(|t| t.name.clone()).unwrap_or_default();
    let role_name = match role {
        None => None,
        Some(r) => Some(
            m.roles
                .iter()
                .find(|x| x.name.eq_ignore_ascii_case(r))
                .map(|x| x.name.clone())
                .ok_or_else(|| format!("no role named `{r}`"))?,
        ),
    };
    if filter.trim().is_empty() {
        return Ok(json!({
            "valid": true, "empty": true, "role": role_name, "table": table_name,
            "errors": 0, "warnings": 0, "diagnostics": [],
        }));
    }
    let ctx = Context {
        model: Some(m),
        table: Some(&table_name),
    };
    let mut v = bicraft_dax::validate_as(filter, &ctx, ExprKind::RowFilter);
    // the query-only INFO functions are refused in a model object's DAX, as role.set_filter does
    let info = bicraft_dax::query_only_info_calls(filter);
    if !info.is_empty() {
        v.valid = false;
        v.diagnostics.extend(info);
    }
    let count = |sev: Severity| v.diagnostics.iter().filter(|d| d.severity == sev).count();
    Ok(json!({
        "valid": v.valid,
        "empty": false,
        "role": role_name,
        "table": table_name,
        "errors": count(Severity::Error),
        "warnings": count(Severity::Warning),
        "diagnostics": v.diagnostics,
    }))
}
