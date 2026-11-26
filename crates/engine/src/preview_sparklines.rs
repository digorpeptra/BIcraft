//! Sparklines and visual-scoped data formats in a table's or matrix's evaluated data
//! (docs/formats/sparklines.md).
//!
//! - A sparkline field (`SparklineData`) is left out of the visual's evaluation ([`split`]);
//!   [`apply`] then evaluates, as one flat table, the visual's grouping columns, the sparkline's
//!   X axis and its measure under the same filters, and gives each row of the visual its points:
//!   the measure per X value of that row's group, in X order, at most 52 (Microsoft Learn: "up
//!   to 52 points per sparkline"). The column is inserted where the projection stands, with the
//!   sparkline's look (`objects.sparklines`); its cells are `{"sparkline": [[x, y], …]}`. The
//!   total row gets the points without grouping, and a matrix's subtotals the points per member
//!   of the levels they total over, as Desktop 2.159.1231.0 draws them.
//! - A projection's `format` (Format pane › General › Data format) replaces its data column's
//!   `format_string` ([`apply_formats`]).

use std::cmp::Ordering;
use std::collections::HashMap;

use bicraft_report::query::{self, Sparkline, sparkline};
use bicraft_report::{FieldBinding, FieldRef, Visual};
use serde_json::{Map, Value, json};

use super::matrix;

/// One sparkline of a visual.
#[derive(Debug, Clone)]
pub(super) struct Spark {
    /// Its position among the visual's fields.
    at: usize,
    spark: Sparkline,
    /// Its data column's name.
    name: String,
    role: String,
    /// Its look (`crate::sparklines::style_of`).
    style: Map<String, Value>,
}

/// A visual with sparklines, split for evaluation.
#[derive(Debug, Clone)]
pub(super) struct Split {
    /// The visual without its sparklines.
    pub(super) base: Visual,
    sparks: Vec<Spark>,
}

/// The split of `v`, `None` when it has no sparkline.
pub(super) fn split(v: &Visual) -> Option<Split> {
    if !v.fields.iter().any(|b| sparkline(&b.field).is_some()) {
        return None;
    }
    let q = query::effective(v);
    let mut full = v.clone();
    if let Some(q) = &q {
        full.visual_extra.insert("query".into(), q.clone());
    }
    let keep: Vec<bool> = v.fields.iter().map(|b| sparkline(&b.field).is_none()).collect();
    let base = matrix::with_fields(&full, &keep);
    let sparks = v
        .fields
        .iter()
        .enumerate()
        .filter_map(|(at, b)| {
            let spark = sparkline(&b.field)?;
            let query_ref = q.clone().and_then(|mut q| {
                query::projection_mut(&mut q, at)?
                    .get("queryRef")?
                    .as_str()
                    .map(str::to_string)
            });
            let style = query_ref
                .as_deref()
                .map(|r| crate::sparklines::style_of(v, r))
                .unwrap_or_default();
            Some(Spark {
                at,
                name: b
                    .display_name
                    .clone()
                    .unwrap_or_else(|| query::default_native_name(&b.field)),
                role: b.role.clone(),
                spark,
                style,
            })
        })
        .collect();
    Some(Split { base, sparks })
}

/// Whether a binding groups the visual's rows (a column or a hierarchy level).
fn grouping(b: &FieldBinding) -> bool {
    match &b.field {
        FieldRef::Column { .. } => true,
        FieldRef::Other { raw } => query::hierarchy_level(raw).is_some(),
        _ => false,
    }
}

/// The positions of `base`'s grouping fields.
fn grouping_fields(base: &Visual) -> Vec<usize> {
    base.fields
        .iter()
        .enumerate()
        .filter(|(_, b)| grouping(b))
        .map(|(i, _)| i)
        .collect()
}

/// The flat table that holds the points of `spark` for every group of the fields of `base` at
/// `groups` (those grouping fields, the X axis, the measure; no group: the total).
pub(super) fn series_visual(base: &Visual, spark: &Sparkline, groups: &[usize]) -> Visual {
    let mut sv = base.clone();
    sv.visual_type = Some("tableEx".into());
    sv.visual_extra.shift_remove("query");
    sv.visual_extra.shift_remove("objects");
    let bind = |field: FieldRef| FieldBinding {
        role: "Values".into(),
        field,
        display_name: None,
        show_as: None,
    };
    let mut fields: Vec<FieldBinding> = groups
        .iter()
        .filter_map(|i| base.fields.get(*i))
        .map(|b| bind(b.field.clone()))
        .collect();
    for g in &spark.groupings {
        if !fields.iter().any(|b| &b.field == g) {
            fields.push(bind(g.clone()));
        }
    }
    fields.push(bind(spark.measure.clone()));
    sv.fields = fields;
    sv
}

/// The points of `spark` per group of the fields of `base` at `groups` (keyed by [`key_of`]
/// their cells), and whether some were cut; `Err` when the X axis is one of those fields (each
/// group would have one point) or the values could not be evaluated.
fn points_for(
    base: &Visual,
    spark: &Sparkline,
    groups: &[usize],
    eval: &mut dyn FnMut(&Visual) -> Value,
) -> Result<(HashMap<String, Vec<Value>>, bool), String> {
    let n_x = spark
        .groupings
        .iter()
        .filter(|g| {
            !groups
                .iter()
                .any(|i| base.fields.get(*i).is_some_and(|b| &b.field == *g))
        })
        .count();
    if n_x == 0 {
        return Err("the X axis is one of the visual's own rows: each row would have one point".into());
    }
    points_by_group(&eval(&series_visual(base, spark, groups)), groups.len(), n_x)
}

/// The sparkline cell of `row` (its group's cells at `groups`).
fn cell_of(row: &[Value], groups: &[usize], pts: &HashMap<String, Vec<Value>>) -> Value {
    let key = key_of(groups.iter().map(|i| row.get(*i).unwrap_or(&Value::Null)));
    match pts.get(&key) {
        Some(p) => json!({"sparkline": p}),
        None => Value::Null,
    }
}

/// A key for a row's group (its grouping cells).
fn key_of<'a>(cells: impl Iterator<Item = &'a Value>) -> String {
    Value::Array(cells.cloned().collect()).to_string()
}

/// Order of X values: numbers by value, text (ISO dates sort as dates) as text, blanks first.
fn cmp_x(a: &Value, b: &Value) -> Ordering {
    match (a, b) {
        (Value::Null, Value::Null) => Ordering::Equal,
        (Value::Null, _) => Ordering::Less,
        (_, Value::Null) => Ordering::Greater,
        (Value::Number(x), Value::Number(y)) => x
            .as_f64()
            .unwrap_or(0.0)
            .partial_cmp(&y.as_f64().unwrap_or(0.0))
            .unwrap_or(Ordering::Equal),
        (Value::Number(_), _) => Ordering::Less,
        (_, Value::Number(_)) => Ordering::Greater,
        _ => a.to_string().cmp(&b.to_string()),
    }
}

/// The points of each group of the flat table `d` (its rows: the groups' cells, the X cells,
/// the measure), keyed by [`key_of`] the group cells; `Err` when it could not be evaluated.
fn points_by_group(
    d: &Value,
    n_groups: usize,
    n_x: usize,
) -> Result<(HashMap<String, Vec<Value>>, bool), String> {
    if d.get("status").and_then(Value::as_str) != Some("ok") {
        let why = d
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("the sparkline's values could not be evaluated");
        return Err(why.to_string());
    }
    let mut series: HashMap<String, Vec<(Value, Value)>> = HashMap::new();
    for row in d
        .get("rows")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_array)
    {
        let key = key_of(row.iter().take(n_groups));
        let x: Vec<Value> = row.iter().skip(n_groups).take(n_x).cloned().collect();
        let x = if x.len() == 1 {
            x.into_iter().next().unwrap_or(Value::Null)
        } else {
            Value::Array(x)
        };
        let y = row
            .get(n_groups.saturating_add(n_x))
            .cloned()
            .unwrap_or(Value::Null);
        series.entry(key).or_default().push((x, y));
    }
    let mut cut = d.get("truncated") == Some(&Value::Bool(true));
    let cap = usize::try_from(query::SPARKLINE_POINTS).unwrap_or(52);
    let out = series
        .into_iter()
        .map(|(k, mut pts)| {
            pts.sort_by(|a, b| cmp_x(&a.0, &b.0));
            if pts.len() > cap {
                cut = true;
                pts.truncate(cap);
            }
            let pts = pts.into_iter().map(|(x, y)| json!([x, y])).collect();
            (k, pts)
        })
        .collect();
    Ok((out, cut))
}

/// Inserts the sparklines' columns into the base visual's data `d`. `eval` evaluates a flat
/// table visual (the same filters as the visual).
pub(super) fn apply(sp: &Split, d: &mut Value, eval: &mut dyn FnMut(&Visual) -> Value) {
    let Some(o) = d.as_object_mut() else { return };
    if o.get("status").and_then(Value::as_str) != Some("ok") {
        return;
    }
    let mut order: Vec<&Spark> = sp.sparks.iter().collect();
    order.sort_by_key(|s| s.at);
    let mut info: Vec<Value> = Vec::new();
    let groups = grouping_fields(&sp.base);
    // a matrix's row and column hierarchies, as its subtotal levels count them
    let is_group: Vec<bool> = sp.base.fields.iter().map(grouping).collect();
    let row_fields = matrix::group_fields(&sp.base, &is_group, "Rows");
    let col_fields = matrix::group_fields(&sp.base, &is_group, "Columns");
    for s in order {
        let result = points_for(&sp.base, &s.spark, &groups, eval);
        let at = s.at;
        let (pts, mut cut) = match &result {
            Ok((p, c)) => (Some(p), *c),
            Err(_) => (None, false),
        };
        if let Some(rows) = o.get_mut("rows").and_then(Value::as_array_mut) {
            for r in rows {
                let Some(r) = r.as_array_mut() else { continue };
                let cell = pts.map_or(Value::Null, |p| cell_of(r, &groups, p));
                let pos = at.min(r.len());
                r.insert(pos, cell);
            }
        }
        // the total row: the points in the total's filter context (Desktop 2.159.1231.0 draws
        // a sparkline there), a blank when they cannot be evaluated
        if let Some(t) = o.get_mut("totals").and_then(Value::as_array_mut) {
            let cell = match points_for(&sp.base, &s.spark, &[], eval) {
                Ok((p, c)) => {
                    cut |= c;
                    cell_of(&[], &[], &p)
                }
                Err(_) => Value::Null,
            };
            let pos = at.min(t.len());
            t.insert(pos, cell);
        }
        // a matrix's subtotals: per member of the levels each one totals over
        if let Some(levels) = o.get_mut("subtotals").and_then(Value::as_array_mut) {
            for l in levels {
                let depth = |k: &str| {
                    l.get(k)
                        .and_then(Value::as_u64)
                        .and_then(|n| usize::try_from(n).ok())
                        .unwrap_or(0)
                };
                let kept: Vec<usize> = row_fields
                    .iter()
                    .take(depth("rows"))
                    .chain(col_fields.iter().take(depth("columns")))
                    .copied()
                    .collect();
                let level_pts = match points_for(&sp.base, &s.spark, &kept, eval) {
                    Ok((p, c)) => {
                        cut |= c;
                        Some(p)
                    }
                    Err(_) => None,
                };
                for row in l
                    .get_mut("data")
                    .and_then(Value::as_array_mut)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_array_mut)
                {
                    let cell = level_pts.as_ref().map_or(Value::Null, |p| cell_of(row, &kept, p));
                    let pos = at.min(row.len());
                    row.insert(pos, cell);
                }
            }
        }
        if let Some(hl) = o.get_mut("highlights").and_then(Value::as_array_mut) {
            for row in hl.iter_mut().filter_map(Value::as_array_mut) {
                let pos = at.min(row.len());
                row.insert(pos, Value::Null);
            }
        }
        for key in ["cell_styles", "error_bars"] {
            let field = if key == "cell_styles" { "c" } else { "column" };
            for e in o.get_mut(key).and_then(Value::as_array_mut).into_iter().flatten() {
                if let Some(c) = e.get(field).and_then(Value::as_u64)
                    && usize::try_from(c).is_ok_and(|c| c >= at)
                {
                    e[field] = json!(c.saturating_add(1));
                }
            }
        }
        if let Some(cols) = o.get_mut("columns").and_then(Value::as_array_mut) {
            let mut col = Map::new();
            col.insert("name".into(), json!(s.name));
            col.insert("role".into(), json!(s.role));
            col.insert("kind".into(), json!("sparkline"));
            col.insert("data_type".into(), json!("sparkline"));
            col.insert("sparkline".into(), Value::Object(s.style.clone()));
            if let Err(r) = &result {
                col.insert("reason".into(), json!(r));
            }
            let pos = at.min(cols.len());
            cols.insert(pos, Value::Object(col));
        }
        let mut entry = json!({"name": s.name, "truncated": cut});
        if let Err(r) = &result {
            entry["reason"] = json!(r);
        }
        info.push(entry);
    }
    if !info.is_empty() {
        o.insert("sparklines".into(), Value::Array(info));
    }
}

/// Sets each data column's `format_string` to its projection's visual-scoped `format` (the
/// columns of `d` line up with `v`'s fields).
pub(super) fn apply_formats(v: &Visual, d: &mut Value) {
    let q = query::effective(v);
    let formats = query::projection_formats(q.as_ref());
    if formats.len() != v.fields.len() || formats.iter().all(Option::is_none) {
        return;
    }
    if d.get("status").and_then(Value::as_str) != Some("ok") {
        return;
    }
    let Some(cols) = d.get_mut("columns").and_then(Value::as_array_mut) else {
        return;
    };
    if cols.len() != formats.len() {
        return;
    }
    for (col, f) in cols.iter_mut().zip(&formats) {
        if let (Some(c), Some(f)) = (col.as_object_mut(), f) {
            c.insert("format_string".into(), json!(f));
            c.insert("visual_format".into(), json!(true));
        }
    }
}

#[cfg(test)]
#[path = "preview_sparklines_tests.rs"]
mod tests;
