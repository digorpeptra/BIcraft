//! Field parameters in the data preview ([`super`]): a visual that binds a field parameter's
//! column (its labels, or its hidden fields column) shows the fields the parameter lists instead,
//! in the parameter's order: those its filters keep (the selection of a slicer of the parameter,
//! a page or report filter on it), all of them when nothing is selected. The filters on the
//! parameter's table choose the fields; they do not filter the data (the table is related to
//! nothing). Slicers are left alone: a slicer of the parameter lists its labels.
//!
//! The parameter is read from its table's expression (`crate::parameters::field_param`), so
//! no data is needed to know its fields. A filter on it that cannot be read keeps every field and
//! says so in `filters_ignored_reason`.

use bicraft_model::SemanticModel;
use bicraft_report::{FieldBinding, FieldRef, Visual};

use super::filters::{Applied, CmpOp, Cond, Item, Operand, TextOp, Val};
use crate::parameters::{FieldParam, ParamField, field_param};
use crate::refs::same_field;

fn eq(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// The visual with each binding of a field parameter replaced by the fields it chooses, and the
/// filters without those on the parameters' tables; `None` when the visual binds no field
/// parameter (or is a slicer).
pub(super) fn expand(model: &SemanticModel, v: &Visual, applied: &Applied) -> Option<(Visual, Applied)> {
    if super::filters::is_slicer(v) {
        return None;
    }
    let mut params: Vec<FieldParam> = Vec::new();
    for b in &v.fields {
        let FieldRef::Column { table, column } = &b.field else {
            continue;
        };
        if params.iter().any(|p| eq(&p.table, table)) {
            continue;
        }
        if let Some(p) = field_param(model, table)
            && (eq(column, &p.display) || eq(column, &p.fields))
        {
            params.push(p);
        }
    }
    if params.is_empty() {
        return None;
    }
    let mut out = applied.clone();
    out.items.clear();
    let mut conds: Vec<Vec<&Cond>> = vec![Vec::new(); params.len()];
    for it in &applied.items {
        if let Item::Cond(c) = it
            && let Some(i) = params.iter().position(|p| reads_only(c, p))
            && let Some(list) = conds.get_mut(i)
        {
            list.push(c);
            continue;
        }
        out.items.push(it.clone());
    }
    let mut chosen: Vec<Vec<&ParamField>> = Vec::with_capacity(params.len());
    for (p, cs) in params.iter().zip(&conds) {
        let mut unread = false;
        let rows: Vec<&ParamField> = p
            .rows
            .iter()
            .filter(|r| {
                cs.iter().all(|c| match eval(c, p, r) {
                    Some(b) => b,
                    None => {
                        unread = true;
                        true
                    }
                })
            })
            .collect();
        if unread {
            out.ignored.push(format!(
                "a filter on field parameter '{}' could not be read: every field it lists is shown",
                p.table
            ));
        }
        chosen.push(rows);
    }
    let mut d = v.clone();
    d.fields = Vec::with_capacity(v.fields.len());
    for b in &v.fields {
        let at = match &b.field {
            FieldRef::Column { table, column } => params
                .iter()
                .position(|p| eq(&p.table, table) && (eq(column, &p.display) || eq(column, &p.fields))),
            _ => None,
        };
        let Some(rows) = at.and_then(|i| chosen.get(i)) else {
            d.fields.push(b.clone());
            continue;
        };
        for r in rows {
            if d.fields
                .iter()
                .any(|x| eq(&x.role, &b.role) && same_field(&x.field, &r.field))
            {
                continue;
            }
            d.fields.push(FieldBinding {
                role: b.role.clone(),
                field: r.field.clone(),
                display_name: None,
                show_as: None,
            });
        }
    }
    Some((d, out))
}

/// Whether every operand of `c` is a column of parameter `p`'s table.
fn reads_only(c: &Cond, p: &FieldParam) -> bool {
    c.operands()
        .iter()
        .all(|o| matches!(o, Operand::Col { table, .. } if eq(table, &p.table)))
}

/// Row `r` of `p` in its column `column`: the label, the field's name, the order.
fn value(p: &FieldParam, r: &ParamField, o: &Operand) -> Option<Val> {
    let Operand::Col { column, .. } = o else {
        return None;
    };
    if eq(column, &p.display) {
        Some(Val::Text(r.label.clone()))
    } else if eq(column, &p.fields) {
        Some(Val::Text(r.name.clone()))
    } else if p.order.as_deref().is_some_and(|c| eq(c, column)) {
        Some(Val::Num(r.order))
    } else {
        None
    }
}

/// DAX's equality: text without case, numbers by value.
fn same(a: &Val, b: &Val) -> bool {
    match (a, b) {
        (Val::Text(x), Val::Text(y)) => x.to_lowercase() == y.to_lowercase(),
        (Val::Num(x), Val::Num(y)) => (x - y).abs() < 1e-9,
        (Val::Null, Val::Null) => true,
        _ => false,
    }
}

fn order(a: &Val, b: &Val) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (Val::Text(x), Val::Text(y)) => Some(x.to_lowercase().cmp(&y.to_lowercase())),
        (Val::Num(x), Val::Num(y)) => x.partial_cmp(y),
        _ => None,
    }
}

/// Whether row `r` passes `c` (`None`: the condition is not one this reads).
fn eval(c: &Cond, p: &FieldParam, r: &ParamField) -> Option<bool> {
    match c {
        Cond::In { exprs, rows } => {
            let vals = exprs
                .iter()
                .map(|o| value(p, r, o))
                .collect::<Option<Vec<Val>>>()?;
            Some(
                rows.iter()
                    .any(|row| row.len() == vals.len() && row.iter().zip(&vals).all(|(a, b)| same(a, b))),
            )
        }
        Cond::Cmp { op, left, right } => {
            let x = value(p, r, left)?;
            if *op == CmpOp::Eq {
                return Some(same(&x, right));
            }
            let o = order(&x, right)?;
            Some(match op {
                CmpOp::Gt => o.is_gt(),
                CmpOp::Ge => o.is_ge(),
                CmpOp::Lt => o.is_lt(),
                CmpOp::Le => o.is_le(),
                CmpOp::Eq => o.is_eq(),
            })
        }
        Cond::Text { op, left, right } => {
            let Val::Text(x) = value(p, r, left)? else {
                return None;
            };
            let (x, y) = (x.to_lowercase(), right.to_lowercase());
            Some(match op {
                TextOp::Contains => x.contains(&y),
                TextOp::StartsWith => x.starts_with(&y),
                TextOp::EndsWith => x.ends_with(&y),
            })
        }
        Cond::Not(c) => eval(c, p, r).map(|b| !b),
        Cond::And(a, b) => Some(eval(a, p, r)? && eval(b, p, r)?),
        Cond::Or(a, b) => Some(eval(a, p, r)? || eval(b, p, r)?),
    }
}
