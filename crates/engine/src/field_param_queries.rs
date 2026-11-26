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
