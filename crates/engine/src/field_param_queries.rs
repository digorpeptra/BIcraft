//! Visuals bound to a field parameter, in the shape Power BI Desktop saves them
//! (docs/formats/parameters.md, "A visual bound to a field parameter"): a binding of a field
//! parameter's label column is written as the parameter's fields, one projection each with its
//! label as `displayName`, and the role's `fieldParameters` entry naming that range. Desktop shows
//! a visual bound to the label column alone as plain text.
//!
//! [`sync`] runs after every successful edit ([`crate::Session::execute`]), so the raw query
//! follows the model: a new binding, an edit of the parameter's fields. The report reads the range
//! back as one binding of the label column (`bicraft_report::query::derive`), so `Visual::fields`
//! and the preview see the parameter, not its fields.

use bicraft_model::SemanticModel;
use bicraft_report::{FieldRef, Report, query};
use serde_json::Value;

use crate::parameters::{FieldParam, field_param};

fn eq(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// Rewrites the query of every visual (slicers aside) that binds a field parameter's label
/// column, when it is not already the parameter's fields as the model lists them. Returns
/// whether a visual changed.
pub(crate) fn sync(model: Option<&SemanticModel>, report: Option<&mut Report>) -> bool {
    let (Some(m), Some(r)) = (model, report) else {
        return false;
    };
    let params: Vec<FieldParam> = m
        .tables
        .iter()
        .filter_map(|t| field_param(m, &t.name))
        .filter(|p| !p.rows.is_empty())
        .collect();
    if params.is_empty() {
        return false;
    }
    let mut changed = false;
    for v in r.pages.iter_mut().flat_map(|p| p.visuals.iter_mut()) {
        if crate::preview::filters::is_slicer(v) {
            continue;
        }
        let bound: Vec<(usize, &FieldParam)> = v
            .fields
            .iter()
            .enumerate()
            .filter_map(|(i, b)| match &b.field {
                FieldRef::Column { table, column } => params
                    .iter()
                    .find(|p| eq(&p.table, table) && eq(&p.display, column))
                    .map(|p| (i, p)),
                _ => None,
            })
            .collect();
        if bound.is_empty() {
            continue;
        }
        let raw = v.visual_extra.get("query");
        let Some(mut q) = query::effective(v) else {
            continue;
        };
        for (i, p) in bound {
            let fields: Vec<(FieldRef, String)> = p
                .rows
                .iter()
                .map(|row| (row.field.clone(), row.label.clone()))
                .collect();
            // the binding's own spelling, so the query reads back to the same binding
            let Some(label) = v.fields.get(i).map(|b| b.field.clone()) else {
                continue;
            };
            query::set_field_parameter(&mut q, i, &label, &fields);
        }
        if raw != Some(&q) {
            v.visual_extra.insert("query".into(), q);
            changed = true;
        }
    }
    changed
}

/// Whether two fields are the same column or measure (names without case, as the model looks
/// them up).
fn same_field(a: &FieldRef, b: &FieldRef) -> bool {
    match (a, b) {
        (
            FieldRef::Column {
                table: t1,
                column: c1,
            },
            FieldRef::Column {
                table: t2,
                column: c2,
            },
        )
        | (
            FieldRef::Measure {
                table: t1,
                measure: c1,
            },
            FieldRef::Measure {
                table: t2,
                measure: c2,
            },
        ) => eq(t1, t2) && eq(c1, c2),
        _ => a == b,
    }
}

/// A legacy layout keeps a visual bound to a field parameter as the parameter's fields alone
/// (`bicraft_layout` writes no range: no legacy key for it has been seen in a Desktop file;
/// docs/formats/parameters.md, "In a legacy layout"). In a report read from one, a run of a
/// role's projections that is exactly a field parameter's fields, in its order and each labelled
/// as the parameter labels it, is read as bound to the parameter again: the role gets its
/// `fieldParameters` entry, so the visual follows the parameter's slicer. Returns whether a
/// visual changed.
pub(crate) fn adopt_legacy(model: Option<&SemanticModel>, report: Option<&mut Report>) -> bool {
    let (Some(m), Some(r)) = (model, report) else {
        return false;
    };
    let params: Vec<FieldParam> = m
        .tables
        .iter()
        .filter_map(|t| field_param(m, &t.name))
        .filter(|p| !p.rows.is_empty())
        .collect();
    if params.is_empty() {
        return false;
    }
    let mut changed = false;
    for v in r.pages.iter_mut().flat_map(|p| p.visuals.iter_mut()) {
        if crate::preview::filters::is_slicer(v) {
            continue;
        }
        let Some(raw) = v.visual_extra.get("query") else {
            continue;
        };
        // only a query the bindings were read from (an edited one is regenerated first)
        if query::derive(Some(raw)) != v.fields {
            continue;
        }
        let mut q = raw.clone();
        let mut any = false;
        let Some(state) = q.get_mut("queryState").and_then(Value::as_object_mut) else {
            continue;
        };
        for role_state in state.values_mut() {
            let Some(o) = role_state.as_object_mut() else {
                continue;
            };
            let projections: Vec<Value> = o
                .get("projections")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let mut entries: Vec<Value> = o
                .get("fieldParameters")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            // the projections an entry already covers
            let mut covered = vec![false; projections.len()];
            for e in &entries {
                let at = e
                    .get("index")
                    .and_then(Value::as_u64)
                    .and_then(|x| usize::try_from(x).ok());
                let n = e
                    .get("length")
                    .and_then(Value::as_u64)
                    .and_then(|x| usize::try_from(x).ok());
                if let (Some(at), Some(n)) = (at, n) {
                    for c in covered.iter_mut().skip(at).take(n) {
                        *c = true;
                    }
                }
            }
            let added = entries.len();
            let mut i = 0usize;
            while i < projections.len() {
                let found = params.iter().find(|p| {
                    let n = p.rows.len();
                    i.checked_add(n).is_some_and(|end| end <= projections.len())
                        && p.rows.iter().enumerate().all(|(k, row)| {
                            let at = i.saturating_add(k);
                            let Some(proj) = projections.get(at) else {
                                return false;
                            };
                            !covered.get(at).copied().unwrap_or(true)
                                && same_field(&query::field_ref(proj.get("field")), &row.field)
                                && query::projection_display_name(proj).as_deref() == Some(row.label.as_str())
                        })
                });
                match found {
                    Some(p) => {
                        let label = FieldRef::Column {
                            table: p.table.clone(),
                            column: p.display.clone(),
                        };
                        let n = p.rows.len();
                        entries.push(serde_json::json!({
                            "parameterExpr": query::field_json(&label),
                            "index": i,
                            "length": n,
                        }));
                        i = i.saturating_add(n);
                    }
                    None => i = i.saturating_add(1),
                }
            }
            if entries.len() > added {
                entries.sort_by_key(|e| e.get("index").and_then(Value::as_u64).unwrap_or(u64::MAX));
                o.insert("fieldParameters".into(), Value::Array(entries));
                any = true;
            }
        }
        if any {
            v.fields = query::derive(Some(&q));
            v.visual_extra.insert("query".into(), q);
            changed = true;
        }
    }
    changed
}
