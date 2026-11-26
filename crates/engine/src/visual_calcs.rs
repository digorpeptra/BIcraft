//! Visual calculations on a visual (docs/formats/visual-calculations.md): `visual.calculation.add`,
//! `visual.calculation.set` (its expression, its name, and whether any field of the visual is
//! hidden) and `visual.calculation.remove`. A calculation is a projection whose field is a
//! `NativeVisualCalculation` (public `semanticQuery` schema), named and referred to by its name;
//! a hidden field is a projection with `hidden: true`. The preview evaluates them
//! (`preview_viscalc.rs`).
//!
//! Each command checks everything first and edits the report once nothing can fail.

use bicraft_dax::visual_calc;
use bicraft_report::query::{self, projection_meta, visual_calc as calc_of, visual_calc_field};
use bicraft_report::{FieldBinding, FieldRef, Visual};
use bicraft_visuals::RoleKind;
use serde_json::{Value, json};

use crate::Session;
use crate::params::{allow, force, opt_bool, opt_str, str_param};
use crate::report::{bindings_json, page_at_mut, page_index, parts, visual_index};

/// Longest calculation name and expression accepted.
const MAX_NAME: usize = 200;
const MAX_EXPRESSION: usize = 32_000;
/// Visual types without visual calculations (Microsoft Learn, "Visual calculations overview",
/// considerations and limitations).
const UNSUPPORTED_TYPES: [&str; 8] = [
    "slicer",
    "advancedSlicerVisual",
    "keyDriversVisual",
    "decompositionTreeVisual",
    "qnaVisual",
    "scriptVisual",
    "pythonVisual",
    "textbox",
];
/// The name Desktop gives a new calculation (then `Calculation 2`, `Calculation 3`…).
pub(crate) const DEFAULT_NAME: &str = "Calculation";

/// Per field of `v`: the names a visual calculation refers to it by, the first canonical (a
/// calculation's name; a field's `nativeQueryRef`, its display name, its default name).
pub(crate) fn field_names(v: &Visual) -> Vec<Vec<String>> {
    let q = query::effective(v);
    let meta = projection_meta(q.as_ref());
    let aligned = meta.len() == v.fields.len();
    v.fields
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let mut n: Vec<String> = Vec::new();
            if let Some(c) = calc_of(&b.field) {
                n.push(c.name);
            } else {
                if aligned && let Some(Some(native)) = meta.get(i).map(|m| m.0.clone()) {
                    n.push(native);
                }
                if let Some(d) = &b.display_name {
                    n.push(d.clone());
                }
                n.push(query::default_native_name(&b.field));
            }
            n.retain(|x| !x.is_empty());
            let mut seen: Vec<String> = Vec::new();
            n.retain(|x| {
                let k = x.to_lowercase();
                let fresh = !seen.contains(&k);
                seen.push(k);
                fresh
            });
            n
        })
        .collect()
}

/// Whether field `i` of `v` is hidden.
pub(crate) fn hidden(v: &Visual, i: usize) -> bool {
    hidden_fields(v).get(i).copied().unwrap_or(false)
}

/// Per field of `v`: whether it is hidden (kept in the visual matrix, not shown).
pub(crate) fn hidden_fields(v: &Visual) -> Vec<bool> {
    let q = query::effective(v);
    let meta = projection_meta(q.as_ref());
    let aligned = meta.len() == v.fields.len();
    (0..v.fields.len())
        .map(|i| aligned && meta.get(i).is_some_and(|m| m.1))
        .collect()
}

/// The field of `v` named `name`.
fn position(v: &Visual, name: &str) -> Result<usize, String> {
    field_names(v)
        .iter()
        .position(|n| n.iter().any(|x| x.eq_ignore_ascii_case(name)))
        .ok_or_else(|| {
            format!(
                "visual `{}` has no field or visual calculation named [{name}]",
                v.name
            )
        })
}

/// Refuses `expression` unless it binds against `v`'s fields (with `force`, its diagnostics
/// come back as warnings).
fn checked(v: &Visual, name: &str, expression: &str, force: bool) -> Result<Vec<String>, String> {
    if expression.trim().is_empty() {
        return Err("the expression is empty".into());
    }
    if expression.len() > MAX_EXPRESSION {
        return Err(format!("the expression is longer than {MAX_EXPRESSION} bytes"));
    }
    let names = field_names(v);
    let all: Vec<&str> = names.iter().flatten().map(String::as_str).collect();
    let c = visual_calc::check(expression, &all, name);
    let diags: Vec<String> = c
        .diagnostics
        .iter()
        .map(|d| format!("{}:{}: {} [{}]", d.line, d.column, d.message, d.code))
        .collect();
    if !c.valid && !force {
        return Err(format!("invalid visual calculation: {}", diags.join("; ")));
    }
    // a cycle through other calculations
    let mut calcs: Vec<(String, Vec<String>)> = Vec::new();
    for b in &v.fields {
        let Some(x) = calc_of(&b.field) else { continue };
        if x.name.eq_ignore_ascii_case(name) {
            continue;
        }
        let refs = visual_calc::check(&x.expression, &all, &x.name).refs;
        let canon = refs
            .iter()
            .filter_map(|r| {
                names
                    .iter()
                    .find(|n| n.iter().any(|y| y.eq_ignore_ascii_case(r)))
                    .and_then(|n| n.first().cloned())
            })
            .collect();
        calcs.push((x.name, canon));
    }
    let own = c
        .refs
        .iter()
        .filter_map(|r| {
            names
                .iter()
                .find(|n| n.iter().any(|y| y.eq_ignore_ascii_case(r)))
                .and_then(|n| n.first().cloned())
        })
        .collect();
    calcs.push((name.to_string(), own));
    if let Err(e) = visual_calc::order(&calcs)
        && !force
    {
        return Err(e);
    }
    Ok(if c.valid { Vec::new() } else { diags })
}

/// The calculations of `v` other than `name` that refer to `name`.
fn referrers(v: &Visual, name: &str) -> Vec<String> {
    let names = field_names(v);
    let all: Vec<&str> = names.iter().flatten().map(String::as_str).collect();
    v.fields
        .iter()
        .filter_map(|b| calc_of(&b.field))
        .filter(|c| !c.name.eq_ignore_ascii_case(name))
        .filter(|c| {
            visual_calc::check(&c.expression, &all, &c.name)
                .refs
                .iter()
                .any(|r| r.eq_ignore_ascii_case(name))
        })
        .map(|c| c.name)
        .collect()
}

/// The visual's query regenerated from its fields and stored, so projection edits apply to it.
fn sync(v: &mut Visual) {
    match query::effective(v) {
        Some(q) => {
            v.visual_extra.insert("query".into(), q);
        }
        None => {
            v.visual_extra.shift_remove("query");
        }
    }
}

fn name_param(p: &Value, key: &str) -> Result<Option<String>, String> {
    let Some(n) = opt_str(p, key)? else {
        return Ok(None);
    };
    let n = n.trim();
    if n.is_empty() {
        return Err(format!("`{key}` is empty"));
    }
    if n.chars().count() > MAX_NAME {
        return Err(format!("`{key}` is longer than {MAX_NAME} characters"));
    }
    if n.contains(['[', ']']) || n.chars().any(char::is_control) {
        return Err(format!("`{key}` cannot hold brackets or control characters"));
    }
    Ok(Some(n.to_string()))
}

/// The role a new calculation goes to: the role of the visual's first value field, else the
/// type's first measure role, else `Values`.
fn value_role(v: &Visual) -> String {
    if let Some(b) = v.fields.iter().find(|b| {
        matches!(b.field, FieldRef::Measure { .. } | FieldRef::Aggregation { .. })
            || calc_of(&b.field).is_some()
    }) {
        return b.role.clone();
    }
    v.visual_type
        .as_deref()
        .and_then(bicraft_visuals::visual)
        .and_then(|t| {
            t.data_roles
                .iter()
                .find(|r| r.kind == RoleKind::Measure)
                .or_else(|| {
                    t.data_roles
                        .iter()
                        .find(|r| r.kind == RoleKind::GroupingOrMeasure)
                })
        })
        .map_or_else(|| "Values".to_string(), |r| r.name.clone())
}

fn visual_mut<'r>(
    r: &'r mut bicraft_report::Report,
    page: &str,
    visual: &str,
) -> Result<(String, &'r mut Visual), String> {
    let pi = page_index(r, page)?;
    let page = page_at_mut(r, pi)?;
    let name = page.name.clone();
    let vi = visual_index(page, visual)?;
    let v = page
        .visuals
        .get_mut(vi)
        .ok_or("internal: visual index out of range")?;
    Ok((name, v))
}

fn result(page: &str, v: &Visual, at: usize, warnings: Vec<String>) -> Value {
    let b = v.fields.get(at);
    let calc = b.and_then(|b| calc_of(&b.field));
    json!({
        "page": page,
        "visual": v.name,
        "name": field_names(v).get(at).and_then(|n| n.first().cloned()),
        "expression": calc.map(|c| c.expression),
        "role": b.map(|b| b.role.clone()),
        "hidden": hidden(v, at),
        "fields": bindings_json(&v.fields),
        "warnings": warnings,
    })
}

/// `visual.calculation.add {page, visual, expression, name?, role?, hidden?, force?}`.
pub(crate) fn add(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &["page", "visual", "expression", "name", "role", "hidden", "force"],
    )?;
    let page = str_param(p, "page")?;
    let visual = str_param(p, "visual")?;
    let expression = str_param(p, "expression")?.to_string();
    let name = name_param(p, "name")?;
    let role = name_param(p, "role")?;
    let hide = opt_bool(p, "hidden")?.unwrap_or(false);
    let force = force(p)?;
    let (_, r) = parts(s)?;
    let (page, v) = visual_mut(r, page, visual)?;
    let t = v.visual_type.clone().unwrap_or_default();
    if t.is_empty() || UNSUPPORTED_TYPES.iter().any(|x| x.eq_ignore_ascii_case(&t)) {
        return Err(format!(
            "visual `{}` ({}) cannot have visual calculations",
            v.name,
            if t.is_empty() { "a group" } else { t.as_str() }
        ));
    }
    let taken = field_names(v);
    let free = |n: &str| !taken.iter().flatten().any(|x| x.eq_ignore_ascii_case(n));
    let name = match name {
        Some(n) if free(&n) => n,
        Some(n) => return Err(format!("visual `{}` already has a field named [{n}]", v.name)),
        None => std::iter::once(DEFAULT_NAME.to_string())
            .chain((2..10_000).map(|i| format!("{DEFAULT_NAME} {i}")))
            .find(|n| free(n))
            .ok_or("no free calculation name")?,
    };
    let warnings = checked(v, &name, &expression, force)?;
    let role = match role {
        Some(r) => {
            crate::report::check_role(&r)?;
            r
        }
        None => value_role(v),
    };
    // after the role's last field, else at the end
    let at = v
        .fields
        .iter()
        .rposition(|b| b.role.eq_ignore_ascii_case(&role))
        .map_or(v.fields.len(), |i| i.saturating_add(1));
    let mut nv = v.clone();
    nv.fields.insert(
        at,
        FieldBinding {
            role,
            field: visual_calc_field(&name, &expression),
            display_name: None,
            show_as: None,
        },
    );
    sync(&mut nv);
    if hide {
        set_hidden(&mut nv, at, true)?;
    }
    *v = nv;
    let out = result(&page, v, at, warnings);
    s.report_dirty = true;
    Ok(out)
}

/// Sets or clears `hidden` on projection `at` of `v` (whose query is in sync).
fn set_hidden(v: &mut Visual, at: usize, hide: bool) -> Result<(), String> {
    let mut q = v
        .visual_extra
        .get("query")
        .cloned()
        .ok_or("internal: a visual with fields has a query")?;
    let proj = query::projection_mut(&mut q, at).ok_or("internal: the field is not in the visual's query")?;
    if hide {
        proj.insert("hidden".into(), Value::Bool(true));
    } else {
        proj.shift_remove("hidden");
    }
    v.visual_extra.insert("query".into(), q);
    Ok(())
}

/// `visual.calculation.set {page, visual, name, expression?, new_name?, hidden?, force?}`.
pub(crate) fn set(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "page",
            "visual",
            "name",
            "expression",
            "new_name",
            "hidden",
            "force",
        ],
    )?;
    let page = str_param(p, "page")?;
    let visual = str_param(p, "visual")?;
    let name = str_param(p, "name")?.to_string();
    let expression = opt_str(p, "expression")?.map(str::to_string);
    let new_name = name_param(p, "new_name")?;
    let hide = opt_bool(p, "hidden")?;
    let force = force(p)?;
    if expression.is_none() && new_name.is_none() && hide.is_none() {
        return Err("nothing to change: give `expression`, `new_name` or `hidden`".into());
    }
    let (_, r) = parts(s)?;
    let (page, v) = visual_mut(r, page, visual)?;
    let at = position(v, &name)?;
    let calc = v.fields.get(at).and_then(|b| calc_of(&b.field));
    let mut warnings = Vec::new();
    let mut nv = v.clone();
    sync(&mut nv);
    if expression.is_some() || new_name.is_some() {
        let Some(calc) = calc else {
            return Err(format!(
                "[{name}] is a field of the visual, not a visual calculation: only `hidden` applies to it"
            ));
        };
        let final_name = match &new_name {
            Some(n) if !n.eq_ignore_ascii_case(&calc.name) => {
                let taken = field_names(v);
                if taken
                    .iter()
                    .enumerate()
                    .any(|(i, ns)| i != at && ns.iter().any(|x| x.eq_ignore_ascii_case(n)))
                {
                    return Err(format!("visual `{}` already has a field named [{n}]", v.name));
                }
                let by = referrers(v, &calc.name);
                if !by.is_empty() {
                    return Err(format!(
                        "[{}] is used by {}: change them first",
                        calc.name,
                        by.iter().map(|x| format!("[{x}]")).collect::<Vec<_>>().join(", ")
                    ));
                }
                n.clone()
            }
            Some(n) => n.clone(),
            None => calc.name.clone(),
        };
        let expr = expression.clone().unwrap_or(calc.expression.clone());
        warnings = checked(v, &final_name, &expr, force)?;
        let field = visual_calc_field(&final_name, &expr);
        // the projection keeps its other keys (hidden, format…); a new name is its queryRef too
        let mut q = nv
            .visual_extra
            .get("query")
            .cloned()
            .ok_or("internal: a visual with fields has a query")?;
        let proj =
            query::projection_mut(&mut q, at).ok_or("internal: the field is not in the visual's query")?;
        proj.insert("field".into(), query::field_json(&field));
        if new_name.is_some() {
            proj.insert("queryRef".into(), json!(final_name));
            proj.insert("nativeQueryRef".into(), json!(final_name));
        }
        nv.visual_extra.insert("query".into(), q);
        if let Some(b) = nv.fields.get_mut(at) {
            b.field = field;
        }
    }
    if let Some(h) = hide {
        set_hidden(&mut nv, at, h)?;
    }
    if query::derive(nv.visual_extra.get("query")) != nv.fields {
        return Err("internal: the visual's query no longer matches its fields".into());
    }
    *v = nv;
    let out = result(&page, v, at, warnings);
    s.report_dirty = true;
    Ok(out)
}

/// `visual.calculation.remove {page, visual, name, force?}`.
pub(crate) fn remove(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "visual", "name", "force"])?;
    let page = str_param(p, "page")?;
    let visual = str_param(p, "visual")?;
    let name = str_param(p, "name")?.to_string();
    let force = force(p)?;
    let (_, r) = parts(s)?;
    let (page, v) = visual_mut(r, page, visual)?;
    let at = position(v, &name)?;
    let Some(calc) = v.fields.get(at).and_then(|b| calc_of(&b.field)) else {
        return Err(format!(
            "[{name}] is a field of the visual, not a visual calculation: remove it from its well"
        ));
    };
    let by = referrers(v, &calc.name);
    if !by.is_empty() && !force {
        return Err(format!(
            "[{}] is used by {} (force removes it anyway)",
            calc.name,
            by.iter().map(|x| format!("[{x}]")).collect::<Vec<_>>().join(", ")
        ));
    }
    let mut nv = v.clone();
    sync(&mut nv);
    nv.fields.remove(at);
    sync(&mut nv);
    *v = nv;
    let out = json!({
        "page": page,
        "visual": v.name,
        "removed": calc.name,
        "fields": bindings_json(&v.fields),
    });
    s.report_dirty = true;
    Ok(out)
}
