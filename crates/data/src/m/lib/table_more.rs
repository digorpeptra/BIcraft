// Ported from DashCraft (MIT OR Apache-2.0, used here under MIT),
// crates/pbi-m/src/stdlib/table_rows.rs, table_ops.rs and table.rs@0958fd82b04656ee1797101a2b4df8d114dfebd1
// Copyright (c) 2026 Jack Sweeney and the DashCraft contributors. Licensed under the MIT License.
// See crates/data/LICENSE-MIT-dashcraft and THIRD_PARTY_NOTICES.md at the repository root.

//! More `Table.*` functions (ROADMAP M11.34): ranks and extremes (`AddRankColumn`, `MaxN`,
//! `MinN`), row lookups (`ContainsAll`, `ContainsAny`, `PositionOfAny`, `FindText`,
//! `IsDistinct`, `RemoveMatchingRows`, `ReplaceMatchingRows`), row slices (`AlternateRows`,
//! `RemoveRows`, `ReplaceRows`, `SingleRow`, `Split`, `SplitAt`, `Partition`), columns
//! (`AddJoinColumn`, `AggregateTableColumn`, `CombineColumnsToRecord`, `ColumnsOfType`,
//! `TransformColumnNames`), metadata (`Schema`, `ApproximateRowCount`) and `ToList`. Each
//! follows its Microsoft Learn reference page (`docs/formats/data-sources.md`, *Library*); the
//! logic is DashCraft's, adapted to `data::m` tables (rows of values).

use std::rc::Rc;

use super::table::{
    col, count_or_while, equation, group_rows, index_of, make, nested_join, sorted_rows, sub_table,
};
use super::{Builtin, arg, b, conv_err, count, func, items, names, opt_number, record, tbl, text};
use crate::m::eval::{Interp, values_equal};
use crate::m::value::{Column, Facet, Func, Kind, MType, Native, Record, Res, Table, Value, expr_err};

pub(crate) static FUNCS: &[Builtin] = &[
    b("Table.AddJoinColumn", 5, 5, add_join_column),
    b("Table.AddRankColumn", 3, 4, add_rank_column),
    b("Table.AggregateTableColumn", 3, 3, aggregate_table_column),
    b("Table.AlternateRows", 4, 4, alternate_rows),
    b("Table.ApproximateRowCount", 1, 1, approximate_row_count),
    b("Table.ColumnsOfType", 2, 2, columns_of_type),
    b("Table.CombineColumnsToRecord", 3, 4, combine_columns_to_record),
    b("Table.ContainsAll", 2, 3, contains_all),
    b("Table.ContainsAny", 2, 3, contains_any),
    b("Table.FindText", 2, 2, find_text),
    b("Table.IsDistinct", 1, 2, is_distinct),
    b("Table.MaxN", 3, 3, max_n),
    b("Table.MinN", 3, 3, min_n),
    b("Table.Partition", 4, 4, partition),
    b("Table.PositionOfAny", 2, 4, position_of_any),
    b("Table.RemoveMatchingRows", 2, 3, remove_matching_rows),
    b("Table.RemoveRows", 2, 3, remove_rows),
    b("Table.ReplaceMatchingRows", 2, 3, replace_matching_rows),
    b("Table.ReplaceRows", 4, 4, replace_rows),
    b("Table.Schema", 1, 1, schema),
    b("Table.SingleRow", 1, 1, single_row),
    b("Table.Split", 2, 2, split),
    b("Table.SplitAt", 2, 2, split_at),
    b("Table.ToList", 1, 2, to_list),
    b("Table.TransformColumnNames", 2, 3, transform_column_names),
];

const NOT_ENOUGH: &str = "There weren't enough elements in the enumeration to complete the operation.";
const TOO_MANY: &str = "There were too many elements in the enumeration to complete the operation.";

fn out_of_range<T>(name: &str) -> Res<T> {
    expr_err(format!("The '{name}' argument is out of range."))
}

/// A non-negative whole argument named `name` in the out-of-range error; null gives `default`.
fn range_arg(v: &Value, name: &str, default: usize) -> Res<usize> {
    match opt_number(v)? {
        None => Ok(default),
        Some(n) if n >= 0.0 && n.fract() == 0.0 => Ok(if n >= usize::MAX as f64 {
            usize::MAX
        } else {
            n as usize
        }),
        Some(_) => out_of_range(name),
    }
}

// ---------------------------------------------------------------------------------------------
// Ranks and extremes
// ---------------------------------------------------------------------------------------------

/// `Table.MaxN` / `Table.MinN`: the rows sorted by the criteria (largest first for `MaxN`), then
/// the first `count` rows or the leading rows that match the condition.
fn extreme_n(interp: &mut Interp, a: &[Value], largest: bool) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let (mut order, cmp) = sorted_rows(interp, &t, arg(a, 1))?;
    if largest {
        // Descending and stable: rows that tie keep their order.
        order.sort_by(|&x, &y| cmp(y, x));
    }
    let sorted = sub_table(&t, &order);
    let n = match arg(a, 2) {
        Value::Null => 0,
        v => count_or_while(interp, &sorted, v, false)?,
    }
    .min(sorted.rows.len());
    let idx: Vec<usize> = (0..n).collect();
    Ok(Value::table(sub_table(&sorted, &idx)))
}

fn max_n(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    extreme_n(interp, a, true)
}

fn min_n(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    extreme_n(interp, a, false)
}

/// `Table.AddRankColumn(table, name, criteria, [RankKind])`: the rows in rank order with their
/// rank, `RankKind.Competition` (0, the default: ties share a rank, the next one skips),
/// `RankKind.Dense` (1: no gaps) or `RankKind.Ordinal` (2: every row its own rank).
fn add_rank_column(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let name = text(arg(a, 1))?;
    let kind = match arg(a, 3) {
        Value::Null => 0.0,
        v => match record(v)?.get("RankKind") {
            None | Some(Value::Null) => 0.0,
            Some(k) => opt_number(k)?.unwrap_or(0.0),
        },
    };
    if ![0.0, 1.0, 2.0].contains(&kind) {
        return expr_err("The specified RankKind is invalid.");
    }
    let (order, cmp) = sorted_rows(interp, &t, arg(a, 2))?;
    let mut rows = Vec::with_capacity(order.len());
    let (mut competition, mut dense) = (0usize, 0usize);
    let mut prev: Option<usize> = None;
    for (i, &r) in order.iter().enumerate() {
        interp.tick(1)?;
        let tie = prev.is_some_and(|p| cmp(p, r).is_eq());
        if !tie {
            competition = i + 1;
            dense += 1;
        }
        let rank = if kind == 1.0 {
            dense
        } else if kind == 2.0 {
            i + 1
        } else {
            competition
        };
        let mut row = t.rows.get(r).cloned().unwrap_or_default();
        row.push(Value::num(rank as f64));
        rows.push(row);
        prev = Some(r);
    }
    let mut cols = t.columns.clone();
    cols.push(col(name, MType::facet(Kind::Number, Facet::Int64).nullable()));
    make(interp, cols, rows)
}

// ---------------------------------------------------------------------------------------------
// Row lookups
// ---------------------------------------------------------------------------------------------

/// Equation criteria of the row lookups: the columns compared (default: the record's fields)
/// and whether texts compare ignoring case.
struct RowCriteria {
    columns: Option<Vec<Rc<str>>>,
    ignore_case: bool,
    /// Without columns, whether a row must equal the whole record (`PositionOf`, `Remove…`)
    /// rather than agree on the record's fields (`Contains…`).
    whole_row: bool,
}

fn is_ci(v: &Value) -> bool {
    matches!(v, Value::Function(f) if matches!(&**f, Func::Native(Native::Comparer { ignore_case: true })))
}

fn row_criteria(v: &Value, whole_row: bool) -> Res<RowCriteria> {
    let mut columns = Vec::new();
    let mut ignore_case = false;
    let mut add = |x: &Value| -> Res<()> {
        match x {
            Value::Text(n) => columns.push(n.clone()),
            Value::Function(_) => ignore_case |= is_ci(x),
            other => return conv_err(other, "Text"),
        }
        Ok(())
    };
    match v {
        Value::Null => {}
        Value::List(l) => {
            for x in l.iter() {
                match x {
                    Value::List(inner) => {
                        for y in inner.iter() {
                            add(y)?;
                        }
                    }
                    other => add(other)?,
                }
            }
        }
        other => add(other)?,
    }
    Ok(RowCriteria {
        columns: (!columns.is_empty()).then_some(columns),
        ignore_case,
        whole_row,
    })
}

fn equal(a: &Value, b: &Value, ci: bool) -> bool {
    match (a, b) {
        (Value::Text(x), Value::Text(y)) if ci => x.to_uppercase() == y.to_uppercase(),
        _ => values_equal(a, b),
    }
}

/// Whether `row` of `t` equals the record `rec` under `crit`.
fn row_matches(t: &Table, row: &[Value], rec: &Record, crit: &RowCriteria) -> Res<bool> {
    let missing = |n: &str| expr_err(format!("The field '{n}' of the record wasn't found."));
    match &crit.columns {
        Some(cols) => {
            for n in cols {
                let Some(want) = rec.get(n) else {
                    return missing(n);
                };
                // the row is compared as a record: a missing column is a missing field
                let Some(c) = t.index(n) else {
                    return missing(n);
                };
                let cell = row.get(c).unwrap_or(&Value::Null);
                if !equal(cell, want, crit.ignore_case) {
                    return Ok(false);
                }
            }
        }
        None => {
            if crit.whole_row && rec.names.len() != t.columns.len() {
                return Ok(false);
            }
            for (n, want) in rec.names.iter().zip(rec.values.iter()) {
                let Some(c) = t.index(n) else {
                    return Ok(false);
                };
                if !equal(row.get(c).unwrap_or(&Value::Null), want, crit.ignore_case) {
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
}

fn row_matches_any(t: &Table, row: &[Value], recs: &[Rc<Record>], crit: &RowCriteria) -> Res<bool> {
    for rec in recs {
        if row_matches(t, row, rec, crit)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn records_arg(v: &Value) -> Res<Vec<Rc<Record>>> {
    items(v)?.iter().map(|x| record(x).cloned()).collect()
}

fn contains_rows(interp: &mut Interp, a: &[Value], all: bool) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let recs = records_arg(arg(a, 1))?;
    let crit = row_criteria(arg(a, 2), false)?;
    for rec in &recs {
        let mut found = false;
        for row in &t.rows {
            interp.tick(1)?;
            if row_matches(&t, row, rec, &crit)? {
                found = true;
                break;
            }
        }
        if found != all {
            return Ok(Value::Logical(found));
        }
    }
    Ok(Value::Logical(all))
}

fn contains_all(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    contains_rows(interp, a, true)
}

fn contains_any(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    contains_rows(interp, a, false)
}

fn position_of_any(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let recs = records_arg(arg(a, 1))?;
    let crit = row_criteria(arg(a, 3), true)?;
    let mut hits = Vec::new();
    for (i, row) in t.rows.iter().enumerate() {
        interp.tick(recs.len().max(1) as u64)?;
        if row_matches_any(&t, row, &recs, &crit)? {
            hits.push(i);
        }
    }
    super::list_more::occurrence(arg(a, 2), hits)
}

fn remove_matching_rows(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let recs = records_arg(arg(a, 1))?;
    let crit = row_criteria(arg(a, 2), true)?;
    let mut keep = Vec::new();
    for (i, row) in t.rows.iter().enumerate() {
        interp.tick(recs.len().max(1) as u64)?;
        if !row_matches_any(&t, row, &recs, &crit)? {
            keep.push(i);
        }
    }
    Ok(Value::table(sub_table(&t, &keep)))
}

/// The cells of record `r` as a row of `columns` (missing fields are null).
fn record_row(r: &Record, columns: &[Column]) -> Vec<Value> {
    columns
        .iter()
        .map(|c| r.get(&c.name).cloned().unwrap_or(Value::Null))
        .collect()
}

/// `Table.ReplaceMatchingRows(table, {{oldRow, newRow}, …})`: each row equal to an old record
/// becomes the new one (the first pair that matches).
fn replace_matching_rows(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let l = items(arg(a, 1))?;
    // one pair may be given alone
    let pairs: Vec<Value> = if matches!(l.first(), Some(Value::Record(_))) {
        vec![Value::list(l)]
    } else {
        l
    };
    let mut replacements: Vec<(Rc<Record>, Rc<Record>)> = Vec::new();
    for p in &pairs {
        match items(p)?.as_slice() {
            [Value::Record(old), Value::Record(new)] => replacements.push((old.clone(), new.clone())),
            _ => return expr_err("Table.ReplaceMatchingRows expects {oldRow, newRow} pairs of records."),
        }
    }
    let crit = row_criteria(arg(a, 2), true)?;
    let mut rows = Vec::with_capacity(t.rows.len());
    for row in &t.rows {
        interp.tick(replacements.len().max(1) as u64)?;
        let mut new_row = None;
        for (old, new) in &replacements {
            if row_matches(&t, row, old, &crit)? {
                new_row = Some(record_row(new, &t.columns));
                break;
            }
        }
        rows.push(new_row.unwrap_or_else(|| row.clone()));
    }
    make(interp, t.columns.clone(), rows)
}

/// `Table.FindText(table, text)`: the rows with a cell that holds `text`, in a text or inside a
/// list or record (not tables, binaries or other values).
fn find_text(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let needle = text(arg(a, 1))?;
    let max_depth = interp.limits.max_eval_depth;
    let mut keep = Vec::new();
    for (i, row) in t.rows.iter().enumerate() {
        for v in row {
            interp.tick(1)?;
            if has_text(interp, v, needle, max_depth)? {
                keep.push(i);
                break;
            }
        }
    }
    Ok(Value::table(sub_table(&t, &keep)))
}

fn has_text(interp: &mut Interp, v: &Value, needle: &str, depth: usize) -> Res<bool> {
    if depth == 0 {
        return crate::m::value::fatal(crate::Error::LimitExceeded(
            "Table.FindText: values nested too deep".into(),
        ));
    }
    match v {
        Value::Text(s) => Ok(s.contains(needle)),
        Value::List(l) => {
            for x in l.iter() {
                interp.tick(1)?;
                if has_text(interp, x, needle, depth - 1)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        Value::Record(r) => {
            for x in &r.values {
                interp.tick(1)?;
                if has_text(interp, x, needle, depth - 1)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        _ => Ok(false),
    }
}

fn is_distinct(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let (cols, ci) = equation(&t, arg(a, 1))?;
    let keys: Vec<Vec<&Value>> = t
        .rows
        .iter()
        .map(|r| cols.iter().map(|&c| r.get(c).unwrap_or(&Value::Null)).collect())
        .collect();
    Ok(Value::Logical(
        group_rows(interp, &keys, ci)?.len() == t.rows.len(),
    ))
}

// ---------------------------------------------------------------------------------------------
// Row slices
// ---------------------------------------------------------------------------------------------

/// `Table.AlternateRows(table, offset, skip, take)`: the first `offset` rows, then alternately
/// `skip` rows left out and `take` rows kept.
fn alternate_rows(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let n = t.rows.len();
    let offset = range_arg(arg(a, 1), "offset", 0)?.min(n);
    let skip = range_arg(arg(a, 2), "skip", 0)?;
    let take = range_arg(arg(a, 3), "take", 0)?;
    let mut rows: Vec<usize> = (0..offset).collect();
    if skip.saturating_add(take) > 0 {
        let mut i = offset;
        while i < n {
            interp.tick(1)?;
            i = i.saturating_add(skip);
            let end = i.saturating_add(take).min(n);
            rows.extend(i.min(n)..end);
            i = end;
        }
    }
    Ok(Value::table(sub_table(&t, &rows)))
}

/// `t` with rows `offset..offset + remove` replaced by `insert`.
fn splice_rows(
    interp: &mut Interp,
    t: &Table,
    offset: usize,
    remove: usize,
    insert: Vec<Vec<Value>>,
) -> Res<Value> {
    let n = t.rows.len();
    let Some(end) = offset.checked_add(remove).filter(|&e| e <= n) else {
        return expr_err(NOT_ENOUGH);
    };
    interp.check_rows(n.saturating_sub(remove).saturating_add(insert.len()))?;
    let mut rows = Vec::with_capacity(n - remove + insert.len());
    rows.extend(t.rows.get(..offset).unwrap_or(&[]).iter().cloned());
    rows.extend(insert);
    rows.extend(t.rows.get(end..).unwrap_or(&[]).iter().cloned());
    make(interp, t.columns.clone(), rows)
}

fn remove_rows(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let offset = count(arg(a, 1))?;
    let n = match arg(a, 2) {
        Value::Null => 1,
        v => count(v)?,
    };
    splice_rows(interp, &t, offset, n, Vec::new())
}

fn replace_rows(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let offset = count(arg(a, 1))?;
    let n = count(arg(a, 2))?;
    let rows: Vec<Vec<Value>> = records_arg(arg(a, 3))?
        .iter()
        .map(|r| record_row(r, &t.columns))
        .collect();
    splice_rows(interp, &t, offset, n, rows)
}

fn single_row(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?;
    match t.rows.as_slice() {
        [] => expr_err(NOT_ENOUGH),
        [row] => Ok(crate::m::eval::row_record(t, row)),
        _ => expr_err(TOO_MANY),
    }
}

fn split(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let size = range_arg(arg(a, 1), "pageSize", 0)?;
    if size == 0 {
        return out_of_range("pageSize");
    }
    let all: Vec<usize> = (0..t.rows.len()).collect();
    interp.tick(all.len() as u64)?;
    Ok(Value::list(
        all.chunks(size)
            .map(|rows| Value::table(sub_table(&t, rows)))
            .collect(),
    ))
}

fn split_at(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let n = count(arg(a, 1))?.min(t.rows.len());
    interp.tick(t.rows.len() as u64)?;
    let first: Vec<usize> = (0..n).collect();
    let rest: Vec<usize> = (n..t.rows.len()).collect();
    Ok(Value::list(vec![
        Value::table(sub_table(&t, &first)),
        Value::table(sub_table(&t, &rest)),
    ]))
}

/// `Table.Partition(table, column, groups, hash)`: row `r` goes to table `hash(r[column])
/// mod groups`; a row whose hash modulo `groups` is negative or fractional is in no table.
fn partition(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let ci = index_of(&t, text(arg(a, 1))?)?;
    let groups = count(arg(a, 2))?;
    let hash = func(arg(a, 3))?.clone();
    interp.check_rows(groups)?;
    let mut buckets: Vec<Vec<usize>> = vec![Vec::new(); groups];
    if groups > 0 {
        for (r, row) in t.rows.iter().enumerate() {
            interp.tick(1)?;
            let v = row.get(ci).cloned().unwrap_or(Value::Null).check()?;
            let h = match interp.call(&hash, vec![v])? {
                Value::Number(n) => n.f(),
                other => return conv_err(&other, "Number"),
            };
            let m = h % groups as f64;
            if m >= 0.0
                && m.fract() == 0.0
                && let Some(bucket) = buckets.get_mut(m as usize)
            {
                bucket.push(r);
            }
        }
    }
    Ok(Value::list(
        buckets
            .iter()
            .map(|rows| Value::table(sub_table(&t, rows)))
            .collect(),
    ))
}

// ---------------------------------------------------------------------------------------------
// Columns
// ---------------------------------------------------------------------------------------------

/// `Table.AddJoinColumn`: `Table.NestedJoin` with `JoinKind.LeftOuter`.
fn add_join_column(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let mut args: Vec<Value> = a.iter().take(5).cloned().collect();
    args.push(Value::num(1.0));
    nested_join(interp, &args)
}

/// `Table.AggregateTableColumn(table, column, {{column, aggregation, name}, …})`: the nested
/// tables of `column` replaced by one column per aggregation (an empty table aggregates
/// `{null}`).
fn aggregate_table_column(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let ci = index_of(&t, text(arg(a, 1))?)?;
    let mut specs: Vec<(Rc<str>, Value, Rc<str>)> = Vec::new();
    for s in items(arg(a, 2))? {
        match items(&s)?.as_slice() {
            [c, f, n] => specs.push((text(c)?.into(), func(f)?.clone(), text(n)?.into())),
            _ => return expr_err("Table.AggregateTableColumn expects {column, function, name} lists."),
        }
    }
    let mut cols: Vec<Column> = Vec::with_capacity(t.columns.len() + specs.len());
    for (i, c) in t.columns.iter().enumerate() {
        if i == ci {
            cols.extend(specs.iter().map(|(_, _, n)| col(n.clone(), MType::any())));
        } else {
            cols.push(c.clone());
        }
    }
    let mut rows = Vec::with_capacity(t.rows.len());
    for row in &t.rows {
        interp.tick(specs.len().max(1) as u64)?;
        let nested = row.get(ci).cloned().unwrap_or(Value::Null).check()?;
        let mut values = Vec::with_capacity(specs.len());
        match &nested {
            Value::Null => values.resize(specs.len(), Value::Null),
            Value::Table(nt) => {
                for (c, f, _) in &specs {
                    let column: Vec<Value> = if nt.rows.is_empty() {
                        vec![Value::Null]
                    } else {
                        let i = index_of(nt, c)?;
                        nt.rows
                            .iter()
                            .map(|r| r.get(i).cloned().unwrap_or(Value::Null))
                            .collect()
                    };
                    values.push(Value::store(interp.call(f, vec![Value::list(column)]))?);
                }
            }
            other => return conv_err(other, "Table"),
        }
        let mut out = Vec::with_capacity(cols.len());
        for (i, v) in row.iter().enumerate() {
            if i == ci {
                out.append(&mut values);
            } else {
                out.push(v.clone());
            }
        }
        rows.push(out);
    }
    make(interp, cols, rows)
}

/// `Table.CombineColumnsToRecord(table, name, columns)`: the columns replaced by one record
/// column at the place of the first of them.
fn combine_columns_to_record(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let name = text(arg(a, 1))?;
    let src: Vec<usize> = names(arg(a, 2))?
        .iter()
        .map(|n| index_of(&t, n))
        .collect::<Res<_>>()?;
    let field_names: Rc<Vec<Rc<str>>> = Rc::new(
        src.iter()
            .filter_map(|&i| t.columns.get(i).map(|c| c.name.clone()))
            .collect(),
    );
    for (i, n) in field_names.iter().enumerate() {
        if field_names.iter().take(i).any(|m| m == n) {
            return expr_err(format!("The field '{n}' already exists in the record."));
        }
    }
    let first = src.iter().copied().min().unwrap_or(0);
    let mut cols: Vec<Column> = Vec::new();
    for (i, c) in t.columns.iter().enumerate() {
        if i == first {
            cols.push(col(name, MType::prim(Kind::Record)));
        } else if !src.contains(&i) {
            cols.push(c.clone());
        }
    }
    let mut rows = Vec::with_capacity(t.rows.len());
    for r in &t.rows {
        interp.tick(1)?;
        let rec = Value::Record(Rc::new(Record {
            names: field_names.clone(),
            values: src
                .iter()
                .map(|&i| r.get(i).cloned().unwrap_or(Value::Null))
                .collect(),
        }));
        let mut row = Vec::with_capacity(cols.len());
        for (i, v) in r.iter().enumerate() {
            if i == first {
                row.push(rec.clone());
            } else if !src.contains(&i) {
                row.push(v.clone());
            }
        }
        rows.push(row);
    }
    make(interp, cols, rows)
}

/// Whether values of type `a` are always compatible with type `b` (`Type.Is`).
pub(crate) fn type_compatible(a: &MType, b: &MType) -> bool {
    match b.kind {
        Kind::Any => true,
        Kind::AnyNonNull => !a.nullable && a.kind != Kind::Null,
        _ if a.kind == Kind::None => true,
        _ if a.kind == Kind::Null => b.nullable || b.kind == Kind::Null,
        k => a.kind == k && (b.nullable || !a.nullable),
    }
}

fn columns_of_type(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?;
    let wanted: Vec<MType> = items(arg(a, 1))?
        .iter()
        .map(|v| match v {
            Value::Type(ty) => Ok((**ty).clone()),
            other => conv_err(other, "Type"),
        })
        .collect::<Res<_>>()?;
    Ok(Value::list(
        t.columns
            .iter()
            .filter(|c| wanted.iter().any(|w| type_compatible(&c.ty, w)))
            .map(|c| Value::Text(c.name.clone()))
            .collect(),
    ))
}

fn transform_column_names(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let f = func(arg(a, 1))?.clone();
    let mut cols = t.columns.clone();
    for c in &mut cols {
        interp.tick(1)?;
        c.name = match interp.call(&f, vec![Value::Text(c.name.clone())])? {
            Value::Text(n) => n,
            other => return conv_err(&other, "Text"),
        };
    }
    make(interp, cols, t.rows.clone())
}

// ---------------------------------------------------------------------------------------------
// Metadata and conversions
// ---------------------------------------------------------------------------------------------

const SCHEMA_COLUMNS: [&str; 19] = [
    "Name",
    "Position",
    "TypeName",
    "Kind",
    "IsNullable",
    "NumericPrecisionBase",
    "NumericPrecision",
    "NumericScale",
    "IsSigned",
    "DateTimePrecision",
    "IsUtc",
    "MaxLength",
    "IsVariableLength",
    "NativeTypeName",
    "NativeDefaultExpression",
    "NativeExpression",
    "Description",
    "IsWritable",
    "FieldCaption",
];

/// The library name of a column type: `Int64.Type` for a facet, else `Text.Type`, ….
fn type_name(ty: &MType) -> String {
    let facet = match ty.facet {
        Facet::None => None,
        Facet::Int64 => Some("Int64"),
        Facet::Int32 => Some("Int32"),
        Facet::Int16 => Some("Int16"),
        Facet::Int8 => Some("Int8"),
        Facet::Byte => Some("Byte"),
        Facet::Double => Some("Double"),
        Facet::Single => Some("Single"),
        Facet::Decimal => Some("Decimal"),
        Facet::Currency => Some("Currency"),
        Facet::Percentage => Some("Percentage"),
    };
    match facet {
        Some(f) => format!("{f}.Type"),
        None => format!("{}.Type", crate::m::eval::title(ty.name())),
    }
}

/// `Table.Schema`: one row per column (name, position, type name, kind, nullable); the
/// columns about native types and precision are null.
fn schema(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let rows: Vec<Vec<Value>> = t
        .columns
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let mut row = vec![
                Value::Text(c.name.clone()),
                Value::num(i as f64),
                Value::text(type_name(&c.ty)),
                Value::text(c.ty.name()),
                Value::Logical(c.ty.nullable),
            ];
            row.resize(SCHEMA_COLUMNS.len(), Value::Null);
            row
        })
        .collect();
    let fixed = [Kind::Text, Kind::Number, Kind::Text, Kind::Text, Kind::Logical];
    let cols = SCHEMA_COLUMNS
        .iter()
        .enumerate()
        .map(|(i, n)| col(*n, fixed.get(i).map_or_else(MType::any, |k| MType::prim(*k))))
        .collect();
    make(interp, cols, rows)
}

fn approximate_row_count(_: &mut Interp, a: &[Value]) -> Res<Value> {
    Ok(Value::num(tbl(arg(a, 0))?.rows.len() as f64))
}

/// `Table.ToList(table, combiner)`: each row's values given to the combiner as a list; the
/// default combiner joins texts with commas (null as empty text).
fn to_list(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let combiner = match arg(a, 1) {
        Value::Null => None,
        v => Some(func(v)?.clone()),
    };
    let mut out = Vec::with_capacity(t.rows.len());
    for row in &t.rows {
        interp.tick(1)?;
        let v = match &combiner {
            Some(f) => interp.call(f, vec![Value::list(row.clone())]),
            None => join_texts(interp, row),
        };
        out.push(Value::store(v)?);
    }
    Ok(Value::list(out))
}

fn join_texts(interp: &mut Interp, cells: &[Value]) -> Res<Value> {
    let mut s = String::new();
    for (i, c) in cells.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        match c.clone().check()? {
            Value::Null => {}
            Value::Text(t) => s.push_str(&t),
            other => return conv_err(&other, "Text"),
        }
        interp.check_text(s.len())?;
    }
    Ok(Value::text(s))
}
