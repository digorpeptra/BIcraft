//! Two field options of a visual, applied to its evaluated data (docs/formats/pbir.md "Show
//! value as" and "Show items with no data"):
//!
//! - **Show items with no data** (`queryState.<role>.showAll`): the category values of a
//!   show-all role that no row has a value for are listed too, with blank values. They are the
//!   rows of the field's table that pass the visual's filters (a filter on the field's table
//!   keeps its rows out; a filter on another table does not, as a filter on a fact table does
//!   not filter its dimension), and several show-all fields of one table list the combinations
//!   its rows hold (several tables: every combination of each table's).
//! - **Show value as** (`FieldBinding::show_as`): each value of the field divided by its total
//!   in the scope Desktop's `ScopedEval` names (none: the grand total; the matrix's column
//!   fields: its column's total; its row fields: its row's total). The totals are the visual
//!   evaluated again at that scope (a ratio's total is the ratio of the totals, as Desktop
//!   evaluates it), so the percentages are exact; a blank or zero total gives a blank, like
//!   `DIVIDE`. The data column is named `%GT …` (`%CT`, `%RT`) and formatted `0.00%`.

use std::collections::{HashMap, HashSet};

use bicraft_report::{ShowAs, Visual};
use bicraft_vertipaq::CellValue;
use serde_json::{Value, json};

use super::{Ctx, Fail, Grid, Item, Miss, Out, Query, general, grid, matrix, out_json};

/// Most category combinations "Show items with no data" lists (beyond, the note says so).
const MAX_ITEMS: usize = 100_000;

/// The format of a percentage ("Show value as").
pub(super) const PERCENT_FORMAT: &str = "0.00%";

fn reason(f: Fail) -> String {
    match f {
        Fail::Unsupported(r) | Fail::Miss(Miss::Unavailable(r) | Miss::Error(r)) => r,
    }
}

/// The text key of some cells (equal cells, equal keys).
fn key_of<'a>(cells: impl Iterator<Item = &'a Out>) -> String {
    Value::Array(cells.map(out_json).collect()).to_string()
}

/// A column value as a grid cell.
fn cell_of(v: &CellValue) -> Out {
    match v {
        CellValue::Null => Out::Blank,
        v => Out::Cell(v.clone()),
    }
}

/// Why Power BI Desktop leaves "Show items with no data" off in `v`, if it does: a value shown as
/// a percentage of the grand total (Desktop 2.159.1231.0 draws the visual without the empty
/// categories and flags it: the two options are not compatible; docs/formats/pbir.md).
pub(crate) fn show_all_conflict(v: &Visual) -> Option<String> {
    v.fields
        .iter()
        .any(|b| b.show_as == Some(ShowAs::GrandTotal))
        .then(|| {
            "Show items with no data is not compatible with a percentage of the grand total: \
             Power BI Desktop leaves it off in this visual"
                .to_string()
        })
}

/// Adds to `grid` the rows of the show-all roles' category values that it lacks ("Show items
/// with no data"). `Err` says why they could not be listed (the grid is then unchanged).
pub(super) fn add_items_with_no_data(
    ctx: &mut Ctx<'_>,
    v: &Visual,
    grid: &mut Grid,
    filters: &[Item],
) -> Result<(), String> {
    let query = v.visual_extra.get("query");
    let any_show_all = v
        .fields
        .iter()
        .any(|b| bicraft_report::query::show_all(query, &b.role));
    if any_show_all && let Some(why) = show_all_conflict(v) {
        return Err(why);
    }
    let shown = v.fields.len();
    let is_group = |i: usize| grid.is_group.get(i).copied().unwrap_or(false);
    let all: Vec<usize> = (0..shown)
        .filter(|i| {
            is_group(*i)
                && v.fields
                    .get(*i)
                    .is_some_and(|b| bicraft_report::query::show_all(query, &b.role))
        })
        .collect();
    if all.is_empty() {
        return Ok(());
    }
    let others: Vec<usize> = (0..shown).filter(|i| is_group(*i) && !all.contains(i)).collect();
    // the show-all fields by table, in field order
    let mut tables: Vec<(String, Vec<(usize, String)>)> = Vec::new();
    for &i in &all {
        let Some(Some((t, c))) = grid.groups.get(i) else {
            return Err("a show-all field is not a model column".into());
        };
        match tables.iter_mut().find(|(x, _)| x.eq_ignore_ascii_case(t)) {
            Some((_, cols)) => cols.push((i, c.clone())),
            None => tables.push((t.clone(), vec![(i, c.clone())])),
        }
    }
    let names: Vec<String> = tables.iter().map(|(t, _)| t.clone()).collect();
    let (masks, _) = general::masks(ctx, filters, &names).map_err(reason)?;
    // each table's combinations, then every combination across the tables
    let mut combos: Vec<Vec<(usize, Out)>> = vec![Vec::new()];
    for (t, cols) in &tables {
        let data = cols
            .iter()
            .map(|(_, c)| ctx.column(t, c))
            .collect::<Result<Vec<_>, Miss>>()
            .map_err(|m| reason(Fail::Miss(m)))?;
        let rows = data.first().map_or(0, |d| d.codes.len());
        let mask = masks.get(&t.to_lowercase()).cloned().flatten();
        let mut seen: HashSet<String> = HashSet::new();
        let mut own: Vec<Vec<(usize, Out)>> = Vec::new();
        for r in 0..rows {
            if mask.as_ref().is_some_and(|m| !m.get(r).copied().unwrap_or(false)) {
                continue;
            }
            let tuple: Vec<(usize, Out)> = cols
                .iter()
                .zip(&data)
                .map(|((i, _), d)| {
                    let value = d
                        .codes
                        .get(r)
                        .and_then(|k| usize::try_from(k).ok())
                        .and_then(|k| d.values.get(k))
                        .map_or(Out::Blank, cell_of);
                    (*i, value)
                })
                .collect();
            if seen.insert(key_of(tuple.iter().map(|(_, o)| o))) {
                own.push(tuple);
                if own.len() > MAX_ITEMS {
                    return Err(format!("more than {MAX_ITEMS} items with no data"));
                }
            }
        }
        let mut next = Vec::new();
        for c in &combos {
            for o in &own {
                let mut x = c.clone();
                x.extend(o.iter().cloned());
                next.push(x);
                if next.len() > MAX_ITEMS {
                    return Err(format!("more than {MAX_ITEMS} items with no data"));
                }
            }
        }
        combos = next;
    }
    // the other categories: the combinations the visual shows
    let groups: Vec<usize> = (0..shown).filter(|i| is_group(*i)).collect();
    let mut present: HashSet<String> = HashSet::new();
    let mut outer: Vec<Vec<Out>> = Vec::new();
    let mut outer_seen: HashSet<String> = HashSet::new();
    for row in &grid.rows {
        present.insert(key_of(groups.iter().map(|i| row.get(*i).unwrap_or(&Out::Blank))));
        let o: Vec<Out> = others
            .iter()
            .map(|i| row.get(*i).cloned().unwrap_or(Out::Blank))
            .collect();
        if outer_seen.insert(key_of(o.iter())) {
            outer.push(o);
        }
    }
    if others.is_empty() && outer.is_empty() {
        outer.push(Vec::new());
    }
    let width = grid.is_group.len();
    let mut added = 0usize;
    for o in &outer {
        for c in &combos {
            let mut row = vec![Out::Blank; width];
            for (i, x) in others.iter().zip(o) {
                if let Some(slot) = row.get_mut(*i) {
                    *slot = x.clone();
                }
            }
            for (i, x) in c {
                if let Some(slot) = row.get_mut(*i) {
                    *slot = x.clone();
                }
            }
            if present.insert(key_of(groups.iter().map(|i| row.get(*i).unwrap_or(&Out::Blank)))) {
                grid.rows.push(row);
                added = added.saturating_add(1);
                if added > MAX_ITEMS {
                    return Err(format!("more than {MAX_ITEMS} items with no data"));
                }
            }
        }
    }
    Ok(())
}

/// One percentage field: its position, and the positions of the grouping fields its total
/// keeps.
struct Target {
    at: usize,
    scope: Vec<usize>,
}

fn targets(v: &Visual, is_group: &[bool]) -> Vec<Target> {
    let group = |i: usize| is_group.get(i).copied().unwrap_or(false);
    v.fields
        .iter()
        .enumerate()
        .filter(|(i, _)| !group(*i))
        .filter_map(|(at, b)| {
            let kind = b.show_as?;
            let scope = match kind.scope_role() {
                None => Vec::new(),
                Some(role) => v
                    .fields
                    .iter()
                    .enumerate()
                    .filter(|(j, x)| group(*j) && x.role.eq_ignore_ascii_case(role))
                    .map(|(j, _)| j)
                    .collect(),
            };
            Some(Target { at, scope })
        })
        .collect()
}

/// The totals of `v` at the grouping fields `kept` (positions in `v.fields`): one row of
/// `v.fields.len()` cells per member combination of the kept fields.
pub(super) fn totals_at(
    ctx: &mut Ctx<'_>,
    v: &Visual,
    is_group: &[bool],
    filters: &[Item],
    kept: &[usize],
) -> Result<Vec<Vec<Out>>, String> {
    let width = v.fields.len();
    if kept.is_empty() {
        return Ok(vec![general::totals(ctx, v, filters).map_err(reason)?]);
    }
    let mut out = Vec::new();
    let keep: Vec<bool> = (0..width)
        .map(|i| !is_group.get(i).copied().unwrap_or(false) || kept.contains(&i))
        .collect();
    let pv = matrix::with_fields(v, &keep);
    let q = Query { filters, extra: &[] };
    let grid = grid(ctx, &pv, &q).map_err(reason)?;
    let at: Vec<usize> = (0..width)
        .filter(|i| keep.get(*i).copied().unwrap_or(false))
        .collect();
    for r in &grid.rows {
        let mut full = vec![Out::Blank; width];
        for (j, x) in r.iter().take(at.len()).enumerate() {
            if let Some(slot) = at.get(j).and_then(|i| full.get_mut(*i)) {
                *slot = x.clone();
            }
        }
        out.push(full);
    }
    Ok(out)
}

fn ratio(value: &Value, total: Option<&Out>) -> Value {
    let den = match total {
        Some(Out::Num(x)) => Some(*x),
        Some(Out::Cell(c)) => super::as_num(c),
        _ => None,
    };
    match (value.as_f64(), den) {
        (Some(x), Some(d)) if d != 0.0 && (x / d).is_finite() => json!(x / d),
        _ => Value::Null,
    }
}

/// The row cells as grid cells (for their keys).
fn cells(row: &Value) -> Vec<Out> {
    row.as_array()
        .into_iter()
        .flatten()
        .map(|x| match x {
            Value::Null => Out::Blank,
            Value::Number(n) => Out::Num(n.as_f64().unwrap_or(0.0)),
            other => Out::Cell(CellValue::Text(other.to_string())),
        })
        .collect()
}

/// Rewrites the percentage fields of `d` (visual `v`'s data, whose grouping fields are
/// `is_group`): the rows, the table total and the matrix subtotals; names and formats their
/// columns. `Err` says why the totals could not be evaluated (`d` then keeps the values).
pub(super) fn apply_show_value_as(
    ctx: &mut Ctx<'_>,
    v: &Visual,
    is_group: &[bool],
    filters: &[Item],
    d: &mut Value,
) -> Result<(), String> {
    let targets = targets(v, is_group);
    if targets.is_empty() {
        return Ok(());
    }
    let mut cache: HashMap<Vec<usize>, HashMap<String, Vec<Out>>> = HashMap::new();
    // which grouping fields each part of the data keeps
    let all_groups: Vec<usize> = (0..v.fields.len())
        .filter(|i| is_group.get(*i).copied().unwrap_or(false))
        .collect();
    let mut parts: Vec<(Vec<usize>, String)> = vec![(all_groups.clone(), "rows".into())];
    let subtotal_levels: Vec<(usize, usize)> = d
        .get("subtotals")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|l| {
            let n = |k: &str| l.get(k).and_then(Value::as_u64).unwrap_or(0) as usize;
            (n("rows"), n("columns"))
        })
        .collect();
    let role_groups = |role: &str| -> Vec<usize> {
        all_groups
            .iter()
            .copied()
            .filter(|i| {
                v.fields
                    .get(*i)
                    .is_some_and(|b| b.role.eq_ignore_ascii_case(role))
            })
            .collect()
    };
    let (rows_g, cols_g) = (role_groups("Rows"), role_groups("Columns"));
    for (k, (r, c)) in subtotal_levels.iter().enumerate() {
        let kept: Vec<usize> = rows_g
            .iter()
            .take(*r)
            .chain(cols_g.iter().take(*c))
            .copied()
            .collect();
        parts.push((kept, format!("subtotals/{k}")));
    }
    if d.get("totals").is_some() {
        parts.push((Vec::new(), "totals".into()));
    }
    for (kept, part) in &parts {
        for t in &targets {
            let scope: Vec<usize> = t.scope.iter().copied().filter(|i| kept.contains(i)).collect();
            if !cache.contains_key(&scope) {
                // keyed by the cells' JSON, as the data rows are
                let keyed = totals_at(ctx, v, is_group, filters, &scope)?
                    .into_iter()
                    .map(|row| {
                        let c = cells(&Value::Array(row.iter().map(out_json).collect()));
                        (
                            key_of(scope.iter().map(|i| c.get(*i).unwrap_or(&Out::Blank))),
                            row,
                        )
                    })
                    .collect();
                cache.insert(scope.clone(), keyed);
            }
            let Some(table) = cache.get(&scope) else {
                continue;
            };
            let rows: Vec<&mut Value> = match part.as_str() {
                "rows" => d
                    .get_mut("rows")
                    .and_then(Value::as_array_mut)
                    .map(|a| a.iter_mut().collect())
                    .unwrap_or_default(),
                "totals" => d.get_mut("totals").into_iter().collect(),
                p => {
                    let k: usize = p.trim_start_matches("subtotals/").parse().unwrap_or(usize::MAX);
                    d.get_mut("subtotals")
                        .and_then(|s| s.get_mut(k))
                        .and_then(|l| l.get_mut("data"))
                        .and_then(Value::as_array_mut)
                        .map(|a| a.iter_mut().collect())
                        .unwrap_or_default()
                }
            };
            for row in rows {
                let c = cells(row);
                let key = key_of(scope.iter().map(|i| c.get(*i).unwrap_or(&Out::Blank)));
                let total = table.get(&key).and_then(|r| r.get(t.at));
                if let Some(cell) = row.get_mut(t.at) {
                    *cell = ratio(cell, total);
                }
            }
        }
    }
    // the columns: Desktop's name (`%GT Sum of Amount`, unless renamed) and a percent format
    if let Some(cols) = d.get_mut("columns").and_then(Value::as_array_mut) {
        for t in &targets {
            let Some(b) = v.fields.get(t.at) else {
                continue;
            };
            let Some(col) = cols.get_mut(t.at).and_then(Value::as_object_mut) else {
                continue;
            };
            if b.display_name.is_none() {
                let kind: ShowAs = b.show_as.unwrap_or(ShowAs::GrandTotal);
                let name = col.get("name").and_then(Value::as_str).unwrap_or_default();
                let name = format!("{} {name}", kind.prefix());
                col.insert("name".into(), json!(name));
            }
            col.insert("format_string".into(), json!(PERCENT_FORMAT));
            if let Some(k) = b.show_as {
                col.insert("show_as".into(), json!(k.as_str()));
            }
        }
    }
    Ok(())
}
