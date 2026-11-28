//! The field wells of a visual, as the app's Build pane edits them (docs/design.md §12):
//! `visual.move_field` (reorder inside a role, or move to another role), `visual.set_aggregation`
//! (how a column is summarized) and `visual.set_type` (change the visual's type, its fields moved
//! to the new type's roles). `visual.bind` and `visual.unbind` are in `report.rs`.
//!
//! Each one checks everything first and edits the report once nothing can fail, so a refused
//! edit leaves it as it was; each is one step of the undo history.

use bicraft_report::{FieldBinding, FieldRef, ShowAs, Visual};
use bicraft_visuals::RoleKind;
use serde_json::{Value, json};

use crate::Session;
use crate::params::{allow, force, opt_bool, opt_str, str_param};
use crate::refs::{field_text, parse_field, same_field};
use crate::report::{
    KNOWN_VISUAL_TYPES, MAX_VISUAL_TYPE, binding_matches, bindings_json, check_role, opt_index, page_at_mut,
    page_index, parts, resolve_for_binding, role_verdict, visual_index,
};

/// The positions in `v.fields` of role `role`'s bindings, in order.
fn role_positions(v: &Visual, role: &str) -> Vec<usize> {
    v.fields
        .iter()
        .enumerate()
        .filter(|(_, b)| b.role.eq_ignore_ascii_case(role))
        .map(|(i, _)| i)
        .collect()
}

/// The position in `v.fields` of the binding `p` names in `role`: by `field` text, else by
/// `index` (its position inside the role).
pub(crate) fn source_position(v: &Visual, p: &Value, role: &str) -> Result<usize, String> {
    let in_role = role_positions(v, role);
    let field = opt_str(p, "field")?;
    let index = opt_index(p, "index")?;
    match (field, index) {
        (Some(_), Some(_)) => Err("give `field` or `index`, not both".into()),
        (None, None) => Err("give the field to change: `field` or `index`".into()),
        (Some(text), None) => {
            let parsed = parse_field(text)?;
            in_role
                .iter()
                .copied()
                .find(|&i| {
                    v.fields
                        .get(i)
                        .is_some_and(|b| binding_matches(&b.field, &parsed))
                })
                .ok_or_else(|| format!("{text} is not bound to role {role} of visual `{}`", v.name))
        }
        (None, Some(i)) => in_role.get(i).copied().ok_or_else(|| {
            format!(
                "index {i} is past the end of role {role} of visual `{}` ({} fields)",
                v.name,
                in_role.len()
            )
        }),
    }
}

/// `visual.move_field {page, visual, role, field | index, to_role?, to_index?, force?}`.
pub(crate) fn visual_move_field(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "page", "visual", "role", "field", "index", "to_role", "to_index", "force",
        ],
    )?;
    let key = str_param(p, "page")?;
    let visual = str_param(p, "visual")?;
    let role = str_param(p, "role")?;
    check_role(role)?;
    let to_role = opt_str(p, "to_role")?.unwrap_or(role);
    check_role(to_role)?;
    let to_index = opt_index(p, "to_index")?;
    let force = force(p)?;
    let (model, r) = parts(s)?;
    let pi = page_index(r, key)?;
    let page = page_at_mut(r, pi)?;
    let vi = visual_index(page, visual)?;
    let v = page
        .visuals
        .get_mut(vi)
        .ok_or("internal: visual index out of range")?;
    let from = source_position(v, p, role)?;
    let mut after = v.fields.clone();
    if from >= after.len() {
        return Err("internal: binding index out of range".into());
    }
    let moving = after.remove(from);
    let same_role = moving.role.eq_ignore_ascii_case(to_role);
    let target: Vec<usize> = after
        .iter()
        .enumerate()
        .filter(|(_, b)| b.role.eq_ignore_ascii_case(to_role))
        .map(|(i, _)| i)
        .collect();
    if !same_role
        && target
            .iter()
            .filter_map(|&i| after.get(i))
            .any(|b| same_field(&b.field, &moving.field))
    {
        return Err(format!(
            "{} is already bound to role {to_role}",
            field_text(&moving.field)
        ));
    }
    // the role's spelling: the visual's when it uses the role, else Desktop's
    let role_name = target
        .first()
        .and_then(|&i| after.get(i))
        .map(|b| b.role.clone())
        .or_else(|| crate::roles::canonical_role(v.visual_type.as_deref(), to_role).map(str::to_string))
        .unwrap_or_else(|| to_role.to_string());
    let at = match to_index {
        Some(i) if i < target.len() => target.get(i).copied().unwrap_or(after.len()),
        Some(i) if i > target.len() => {
            return Err(format!(
                "to_index {i} is past the end of role {to_role} ({} fields)",
                target.len()
            ));
        }
        // the end of the role; an empty role goes where the field was
        _ => target
            .last()
            .map_or(from.min(after.len()), |&l| l.saturating_add(1)),
    };
    let text = field_text(&moving.field);
    after.insert(
        at,
        FieldBinding {
            display_name: moving.display_name,
            show_as: moving.show_as,
            role: role_name.clone(),
            field: moving.field,
        },
    );
    if after == v.fields {
        return Err(format!("{text} is already there"));
    }
    let mut warnings = Vec::new();
    let before = crate::roles::check_bindings(model, v.visual_type.as_deref(), &v.fields);
    let added = crate::roles::new_issues(
        &before,
        crate::roles::check_bindings(model, v.visual_type.as_deref(), &after),
    );
    role_verdict(added, force, &mut warnings)?;
    v.fields = after;
    let out = json!({
        "page": page.name,
        "visual": v.name,
        "field": text,
        "role": role_name,
        "fields": bindings_json(&v.fields),
        "warnings": warnings,
    });
    s.report_dirty = true;
    Ok(out)
}

/// `visual.set_aggregation {page, visual, role, field | index, aggregation, force?}`: a column
/// binding gets aggregation `aggregation` (Sum, Avg, Count, Min, Max, CountNonNull, Median,
/// StandardDeviation, Variance), or none (`null`: the column itself, "Don't summarize").
pub(crate) fn visual_set_aggregation(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &["page", "visual", "role", "field", "index", "aggregation", "force"],
    )?;
    let key = str_param(p, "page")?;
    let visual = str_param(p, "visual")?;
    let role = str_param(p, "role")?;
    check_role(role)?;
    let aggregation = match p.get("aggregation") {
        None => return Err("missing parameter `aggregation` (a function name, or null)".into()),
        Some(Value::Null) => None,
        Some(Value::String(f)) if f.is_empty() || f.eq_ignore_ascii_case("none") => None,
        Some(Value::String(f)) => Some(f.as_str()),
        Some(_) => return Err("`aggregation` must be a function name such as Sum, or null".into()),
    };
    if let Some(f) = aggregation
        && (f.len() > 40 || !f.chars().all(|c| c.is_ascii_alphabetic()))
    {
        return Err(format!("`{f}` is not an aggregation such as Sum, Avg or Count"));
    }
    let force = force(p)?;
    let (model, r) = parts(s)?;
    let measures = crate::report_measures::report_measures(r);
    let pi = page_index(r, key)?;
    let page = page_at_mut(r, pi)?;
    let vi = visual_index(page, visual)?;
    let v = page
        .visuals
        .get_mut(vi)
        .ok_or("internal: visual index out of range")?;
    let at = source_position(v, p, role)?;
    let old = v
        .fields
        .get(at)
        .ok_or("internal: binding index out of range")?
        .clone();
    let (table, column) = match &old.field {
        FieldRef::Column { table, column } | FieldRef::Aggregation { table, column, .. } => {
            (table.clone(), column.clone())
        }
        FieldRef::Measure { .. } => {
            return Err(format!(
                "{} is a measure: only columns are summarized",
                field_text(&old.field)
            ));
        }
        FieldRef::Other { .. } => return Err("only column fields are summarized".into()),
    };
    let column_text = crate::edit::dax_column(&table, &column);
    let text = match aggregation {
        Some(f) => format!("{f}({column_text})"),
        None => column_text,
    };
    let mut warnings = Vec::new();
    let field = resolve_for_binding(model, &measures, &text, force, &mut warnings)?;
    if same_field(&field, &old.field) {
        return Err(format!("{} is already summarized that way", field_text(&field)));
    }
    if v.fields
        .iter()
        .enumerate()
        .any(|(i, b)| i != at && b.role.eq_ignore_ascii_case(role) && same_field(&b.field, &field))
    {
        return Err(format!("{} is already bound to role {role}", field_text(&field)));
    }
    let mut after = v.fields.clone();
    if let Some(b) = after.get_mut(at) {
        b.field = field;
    }
    let before = crate::roles::check_bindings(model, v.visual_type.as_deref(), &v.fields);
    let added = crate::roles::new_issues(
        &before,
        crate::roles::check_bindings(model, v.visual_type.as_deref(), &after),
    );
    role_verdict(added, force, &mut warnings)?;
    v.fields = after;
    let now = v.fields.get(at).map(|b| field_text(&b.field));
    let out = json!({
        "page": page.name,
        "visual": v.name,
        "role": old.role,
        "from": field_text(&old.field),
        "field": now,
        "fields": bindings_json(&v.fields),
        "warnings": warnings,
    });
    s.report_dirty = true;
    Ok(out)
}

/// Whether a binding can go to a role of kind `kind`: measures and aggregations to roles that
/// take measures, plain columns anywhere (a column in a measure role is summarized by Desktop).
fn fits(kind: RoleKind, field: &FieldRef) -> bool {
    match field {
        FieldRef::Measure { .. } | FieldRef::Aggregation { .. } => kind != RoleKind::Grouping,
        FieldRef::Column { .. } => kind != RoleKind::Measure,
        FieldRef::Other { .. } => true,
    }
}

/// `visual.set_type {page, visual, visual_type}`: the visual's type changes; each field keeps its
/// role when the new type has it, else goes to the first role of the new type that takes it (a
/// column to a grouping role, a measure to a value role) and has room; a field no role takes is
/// dropped and listed in `dropped`. Formatting is kept. A type whose roles are not known (a
/// custom visual) keeps every field as it is.
pub(crate) fn visual_set_type(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "visual", "visual_type"])?;
    let key = str_param(p, "page")?;
    let visual = str_param(p, "visual")?;
    let visual_type = str_param(p, "visual_type")?;
    if visual_type.is_empty()
        || visual_type.len() > MAX_VISUAL_TYPE
        || !visual_type.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(format!(
            "`{visual_type}` is not a visual type like card, clusteredColumnChart, tableEx or slicer"
        ));
    }
    // one of BIcraft's visuals (by name or guid): the visual carries its guid, and its package
    // goes in (on first use) once nothing else can refuse the change
    let builtin = crate::builtin_visuals::find(visual_type);
    let visual_type = builtin.map_or(visual_type, |b| b.guid);
    let check = |v: &Visual| match &v.visual_type {
        None => Err(format!("`{}` is a visual group; groups have no type", v.name)),
        Some(t) if t == visual_type => Err(format!("`{}` is already a {visual_type}", v.name)),
        Some(t) => Ok(t.clone()),
    };
    let installed = match builtin {
        None => None,
        Some(b) => {
            let r = crate::report::report_ref(s)?;
            let page = r
                .pages
                .get(page_index(r, key)?)
                .ok_or("internal: page index out of range")?;
            let v = page
                .visuals
                .get(visual_index(page, visual)?)
                .ok_or("internal: visual index out of range")?;
            check(v)?;
            Some(crate::builtin_visuals::ensure(s, b)?)
        }
    };
    let (model, r) = parts(s)?;
    let pi = page_index(r, key)?;
    let page = page_at_mut(r, pi)?;
    let vi = visual_index(page, visual)?;
    let v = page
        .visuals
        .get_mut(vi)
        .ok_or("internal: visual index out of range")?;
    let old_type = check(v)?;
    let mut warnings = Vec::new();
    if builtin.is_none() && !KNOWN_VISUAL_TYPES.contains(&visual_type) {
        warnings.push(format!(
            "`{visual_type}` is not a built-in visual type; it must be a custom visual installed in the report"
        ));
    }
    let own = builtin.map(crate::builtin_visuals::BuiltinVisual::visual_type);
    let catalog = bicraft_visuals::visual(visual_type)
        .filter(|t| t.roles_known)
        .or(own.as_ref());
    let mut fields: Vec<FieldBinding> = Vec::with_capacity(v.fields.len());
    let mut dropped: Vec<Value> = Vec::new();
    match catalog {
        None => fields = v.fields.clone(),
        Some(t) => {
            let count = |fields: &[FieldBinding], role: &str| {
                fields
                    .iter()
                    .filter(|b| b.role.eq_ignore_ascii_case(role))
                    .count()
            };
            let room = |fields: &[FieldBinding], r: &bicraft_visuals::DataRole| {
                r.max
                    .and_then(|m| usize::try_from(m).ok())
                    .is_none_or(|m| count(fields, &r.name) < m)
            };
            for b in &v.fields {
                let duplicate = |fields: &[FieldBinding], role: &str| {
                    fields
                        .iter()
                        .any(|x| x.role.eq_ignore_ascii_case(role) && same_field(&x.field, &b.field))
                };
                let same = t
                    .data_roles
                    .iter()
                    .find(|r| r.name.eq_ignore_ascii_case(&b.role))
                    .filter(|r| fits(r.kind, &b.field) && room(&fields, r) && !duplicate(&fields, &r.name));
                // a column goes to a grouping role first, a measure to a value role first
                let preferred = |r: &&bicraft_visuals::DataRole| match b.field {
                    FieldRef::Column { .. } => r.kind != RoleKind::Measure,
                    _ => r.kind != RoleKind::Grouping,
                };
                let other = || {
                    let usable =
                        |r: &&bicraft_visuals::DataRole| room(&fields, r) && !duplicate(&fields, &r.name);
                    t.data_roles.iter().filter(preferred).find(usable).or_else(|| {
                        t.data_roles
                            .iter()
                            .filter(|r| fits(r.kind, &b.field))
                            .find(usable)
                    })
                };
                match same.or_else(other) {
                    Some(role) => fields.push(FieldBinding {
                        display_name: b.display_name.clone(),
                        // column and row totals are a matrix's
                        show_as: b
                            .show_as
                            .filter(|k| *k == ShowAs::GrandTotal || visual_type == MATRIX),
                        role: role.name.clone(),
                        field: b.field.clone(),
                    }),
                    None => dropped.push(json!({"role": b.role, "field": field_text(&b.field)})),
                }
            }
        }
    }
    // what the new type still needs is a warning, never a refusal: the person changes it on
    for i in crate::roles::check_bindings(model, Some(visual_type), &fields) {
        warnings.push(i.message);
    }
    let moved: Vec<Value> = v
        .fields
        .iter()
        .filter_map(|b| {
            let now = fields
                .iter()
                .find(|x| same_field(&x.field, &b.field) && !x.role.eq_ignore_ascii_case(&b.role))?;
            Some(json!({"field": field_text(&b.field), "from": b.role, "to": now.role}))
        })
        .collect();
    v.visual_type = Some(visual_type.to_string());
    v.fields = fields;
    // a Spec visual without a spec starts with visual.spec_template's for its fields
    let spec_template = crate::spec_visual::default_spec(v).unwrap_or_else(|e| {
        warnings.push(format!("no starting spec: {e}"));
        None
    });
    let mut out = json!({
        "page": page.name,
        "visual": v.name,
        "from": old_type,
        "visual_type": visual_type,
        "fields": bindings_json(&v.fields),
        "moved": moved,
        "dropped": dropped,
        "warnings": warnings,
    });
    if let (Some(installed), Some(o)) = (installed, out.as_object_mut()) {
        o.insert("installed".into(), json!(installed));
    }
    if let (Some(kind), Some(o)) = (spec_template, out.as_object_mut()) {
        o.insert("spec_template".into(), json!(kind));
    }
    s.report_dirty = true;
    Ok(out)
}

/// Longest per-visual name of a field (Desktop's display names are short; this caps hostile
/// input).
const MAX_DISPLAY_NAME: usize = 500;

/// `visual.rename_field {page, visual, role, field | index, name}`: "Rename for this visual".
pub(crate) fn visual_rename_field(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "visual", "role", "field", "index", "name"])?;
    let key = str_param(p, "page")?;
    let visual = str_param(p, "visual")?;
    let role = str_param(p, "role")?;
    check_role(role)?;
    let name = match p.get("name") {
        None => {
            return Err("missing parameter `name` (the new name, or null to restore the field's own)".into());
        }
        Some(Value::Null) => None,
        Some(Value::String(n)) if n.trim().is_empty() => None,
        Some(Value::String(n)) => {
            if n.chars().count() > MAX_DISPLAY_NAME || n.chars().any(char::is_control) {
                return Err(format!(
                    "`name` must be at most {MAX_DISPLAY_NAME} characters, without control characters"
                ));
            }
            Some(n.clone())
        }
        Some(_) => return Err("parameter `name` must be a string or null".into()),
    };
    let (_, r) = parts(s)?;
    let pi = page_index(r, key)?;
    let page = page_at_mut(r, pi)?;
    let vi = visual_index(page, visual)?;
    let v = page
        .visuals
        .get_mut(vi)
        .ok_or("internal: visual index out of range")?;
    let at = source_position(v, p, role)?;
    let b = v
        .fields
        .get_mut(at)
        .ok_or("internal: binding index out of range")?;
    if b.display_name == name {
        return Err(match &name {
            Some(n) => format!("{} is already named `{n}` in this visual", field_text(&b.field)),
            None => format!("{} is not renamed in this visual", field_text(&b.field)),
        });
    }
    b.display_name = name.clone();
    let (role_out, text) = (b.role.clone(), field_text(&b.field));
    let out = json!({
        "page": page.name,
        "visual": v.name,
        "role": role_out,
        "field": text,
        "renamed": name,
        "fields": bindings_json(&v.fields),
    });
    s.report_dirty = true;
    Ok(out)
}

/// The matrix visual type (the only one with column and row totals to show a value against).
const MATRIX: &str = "pivotTable";

/// `visual.show_value_as`'s choices.
pub(crate) const SHOW_VALUE_AS: &[&str] = &[
    "none",
    "percent_of_grand_total",
    "percent_of_column_total",
    "percent_of_row_total",
];

fn parse_show_as(p: &Value) -> Result<Option<ShowAs>, String> {
    let text = match p.get("as") {
        None => return Err("missing parameter `as` (none, percent_of_grand_total, …)".into()),
        Some(Value::Null) => return Ok(None),
        Some(Value::String(t)) => t.as_str(),
        Some(_) => return Err("parameter `as` must be a string or null".into()),
    };
    match text.to_ascii_lowercase().as_str() {
        "none" | "" => Ok(None),
        "percent_of_grand_total" => Ok(Some(ShowAs::GrandTotal)),
        "percent_of_column_total" => Ok(Some(ShowAs::ColumnTotal)),
        "percent_of_row_total" => Ok(Some(ShowAs::RowTotal)),
        _ => Err(format!("`as` must be one of {}", SHOW_VALUE_AS.join(", "))),
    }
}

/// `visual.show_value_as {page, visual, role, field | index, as}`: "Show value as" on a value
/// field: none (the value), or its percentage of the grand total, or (matrix) of its column's
/// or row's total. Written as the projection's `Arithmetic` / `ScopedEval` field.
pub(crate) fn visual_show_value_as(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "visual", "role", "field", "index", "as"])?;
    let key = str_param(p, "page")?;
    let visual = str_param(p, "visual")?;
    let role = str_param(p, "role")?;
    check_role(role)?;
    let kind = parse_show_as(p)?;
    let (_, r) = parts(s)?;
    let pi = page_index(r, key)?;
    let page = page_at_mut(r, pi)?;
    let vi = visual_index(page, visual)?;
    let v = page
        .visuals
        .get_mut(vi)
        .ok_or("internal: visual index out of range")?;
    let at = source_position(v, p, role)?;
    let b = v
        .fields
        .get(at)
        .ok_or("internal: binding index out of range")?
        .clone();
    if !matches!(b.field, FieldRef::Measure { .. } | FieldRef::Aggregation { .. }) {
        return Err(format!(
            "{} is not a value: only measures and summarized columns are shown as a percentage",
            field_text(&b.field)
        ));
    }
    if let Some(k) = kind
        && let Some(scope) = k.scope_role()
    {
        if v.visual_type.as_deref() != Some(MATRIX) {
            return Err(format!(
                "{} is a matrix's: visual `{}` is not a matrix",
                k.as_str(),
                v.name
            ));
        }
        if !v.fields.iter().any(|x| {
            x.role.eq_ignore_ascii_case(scope)
                && match &x.field {
                    FieldRef::Column { .. } => true,
                    FieldRef::Other { raw } => bicraft_report::query::hierarchy_level(raw).is_some(),
                    _ => false,
                }
        }) {
            return Err(format!(
                "{} needs a column or a hierarchy level in the matrix's {scope}",
                k.as_str()
            ));
        }
    }
    if b.show_as == kind {
        return Err(format!(
            "{} is already shown as {}",
            field_text(&b.field),
            kind.map_or("its value", ShowAs::as_str)
        ));
    }
    if v.fields.iter().enumerate().any(|(i, x)| {
        i != at && x.role.eq_ignore_ascii_case(&b.role) && same_field(&x.field, &b.field) && x.show_as == kind
    }) {
        return Err(format!(
            "{} is already bound to role {} that way",
            field_text(&b.field),
            b.role
        ));
    }
    if let Some(x) = v.fields.get_mut(at) {
        x.show_as = kind;
    }
    let mut out = json!({
        "page": page.name,
        "visual": v.name,
        "role": b.role,
        "field": field_text(&b.field),
        "as": kind.map_or("none", ShowAs::as_str),
        "fields": bindings_json(&v.fields),
    });
    let query = bicraft_report::query::effective(v);
    if v.fields
        .iter()
        .any(|x| bicraft_report::query::show_all(query.as_ref(), &x.role))
        && let Some(w) = crate::preview::show::show_all_conflict(v)
        && let Some(o) = out.as_object_mut()
    {
        o.insert("warnings".into(), json!([w]));
    }
    s.report_dirty = true;
    Ok(out)
}

/// `visual.show_items_no_data {page, visual, role, show}`: "Show items with no data" on a
/// grouping role: its category values are listed even where every value is blank. Written as
/// the role's `showAll` in the visual's query (the legacy layout's `showAllRoles`).
pub(crate) fn visual_show_items_no_data(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "visual", "role", "show"])?;
    let key = str_param(p, "page")?;
    let visual = str_param(p, "visual")?;
    let role = str_param(p, "role")?;
    check_role(role)?;
    let show = opt_bool(p, "show")?.ok_or("missing parameter `show` (true or false)")?;
    let (_, r) = parts(s)?;
    let pi = page_index(r, key)?;
    let page = page_at_mut(r, pi)?;
    let vi = visual_index(page, visual)?;
    let v = page
        .visuals
        .get_mut(vi)
        .ok_or("internal: visual index out of range")?;
    let grouping = |f: &FieldRef| match f {
        FieldRef::Column { .. } => true,
        FieldRef::Other { raw } => bicraft_report::query::hierarchy_level(raw).is_some(),
        _ => false,
    };
    let Some(spelled) = v
        .fields
        .iter()
        .find(|b| b.role.eq_ignore_ascii_case(role) && grouping(&b.field))
        .map(|b| b.role.clone())
    else {
        return Err(format!(
            "role {role} of visual `{}` has no category field to show items with no data for",
            v.name
        ));
    };
    let query = bicraft_report::query::effective(v);
    if bicraft_report::query::show_all(query.as_ref(), &spelled) == show {
        return Err(format!(
            "role {spelled} {} its items with no data",
            if show { "already shows" } else { "does not show" }
        ));
    }
    let mut query = query.ok_or("internal: a visual with fields has a query")?;
    let state = query
        .get_mut("queryState")
        .and_then(|s| s.as_object_mut())
        .ok_or("internal: the visual's query has no queryState")?;
    let role_state = state
        .iter_mut()
        .find(|(k, _)| k.eq_ignore_ascii_case(&spelled))
        .and_then(|(_, r)| r.as_object_mut())
        .ok_or("internal: the role is not in the visual's query")?;
    if show {
        role_state.insert("showAll".into(), Value::Bool(true));
    } else {
        role_state.shift_remove("showAll");
    }
    v.visual_extra.insert("query".into(), query);
    let mut out = json!({
        "page": page.name,
        "visual": v.name,
        "role": spelled,
        "show": show,
    });
    if show
        && let Some(w) = crate::preview::show::show_all_conflict(v)
        && let Some(o) = out.as_object_mut()
    {
        o.insert("warnings".into(), json!([w]));
    }
    s.report_dirty = true;
    Ok(out)
}

/// `visual.bind_hierarchy {page, visual, role, hierarchy, table?, levels?, index?, force?}`.
pub(crate) fn visual_bind_hierarchy(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "page",
            "visual",
            "role",
            "hierarchy",
            "table",
            "levels",
            "index",
            "force",
        ],
    )?;
    let key = str_param(p, "page")?;
    let visual = str_param(p, "visual")?;
    let role = str_param(p, "role")?;
    check_role(role)?;
    let hierarchy = str_param(p, "hierarchy")?;
    let table = opt_str(p, "table")?;
    let index = opt_index(p, "index")?;
    let force = force(p)?;
    let wanted: Option<Vec<String>> = match p.get("levels") {
        None | Some(Value::Null) => None,
        Some(Value::Array(a)) if !a.is_empty() && a.len() <= 64 => Some(
            a.iter()
                .map(|x| x.as_str().map(str::to_string))
                .collect::<Option<Vec<_>>>()
                .ok_or("`levels` must be a list of level names")?,
        ),
        Some(_) => return Err("`levels` must be a list of 1 to 64 level names".into()),
    };
    let (model, r) = parts(s)?;
    let model = model.ok_or("binding a hierarchy needs the model (open one with the report)")?;
    // the hierarchy: in `table`, else in the one table that has it
    let found = match table {
        Some(t) => crate::hierarchy::levels(model, t, hierarchy)
            .ok_or_else(|| format!("table `{t}` has no hierarchy `{hierarchy}`"))?,
        None => {
            let mut hits = model
                .tables
                .iter()
                .filter_map(|t| crate::hierarchy::levels(model, &t.name, hierarchy));
            let first = hits
                .next()
                .ok_or_else(|| format!("no table has a hierarchy `{hierarchy}`"))?;
            if hits.next().is_some() {
                return Err(format!(
                    "several tables have a hierarchy `{hierarchy}`: give `table`"
                ));
            }
            first
        }
    };
    let (t_name, h_name, all) = found;
    let levels: Vec<String> = match &wanted {
        None => all.iter().map(|(n, _)| n.clone()).collect(),
        Some(w) => {
            let mut out: Vec<String> = Vec::new();
            for l in w {
                let hit = all
                    .iter()
                    .find(|(n, _)| n.eq_ignore_ascii_case(l))
                    .ok_or_else(|| format!("hierarchy `{h_name}` has no level `{l}`"))?;
                if !out.contains(&hit.0) {
                    out.push(hit.0.clone());
                }
            }
            out
        }
    };
    if levels.is_empty() {
        return Err(format!("hierarchy `{h_name}` has no levels"));
    }
    let pi = page_index(r, key)?;
    let page = page_at_mut(r, pi)?;
    let vi = visual_index(page, visual)?;
    let v = page
        .visuals
        .get_mut(vi)
        .ok_or("internal: visual index out of range")?;
    let Some(vtype) = v.visual_type.clone() else {
        return Err(format!("`{}` is a visual group; groups have no fields", v.name));
    };
    let info = bicraft_visuals::visual(&vtype)
        .filter(|t| t.roles_known)
        .and_then(|t| t.data_roles.iter().find(|r| r.name.eq_ignore_ascii_case(role)));
    if info.is_some_and(|r| r.kind == RoleKind::Measure) && !force {
        return Err(format!(
            "role {role} takes measures; a hierarchy goes to a role that takes columns (pass \"force\": true to bind it anyway)"
        ));
    }
    let drill = info.is_some_and(|r| r.drill);
    let in_role = role_positions(v, role);
    let role_name = in_role
        .first()
        .and_then(|&i| v.fields.get(i))
        .map(|b| b.role.clone())
        .or_else(|| crate::roles::canonical_role(Some(&vtype), role).map(str::to_string))
        .unwrap_or_else(|| role.to_string());
    let new: Vec<FieldBinding> = levels
        .iter()
        .map(|l| FieldBinding {
            role: role_name.clone(),
            field: FieldRef::Other {
                raw: bicraft_report::query::hierarchy_level_json(&t_name, &h_name, l),
            },
            display_name: None,
            show_as: None,
        })
        .collect();
    if let Some((dup, _)) = new.iter().zip(&levels).find(|(n, _)| {
        in_role
            .iter()
            .filter_map(|&i| v.fields.get(i))
            .any(|b| b.field == n.field)
    }) {
        let _ = dup;
        return Err(format!(
            "a level of hierarchy `{h_name}` is already bound to role {role_name}"
        ));
    }
    let at = match index {
        Some(i) if i < in_role.len() => in_role.get(i).copied().unwrap_or(v.fields.len()),
        Some(i) if i > in_role.len() => {
            return Err(format!(
                "index {i} is past the end of role {role_name} ({} fields)",
                in_role.len()
            ));
        }
        _ => in_role.last().map_or(v.fields.len(), |&l| l.saturating_add(1)),
    };
    let mut after = v.fields.clone();
    for (k, b) in new.into_iter().enumerate() {
        let pos = at.saturating_add(k).min(after.len());
        after.insert(pos, b);
    }
    let mut warnings = Vec::new();
    let before = crate::roles::check_bindings(Some(model), Some(&vtype), &v.fields);
    let added = crate::roles::new_issues(
        &before,
        crate::roles::check_bindings(Some(model), Some(&vtype), &after),
    );
    role_verdict(added, force, &mut warnings)?;
    // the query, written now: a drill role marks its active level (Desktop: the first one when
    // none was active, the others not)
    let mut query = bicraft_report::query::regenerate(v.visual_extra.get("query"), &after);
    if drill
        && let Some(ps) = query
            .as_mut()
            .and_then(|q| q.get_mut("queryState"))
            .and_then(|st| st.get_mut(&role_name))
            .and_then(|r| r.get_mut("projections"))
            .and_then(Value::as_array_mut)
    {
        for p in ps.iter_mut() {
            if let Some(o) = p.as_object_mut()
                && !o.contains_key("active")
            {
                o.insert("active".into(), json!(false));
            }
        }
        if !ps.iter().any(|p| p.get("active") == Some(&json!(true)))
            && let Some(o) = ps.first_mut().and_then(Value::as_object_mut)
        {
            o.insert("active".into(), json!(true));
        }
    }
    v.fields = after;
    match query {
        Some(q) => {
            v.visual_extra.insert("query".into(), q);
        }
        None => {
            v.visual_extra.shift_remove("query");
        }
    }
    let out = json!({
        "page": page.name,
        "visual": v.name,
        "role": role_name,
        "table": t_name,
        "hierarchy": h_name,
        "levels": levels,
        "fields": bindings_json(&v.fields),
        "warnings": warnings,
    });
    s.report_dirty = true;
    Ok(out)
}
