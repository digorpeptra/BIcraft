//! The rows row-level security keeps under View as (`preview_rls.rs`), with the viewer's DAX
//! evaluator ([`super`]) on the data as stored.
//!
//! Per role (Microsoft Learn, "Row-level security (RLS) with Power BI" and the tabular model's
//! `TablePermission.FilterExpression`): each table filter is a DAX boolean expression evaluated
//! in the row context of each row of its table, with no filter; the rows it keeps then filter
//! the other tables through the model's active relationships, as a filter on the one side
//! filters the many side, and back from the many side to the one side when the relationship's
//! `securityFilteringBehavior` is `bothDirections` (`none`: the relationship carries no
//! security filter). A table filtered this way loses its rows whose key matches no kept row
//! (the virtual blank row is not kept). Several roles: the rows each role keeps are united, per
//! table; a role that filters nothing (or an administrator's) keeps every row, a role whose
//! model permission is `none` keeps none.
//!
//! Not checked against Analysis Services: whether a filter that reads another secured table
//! sees that table filtered (here it sees every row), and the one-to-one case (filtered like
//! one-to-many, back only with `bothDirections`).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use bicraft_model::Role;

use super::super::store::Miss;
use super::super::{Ctx, Fail};
use super::{Engine, Env, FCtx, NONE, RowB, Scope, truth};

/// Longest time spent on the row filters of one View as.
pub(in crate::preview) const MAX_TOTAL: Duration = Duration::from_secs(120);
/// Rounds of propagation through relationships before giving up (each round narrows a table).
const MAX_ROUNDS: usize = 256;

fn fail_text(f: Fail) -> String {
    match f {
        Fail::Unsupported(r) => format!("not supported by bicraft's DAX evaluator: {r}"),
        Fail::Miss(Miss::Unavailable(r) | Miss::Error(r)) => r,
    }
}

/// A relationship as security filters cross it: (from table, to table, from column, to column,
/// back from the from side to the to side).
struct SecRel {
    from_t: usize,
    to_t: usize,
    from: usize,
    to: usize,
    both: bool,
}

/// Lower-cased table name → rows kept, for every table some selected role restricts.
pub(in crate::preview) fn secured_rows(
    ctx: &mut Ctx<'_>,
    roles: &[&Role],
) -> Result<HashMap<String, Vec<u32>>, String> {
    let model = ctx.model;
    let mut eng = Engine::new(ctx);
    let start = Instant::now();
    let tables = eng.tabs.len();
    let mut rels = Vec::new();
    for r in model.relationships.iter().filter(|r| r.is_active) {
        let behavior = r
            .extra
            .get("securityFilteringBehavior")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("oneDirection");
        if behavior.eq_ignore_ascii_case("none") {
            continue;
        }
        let (Some(from), Some(to)) = (
            eng.col_id(&r.from_table, &r.from_column),
            eng.col_id(&r.to_table, &r.to_column),
        ) else {
            continue;
        };
        let (Ok(from_t), Ok(to_t)) = (eng.col_table(from), eng.col_table(to)) else {
            continue;
        };
        if from_t != to_t {
            rels.push(SecRel {
                from_t,
                to_t,
                from,
                to,
                both: behavior.eq_ignore_ascii_case("bothDirections"),
            });
        }
    }
    // per table: what the roles keep so far (`None`: no role restricted it yet)
    let mut union: Vec<Option<Vec<bool>>> = vec![None; tables];
    // tables some role leaves unrestricted (they keep every row)
    let mut open = vec![false; tables];
    for role in roles {
        let perm = role.model_permission.as_deref().unwrap_or("read");
        let mut masks: Vec<Option<Vec<bool>>> = vec![None; tables];
        if perm.eq_ignore_ascii_case("administrator") {
            // administrators are not filtered
        } else if perm.eq_ignore_ascii_case("none") {
            for (t, m) in masks.iter_mut().enumerate() {
                *m = Some(vec![false; eng.rows(t).map_err(fail_text)?]);
            }
        } else {
            for tp in &role.table_permissions {
                let Some(text) = tp.filter_expression.as_deref().filter(|t| !t.trim().is_empty()) else {
                    continue;
                };
                let t = eng.table_id(&tp.table).ok_or_else(|| {
                    format!(
                        "role `{}` filters table `{}`, which is not in the model",
                        role.name, tp.table
                    )
                })?;
                let m = row_filter(&mut eng, t, text, start)
                    .map_err(|e| format!("role `{}`, table `{}`: {e}", role.name, tp.table))?;
                and_into(&mut masks, t, m);
            }
            propagate(&mut eng, &rels, &mut masks, start)?;
        }
        for t in 0..tables {
            match masks.get_mut(t).and_then(Option::take) {
                None => {
                    if let Some(o) = open.get_mut(t) {
                        *o = true;
                    }
                }
                Some(m) => match union.get_mut(t) {
                    Some(slot @ None) => *slot = Some(m),
                    Some(Some(u)) => {
                        for (a, b) in u.iter_mut().zip(m) {
                            *a |= b;
                        }
                    }
                    None => {}
                },
            }
        }
    }
    let mut out = HashMap::new();
    for (t, m) in union.into_iter().enumerate() {
        if open.get(t).copied().unwrap_or(true) {
            continue;
        }
        let (Some(m), Some(info)) = (m, eng.tabs.get(t)) else {
            continue;
        };
        let rows: Vec<u32> = m
            .iter()
            .enumerate()
            .filter(|(_, k)| **k)
            .filter_map(|(i, _)| u32::try_from(i).ok())
            .collect();
        out.insert(info.lower.clone(), rows);
    }
    Ok(out)
}

/// `masks[t]` narrowed to `m`.
fn and_into(masks: &mut [Option<Vec<bool>>], t: usize, m: Vec<bool>) {
    if let Some(slot) = masks.get_mut(t) {
        match slot {
            None => *slot = Some(m),
            Some(old) => {
                for (a, b) in old.iter_mut().zip(m) {
                    *a &= b;
                }
            }
        }
    }
}

/// Which rows of table `t` the filter `text` keeps (TRUE in each row's context).
fn row_filter(eng: &mut Engine<'_, '_>, t: usize, text: &str, start: Instant) -> Result<Vec<bool>, String> {
    let (expr, diags) = bicraft_dax::parse_expression(text);
    if let Some(d) = diags.iter().find(|d| d.severity == bicraft_dax::Severity::Error) {
        return Err(format!("the filter does not parse: {}", d.message));
    }
    let expr = expr.ok_or("the filter does not parse")?;
    let n = eng.rows(t).map_err(fail_text)?;
    let fc = FCtx::default();
    let env = Env::new();
    let mut out = Vec::with_capacity(n);
    for row in 0..n {
        if start.elapsed() > MAX_TOTAL {
            return Err(format!(
                "the row filters take more than {} s",
                MAX_TOTAL.as_secs()
            ));
        }
        // the evaluator's per-expression budget applies to each row
        eng.work = 0;
        eng.start = Instant::now();
        let rc = [RowB::Row {
            table: t,
            row: u32::try_from(row).map_err(|_| "too many rows".to_string())?,
        }];
        let scope = Scope {
            fc: &fc,
            rc: &rc,
            env: &env,
            depth: 0,
            mdepth: 0,
        };
        let v = eng
            .scalar(&expr, &scope)
            .map_err(|f| format!("row {}: {}", row.saturating_add(1), fail_text(f)))?;
        out.push(truth(&v).map_err(|f| format!("row {}: {}", row.saturating_add(1), fail_text(f)))?);
    }
    Ok(out)
}

/// The rows kept filter the tables the relationships reach, until nothing narrows.
fn propagate(
    eng: &mut Engine<'_, '_>,
    rels: &[SecRel],
    masks: &mut [Option<Vec<bool>>],
    start: Instant,
) -> Result<(), String> {
    for _ in 0..MAX_ROUNDS {
        if start.elapsed() > MAX_TOTAL {
            return Err(format!(
                "the row filters take more than {} s",
                MAX_TOTAL.as_secs()
            ));
        }
        let mut changed = false;
        for r in rels {
            // the one side filters the many side
            if masks.get(r.to_t).is_some_and(Option::is_some) {
                let m = cross(eng, masks, r, true)?;
                changed |= narrow(masks, r.from_t, m);
            }
            if r.both && masks.get(r.from_t).is_some_and(Option::is_some) {
                let m = cross(eng, masks, r, false)?;
                changed |= narrow(masks, r.to_t, m);
            }
        }
        if !changed {
            return Ok(());
        }
    }
    Err("the row filters do not settle through the relationships".into())
}

/// `masks[t]` narrowed to `m`; whether it changed.
fn narrow(masks: &mut [Option<Vec<bool>>], t: usize, m: Vec<bool>) -> bool {
    let Some(slot) = masks.get_mut(t) else {
        return false;
    };
    match slot {
        None => {
            *slot = Some(m);
            true
        }
        Some(old) => {
            let mut changed = false;
            for (a, b) in old.iter_mut().zip(m) {
                if *a && !b {
                    *a = false;
                    changed = true;
                }
            }
            changed
        }
    }
}

/// The rows of one end of `r` related to the kept rows of the other: of the from table when
/// `down` (from the to side), else of the to table.
fn cross(
    eng: &mut Engine<'_, '_>,
    masks: &[Option<Vec<bool>>],
    r: &SecRel,
    down: bool,
) -> Result<Vec<bool>, String> {
    let from = eng.col(r.from).map_err(fail_text)?;
    let to = eng.col(r.to).map_err(fail_text)?;
    // from code → to code
    let map = eng.rel_map_of(&from, &to).map;
    let code = |c: &super::super::store::ColData, i: usize| c.codes.get(i).unwrap_or(0) as usize;
    if down {
        let Some(Some(kept)) = masks.get(r.to_t) else {
            return Err("internal error in View as".into());
        };
        let mut ok = vec![false; to.values.len()];
        for (i, k) in kept.iter().enumerate() {
            if *k && let Some(slot) = ok.get_mut(code(&to, i)) {
                *slot = true;
            }
        }
        let n = from.codes.len();
        Ok((0..n)
            .map(|i| {
                map.get(code(&from, i))
                    .copied()
                    .filter(|t| *t != NONE)
                    .and_then(|t| ok.get(t as usize).copied())
                    .unwrap_or(false)
            })
            .collect())
    } else {
        let Some(Some(kept)) = masks.get(r.from_t) else {
            return Err("internal error in View as".into());
        };
        let mut ok = vec![false; to.values.len()];
        for (i, k) in kept.iter().enumerate() {
            if *k
                && let Some(t) = map.get(code(&from, i)).copied().filter(|t| *t != NONE)
                && let Some(slot) = ok.get_mut(t as usize)
            {
                *slot = true;
            }
        }
        let n = to.codes.len();
        Ok((0..n)
            .map(|i| ok.get(code(&to, i)).copied().unwrap_or(false))
            .collect())
    }
}
