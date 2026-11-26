// Ported from DashCraft (MIT OR Apache-2.0, used here under MIT),
// crates/pbi-m/src/stdlib/list.rs@0958fd82b04656ee1797101a2b4df8d114dfebd1
// Copyright (c) 2026 Jack Sweeney and the DashCraft contributors. Licensed under the MIT License.
// See crates/data/LICENSE-MIT-dashcraft and THIRD_PARTY_NOTICES.md at the repository root.

//! More `List.*` functions (ROADMAP M11.34): `Modes`, `Covariance`, `Percentile`, `MaxN`,
//! `MinN`, `PositionOfAny`, `Positions`, `IsDistinct`, `RemoveMatchingItems`, `RemoveRange`,
//! `ReplaceMatchingItems`, `ReplaceRange`, `DateTimeZones`. Each follows its Microsoft Learn
//! reference page (`docs/formats/data-sources.md`, *Library*); the logic is DashCraft's,
//! adapted to `data::m` values.

use super::list::{count_or_while, eq_ci, ignore_case, numeric, sort_values};
use super::{Builtin, arg, b, conv_err, count, items, list, opt_number, record};
use crate::m::eval::Interp;
use crate::m::lib::table::hash_key;
use crate::m::value::{Res, Value, expr_err};

pub(crate) static FUNCS: &[Builtin] = &[
    b("List.Modes", 1, 2, modes),
    b("List.Covariance", 2, 2, covariance),
    b("List.Percentile", 2, 3, percentile),
    b("List.MaxN", 2, 4, max_n),
    b("List.MinN", 2, 4, min_n),
    b("List.PositionOfAny", 2, 4, position_of_any),
    b("List.Positions", 1, 1, positions),
    b("List.IsDistinct", 1, 2, is_distinct),
    b("List.RemoveMatchingItems", 2, 3, remove_matching_items),
    b("List.RemoveRange", 2, 3, remove_range),
    b("List.ReplaceMatchingItems", 2, 3, replace_matching_items),
    b("List.ReplaceRange", 4, 4, replace_range),
    b("List.DateTimeZones", 3, 3, datetimezones),
];

const NOT_ENOUGH: &str = "There weren't enough elements in the enumeration to complete the operation.";

/// The distinct values of `values` (in order of first appearance) with their counts.
pub(crate) fn value_counts(interp: &mut Interp, values: Vec<Value>, ci: bool) -> Res<Vec<(Value, usize)>> {
    let mut index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut out: Vec<(Value, usize)> = Vec::new();
    for v in values {
        interp.tick(1)?;
        let slot = match hash_key(&[&v], ci) {
            Some(h) => match index.get(&h) {
                Some(&i) => Some(i),
                None => {
                    index.insert(h, out.len());
                    None
                }
            },
            // lists, records, tables: compared one by one
            None => out.iter().position(|(w, _)| eq_ci(w, &v, ci)),
        };
        match slot.and_then(|i| out.get_mut(i)) {
            Some(e) => e.1 = e.1.saturating_add(1),
            None => out.push((v, 1)),
        }
    }
    Ok(out)
}

/// `List.Modes`: every value that appears most often, in order of first appearance.
fn modes(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let counts = value_counts(interp, items(arg(a, 0))?, ignore_case(arg(a, 1)))?;
    let max = counts.iter().map(|(_, c)| *c).max().unwrap_or(0);
    Ok(Value::list(
        counts
            .into_iter()
            .filter(|(_, c)| *c == max)
            .map(|(v, _)| v)
            .collect(),
    ))
}

/// `List.Covariance`: the population covariance of two lists of numbers (running means).
fn covariance(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let (xs, _) = numeric(interp, arg(a, 0))?;
    let (ys, _) = numeric(interp, arg(a, 1))?;
    if xs.len() != ys.len() {
        return expr_err("The lists must have the same number of items.");
    }
    if xs.is_empty() {
        return Ok(Value::Null);
    }
    let (mut mx, mut my, mut mxy) = (0.0, 0.0, 0.0);
    for (i, (x, y)) in xs.iter().zip(&ys).enumerate() {
        let n = (i + 1) as f64;
        mx += (x - mx) / n;
        my += (y - my) / n;
        mxy += (x * y - mxy) / n;
    }
    Ok(Value::num(mxy - mx * my))
}

/// Whether `List.Percentile` can interpolate values of this kind.
fn percentile_kind(v: &Value) -> bool {
    matches!(
        v,
        Value::Number(_)
            | Value::Date(_)
            | Value::DateTime(_)
            | Value::DateTimeZone(..)
            | Value::Duration(_)
            | Value::Time(_)
    )
}

/// `lo + (hi - lo) * frac`, in the values' own type.
fn interpolate(interp: &mut Interp, lo: &Value, hi: &Value, frac: f64) -> Res<Value> {
    if frac == 0.0 {
        return Ok(lo.clone());
    }
    if let (Value::Number(x), Value::Number(y)) = (lo, hi) {
        let (x, y) = (x.f(), y.f());
        return Ok(Value::num(x + frac * (y - x)));
    }
    use crate::m::ast::BinOp;
    let delta = interp.binary(BinOp::Sub, hi.clone(), lo.clone())?;
    let step = interp.binary(BinOp::Mul, delta, Value::num(frac))?;
    interp.binary(BinOp::Add, lo.clone(), step)
}

/// `List.Percentile(list, percentiles, [PercentileMode])`: `ExcelInc` (1, the default) and
/// `SqlCont` (4) interpolate between ranks `(n - 1) p`, `ExcelExc` (2) between ranks
/// `(n + 1) p` (null outside 1..n), `SqlDisc` (3) takes the value at rank `ceil(n p)`.
fn percentile(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let values: Vec<Value> = items(arg(a, 0))?.into_iter().filter(|v| !v.is_null()).collect();
    if let Some(first) = values.first() {
        if values.iter().any(|v| v.kind() != first.kind()) {
            return expr_err(
                "List.Percentile requires a list where all the values have the same type or are null.",
            );
        }
        if !percentile_kind(first) {
            return expr_err(
                "List.Percentile requires a list of dates, datetimes, datetimezones, durations, numbers or times.",
            );
        }
    }
    let mode = match arg(a, 2) {
        Value::Null => 1.0,
        v => match record(v)?.get("PercentileMode") {
            None | Some(Value::Null) => 1.0,
            Some(m) => opt_number(m)?.unwrap_or(1.0),
        },
    };
    let sorted = sort_values(interp, values, &Value::Null)?;
    let n = sorted.len();
    let one = |interp: &mut Interp, p: &Value| -> Res<Value> {
        let Value::Number(p) = p else {
            return conv_err(p, "Number");
        };
        let p = p.f();
        if !(0.0..=1.0).contains(&p) {
            return expr_err("Percentile values must be between 0.0 and 1.0, inclusive.");
        }
        if n == 0 {
            return Ok(Value::Null);
        }
        // the value at rank `rank` (0-based after `offset`), interpolated with the next one
        let at = |interp: &mut Interp, rank: f64, offset: usize| -> Res<Value> {
            let lo = rank.floor() as usize;
            let frac = rank - rank.floor();
            let (Some(i), Some(j)) = (
                lo.checked_sub(offset),
                lo.saturating_add(1).checked_sub(offset).map(|j| j.min(n - 1)),
            ) else {
                return Ok(Value::Null);
            };
            match (sorted.get(i), sorted.get(j)) {
                (Some(x), Some(y)) => interpolate(interp, x, y, frac),
                _ => Ok(Value::Null),
            }
        };
        if mode == 2.0 {
            let rank = (n + 1) as f64 * p;
            if rank < 1.0 || rank > n as f64 {
                Ok(Value::Null)
            } else {
                at(interp, rank, 1)
            }
        } else if mode == 3.0 {
            let i = ((p * n as f64).ceil() as usize).saturating_sub(1).min(n - 1);
            Ok(sorted.get(i).cloned().unwrap_or(Value::Null))
        } else {
            at(interp, (n - 1) as f64 * p, 0)
        }
    };
    match arg(a, 1) {
        Value::List(l) => {
            let ps = l.clone();
            let mut out = Vec::with_capacity(ps.len());
            for p in ps.iter() {
                interp.tick(1)?;
                out.push(one(interp, &p.clone().check()?)?);
            }
            Ok(Value::list(out))
        }
        p => one(interp, p),
    }
}

/// `List.MaxN` / `List.MinN`: the list sorted (largest first for `MaxN`), then the first
/// `count` items or the leading items that match the condition; nulls are left out unless
/// `includeNulls`.
fn extreme_n(interp: &mut Interp, a: &[Value], largest: bool) -> Res<Value> {
    let include_nulls = match arg(a, 3) {
        Value::Null => false,
        Value::Logical(b) => *b,
        other => return conv_err(other, "Logical"),
    };
    let values: Vec<Value> = items(arg(a, 0))?
        .into_iter()
        .filter(|v| include_nulls || !v.is_null())
        .collect();
    let mut sorted = sort_values(interp, values, arg(a, 2))?;
    if largest {
        sorted.reverse();
    }
    let n = match arg(a, 1) {
        Value::Null => 0,
        v => count_or_while(interp, &sorted, v, false)?,
    }
    .min(sorted.len());
    sorted.truncate(n);
    Ok(Value::list(sorted))
}

fn max_n(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    extreme_n(interp, a, true)
}

fn min_n(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    extreme_n(interp, a, false)
}

/// `Occurrence.First` (default), `Occurrence.Last` or `Occurrence.All` of `hits`.
pub(crate) fn occurrence(v: &Value, hits: Vec<usize>) -> Res<Value> {
    let pos = |p: Option<&usize>| Value::num(p.map_or(-1.0, |&p| p as f64));
    Ok(match opt_number(v)? {
        None | Some(0.0) => pos(hits.first()),
        Some(1.0) => pos(hits.last()),
        Some(2.0) => Value::list(hits.into_iter().map(|p| Value::num(p as f64)).collect()),
        Some(_) => return expr_err("The specified Occurrence is invalid."),
    })
}

fn position_of_any(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let l = items(arg(a, 0))?;
    let wanted = items(arg(a, 1))?;
    interp.tick((l.len() as u64).saturating_mul(wanted.len() as u64))?;
    let ci = ignore_case(arg(a, 3));
    let hits: Vec<usize> = l
        .iter()
        .enumerate()
        .filter(|(_, x)| wanted.iter().any(|w| eq_ci(x, w, ci)))
        .map(|(i, _)| i)
        .collect();
    occurrence(arg(a, 2), hits)
}

fn positions(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let n = list(arg(a, 0))?.len();
    interp.tick(n as u64)?;
    Ok(Value::list((0..n).map(|i| Value::num(i as f64)).collect()))
}

fn is_distinct(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let xs = items(arg(a, 0))?;
    let n = xs.len();
    Ok(Value::Logical(
        value_counts(interp, xs, ignore_case(arg(a, 1)))?.len() == n,
    ))
}

fn remove_matching_items(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let l = items(arg(a, 0))?;
    let remove = items(arg(a, 1))?;
    interp.tick((l.len() as u64).saturating_mul(remove.len() as u64))?;
    let ci = ignore_case(arg(a, 2));
    Ok(Value::list(
        l.into_iter()
            .filter(|x| !remove.iter().any(|r| eq_ci(x, r, ci)))
            .collect(),
    ))
}

/// `l` with `remove` items from `index` replaced by `insert`.
fn splice(interp: &mut Interp, l: &[Value], index: usize, remove: usize, insert: &[Value]) -> Res<Value> {
    let Some(end) = index.checked_add(remove).filter(|&e| e <= l.len()) else {
        return expr_err(NOT_ENOUGH);
    };
    interp.check_rows(l.len().saturating_sub(remove).saturating_add(insert.len()))?;
    let mut out = Vec::with_capacity(l.len() - remove + insert.len());
    out.extend_from_slice(l.get(..index).unwrap_or(&[]));
    out.extend_from_slice(insert);
    out.extend_from_slice(l.get(end..).unwrap_or(&[]));
    Ok(Value::list(out))
}

fn remove_range(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let l = list(arg(a, 0))?.clone();
    let index = count(arg(a, 1))?;
    let n = match arg(a, 2) {
        Value::Null => 1,
        v => count(v)?,
    };
    splice(interp, &l, index, n, &[])
}

fn replace_range(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let l = list(arg(a, 0))?.clone();
    let index = count(arg(a, 1))?;
    let n = count(arg(a, 2))?;
    let with = list(arg(a, 3))?.clone();
    splice(interp, &l, index, n, &with)
}

/// `List.ReplaceMatchingItems(list, {{old, new}, …})`: each item equal to an `old` becomes its
/// `new` (the first pair that matches).
fn replace_matching_items(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let mut pairs = Vec::new();
    for r in items(arg(a, 1))? {
        let pair = items(&r)?;
        match pair.as_slice() {
            [old, new] => pairs.push((old.clone(), new.clone())),
            _ => return expr_err("A replacement must be a list of an old and a new value."),
        }
    }
    let ci = ignore_case(arg(a, 2));
    let l = items(arg(a, 0))?;
    interp.tick((l.len() as u64).saturating_mul(pairs.len().max(1) as u64))?;
    Ok(Value::list(
        l.into_iter()
            .map(|v| match pairs.iter().find(|(old, _)| eq_ci(&v, old, ci)) {
                Some((_, new)) => new.clone(),
                None => v,
            })
            .collect(),
    ))
}

/// `List.DateTimeZones(start, count, step)`.
fn datetimezones(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let Value::DateTimeZone(start, offset) = arg(a, 0) else {
        return conv_err(arg(a, 0), "DateTimeZone");
    };
    let (start, offset) = (*start, *offset);
    let n = count(arg(a, 1))?;
    let Value::Duration(step) = arg(a, 2) else {
        return conv_err(arg(a, 2), "Duration");
    };
    let step = *step;
    interp.check_rows(n)?;
    interp.tick(n as u64)?;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let ms = i64::try_from(i)
            .ok()
            .and_then(|i| step.checked_mul(i))
            .and_then(|x| x.checked_add(start));
        match ms {
            Some(ms) => out.push(crate::m::eval::in_range(Value::DateTimeZone(ms, offset))?),
            None => return expr_err("The datetimezone is out of range."),
        }
    }
    Ok(Value::list(out))
}
