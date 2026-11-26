//! `List.*`.

use std::rc::Rc;

use super::{Builtin, arg, b, conv_err, count, decimal_precision, func, items, list, number, opt_number};
use crate::m::ast::BinOp;
use crate::m::eval::{Interp, compare_values, values_equal};
use crate::m::lib::table::hash_key;
use crate::m::value::{Func, Native, Num, Res, Value, expr_err, unsupported};

pub(crate) static FUNCS: &[Builtin] = &[
    b("List.Count", 1, 1, count_fn),
    b("List.NonNullCount", 1, 1, non_null_count),
    b("List.IsEmpty", 1, 1, is_empty),
    b("List.Sum", 1, 2, sum),
    b("List.Product", 1, 2, product),
    b("List.Average", 1, 2, average),
    b("List.Min", 1, 4, min),
    b("List.Max", 1, 4, max),
    b("List.Median", 1, 2, median),
    b("List.First", 1, 2, first),
    b("List.Last", 1, 2, last),
    b("List.FirstN", 2, 2, first_n),
    b("List.LastN", 1, 2, last_n),
    b("List.Skip", 1, 2, skip),
    b("List.RemoveFirstN", 1, 2, skip),
    b("List.RemoveLastN", 1, 2, remove_last_n),
    b("List.Range", 2, 3, range),
    b("List.Reverse", 1, 1, reverse),
    b("List.Distinct", 1, 2, distinct),
    b("List.Sort", 1, 2, sort),
    b("List.Transform", 2, 2, transform),
    b("List.TransformMany", 3, 3, transform_many),
    b("List.Select", 2, 2, select),
    b("List.Contains", 2, 3, contains),
    b("List.ContainsAny", 2, 3, contains_any),
    b("List.ContainsAll", 2, 3, contains_all),
    b("List.PositionOf", 2, 4, position_of),
    b("List.Combine", 1, 1, combine),
    b("List.Zip", 1, 1, zip),
    b("List.Numbers", 2, 3, numbers),
    b("List.Dates", 3, 3, dates),
    b("List.DateTimes", 3, 3, datetimes),
    b("List.Durations", 3, 3, durations),
    b("List.Times", 3, 3, times),
    b("List.Repeat", 2, 2, repeat),
    b("List.RemoveNulls", 1, 1, remove_nulls),
    b("List.RemoveItems", 2, 2, remove_items),
    b("List.Difference", 2, 3, difference),
    b("List.Intersect", 1, 2, intersect),
    b("List.Union", 1, 2, union),
    b("List.Accumulate", 3, 3, accumulate),
    b("List.Generate", 3, 4, generate),
    b("List.AnyTrue", 1, 1, any_true),
    b("List.AllTrue", 1, 1, all_true),
    b("List.MatchesAny", 2, 2, matches_any),
    b("List.MatchesAll", 2, 2, matches_all),
    b("List.Single", 1, 1, single),
    b("List.SingleOrDefault", 1, 2, single_or_default),
    b("List.Buffer", 1, 1, identity),
    b("List.InsertRange", 3, 3, insert_range),
    b("List.ReplaceValue", 4, 4, replace_value),
    b("List.Split", 2, 2, split),
    b("List.Alternate", 2, 4, alternate),
    b("List.StandardDeviation", 1, 1, std_dev),
    b("List.Mode", 1, 2, mode),
];

fn identity(_: &mut Interp, a: &[Value]) -> Res<Value> {
    Ok(arg(a, 0).clone())
}

fn count_fn(_: &mut Interp, a: &[Value]) -> Res<Value> {
    Ok(Value::num(list(arg(a, 0))?.len() as f64))
}

fn non_null_count(_: &mut Interp, a: &[Value]) -> Res<Value> {
    Ok(Value::num(
        items(arg(a, 0))?.iter().filter(|v| !v.is_null()).count() as f64,
    ))
}

fn is_empty(_: &mut Interp, a: &[Value]) -> Res<Value> {
    Ok(Value::Logical(list(arg(a, 0))?.is_empty()))
}

/// Numbers or durations, nulls skipped.
pub(crate) fn numeric(interp: &mut Interp, v: &Value) -> Res<(Vec<f64>, bool)> {
    let xs = items(v)?;
    interp.tick(xs.len() as u64)?;
    let mut out = Vec::with_capacity(xs.len());
    let mut durations = false;
    for x in &xs {
        match x {
            Value::Null => {}
            Value::Number(n) => out.push(n.f()),
            Value::Duration(d) => {
                durations = true;
                out.push(*d as f64);
            }
            other => return conv_err(other, "Number"),
        }
    }
    Ok((out, durations))
}

fn wrap(n: f64, durations: bool) -> Res<Value> {
    if durations {
        Ok(Value::Duration(crate::m::eval::ms_from_f64(n)?))
    } else {
        Ok(Value::num(n))
    }
}

/// The numbers of a list as they are (nulls skipped), for `Precision.Decimal`; `None` when
/// the list holds durations.
fn exact_numbers(interp: &mut Interp, v: &Value) -> Res<Option<Vec<Num>>> {
    let xs = items(v)?;
    interp.tick(xs.len() as u64)?;
    let mut out = Vec::with_capacity(xs.len());
    for x in &xs {
        match x {
            Value::Null => {}
            Value::Number(n) => out.push(*n),
            Value::Duration(_) => return Ok(None),
            other => return conv_err(other, "Number"),
        }
    }
    Ok(Some(out))
}

/// The decimal sum of numbers (`Precision.Decimal`); `None` for an empty list.
fn decimal_sum(xs: &[Num]) -> Res<Option<Value>> {
    let mut it = xs.iter();
    let Some(first) = it.next() else {
        return Ok(None);
    };
    let Some(d) = first.dec() else {
        return expr_err("The number is out of the range of a decimal.");
    };
    let mut acc = Value::dec(d);
    for x in it {
        acc = match acc {
            Value::Number(s @ Num::D(_)) => super::misc::decimal_op(s, *x, BinOp::Add)?,
            // An overflow already gave an infinity.
            other => other,
        };
    }
    Ok(Some(acc))
}

fn sum(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    if decimal_precision(arg(a, 1))?
        && let Some(xs) = exact_numbers(interp, arg(a, 0))?
    {
        return Ok(decimal_sum(&xs)?.unwrap_or(Value::Null));
    }
    let (xs, d) = numeric(interp, arg(a, 0))?;
    if xs.is_empty() {
        return Ok(Value::Null);
    }
    wrap(xs.iter().sum(), d)
}

fn product(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let (xs, _) = numeric(interp, arg(a, 0))?;
    if xs.is_empty() {
        return Ok(Value::Null);
    }
    Ok(Value::num(xs.iter().product()))
}

/// `List.Average`. In double precision (the default) Desktop computes `x0 + Σ(x - x0) / n`
/// (deviations from the first value), which is what its last digits show; with
/// `Precision.Decimal`, the decimal sum divided by the count.
fn average(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    if decimal_precision(arg(a, 1))?
        && let Some(xs) = exact_numbers(interp, arg(a, 0))?
    {
        let Some(s) = decimal_sum(&xs)? else {
            return Ok(Value::Null);
        };
        return match s {
            Value::Number(s @ Num::D(_)) => super::misc::decimal_op(s, Num::F(xs.len() as f64), BinOp::Div),
            other => Ok(other),
        };
    }
    let (xs, d) = numeric(interp, arg(a, 0))?;
    let Some(x0) = xs.first().copied() else {
        return Ok(Value::Null);
    };
    let dev: f64 = xs.iter().map(|x| x - x0).sum();
    wrap(x0 + dev / xs.len() as f64, d)
}

fn median(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let (mut xs, d) = numeric(interp, arg(a, 0))?;
    if xs.is_empty() {
        return Ok(Value::Null);
    }
    xs.sort_by(f64::total_cmp);
    let n = xs.len();
    let m = if n % 2 == 1 {
        xs.get(n / 2).copied().unwrap_or(0.0)
    } else {
        (xs.get(n / 2 - 1).copied().unwrap_or(0.0) + xs.get(n / 2).copied().unwrap_or(0.0)) / 2.0
    };
    wrap(m, d)
}

fn std_dev(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let (xs, _) = numeric(interp, arg(a, 0))?;
    if xs.len() < 2 {
        return expr_err("The list must contain at least two values.");
    }
    let mean = xs.iter().sum::<f64>() / xs.len() as f64;
    let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (xs.len() - 1) as f64;
    Ok(Value::num(var.sqrt()))
}

fn extreme(interp: &mut Interp, a: &[Value], max: bool) -> Res<Value> {
    let xs = items(arg(a, 0))?;
    interp.tick(xs.len() as u64)?;
    let mut best: Option<&Value> = None;
    for x in xs.iter().filter(|x| !x.is_null()) {
        best = match best {
            None => Some(x),
            Some(b) => {
                let o = compare_values(x, b, false);
                if (max && o.is_gt()) || (!max && o.is_lt()) {
                    Some(x)
                } else {
                    Some(b)
                }
            }
        };
    }
    Ok(best.cloned().unwrap_or_else(|| arg(a, 1).clone()))
}

fn min(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    extreme(interp, a, false)
}

fn max(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    extreme(interp, a, true)
}

/// `List.Mode`: among the most frequent values, the one whose first appearance comes last
/// (Learn's example: `List.Mode({"A", 1, 2, 3, 3, 4, 5, 5})` is 5).
fn mode(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let counts = super::list_more::value_counts(interp, items(arg(a, 0))?, ignore_case(arg(a, 1)))?;
    let max = counts.iter().map(|(_, c)| *c).max().unwrap_or(0);
    match counts.into_iter().rev().find(|(_, c)| *c == max) {
        Some((v, _)) => Ok(v),
        None => expr_err("The list is empty."),
    }
}

fn first(_: &mut Interp, a: &[Value]) -> Res<Value> {
    match list(arg(a, 0))?.first() {
        Some(v) => v.clone().check(),
        None => Ok(arg(a, 1).clone()),
    }
}

fn last(_: &mut Interp, a: &[Value]) -> Res<Value> {
    match list(arg(a, 0))?.last() {
        Some(v) => v.clone().check(),
        None => Ok(arg(a, 1).clone()),
    }
}

pub(crate) fn count_or_while(interp: &mut Interp, l: &[Value], v: &Value, rev: bool) -> Res<usize> {
    match v {
        Value::Function(_) => {
            let mut n = 0;
            let it: Vec<&Value> = if rev {
                l.iter().rev().collect()
            } else {
                l.iter().collect()
            };
            for x in it {
                interp.tick(1)?;
                match interp.call(v, vec![x.clone()])? {
                    Value::Logical(true) => n += 1,
                    _ => break,
                }
            }
            Ok(n)
        }
        Value::Null => Ok(1),
        other => count(other),
    }
}

fn first_n(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let l = list(arg(a, 0))?.clone();
    let n = count_or_while(interp, &l, arg(a, 1), false)?.min(l.len());
    Ok(Value::list(l.iter().take(n).cloned().collect()))
}

fn last_n(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let l = list(arg(a, 0))?.clone();
    let n = count_or_while(interp, &l, arg(a, 1), true)?.min(l.len());
    Ok(Value::list(l.iter().skip(l.len() - n).cloned().collect()))
}

fn skip(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let l = list(arg(a, 0))?.clone();
    let n = count_or_while(interp, &l, arg(a, 1), false)?.min(l.len());
    Ok(Value::list(l.iter().skip(n).cloned().collect()))
}

fn remove_last_n(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let l = list(arg(a, 0))?.clone();
    let n = count_or_while(interp, &l, arg(a, 1), true)?.min(l.len());
    Ok(Value::list(l.iter().take(l.len() - n).cloned().collect()))
}

fn range(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let l = list(arg(a, 0))?;
    let off = count(arg(a, 1))?.min(l.len());
    let n = match arg(a, 2) {
        Value::Null => l.len() - off,
        v => count(v)?.min(l.len() - off),
    };
    Ok(Value::list(l.iter().skip(off).take(n).cloned().collect()))
}

fn reverse(_: &mut Interp, a: &[Value]) -> Res<Value> {
    Ok(Value::list(list(arg(a, 0))?.iter().rev().cloned().collect()))
}

pub(crate) fn ignore_case(v: &Value) -> bool {
    matches!(v, Value::Function(f) if matches!(&**f, Func::Native(Native::Comparer { ignore_case: true })))
}

pub(crate) fn eq_ci(a: &Value, b: &Value, ci: bool) -> bool {
    match (a, b) {
        (Value::Text(x), Value::Text(y)) if ci => x.to_uppercase() == y.to_uppercase(),
        _ => values_equal(a, b),
    }
}

fn distinct(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let xs = items(arg(a, 0))?;
    let ci = ignore_case(arg(a, 1));
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<Value> = Vec::new();
    for x in &xs {
        interp.tick(1)?;
        match hash_key(&[x], ci) {
            Some(h) => {
                if seen.insert(h) {
                    out.push(x.clone());
                }
            }
            None => {
                if !out.iter().any(|y| eq_ci(x, y, ci)) {
                    out.push(x.clone());
                }
            }
        }
    }
    Ok(Value::list(out))
}

fn sort(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let xs = items(arg(a, 0))?;
    Ok(Value::list(sort_values(interp, xs, arg(a, 1))?))
}

/// `xs` sorted by the comparison criteria `crit` of `List.Sort` (stable).
pub(crate) fn sort_values(interp: &mut Interp, mut xs: Vec<Value>, crit: &Value) -> Res<Vec<Value>> {
    let n = xs.len() as u64;
    interp.tick(n.saturating_mul(64 - n.leading_zeros() as u64))?;
    let (desc, key) = match crit {
        Value::Null => (false, None),
        Value::Number(n) => (n.f() == 1.0, None),
        f @ Value::Function(_) if Interp::arity(f) == 1 => (false, Some(f.clone())),
        Value::List(l) => {
            let f = l.first().cloned();
            let desc = matches!(l.get(1), Some(Value::Number(n)) if n.f() == 1.0);
            (desc, f)
        }
        Value::Function(_) => return unsupported("a two-argument comparer function in `List.Sort`"),
        other => return conv_err(other, "Number"),
    };
    let keys: Vec<Value> = match &key {
        Some(f) => xs
            .iter()
            .map(|x| interp.call(f, vec![x.clone()]))
            .collect::<Res<_>>()?,
        None => xs.clone(),
    };
    let mut idx: Vec<usize> = (0..xs.len()).collect();
    idx.sort_by(|&i, &j| {
        let o = match (keys.get(i), keys.get(j)) {
            (Some(x), Some(y)) => compare_values(x, y, false),
            _ => std::cmp::Ordering::Equal,
        };
        if desc { o.reverse() } else { o }
    });
    let sorted: Vec<Value> = idx.iter().filter_map(|&i| xs.get(i).cloned()).collect();
    xs = sorted;
    Ok(xs)
}

fn transform(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let l = list(arg(a, 0))?.clone();
    let f = func(arg(a, 1))?.clone();
    let mut out = Vec::with_capacity(l.len());
    for x in l.iter() {
        interp.tick(1)?;
        let x = x.clone().check()?;
        out.push(Value::store(interp.call(&f, vec![x]))?);
    }
    Ok(Value::list(out))
}

fn transform_many(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let l = items(arg(a, 0))?;
    let coll = func(arg(a, 1))?.clone();
    let res = func(arg(a, 2))?.clone();
    let mut out = Vec::new();
    for x in &l {
        let inner = interp.call(&coll, vec![x.clone()])?;
        for y in items(&inner)? {
            interp.tick(1)?;
            out.push(interp.call(&res, vec![x.clone(), y])?);
            interp.check_rows(out.len())?;
        }
    }
    Ok(Value::list(out))
}

fn select(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let l = items(arg(a, 0))?;
    let f = func(arg(a, 1))?.clone();
    let mut out = Vec::new();
    for x in l {
        interp.tick(1)?;
        match interp.call(&f, vec![x.clone()])? {
            Value::Logical(true) => out.push(x),
            Value::Logical(false) | Value::Null => {}
            other => return conv_err(&other, "Logical"),
        }
    }
    Ok(Value::list(out))
}

fn contains(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let l = items(arg(a, 0))?;
    interp.tick(l.len() as u64)?;
    let ci = ignore_case(arg(a, 2));
    Ok(Value::Logical(l.iter().any(|x| eq_ci(x, arg(a, 1), ci))))
}

fn contains_any(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let l = items(arg(a, 0))?;
    let v = items(arg(a, 1))?;
    interp.tick((l.len() as u64).saturating_mul(v.len() as u64))?;
    let ci = ignore_case(arg(a, 2));
    Ok(Value::Logical(
        v.iter().any(|y| l.iter().any(|x| eq_ci(x, y, ci))),
    ))
}

fn contains_all(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let l = items(arg(a, 0))?;
    let v = items(arg(a, 1))?;
    interp.tick((l.len() as u64).saturating_mul(v.len() as u64))?;
    let ci = ignore_case(arg(a, 2));
    Ok(Value::Logical(
        v.iter().all(|y| l.iter().any(|x| eq_ci(x, y, ci))),
    ))
}

fn position_of(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let l = items(arg(a, 0))?;
    interp.tick(l.len() as u64)?;
    let ci = ignore_case(arg(a, 3));
    let occ = opt_number(arg(a, 2))?.unwrap_or(0.0);
    let hits: Vec<f64> = l
        .iter()
        .enumerate()
        .filter(|(_, x)| eq_ci(x, arg(a, 1), ci))
        .map(|(i, _)| i as f64)
        .collect();
    Ok(match occ as i64 {
        2 => Value::list(hits.into_iter().map(Value::num).collect()),
        1 => Value::num(hits.last().copied().unwrap_or(-1.0)),
        _ => Value::num(hits.first().copied().unwrap_or(-1.0)),
    })
}

fn combine(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let ls = items(arg(a, 0))?;
    let mut out = Vec::new();
    for l in &ls {
        let l = list(l)?;
        interp.tick(l.len() as u64)?;
        out.extend(l.iter().cloned());
        interp.check_rows(out.len())?;
    }
    Ok(Value::list(out))
}

fn zip(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let ls: Vec<Rc<Vec<Value>>> = items(arg(a, 0))?
        .iter()
        .map(|l| list(l).cloned())
        .collect::<Res<_>>()?;
    let n = ls.iter().map(|l| l.len()).max().unwrap_or(0);
    interp.tick(n as u64)?;
    Ok(Value::list(
        (0..n)
            .map(|i| {
                Value::list(
                    ls.iter()
                        .map(|l| l.get(i).cloned().unwrap_or(Value::Null))
                        .collect(),
                )
            })
            .collect(),
    ))
}

fn generate_n(interp: &mut Interp, n: usize, mut f: impl FnMut(usize) -> Res<Value>) -> Res<Value> {
    interp.check_rows(n)?;
    interp.tick(n as u64)?;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        out.push(f(i)?);
    }
    Ok(Value::list(out))
}

fn numbers(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let start = number(arg(a, 0))?;
    let n = count(arg(a, 1))?;
    let step = opt_number(arg(a, 2))?.unwrap_or(1.0);
    generate_n(interp, n, |i| Ok(Value::num(start + step * i as f64)))
}

fn step_ms(v: &Value) -> Res<i64> {
    match v {
        Value::Duration(d) => Ok(*d),
        other => conv_err(other, "Duration"),
    }
}

fn dates(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let Value::Date(start) = arg(a, 0) else {
        return conv_err(arg(a, 0), "Date");
    };
    let start = *start;
    let n = count(arg(a, 1))?;
    let step = step_ms(arg(a, 2))?;
    generate_n(interp, n, |i| {
        let ms = step
            .checked_mul(i as i64)
            .and_then(|x| x.checked_add(start.checked_mul(86_400_000)?));
        match ms {
            Some(ms) => Ok(Value::Date(ms.div_euclid(86_400_000))),
            None => expr_err("The date is out of range."),
        }
    })
}

fn datetimes(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let Value::DateTime(start) = arg(a, 0) else {
        return conv_err(arg(a, 0), "DateTime");
    };
    let start = *start;
    let n = count(arg(a, 1))?;
    let step = step_ms(arg(a, 2))?;
    generate_n(interp, n, |i| {
        match step.checked_mul(i as i64).and_then(|x| x.checked_add(start)) {
            Some(ms) => Ok(Value::DateTime(ms)),
            None => expr_err("The datetime is out of range."),
        }
    })
}

fn durations(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let start = step_ms(arg(a, 0))?;
    let n = count(arg(a, 1))?;
    let step = step_ms(arg(a, 2))?;
    generate_n(interp, n, |i| {
        match step.checked_mul(i as i64).and_then(|x| x.checked_add(start)) {
            Some(ms) => Ok(Value::Duration(ms)),
            None => expr_err("The duration is out of range."),
        }
    })
}

fn times(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let Value::Time(start) = arg(a, 0) else {
        return conv_err(arg(a, 0), "Time");
    };
    let start = *start;
    let n = count(arg(a, 1))?;
    let step = step_ms(arg(a, 2))?;
    generate_n(interp, n, |i| {
        match step.checked_mul(i as i64).and_then(|x| x.checked_add(start)) {
            Some(ms) if (0..86_400_000).contains(&ms) => Ok(Value::Time(ms)),
            _ => expr_err("The time is out of range."),
        }
    })
}

fn repeat(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let l = list(arg(a, 0))?.clone();
    let n = count(arg(a, 1))?;
    let total = l.len().saturating_mul(n);
    generate_n(interp, total, |i| {
        Ok(l.get(i % l.len().max(1)).cloned().unwrap_or(Value::Null))
    })
}

fn remove_nulls(_: &mut Interp, a: &[Value]) -> Res<Value> {
    Ok(Value::list(
        list(arg(a, 0))?
            .iter()
            .filter(|v| !v.is_null())
            .cloned()
            .collect(),
    ))
}

fn remove_items(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let l = items(arg(a, 0))?;
    let r = items(arg(a, 1))?;
    interp.tick((l.len() as u64).saturating_mul(r.len() as u64))?;
    Ok(Value::list(
        l.into_iter()
            .filter(|x| !r.iter().any(|y| values_equal(x, y)))
            .collect(),
    ))
}

fn difference(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let l = items(arg(a, 0))?;
    let mut r = items(arg(a, 1))?;
    interp.tick((l.len() as u64).saturating_mul(r.len() as u64))?;
    let ci = ignore_case(arg(a, 2));
    let mut out = Vec::new();
    for x in l {
        match r.iter().position(|y| eq_ci(&x, y, ci)) {
            Some(p) => {
                r.remove(p);
            }
            None => out.push(x),
        }
    }
    Ok(Value::list(out))
}

fn intersect(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let ls: Vec<Vec<Value>> = items(arg(a, 0))?.iter().map(items).collect::<Res<_>>()?;
    let ci = ignore_case(arg(a, 1));
    let Some((first, rest)) = ls.split_first() else {
        return Ok(Value::list(Vec::new()));
    };
    let mut rest: Vec<Vec<Value>> = rest.to_vec();
    let mut out = Vec::new();
    for x in first {
        interp.tick(rest.len() as u64 + 1)?;
        let pos: Vec<Option<usize>> = rest
            .iter()
            .map(|l| l.iter().position(|y| eq_ci(x, y, ci)))
            .collect();
        if pos.iter().all(Option::is_some) {
            for (l, p) in rest.iter_mut().zip(pos) {
                if let Some(p) = p {
                    l.remove(p);
                }
            }
            out.push(x.clone());
        }
    }
    Ok(Value::list(out))
}

fn union(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let ls: Vec<Vec<Value>> = items(arg(a, 0))?.iter().map(items).collect::<Res<_>>()?;
    let ci = ignore_case(arg(a, 1));
    let mut out: Vec<Value> = Vec::new();
    for l in ls {
        let mut avail: Vec<bool> = vec![true; out.len()];
        for x in l {
            interp.tick(1)?;
            match out
                .iter()
                .zip(avail.iter())
                .position(|(y, ok)| *ok && eq_ci(&x, y, ci))
            {
                Some(p) => {
                    if let Some(s) = avail.get_mut(p) {
                        *s = false;
                    }
                }
                None => out.push(x),
            }
        }
    }
    Ok(Value::list(out))
}

fn accumulate(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let l = items(arg(a, 0))?;
    let mut acc = arg(a, 1).clone();
    let f = func(arg(a, 2))?.clone();
    for x in l {
        interp.tick(1)?;
        acc = interp.call(&f, vec![acc, x])?;
    }
    Ok(acc)
}

fn generate(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let init = func(arg(a, 0))?.clone();
    let cond = func(arg(a, 1))?.clone();
    let next = func(arg(a, 2))?.clone();
    let sel = arg(a, 3).clone();
    let mut state = interp.call(&init, vec![])?;
    let mut out = Vec::new();
    loop {
        interp.tick(1)?;
        match interp.call(&cond, vec![state.clone()])? {
            Value::Logical(true) => {}
            Value::Logical(false) => break,
            other => return conv_err(&other, "Logical"),
        }
        out.push(match &sel {
            Value::Null => state.clone(),
            f => interp.call(f, vec![state.clone()])?,
        });
        interp.check_rows(out.len())?;
        state = interp.call(&next, vec![state])?;
    }
    Ok(Value::list(out))
}

fn any_true(_: &mut Interp, a: &[Value]) -> Res<Value> {
    Ok(Value::Logical(
        items(arg(a, 0))?
            .iter()
            .any(|v| matches!(v, Value::Logical(true))),
    ))
}

fn all_true(_: &mut Interp, a: &[Value]) -> Res<Value> {
    Ok(Value::Logical(
        items(arg(a, 0))?
            .iter()
            .all(|v| matches!(v, Value::Logical(true))),
    ))
}

fn matches(interp: &mut Interp, a: &[Value], all: bool) -> Res<Value> {
    let l = items(arg(a, 0))?;
    let f = func(arg(a, 1))?.clone();
    for x in l {
        interp.tick(1)?;
        let hit = matches!(interp.call(&f, vec![x])?, Value::Logical(true));
        if hit != all {
            return Ok(Value::Logical(!all));
        }
    }
    Ok(Value::Logical(all))
}

fn matches_any(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    matches(interp, a, false)
}

fn matches_all(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    matches(interp, a, true)
}

fn single(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let l = list(arg(a, 0))?;
    match l.as_slice() {
        [v] => v.clone().check(),
        _ => expr_err("There were too many elements in the enumeration to complete the operation."),
    }
}

fn single_or_default(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let l = list(arg(a, 0))?;
    match l.as_slice() {
        [] => Ok(arg(a, 1).clone()),
        [v] => v.clone().check(),
        _ => expr_err("There were too many elements in the enumeration to complete the operation."),
    }
}

fn insert_range(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let l = list(arg(a, 0))?.clone();
    let i = count(arg(a, 1))?;
    let v = list(arg(a, 2))?.clone();
    if i > l.len() {
        return expr_err("The index is beyond the end of the list.");
    }
    interp.check_rows(l.len().saturating_add(v.len()))?;
    let mut out: Vec<Value> = l.iter().take(i).cloned().collect();
    out.extend(v.iter().cloned());
    out.extend(l.iter().skip(i).cloned());
    Ok(Value::list(out))
}

fn replace_value(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let l = items(arg(a, 0))?;
    let replacer = func(arg(a, 3))?.clone();
    let mut out = Vec::with_capacity(l.len());
    for x in l {
        interp.tick(1)?;
        out.push(interp.call(&replacer, vec![x, arg(a, 1).clone(), arg(a, 2).clone()])?);
    }
    Ok(Value::list(out))
}

fn split(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let l = list(arg(a, 0))?;
    let n = count(arg(a, 1))?;
    if n == 0 {
        return expr_err("The page size must be greater than zero.");
    }
    Ok(Value::list(
        l.chunks(n).map(|c| Value::list(c.to_vec())).collect(),
    ))
}

fn alternate(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let l = list(arg(a, 0))?;
    let c = count(arg(a, 1))?;
    // Without a repeat interval, only the first `count` values (after the offset) are skipped.
    let r = opt_number(arg(a, 2))?.map_or(Ok(usize::MAX / 2), |_| count(arg(a, 2)))?;
    let off = opt_number(arg(a, 3))?.map_or(Ok(0), |_| count(arg(a, 3)))?;
    let period = c.saturating_add(r).max(1);
    Ok(Value::list(
        l.iter()
            .enumerate()
            .filter(|(i, _)| *i < off || (i - off) % period >= c)
            .map(|(_, v)| v.clone())
            .collect(),
    ))
}
