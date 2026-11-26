//! `Table.*` and `#table`.

use std::collections::HashMap;
use std::rc::Rc;

use super::{
    Builtin, arg, b, conv_err, count, func, items, list, mtype, names, opt_number, record, tbl, text,
};
use crate::m::convert;
use crate::m::eval::{Interp, compare_values, row_record, values_equal};
use crate::m::value::{Column, Func, Kind, MType, Native, Record, Res, Table, Value, expr_err, unsupported};

pub(crate) static FUNCS: &[Builtin] = &[
    b("#table", 2, 2, hash_table),
    b("Table.FromRows", 1, 2, from_rows),
    b("Table.FromRecords", 1, 3, from_records),
    b("Table.FromColumns", 1, 2, from_columns),
    b("Table.FromList", 1, 5, from_list),
    b("Table.FromValue", 1, 2, from_value),
    b("Table.ToRows", 1, 1, to_rows),
    b("Table.ToRecords", 1, 1, to_records),
    b("Table.ToColumns", 1, 1, to_columns),
    b("Table.TransformRows", 2, 2, transform_rows),
    b("Table.Column", 2, 2, column),
    b("Table.ColumnNames", 1, 1, column_names),
    b("Table.ColumnCount", 1, 1, column_count),
    b("Table.RowCount", 1, 1, row_count),
    b("Table.IsEmpty", 1, 1, is_empty),
    b("Table.First", 1, 2, first),
    b("Table.Last", 1, 2, last),
    b("Table.FirstValue", 1, 2, first_value),
    b("Table.HasColumns", 2, 2, has_columns),
    b("Table.PromoteHeaders", 1, 2, promote_headers),
    b("Table.DemoteHeaders", 1, 1, demote_headers),
    b("Table.TransformColumnTypes", 2, 3, transform_column_types),
    b("Table.TransformColumns", 2, 4, transform_columns),
    b("Table.RenameColumns", 2, 3, rename_columns),
    b("Table.SelectColumns", 2, 3, select_columns_fn),
    b("Table.RemoveColumns", 2, 3, remove_columns),
    b("Table.ReorderColumns", 2, 3, reorder_columns),
    b("Table.DuplicateColumn", 3, 4, duplicate_column),
    b("Table.PrefixColumns", 2, 2, prefix_columns),
    b("Table.SelectRows", 2, 2, select_rows),
    b("Table.AddColumn", 3, 4, add_column),
    b("Table.AddIndexColumn", 2, 5, add_index_column),
    b("Table.Sort", 2, 2, sort),
    b("Table.Distinct", 1, 2, distinct),
    b("Table.FirstN", 2, 2, first_n),
    b("Table.LastN", 2, 2, last_n),
    b("Table.Skip", 1, 2, skip),
    b("Table.RemoveFirstN", 1, 2, skip),
    b("Table.RemoveLastN", 1, 2, remove_last_n),
    b("Table.Range", 2, 3, range),
    b("Table.ReverseRows", 1, 1, reverse_rows),
    b("Table.Repeat", 2, 2, repeat),
    b("Table.ReplaceValue", 5, 5, replace_value),
    b("Table.ReplaceErrorValues", 2, 2, replace_error_values),
    b("Table.RemoveRowsWithErrors", 1, 2, remove_rows_with_errors),
    b("Table.SelectRowsWithErrors", 1, 2, select_rows_with_errors),
    b("Table.Group", 3, 5, group),
    b("Table.NestedJoin", 5, 7, nested_join),
    b("Table.Join", 4, 7, join),
    b("Table.ExpandTableColumn", 3, 4, expand_table_column),
    b("Table.ExpandRecordColumn", 3, 4, expand_record_column),
    b("Table.ExpandListColumn", 2, 2, expand_list_column),
    b("Table.Combine", 1, 2, combine_fn),
    b("Table.FillDown", 2, 2, fill_down),
    b("Table.FillUp", 2, 2, fill_up),
    b("Table.Unpivot", 4, 4, unpivot),
    b("Table.UnpivotOtherColumns", 4, 4, unpivot_other),
    b("Table.Pivot", 4, 5, pivot),
    b("Table.SplitColumn", 3, 6, split_column),
    b("Table.CombineColumns", 4, 4, combine_columns),
    b("Table.Transpose", 1, 2, transpose),
    b("Table.InsertRows", 3, 3, insert_rows),
    b("Table.Buffer", 1, 2, identity),
    b("Table.StopFolding", 1, 1, identity),
    b("Table.AddKey", 3, 3, identity),
    b("Table.ReplaceKeys", 2, 2, identity),
    b("Table.Max", 2, 3, table_max),
    b("Table.Min", 2, 3, table_min),
    b("Table.MatchesAnyRows", 2, 2, matches_any_rows),
    b("Table.MatchesAllRows", 2, 2, matches_all_rows),
    b("Table.Contains", 2, 3, contains),
    b("Table.PositionOf", 2, 4, position_of),
];

// ---------------------------------------------------------------------------
// Helpers.

const MISSING_ERROR: f64 = 0.0;
const MISSING_IGNORE: f64 = 1.0;
const MISSING_NULL: f64 = 2.0;

pub(crate) fn missing_mode(v: &Value) -> Res<f64> {
    Ok(opt_number(v)?.unwrap_or(MISSING_ERROR))
}

fn missing_column<T>(name: &str) -> Res<T> {
    expr_err(format!("The column '{name}' of the table wasn't found."))
}

pub(crate) fn col(name: impl Into<Rc<str>>, ty: MType) -> Column {
    Column {
        name: name.into(),
        ty,
    }
}

fn any_cols(n: impl IntoIterator<Item = Rc<str>>) -> Vec<Column> {
    n.into_iter().map(|n| col(n, MType::any())).collect()
}

fn default_names(n: usize) -> Vec<Rc<str>> {
    (1..=n).map(|i| Rc::from(format!("Column{i}"))).collect()
}

pub(crate) fn index_of(t: &Table, name: &str) -> Res<usize> {
    match t.index(name) {
        Some(i) => Ok(i),
        None => missing_column(name),
    }
}

pub(crate) fn make(interp: &mut Interp, columns: Vec<Column>, rows: Vec<Vec<Value>>) -> Res<Value> {
    interp.check_rows(rows.len())?;
    for (i, c) in columns.iter().enumerate() {
        if columns.iter().take(i).any(|d| d.name == c.name) {
            return expr_err(format!("The column '{}' already exists in the table.", c.name));
        }
    }
    if columns.len() > interp.limits.max_columns {
        return crate::m::value::fatal(crate::Error::LimitExceeded(format!(
            "{} columns (max {})",
            columns.len(),
            interp.limits.max_columns
        )));
    }
    Ok(Value::table(Table { columns, rows }))
}

pub(crate) fn row_fn(interp: &mut Interp, t: &Table, row: &[Value], f: &Value) -> Res<Value> {
    let names = t.names();
    interp.call(
        f,
        vec![Value::Record(Rc::new(Record {
            names,
            values: row.to_vec(),
        }))],
    )
}

/// Calls `f` on every row with a shared names vector.
fn map_rows(
    interp: &mut Interp,
    t: &Table,
    f: &Value,
    mut each: impl FnMut(&mut Interp, usize, Res<Value>) -> Res<()>,
) -> Res<()> {
    let names = t.names();
    for (i, row) in t.rows.iter().enumerate() {
        interp.tick(1)?;
        let rec = Value::Record(Rc::new(Record {
            names: names.clone(),
            values: row.clone(),
        }));
        let r = interp.call(f, vec![rec]);
        each(interp, i, r)?;
    }
    Ok(())
}

pub(crate) fn truthy(v: Res<Value>) -> Res<bool> {
    match v? {
        Value::Logical(b) => Ok(b),
        Value::Null => Ok(false),
        other => conv_err(&other, "Logical"),
    }
}

/// Canonical hash key for scalar values (`None` for lists/records/tables).
pub(crate) fn hash_key(values: &[&Value], ignore_case: bool) -> Option<String> {
    let mut s = String::new();
    for v in values {
        match v {
            Value::Null => s.push('n'),
            Value::Logical(b) => s.push(if *b { 'T' } else { 'F' }),
            Value::Number(n) => {
                let n = if n.f() == 0.0 { 0.0 } else { n.f() };
                s.push('#');
                s.push_str(&n.to_bits().to_string());
            }
            Value::Text(t) => {
                s.push('"');
                if ignore_case {
                    s.push_str(&t.to_uppercase());
                } else {
                    s.push_str(t);
                }
            }
            Value::Date(d) => s.push_str(&format!("D{d}")),
            Value::DateTime(d) => s.push_str(&format!("T{d}")),
            Value::Time(d) => s.push_str(&format!("t{d}")),
            Value::Duration(d) => s.push_str(&format!("d{d}")),
            Value::DateTimeZone(d, o) => {
                s.push_str(&format!("z{}", d.saturating_sub(i64::from(*o) * 60_000)))
            }
            _ => return None,
        }
        s.push('\u{1}');
    }
    Some(s)
}

fn keys_equal(a: &[&Value], b: &[&Value], ignore_case: bool) -> bool {
    a.len() == b.len()
        && a.iter().zip(b.iter()).all(|(x, y)| match (x, y) {
            (Value::Text(p), Value::Text(q)) if ignore_case => p.to_uppercase() == q.to_uppercase(),
            _ => values_equal(x, y),
        })
}

/// Groups row indices by key, in order of first appearance.
pub(crate) fn group_rows(
    interp: &mut Interp,
    keys: &[Vec<&Value>],
    ignore_case: bool,
) -> Res<Vec<Vec<usize>>> {
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut complex: Vec<usize> = Vec::new();
    for (i, k) in keys.iter().enumerate() {
        interp.tick(1)?;
        match hash_key(k, ignore_case) {
            Some(h) => {
                if let Some(&g) = index.get(&h) {
                    if let Some(grp) = groups.get_mut(g) {
                        grp.push(i);
                    }
                } else {
                    index.insert(h, groups.len());
                    groups.push(vec![i]);
                }
            }
            None => {
                let mut hit = None;
                for &g in &complex {
                    let first = groups
                        .get(g)
                        .and_then(|grp| grp.first())
                        .and_then(|&r| keys.get(r));
                    interp.tick(1)?;
                    if first.is_some_and(|f| keys_equal(f, k, ignore_case)) {
                        hit = Some(g);
                        break;
                    }
                }
                match hit.and_then(|g| groups.get_mut(g)) {
                    Some(grp) => grp.push(i),
                    None => {
                        complex.push(groups.len());
                        groups.push(vec![i]);
                    }
                }
            }
        }
    }
    Ok(groups)
}

pub(crate) fn sub_table(t: &Table, rows: &[usize]) -> Table {
    Table {
        columns: t.columns.clone(),
        rows: rows.iter().filter_map(|&i| t.rows.get(i).cloned()).collect(),
    }
}

/// A column count, checked against `max_columns` before anything is allocated.
pub(crate) fn column_count_arg(interp: &Interp, v: &Value) -> Res<usize> {
    let n = count(v)?;
    if n > interp.limits.max_columns {
        return crate::m::value::fatal(crate::Error::LimitExceeded(format!(
            "{n} columns (max {})",
            interp.limits.max_columns
        )));
    }
    Ok(n)
}

/// Column types from a `type table` value, a list of names or a count.
fn header(interp: &Interp, v: &Value, width: usize) -> Res<Vec<Column>> {
    match v {
        Value::Null => Ok(any_cols(default_names(width))),
        Value::Number(_) => Ok(any_cols(default_names(column_count_arg(interp, v)?))),
        Value::List(_) => Ok(any_cols(names(v)?)),
        Value::Type(t) => match (&t.kind, &t.fields) {
            (Kind::Table | Kind::Record, Some(f)) => {
                Ok(f.iter().map(|(n, t)| col(n.clone(), t.clone())).collect())
            }
            _ => expr_err("A table type was expected."),
        },
        other => conv_err(other, "Type"),
    }
}

// ---------------------------------------------------------------------------
// Construction.

fn rows_from_lists(
    interp: &mut Interp,
    rows: &[Value],
    width: Option<usize>,
) -> Res<(usize, Vec<Vec<Value>>)> {
    let mut out = Vec::with_capacity(rows.len());
    let mut w = width;
    for r in rows {
        interp.tick(1)?;
        // Cell errors stay in their cells; only the row list itself must be readable.
        let row = list(&r.clone().check()?)?.to_vec();
        let n = *w.get_or_insert(row.len());
        if row.len() != n {
            return expr_err(format!(
                "{} keys were specified, but {} values were provided.",
                n,
                row.len()
            ));
        }
        out.push(row);
    }
    Ok((w.unwrap_or(0), out))
}

fn hash_table(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let rows = list(arg(a, 1))?.clone();
    let cols = match arg(a, 0) {
        Value::Number(_) => any_cols(default_names(column_count_arg(interp, arg(a, 0))?)),
        other => header(interp, other, 0)?,
    };
    let (_, rows) = rows_from_lists(interp, &rows, Some(cols.len()))?;
    make(interp, cols, rows)
}

fn from_rows(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let rows = list(arg(a, 0))?.clone();
    let hdr = arg(a, 1);
    let declared = match hdr {
        Value::Null => None,
        v => Some(header(interp, v, 0)?),
    };
    let (w, rows) = rows_from_lists(interp, &rows, declared.as_ref().map(Vec::len))?;
    let cols = declared.unwrap_or_else(|| any_cols(default_names(w)));
    make(interp, cols, rows)
}

fn from_records(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let recs = items(arg(a, 0))?;
    let missing = missing_mode(arg(a, 2))?;
    let cols: Vec<Column> = match arg(a, 1) {
        Value::Null => match recs.first() {
            Some(Value::Record(r)) => any_cols(r.names.iter().cloned()),
            Some(other) => return conv_err(other, "Record"),
            None => Vec::new(),
        },
        v => header(interp, v, 0)?,
    };
    let mut rows = Vec::with_capacity(recs.len());
    for r in &recs {
        interp.tick(1)?;
        let r = record(r)?;
        let mut row = Vec::with_capacity(cols.len());
        for c in &cols {
            match r.get(&c.name) {
                Some(v) => row.push(v.clone()),
                None if missing == MISSING_NULL => row.push(Value::Null),
                None => return expr_err(format!("The field '{}' of the record wasn't found.", c.name)),
            }
        }
        rows.push(row);
    }
    make(interp, cols, rows)
}

fn from_columns(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let lists = items(arg(a, 0))?;
    let lists: Vec<Rc<Vec<Value>>> = lists.iter().map(|l| list(l).cloned()).collect::<Res<_>>()?;
    let height = lists.iter().map(|l| l.len()).max().unwrap_or(0);
    let cols = header(interp, arg(a, 1), lists.len())?;
    let mut rows = Vec::with_capacity(height);
    for i in 0..height {
        interp.tick(1)?;
        rows.push(
            (0..cols.len())
                .map(|c| {
                    lists
                        .get(c)
                        .and_then(|l| l.get(i))
                        .cloned()
                        .unwrap_or(Value::Null)
                })
                .collect(),
        );
    }
    make(interp, cols, rows)
}

fn from_list(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let src = items(arg(a, 0))?;
    let splitter = arg(a, 1).clone();
    let default = arg(a, 3).clone();
    let extra = opt_number(arg(a, 4))?.unwrap_or(1.0);
    let mut split_rows: Vec<Vec<Value>> = Vec::with_capacity(src.len());
    for v in &src {
        interp.tick(1)?;
        let parts = match &splitter {
            Value::Null => match v {
                Value::Text(t) => t.split(',').map(Value::text).collect(),
                other => return conv_err(other, "Text"),
            },
            f => match interp.call(f, vec![v.clone()])? {
                Value::List(l) => l.to_vec(),
                other => vec![other],
            },
        };
        split_rows.push(parts);
    }
    let cols = match arg(a, 2) {
        Value::Null => any_cols(default_names(
            split_rows.iter().map(Vec::len).max().unwrap_or(1).max(1),
        )),
        v => header(interp, v, 0)?,
    };
    let w = cols.len();
    let mut rows = Vec::with_capacity(split_rows.len());
    for mut r in split_rows {
        if r.len() > w {
            match extra as i64 {
                2 => r.truncate(w),
                0 => {
                    let rest: Vec<Value> = r.split_off(w.saturating_sub(1));
                    r.push(Value::list(rest));
                }
                _ => return expr_err("There were more columns in the result than expected."),
            }
        }
        r.resize(w, default.clone());
        rows.push(r);
    }
    make(interp, cols, rows)
}

fn from_value(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let v = arg(a, 0).clone();
    match v {
        Value::Record(r) => {
            let rows = r
                .names
                .iter()
                .zip(r.values.iter())
                .map(|(n, v)| vec![Value::Text(n.clone()), v.clone()])
                .collect();
            make(interp, any_cols(["Name".into(), "Value".into()]), rows)
        }
        Value::List(l) => make(
            interp,
            any_cols(["Value".into()]),
            l.iter().map(|v| vec![v.clone()]).collect(),
        ),
        other => make(interp, any_cols(["Value".into()]), vec![vec![other]]),
    }
}

fn to_rows(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?;
    Ok(Value::list(
        t.rows.iter().map(|r| Value::list(r.clone())).collect(),
    ))
}

fn to_records(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?;
    let names = t.names();
    Ok(Value::list(
        t.rows
            .iter()
            .map(|r| {
                Value::Record(Rc::new(Record {
                    names: names.clone(),
                    values: r.clone(),
                }))
            })
            .collect(),
    ))
}

fn to_columns(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?;
    Ok(Value::list(
        (0..t.columns.len())
            .map(|c| {
                Value::list(
                    t.rows
                        .iter()
                        .map(|r| r.get(c).cloned().unwrap_or(Value::Null))
                        .collect(),
                )
            })
            .collect(),
    ))
}

fn transform_rows(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let f = func(arg(a, 1))?.clone();
    let mut out = Vec::with_capacity(t.rows.len());
    map_rows(interp, &t, &f, |_, _, r| {
        out.push(Value::store(r)?);
        Ok(())
    })?;
    Ok(Value::list(out))
}

fn column(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let name = text(arg(a, 1))?;
    interp.field(&Value::Table(t), name, false)
}

fn column_names(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?;
    Ok(Value::list(
        t.columns.iter().map(|c| Value::Text(c.name.clone())).collect(),
    ))
}

fn column_count(_: &mut Interp, a: &[Value]) -> Res<Value> {
    Ok(Value::num(tbl(arg(a, 0))?.columns.len() as f64))
}

fn row_count(_: &mut Interp, a: &[Value]) -> Res<Value> {
    Ok(Value::num(tbl(arg(a, 0))?.rows.len() as f64))
}

fn is_empty(_: &mut Interp, a: &[Value]) -> Res<Value> {
    Ok(Value::Logical(tbl(arg(a, 0))?.rows.is_empty()))
}

fn first(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?;
    Ok(match t.rows.first() {
        Some(r) => row_record(t, r),
        None => arg(a, 1).clone(),
    })
}

fn last(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?;
    Ok(match t.rows.last() {
        Some(r) => row_record(t, r),
        None => arg(a, 1).clone(),
    })
}

fn first_value(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?;
    match t.rows.first().and_then(|r| r.first()) {
        Some(v) => v.clone().check(),
        None => Ok(arg(a, 1).clone()),
    }
}

fn has_columns(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?;
    let n = names(arg(a, 1))?;
    Ok(Value::Logical(n.iter().all(|n| t.index(n).is_some())))
}

fn identity(_: &mut Interp, a: &[Value]) -> Res<Value> {
    Ok(arg(a, 0).clone())
}

// ---------------------------------------------------------------------------
// Headers and column shape.

pub(crate) fn unique_name(existing: &[Column], base: &str) -> Rc<str> {
    if !existing.iter().any(|c| &*c.name == base) {
        return base.into();
    }
    let mut i = 1usize;
    loop {
        let candidate = format!("{base}_{i}");
        if !existing.iter().any(|c| *c.name == *candidate) {
            return candidate.into();
        }
        i += 1;
    }
}

fn promote_headers(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let all_scalars = match arg(a, 1) {
        Value::Record(r) => matches!(r.get("PromoteAllScalars"), Some(Value::Logical(true))),
        _ => false,
    };
    let culture = super::culture_arg(interp, arg(a, 1))?;
    promote(interp, &t, all_scalars, &culture)
}

/// `Table.PromoteHeaders(t, [PromoteAllScalars = true])`.
pub(crate) fn promote_all(interp: &mut Interp, v: &Value) -> Res<Value> {
    let t = tbl(v)?.clone();
    let culture = interp.culture.clone();
    promote(interp, &t, true, &culture)
}

fn promote(
    interp: &mut Interp,
    t: &Rc<Table>,
    all_scalars: bool,
    culture: &convert::CultureInfo,
) -> Res<Value> {
    let Some(first) = t.rows.first() else {
        return Ok(Value::Table(t.clone()));
    };
    let mut cols: Vec<Column> = Vec::with_capacity(t.columns.len());
    for (i, c) in t.columns.iter().enumerate() {
        let v = first.get(i).cloned().unwrap_or(Value::Null).check()?;
        let name: Option<String> = match &v {
            Value::Text(s) => Some(s.to_string()),
            Value::Null => None,
            Value::Number(_) => convert::to_text(&v, culture),
            other if all_scalars => convert::to_text(other, culture),
            _ => None,
        };
        let name = match name {
            Some(n) if !n.is_empty() => n,
            Some(_) | None => c.name.to_string(),
        };
        let name = unique_name(&cols, &name);
        cols.push(col(name, MType::any()));
    }
    let rows = t.rows.iter().skip(1).cloned().collect();
    make(interp, cols, rows)
}

fn demote_headers(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?;
    let mut rows = Vec::with_capacity(t.rows.len() + 1);
    rows.push(t.columns.iter().map(|c| Value::Text(c.name.clone())).collect());
    rows.extend(t.rows.iter().cloned());
    let cols = any_cols(default_names(t.columns.len()));
    make(interp, cols, rows)
}

/// `{"A", fn}` / `{{"A", fn}, {"B", fn}}` → pairs.
pub(crate) fn pairs(v: &Value) -> Res<Vec<Vec<Value>>> {
    let l = items(v)?;
    if l.first().is_some_and(|x| !matches!(x, Value::List(_))) {
        return Ok(vec![l]);
    }
    l.iter().map(items).collect()
}

fn transform_column_types(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let culture = super::culture_arg(interp, arg(a, 2))?;
    let mut cols = t.columns.clone();
    let mut ops: Vec<(usize, MType)> = Vec::new();
    for p in pairs(arg(a, 1))? {
        let name = text(p.first().unwrap_or(&Value::Null))?;
        let ty = mtype(p.get(1).unwrap_or(&Value::Null))?;
        let i = index_of(&t, name)?;
        if let Some(c) = cols.get_mut(i) {
            c.ty = ty.clone();
        }
        ops.push((i, ty));
    }
    let mut rows = t.rows.clone();
    for row in &mut rows {
        interp.tick(ops.len() as u64)?;
        for (i, ty) in &ops {
            if let Some(cell) = row.get_mut(*i) {
                if matches!(cell, Value::Error(_)) {
                    continue;
                }
                let r = convert::convert_to(cell, ty, &culture);
                *cell = Value::store(r)?;
            }
        }
    }
    make(interp, cols, rows)
}

fn transform_columns(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let missing = missing_mode(arg(a, 3))?;
    let mut cols = t.columns.clone();
    let mut ops: Vec<(usize, Value)> = Vec::new();
    for p in pairs(arg(a, 1))? {
        let name = text(p.first().unwrap_or(&Value::Null))?;
        let f = func(p.get(1).unwrap_or(&Value::Null))?.clone();
        let Some(i) = t.index(name) else {
            if missing == MISSING_IGNORE || missing == MISSING_NULL {
                continue;
            }
            return missing_column(name);
        };
        if let (Some(c), Some(ty)) = (cols.get_mut(i), p.get(2)) {
            c.ty = mtype(ty)?;
        } else if let Some(c) = cols.get_mut(i) {
            c.ty = MType::any();
        }
        ops.push((i, f));
    }
    let default = match arg(a, 2) {
        Value::Null => None,
        f => Some(func(f)?.clone()),
    };
    if default.is_some() {
        for (i, c) in cols.iter_mut().enumerate() {
            if !ops.iter().any(|(j, _)| *j == i) {
                c.ty = MType::any();
            }
        }
    }
    let mut rows = t.rows.clone();
    for row in &mut rows {
        for i in 0..row.len() {
            let f = match ops.iter().find(|(j, _)| *j == i) {
                Some((_, f)) => f,
                None => match &default {
                    Some(f) => f,
                    None => continue,
                },
            };
            interp.tick(1)?;
            if let Some(cell) = row.get_mut(i) {
                if matches!(cell, Value::Error(_)) {
                    continue;
                }
                let r = interp.call(f, vec![cell.clone()]);
                *cell = Value::store(r)?;
            }
        }
    }
    make(interp, cols, rows)
}

fn rename_columns(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let missing = missing_mode(arg(a, 2))?;
    let mut cols = t.columns.clone();
    for p in pairs(arg(a, 1))? {
        let old = text(p.first().unwrap_or(&Value::Null))?;
        let new = text(p.get(1).unwrap_or(&Value::Null))?;
        match cols.iter_mut().find(|c| &*c.name == old) {
            Some(c) => c.name = new.into(),
            None if missing == MISSING_ERROR => return missing_column(old),
            None => {}
        }
    }
    make(interp, cols, t.rows.clone())
}

pub(crate) fn select_columns(
    interp: &mut Interp,
    t: &Rc<Table>,
    which: &Value,
    missing: &Value,
) -> Res<Value> {
    let missing = missing_mode(missing)?;
    let wanted = names(which)?;
    let mut src: Vec<Option<usize>> = Vec::with_capacity(wanted.len());
    let mut cols = Vec::with_capacity(wanted.len());
    for n in &wanted {
        match t.index(n) {
            Some(i) => {
                src.push(Some(i));
                cols.push(
                    t.columns
                        .get(i)
                        .cloned()
                        .unwrap_or_else(|| col(n.clone(), MType::any())),
                );
            }
            None if missing == MISSING_NULL => {
                src.push(None);
                cols.push(col(n.clone(), MType::any()));
            }
            None if missing == MISSING_IGNORE => {}
            None => return missing_column(n),
        }
    }
    interp.tick(t.rows.len() as u64)?;
    let rows = t
        .rows
        .iter()
        .map(|r| {
            src.iter()
                .map(|s| s.and_then(|i| r.get(i)).cloned().unwrap_or(Value::Null))
                .collect()
        })
        .collect();
    make(interp, cols, rows)
}

fn select_columns_fn(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    select_columns(interp, &t, arg(a, 1), arg(a, 2))
}

fn remove_columns(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let missing = missing_mode(arg(a, 2))?;
    let drop = names(arg(a, 1))?;
    for n in &drop {
        if t.index(n).is_none() && missing == MISSING_ERROR {
            return missing_column(n);
        }
    }
    let keep: Vec<Value> = t
        .columns
        .iter()
        .filter(|c| !drop.contains(&c.name))
        .map(|c| Value::Text(c.name.clone()))
        .collect();
    select_columns(interp, &t, &Value::list(keep), &Value::Null)
}

fn reorder_columns(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let missing = missing_mode(arg(a, 2))?;
    let order: Vec<Rc<str>> = names(arg(a, 1))?
        .into_iter()
        .filter(|n| t.index(n).is_some() || missing == MISSING_ERROR)
        .collect();
    for n in &order {
        index_of(&t, n)?;
    }
    // Columns named in `order` take the slots those columns occupy, in the new order.
    let mut slots: Vec<Rc<str>> = t.columns.iter().map(|c| c.name.clone()).collect();
    let positions: Vec<usize> = slots
        .iter()
        .enumerate()
        .filter(|(_, n)| order.contains(n))
        .map(|(i, _)| i)
        .collect();
    for (p, n) in positions.iter().zip(order.iter()) {
        if let Some(s) = slots.get_mut(*p) {
            *s = n.clone();
        }
    }
    let list = Value::list(slots.into_iter().map(Value::Text).collect());
    select_columns(interp, &t, &list, &Value::Null)
}

fn duplicate_column(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let i = index_of(&t, text(arg(a, 1))?)?;
    let name = text(arg(a, 2))?;
    let ty = match arg(a, 3) {
        Value::Null => t.columns.get(i).map_or_else(MType::any, |c| c.ty.clone()),
        v => mtype(v)?,
    };
    let mut cols = t.columns.clone();
    cols.push(col(name, ty));
    let rows = t
        .rows
        .iter()
        .map(|r| {
            let mut r = r.clone();
            r.push(r.get(i).cloned().unwrap_or(Value::Null));
            r
        })
        .collect();
    make(interp, cols, rows)
}

fn prefix_columns(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let p = text(arg(a, 1))?;
    let cols = t
        .columns
        .iter()
        .map(|c| col(format!("{p}.{}", c.name), c.ty.clone()))
        .collect();
    make(interp, cols, t.rows.clone())
}

// ---------------------------------------------------------------------------
// Rows.

fn select_rows(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let f = func(arg(a, 1))?.clone();
    let mut keep = Vec::new();
    map_rows(interp, &t, &f, |_, i, r| {
        if truthy(r)? {
            keep.push(i);
        }
        Ok(())
    })?;
    Ok(Value::table(sub_table(&t, &keep)))
}

fn add_column(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let name = text(arg(a, 1))?;
    let f = func(arg(a, 2))?.clone();
    let ty = mtype(arg(a, 3))?;
    let mut rows = t.rows.clone();
    map_rows(interp, &t, &f, |_, i, r| {
        let v = Value::store(r)?;
        if let Some(row) = rows.get_mut(i) {
            row.push(v);
        }
        Ok(())
    })?;
    let mut cols = t.columns.clone();
    cols.push(col(name, ty));
    make(interp, cols, rows)
}

fn add_index_column(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let name = text(arg(a, 1))?;
    let start_num = super::opt_num(arg(a, 2))?;
    let start = start_num.map_or(0.0, |n| n.f());
    let step = opt_number(arg(a, 3))?.unwrap_or(1.0);
    let ty = match arg(a, 4) {
        Value::Null => MType::facet(Kind::Number, crate::m::value::Facet::Int64),
        v => mtype(v)?,
    };
    let mut cols = t.columns.clone();
    cols.push(col(name, ty));
    let rows = t
        .rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let mut r = r.clone();
            r.push(match start_num {
                Some(n) if i == 0 => Value::Number(n),
                _ => Value::num(start + step * i as f64),
            });
            r
        })
        .collect();
    make(interp, cols, rows)
}

/// One sort criterion: a column index or key function, and descending.
pub(crate) enum Key {
    Col(usize),
    Fn(Value),
}

pub(crate) fn criteria(t: &Table, v: &Value) -> Res<Vec<(Key, bool)>> {
    let one = |x: &Value| -> Res<(Key, bool)> {
        match x {
            Value::Text(n) => Ok((Key::Col(index_of(t, n)?), false)),
            Value::Function(_) => {
                if Interp::arity(x) == 2 {
                    return unsupported("a two-argument comparer function in `Table.Sort`");
                }
                Ok((Key::Fn(x.clone()), false))
            }
            Value::List(l) => {
                let k = match l.first() {
                    Some(Value::Text(n)) => Key::Col(index_of(t, n)?),
                    Some(f @ Value::Function(_)) => Key::Fn(f.clone()),
                    _ => return expr_err("A sort criterion must name a column."),
                };
                let desc = match l.get(1) {
                    Some(Value::Number(n)) => n.f() == 1.0,
                    None | Some(Value::Null) => false,
                    Some(other) => return conv_err(other, "Number"),
                };
                Ok((k, desc))
            }
            other => conv_err(other, "Text"),
        }
    };
    match v {
        Value::List(l) if l.first().is_some_and(|x| matches!(x, Value::List(_))) => {
            l.iter().map(one).collect()
        }
        Value::List(l) if l.len() == 2 && matches!(l.get(1), Some(Value::Number(_))) => Ok(vec![one(v)?]),
        Value::List(l) => l.iter().map(one).collect(),
        other => Ok(vec![one(other)?]),
    }
}

fn sort(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let (order, _) = sorted_rows(interp, &t, arg(a, 1))?;
    Ok(Value::table(sub_table(&t, &order)))
}

/// The rows of `t` in the order of the comparison criteria `v` (stable), and the comparison
/// of two rows by them (`Equal` for ties).
pub(crate) fn sorted_rows(
    interp: &mut Interp,
    t: &Table,
    v: &Value,
) -> Res<(Vec<usize>, impl Fn(usize, usize) -> std::cmp::Ordering + use<>)> {
    let crit = criteria(t, v)?;
    // Precompute the keys of every row.
    let mut keys: Vec<Vec<Value>> = Vec::with_capacity(t.rows.len());
    let names = t.names();
    for r in &t.rows {
        interp.tick(crit.len() as u64)?;
        let mut k = Vec::with_capacity(crit.len());
        for (c, _) in &crit {
            k.push(match c {
                Key::Col(i) => r.get(*i).cloned().unwrap_or(Value::Null).check()?,
                Key::Fn(f) => interp.call(
                    f,
                    vec![Value::Record(Rc::new(Record {
                        names: names.clone(),
                        values: r.clone(),
                    }))],
                )?,
            });
        }
        keys.push(k);
    }
    let n = t.rows.len();
    interp.tick((n as u64).saturating_mul(64 - (n as u64).leading_zeros() as u64))?;
    let cmp = move |x: usize, y: usize| {
        let (Some(kx), Some(ky)) = (keys.get(x), keys.get(y)) else {
            return std::cmp::Ordering::Equal;
        };
        for (((_, desc), a), b) in crit.iter().zip(kx.iter()).zip(ky.iter()) {
            let o = compare_values(a, b, false);
            let o = if *desc { o.reverse() } else { o };
            if o != std::cmp::Ordering::Equal {
                return o;
            }
        }
        std::cmp::Ordering::Equal
    };
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&x, &y| cmp(x, y));
    Ok((order, cmp))
}

/// Equation criteria: column names and whether text compares ignoring case.
pub(crate) fn equation(t: &Table, v: &Value) -> Res<(Vec<usize>, bool)> {
    let all = || (0..t.columns.len()).collect::<Vec<_>>();
    let is_ci = |x: &Value| match x {
        Value::Function(f) => matches!(&**f, Func::Native(Native::Comparer { ignore_case: true })),
        _ => false,
    };
    Ok(match v {
        Value::Null => (all(), false),
        Value::Function(_) => (all(), is_ci(v)),
        Value::Text(n) => (vec![index_of(t, n)?], false),
        Value::List(l) => {
            if l.len() == 2
                && let (Some(Value::Text(n)), Some(f @ Value::Function(_))) = (l.first(), l.get(1))
            {
                return Ok((vec![index_of(t, n)?], is_ci(f)));
            }
            let mut cols = Vec::new();
            let mut ci = false;
            for x in l.iter() {
                match x {
                    Value::Text(n) => cols.push(index_of(t, n)?),
                    Value::Function(_) => ci = is_ci(x),
                    Value::List(p) => {
                        if let Some(Value::Text(n)) = p.first() {
                            cols.push(index_of(t, n)?);
                        }
                        if let Some(f) = p.get(1) {
                            ci = is_ci(f);
                        }
                    }
                    other => return conv_err(other, "Text"),
                }
            }
            (cols, ci)
        }
        other => return conv_err(other, "List"),
    })
}

fn distinct(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let (cols, ci) = equation(&t, arg(a, 1))?;
    let keys: Vec<Vec<&Value>> = t
        .rows
        .iter()
        .map(|r| cols.iter().map(|&c| r.get(c).unwrap_or(&Value::Null)).collect())
        .collect();
    let groups = group_rows(interp, &keys, ci)?;
    let firsts: Vec<usize> = groups.iter().filter_map(|g| g.first().copied()).collect();
    Ok(Value::table(sub_table(&t, &firsts)))
}

/// `count` or `each condition` (rows while the condition holds).
pub(crate) fn count_or_while(interp: &mut Interp, t: &Table, v: &Value, rev: bool) -> Res<usize> {
    match v {
        Value::Function(_) => {
            let mut n = 0usize;
            let rows: Vec<&Vec<Value>> = if rev {
                t.rows.iter().rev().collect()
            } else {
                t.rows.iter().collect()
            };
            for r in rows {
                interp.tick(1)?;
                if !truthy(row_fn(interp, t, r, v))? {
                    break;
                }
                n += 1;
            }
            Ok(n)
        }
        Value::Null => Ok(1),
        other => count(other),
    }
}

fn first_n(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let n = count_or_while(interp, &t, arg(a, 1), false)?.min(t.rows.len());
    let idx: Vec<usize> = (0..n).collect();
    Ok(Value::table(sub_table(&t, &idx)))
}

fn last_n(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let n = count_or_while(interp, &t, arg(a, 1), true)?.min(t.rows.len());
    let idx: Vec<usize> = (t.rows.len() - n..t.rows.len()).collect();
    Ok(Value::table(sub_table(&t, &idx)))
}

fn skip(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let n = count_or_while(interp, &t, arg(a, 1), false)?.min(t.rows.len());
    let idx: Vec<usize> = (n..t.rows.len()).collect();
    Ok(Value::table(sub_table(&t, &idx)))
}

fn remove_last_n(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let n = count_or_while(interp, &t, arg(a, 1), true)?.min(t.rows.len());
    let idx: Vec<usize> = (0..t.rows.len() - n).collect();
    Ok(Value::table(sub_table(&t, &idx)))
}

fn range(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let off = count(arg(a, 1))?.min(t.rows.len());
    let n = match arg(a, 2) {
        Value::Null => t.rows.len() - off,
        v => count(v)?.min(t.rows.len() - off),
    };
    let idx: Vec<usize> = (off..off + n).collect();
    Ok(Value::table(sub_table(&t, &idx)))
}

fn reverse_rows(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let idx: Vec<usize> = (0..t.rows.len()).rev().collect();
    Ok(Value::table(sub_table(&t, &idx)))
}

fn repeat(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let n = count(arg(a, 1))?;
    interp.check_rows(t.rows.len().saturating_mul(n))?;
    let mut rows = Vec::new();
    for _ in 0..n {
        interp.tick(t.rows.len() as u64)?;
        rows.extend(t.rows.iter().cloned());
    }
    make(interp, t.columns.clone(), rows)
}

fn insert_rows(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let off = count(arg(a, 1))?;
    if off > t.rows.len() {
        return expr_err("The offset is beyond the end of the table.");
    }
    let mut new_rows = Vec::new();
    for r in items(arg(a, 2))? {
        let r = record(&r)?;
        new_rows.push(
            t.columns
                .iter()
                .map(|c| r.get(&c.name).cloned().unwrap_or(Value::Null))
                .collect::<Vec<_>>(),
        );
    }
    let mut rows = t.rows.clone();
    let tail = rows.split_off(off);
    rows.extend(new_rows);
    rows.extend(tail);
    make(interp, t.columns.clone(), rows)
}

// ---------------------------------------------------------------------------
// Values.

fn replace_value(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let old = arg(a, 1).clone();
    let new = arg(a, 2).clone();
    let replacer = func(arg(a, 3))?.clone();
    let cols: Vec<usize> = names(arg(a, 4))?
        .iter()
        .map(|n| index_of(&t, n))
        .collect::<Res<_>>()?;
    let row_dependent = matches!(old, Value::Function(_)) || matches!(new, Value::Function(_));
    let mut rows = t.rows.clone();
    for (ri, row) in rows.iter_mut().enumerate() {
        let (o, n) = if row_dependent {
            let src = t.rows.get(ri).map(Vec::as_slice).unwrap_or(&[]);
            let o = match &old {
                f @ Value::Function(_) => row_fn(interp, &t, src, f)?,
                v => v.clone(),
            };
            let n = match &new {
                f @ Value::Function(_) => row_fn(interp, &t, src, f)?,
                v => v.clone(),
            };
            (o, n)
        } else {
            (old.clone(), new.clone())
        };
        for &c in &cols {
            interp.tick(1)?;
            if let Some(cell) = row.get_mut(c) {
                if matches!(cell, Value::Error(_)) {
                    continue;
                }
                let r = interp.call(&replacer, vec![cell.clone(), o.clone(), n.clone()]);
                *cell = Value::store(r)?;
            }
        }
    }
    make(interp, t.columns.clone(), rows)
}

fn replace_error_values(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let mut ops = Vec::new();
    for p in pairs(arg(a, 1))? {
        let i = index_of(&t, text(p.first().unwrap_or(&Value::Null))?)?;
        ops.push((i, p.get(1).cloned().unwrap_or(Value::Null)));
    }
    let mut rows = t.rows.clone();
    for row in &mut rows {
        for (i, v) in &ops {
            if let Some(cell) = row.get_mut(*i)
                && matches!(cell, Value::Error(_))
            {
                *cell = v.clone();
            }
        }
    }
    make(interp, t.columns.clone(), rows)
}

fn error_rows(t: &Table, cols: &Value, with_errors: bool) -> Res<Vec<usize>> {
    let idx: Vec<usize> = match cols {
        Value::Null => (0..t.columns.len()).collect(),
        v => names(v)?.iter().map(|n| index_of(t, n)).collect::<Res<_>>()?,
    };
    Ok(t.rows
        .iter()
        .enumerate()
        .filter(|(_, r)| idx.iter().any(|&c| matches!(r.get(c), Some(Value::Error(_)))) == with_errors)
        .map(|(i, _)| i)
        .collect())
}

fn remove_rows_with_errors(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let keep = error_rows(&t, arg(a, 1), false)?;
    Ok(Value::table(sub_table(&t, &keep)))
}

fn select_rows_with_errors(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let keep = error_rows(&t, arg(a, 1), true)?;
    Ok(Value::table(sub_table(&t, &keep)))
}

fn fill(interp: &mut Interp, a: &[Value], up: bool) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let cols: Vec<usize> = names(arg(a, 1))?
        .iter()
        .map(|n| index_of(&t, n))
        .collect::<Res<_>>()?;
    let mut rows = t.rows.clone();
    for &c in &cols {
        let mut last: Option<Value> = None;
        let order: Vec<usize> = if up {
            (0..rows.len()).rev().collect()
        } else {
            (0..rows.len()).collect()
        };
        for r in order {
            if let Some(cell) = rows.get_mut(r).and_then(|row| row.get_mut(c)) {
                if cell.is_null() {
                    if let Some(v) = &last {
                        *cell = v.clone();
                    }
                } else {
                    last = Some(cell.clone());
                }
            }
        }
        interp.tick(rows.len() as u64)?;
    }
    make(interp, t.columns.clone(), rows)
}

fn fill_down(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    fill(interp, a, false)
}

fn fill_up(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    fill(interp, a, true)
}

fn column_extreme(interp: &mut Interp, a: &[Value], max: bool) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let i = index_of(&t, text(arg(a, 1))?)?;
    let mut best: Option<usize> = None;
    for (r, row) in t.rows.iter().enumerate() {
        interp.tick(1)?;
        let v = row.get(i).cloned().unwrap_or(Value::Null).check()?;
        let better = match best.and_then(|b| t.rows.get(b)).and_then(|row| row.get(i)) {
            None => true,
            Some(cur) => {
                let o = compare_values(&v, cur, false);
                if max { o.is_gt() } else { o.is_lt() }
            }
        };
        if better {
            best = Some(r);
        }
    }
    Ok(match best.and_then(|b| t.rows.get(b)) {
        Some(r) => row_record(&t, r),
        None => arg(a, 2).clone(),
    })
}

fn table_max(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    column_extreme(interp, a, true)
}

fn table_min(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    column_extreme(interp, a, false)
}

fn matches_rows(interp: &mut Interp, a: &[Value], all: bool) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let f = func(arg(a, 1))?.clone();
    for r in &t.rows {
        interp.tick(1)?;
        let hit = truthy(row_fn(interp, &t, r, &f))?;
        if hit != all {
            return Ok(Value::Logical(!all));
        }
    }
    Ok(Value::Logical(all))
}

fn matches_any_rows(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    matches_rows(interp, a, false)
}

fn matches_all_rows(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    matches_rows(interp, a, true)
}

pub(crate) fn row_matches(t: &Table, row: &[Value], r: &Record) -> bool {
    r.names.iter().zip(r.values.iter()).all(|(n, v)| {
        t.index(n)
            .and_then(|i| row.get(i))
            .is_some_and(|c| values_equal(c, v))
    })
}

fn contains(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let r = record(arg(a, 1))?.clone();
    interp.tick(t.rows.len() as u64)?;
    Ok(Value::Logical(t.rows.iter().any(|row| row_matches(&t, row, &r))))
}

fn position_of(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let r = record(arg(a, 1))?.clone();
    interp.tick(t.rows.len() as u64)?;
    Ok(Value::num(
        t.rows
            .iter()
            .position(|row| row_matches(&t, row, &r))
            .map_or(-1.0, |i| i as f64),
    ))
}

// ---------------------------------------------------------------------------
// Grouping and joins.

fn group(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let key_names = names(arg(a, 1))?;
    let key_cols: Vec<usize> = key_names.iter().map(|n| index_of(&t, n)).collect::<Res<_>>()?;
    let aggs: Vec<Vec<Value>> = match arg(a, 2) {
        Value::List(l) if l.is_empty() => Vec::new(),
        v => pairs(v)?,
    };
    let local = opt_number(arg(a, 3))? == Some(0.0);
    let ignore_case = matches!(arg(a, 4), Value::Function(f) if matches!(&**f, Func::Native(Native::Comparer { ignore_case: true })));
    let keys: Vec<Vec<&Value>> = t
        .rows
        .iter()
        .map(|r| {
            key_cols
                .iter()
                .map(|&c| r.get(c).unwrap_or(&Value::Null))
                .collect()
        })
        .collect();
    let groups: Vec<Vec<usize>> = if local {
        let mut gs: Vec<Vec<usize>> = Vec::new();
        for (i, k) in keys.iter().enumerate() {
            let same = gs
                .last()
                .and_then(|g| g.first())
                .and_then(|&f| keys.get(f))
                .is_some_and(|f| keys_equal(f, k, ignore_case));
            match gs.last_mut() {
                Some(g) if same => g.push(i),
                _ => gs.push(vec![i]),
            }
        }
        gs
    } else {
        group_rows(interp, &keys, ignore_case)?
    };
    let mut cols: Vec<Column> = key_cols
        .iter()
        .filter_map(|&c| t.columns.get(c).cloned())
        .collect();
    let mut fns = Vec::with_capacity(aggs.len());
    for p in &aggs {
        let name = text(p.first().unwrap_or(&Value::Null))?;
        let f = func(p.get(1).unwrap_or(&Value::Null))?.clone();
        let ty = mtype(p.get(2).unwrap_or(&Value::Null))?;
        cols.push(col(name, ty));
        fns.push(f);
    }
    let mut rows = Vec::with_capacity(groups.len());
    for g in &groups {
        let Some(&first) = g.first() else { continue };
        let mut row: Vec<Value> = key_cols
            .iter()
            .map(|&c| {
                t.rows
                    .get(first)
                    .and_then(|r| r.get(c))
                    .cloned()
                    .unwrap_or(Value::Null)
            })
            .collect();
        let sub = Value::table(sub_table(&t, g));
        for f in &fns {
            interp.tick(1)?;
            let r = interp.call(f, vec![sub.clone()]);
            row.push(Value::store(r)?);
        }
        rows.push(row);
    }
    make(interp, cols, rows)
}

fn join_kind(v: &Value) -> Res<u8> {
    let k = opt_number(v)?.unwrap_or(1.0);
    match k as i64 {
        0 => Ok(0),
        1 => Ok(1),
        4 => Ok(4),
        2 => unsupported("`JoinKind.RightOuter`"),
        3 => unsupported("`JoinKind.FullOuter`"),
        5 => unsupported("`JoinKind.RightAnti`"),
        _ => expr_err("Unknown join kind."),
    }
}

/// For each left row, the matching right rows (key equality as in M; null matches null).
fn match_rows(
    interp: &mut Interp,
    left: &Table,
    lk: &[usize],
    right: &Table,
    rk: &[usize],
) -> Res<Vec<Vec<usize>>> {
    if lk.len() != rk.len() {
        return expr_err("The number of keys in the two tables must be the same.");
    }
    let mut index: HashMap<String, Vec<usize>> = HashMap::new();
    let mut complex: Vec<usize> = Vec::new();
    for (i, r) in right.rows.iter().enumerate() {
        interp.tick(1)?;
        let k: Vec<&Value> = rk.iter().map(|&c| r.get(c).unwrap_or(&Value::Null)).collect();
        match hash_key(&k, false) {
            Some(h) => index.entry(h).or_default().push(i),
            None => complex.push(i),
        }
    }
    let mut out = Vec::with_capacity(left.rows.len());
    for r in &left.rows {
        interp.tick(1)?;
        let k: Vec<&Value> = lk.iter().map(|&c| r.get(c).unwrap_or(&Value::Null)).collect();
        let mut hits = match hash_key(&k, false) {
            Some(h) => index.get(&h).cloned().unwrap_or_default(),
            None => Vec::new(),
        };
        for &i in &complex {
            let rkv: Vec<&Value> = rk
                .iter()
                .map(|&c| {
                    right
                        .rows
                        .get(i)
                        .and_then(|row| row.get(c))
                        .unwrap_or(&Value::Null)
                })
                .collect();
            if keys_equal(&k, &rkv, false) {
                hits.push(i);
            }
        }
        hits.sort_unstable();
        out.push(hits);
    }
    Ok(out)
}

pub(crate) fn key_cols(t: &Table, v: &Value) -> Res<Vec<usize>> {
    names(v)?.iter().map(|n| index_of(t, n)).collect()
}

pub(crate) fn nested_join(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let left = tbl(arg(a, 0))?.clone();
    let lk = key_cols(&left, arg(a, 1))?;
    let right = tbl(arg(a, 2))?.clone();
    let rk = key_cols(&right, arg(a, 3))?;
    let name = text(arg(a, 4))?;
    let kind = join_kind(arg(a, 5))?;
    let matches = match_rows(interp, &left, &lk, &right, &rk)?;
    let mut cols = left.columns.clone();
    cols.push(col(name, right.ty()));
    let mut rows = Vec::new();
    for (r, hits) in left.rows.iter().zip(matches.iter()) {
        interp.tick(1)?;
        let keep = match kind {
            0 => !hits.is_empty(),
            4 => hits.is_empty(),
            _ => true,
        };
        if !keep {
            continue;
        }
        let mut row = r.clone();
        row.push(Value::table(sub_table(&right, hits)));
        rows.push(row);
    }
    make(interp, cols, rows)
}

fn join(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let left = tbl(arg(a, 0))?.clone();
    let lk = key_cols(&left, arg(a, 1))?;
    let right = tbl(arg(a, 2))?.clone();
    let rk = key_cols(&right, arg(a, 3))?;
    let kind = match opt_number(arg(a, 4))? {
        None => 0,
        Some(_) => join_kind(arg(a, 4))?,
    };
    let matches = match_rows(interp, &left, &lk, &right, &rk)?;
    let mut cols = left.columns.clone();
    if kind != 4 {
        cols.extend(right.columns.iter().cloned());
    }
    let mut rows = Vec::new();
    for (r, hits) in left.rows.iter().zip(matches.iter()) {
        interp.tick(1)?;
        match kind {
            4 => {
                if hits.is_empty() {
                    rows.push(r.clone());
                }
            }
            _ => {
                if hits.is_empty() && kind == 1 {
                    let mut row = r.clone();
                    row.extend(std::iter::repeat_n(Value::Null, right.columns.len()));
                    rows.push(row);
                }
                for &h in hits {
                    let mut row = r.clone();
                    row.extend(right.rows.get(h).cloned().unwrap_or_default());
                    rows.push(row);
                    interp.check_rows(rows.len())?;
                }
            }
        }
    }
    make(interp, cols, rows)
}

fn expand_table_column(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let c = index_of(&t, text(arg(a, 1))?)?;
    let fields = names(arg(a, 2))?;
    let new_names = match arg(a, 3) {
        Value::Null => fields.clone(),
        v => names(v)?,
    };
    if new_names.len() != fields.len() {
        return expr_err("The number of new column names must match the number of columns to expand.");
    }
    let declared = t.columns.get(c).and_then(|cc| cc.ty.fields.clone());
    let mut cols: Vec<Column> = Vec::new();
    for (i, cc) in t.columns.iter().enumerate() {
        if i == c {
            for (f, n) in fields.iter().zip(new_names.iter()) {
                let ty = declared
                    .as_ref()
                    .and_then(|d| d.iter().find(|(m, _)| m == f))
                    .map_or_else(MType::any, |(_, t)| t.clone());
                cols.push(col(n.clone(), ty));
            }
        } else {
            cols.push(cc.clone());
        }
    }
    let mut rows = Vec::new();
    for r in &t.rows {
        interp.tick(1)?;
        let cell = r.get(c).cloned().unwrap_or(Value::Null).check()?;
        let push = |rows: &mut Vec<Vec<Value>>, vals: Vec<Value>| {
            let mut row = Vec::with_capacity(r.len() + vals.len());
            row.extend(r.iter().take(c).cloned());
            row.extend(vals);
            row.extend(r.iter().skip(c + 1).cloned());
            rows.push(row);
        };
        match cell {
            Value::Null => push(&mut rows, vec![Value::Null; fields.len()]),
            Value::Table(nt) => {
                let idx: Vec<Option<usize>> = fields.iter().map(|f| nt.index(f)).collect();
                if let Some((f, _)) = fields.iter().zip(idx.iter()).find(|(_, i)| i.is_none()) {
                    return missing_column(f);
                }
                if nt.rows.is_empty() {
                    push(&mut rows, vec![Value::Null; fields.len()]);
                }
                for nr in &nt.rows {
                    push(
                        &mut rows,
                        idx.iter()
                            .map(|i| i.and_then(|i| nr.get(i)).cloned().unwrap_or(Value::Null))
                            .collect(),
                    );
                    interp.check_rows(rows.len())?;
                }
            }
            other => return conv_err(&other, "Table"),
        }
    }
    make(interp, cols, rows)
}

fn expand_record_column(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let c = index_of(&t, text(arg(a, 1))?)?;
    let fields = names(arg(a, 2))?;
    let new_names = match arg(a, 3) {
        Value::Null => fields.clone(),
        v => names(v)?,
    };
    let mut cols: Vec<Column> = Vec::new();
    for (i, cc) in t.columns.iter().enumerate() {
        if i == c {
            cols.extend(new_names.iter().map(|n| col(n.clone(), MType::any())));
        } else {
            cols.push(cc.clone());
        }
    }
    let mut rows = Vec::with_capacity(t.rows.len());
    for r in &t.rows {
        interp.tick(1)?;
        let cell = r.get(c).cloned().unwrap_or(Value::Null).check()?;
        let vals: Vec<Value> = match &cell {
            Value::Null => vec![Value::Null; fields.len()],
            Value::Record(rec) => fields
                .iter()
                .map(|f| rec.get(f).cloned().unwrap_or(Value::Null))
                .collect(),
            other => return conv_err(other, "Record"),
        };
        let mut row = Vec::with_capacity(r.len() + vals.len());
        row.extend(r.iter().take(c).cloned());
        row.extend(vals);
        row.extend(r.iter().skip(c + 1).cloned());
        rows.push(row);
    }
    make(interp, cols, rows)
}

fn expand_list_column(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let c = index_of(&t, text(arg(a, 1))?)?;
    let mut cols = t.columns.clone();
    if let Some(cc) = cols.get_mut(c) {
        cc.ty = MType::any();
    }
    let mut rows = Vec::new();
    for r in &t.rows {
        interp.tick(1)?;
        let cell = r.get(c).cloned().unwrap_or(Value::Null).check()?;
        match cell {
            Value::List(l) => {
                if l.is_empty() {
                    let mut row = r.clone();
                    if let Some(x) = row.get_mut(c) {
                        *x = Value::Null;
                    }
                    rows.push(row);
                }
                for v in l.iter() {
                    let mut row = r.clone();
                    if let Some(x) = row.get_mut(c) {
                        *x = v.clone();
                    }
                    rows.push(row);
                    interp.check_rows(rows.len())?;
                }
            }
            Value::Null => rows.push(r.clone()),
            other => return conv_err(&other, "List"),
        }
    }
    make(interp, cols, rows)
}

/// Appends tables, matching columns by name (first-seen order; missing → null).
pub(crate) fn combine(interp: &mut Interp, tables: &[Rc<Table>]) -> Res<Value> {
    let mut cols: Vec<Column> = Vec::new();
    for t in tables {
        for c in &t.columns {
            match cols.iter_mut().find(|d| d.name == c.name) {
                Some(d) => {
                    if d.ty.kind != c.ty.kind || d.ty.facet != c.ty.facet {
                        d.ty = MType::any();
                    } else if c.ty.nullable {
                        d.ty.nullable = true;
                    }
                }
                None => cols.push(c.clone()),
            }
        }
    }
    // A column missing from one of the tables becomes nullable any-compatible.
    let mut rows = Vec::new();
    for t in tables {
        let map: Vec<Option<usize>> = cols.iter().map(|c| t.index(&c.name)).collect();
        for r in &t.rows {
            interp.tick(1)?;
            rows.push(
                map.iter()
                    .map(|m| m.and_then(|i| r.get(i)).cloned().unwrap_or(Value::Null))
                    .collect(),
            );
        }
        interp.check_rows(rows.len())?;
    }
    make(interp, cols, rows)
}

fn combine_fn(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let ts: Vec<Rc<Table>> = items(arg(a, 0))?
        .iter()
        .map(|t| tbl(t).cloned())
        .collect::<Res<_>>()?;
    let out = combine(interp, &ts)?;
    match arg(a, 1) {
        Value::Null => Ok(out),
        cols => {
            let t = tbl(&out)?.clone();
            select_columns(interp, &t, cols, &Value::num(MISSING_NULL))
        }
    }
}

// ---------------------------------------------------------------------------
// Reshaping.

fn unpivot_impl(interp: &mut Interp, t: &Table, pivot: &[usize], attr: &str, value: &str) -> Res<Value> {
    let keep: Vec<usize> = (0..t.columns.len()).filter(|i| !pivot.contains(i)).collect();
    let mut cols: Vec<Column> = keep.iter().filter_map(|&i| t.columns.get(i).cloned()).collect();
    cols.push(col(attr, MType::prim(Kind::Text)));
    cols.push(col(value, MType::any()));
    let mut rows = Vec::new();
    for r in &t.rows {
        for &p in pivot {
            interp.tick(1)?;
            let v = r.get(p).cloned().unwrap_or(Value::Null);
            if v.is_null() {
                continue;
            }
            let mut row: Vec<Value> = keep
                .iter()
                .map(|&i| r.get(i).cloned().unwrap_or(Value::Null))
                .collect();
            row.push(Value::Text(
                t.columns
                    .get(p)
                    .map(|c| c.name.clone())
                    .unwrap_or_else(|| "".into()),
            ));
            row.push(v);
            rows.push(row);
        }
        interp.check_rows(rows.len())?;
    }
    make(interp, cols, rows)
}

fn unpivot(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let pivot = key_cols(&t, arg(a, 1))?;
    unpivot_impl(interp, &t, &pivot, text(arg(a, 2))?, text(arg(a, 3))?)
}

fn unpivot_other(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let keep = key_cols(&t, arg(a, 1))?;
    let pivot: Vec<usize> = (0..t.columns.len()).filter(|i| !keep.contains(i)).collect();
    unpivot_impl(interp, &t, &pivot, text(arg(a, 2))?, text(arg(a, 3))?)
}

fn pivot(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let values: Vec<Rc<str>> = names(arg(a, 1))?;
    let attr = index_of(&t, text(arg(a, 2))?)?;
    let val = index_of(&t, text(arg(a, 3))?)?;
    let agg = arg(a, 4).clone();
    let keep: Vec<usize> = (0..t.columns.len()).filter(|&i| i != attr && i != val).collect();
    let keys: Vec<Vec<&Value>> = t
        .rows
        .iter()
        .map(|r| keep.iter().map(|&c| r.get(c).unwrap_or(&Value::Null)).collect())
        .collect();
    let groups = group_rows(interp, &keys, false)?;
    let mut cols: Vec<Column> = keep.iter().filter_map(|&i| t.columns.get(i).cloned()).collect();
    cols.extend(values.iter().map(|v| col(v.clone(), MType::any())));
    let mut rows = Vec::with_capacity(groups.len());
    for g in &groups {
        let Some(&first) = g.first() else { continue };
        let mut row: Vec<Value> = keep
            .iter()
            .map(|&c| {
                t.rows
                    .get(first)
                    .and_then(|r| r.get(c))
                    .cloned()
                    .unwrap_or(Value::Null)
            })
            .collect();
        for v in &values {
            interp.tick(g.len() as u64)?;
            let hits: Vec<Value> = g
                .iter()
                .filter_map(|&i| t.rows.get(i))
                .filter(|r| matches!(r.get(attr), Some(Value::Text(x)) if x == v))
                .map(|r| r.get(val).cloned().unwrap_or(Value::Null))
                .collect();
            let cell = match &agg {
                Value::Null => match hits.len() {
                    0 => Value::Null,
                    1 => hits.into_iter().next().unwrap_or(Value::Null),
                    _ => Value::store(expr_err(
                        "There were too many elements in the enumeration to complete the operation.",
                    ))?,
                },
                f => Value::store(interp.call(f, vec![Value::list(hits)]))?,
            };
            row.push(cell);
        }
        rows.push(row);
    }
    make(interp, cols, rows)
}

fn split_column(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let name = text(arg(a, 1))?;
    let c = index_of(&t, name)?;
    let splitter = func(arg(a, 2))?.clone();
    let default = arg(a, 4).clone();
    let mut parts: Vec<Vec<Value>> = Vec::with_capacity(t.rows.len());
    for r in &t.rows {
        interp.tick(1)?;
        let cell = r.get(c).cloned().unwrap_or(Value::Null).check()?;
        let p = match cell {
            Value::Null => vec![Value::Null],
            v => match interp.call(&splitter, vec![v])? {
                Value::List(l) => l.to_vec(),
                other => vec![other],
            },
        };
        parts.push(p);
    }
    let new_names: Vec<Rc<str>> = match arg(a, 3) {
        Value::Null => {
            let n = parts.iter().map(Vec::len).max().unwrap_or(1).max(1);
            (1..=n).map(|i| Rc::from(format!("{name}.{i}"))).collect()
        }
        Value::Number(_) => (1..=column_count_arg(interp, arg(a, 3))?)
            .map(|i| Rc::from(format!("{name}.{i}")))
            .collect(),
        v => names(v)?,
    };
    let w = new_names.len();
    let mut cols: Vec<Column> = Vec::new();
    for (i, cc) in t.columns.iter().enumerate() {
        if i == c {
            cols.extend(new_names.iter().map(|n| col(n.clone(), MType::any())));
        } else {
            cols.push(cc.clone());
        }
    }
    let mut rows = Vec::with_capacity(t.rows.len());
    for (r, mut p) in t.rows.iter().zip(parts) {
        p.truncate(w);
        p.resize(w, default.clone());
        let mut row = Vec::with_capacity(r.len() + w);
        row.extend(r.iter().take(c).cloned());
        row.extend(p);
        row.extend(r.iter().skip(c + 1).cloned());
        rows.push(row);
    }
    make(interp, cols, rows)
}

fn combine_columns(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let src = key_cols(&t, arg(a, 1))?;
    let combiner = func(arg(a, 2))?.clone();
    let name = text(arg(a, 3))?;
    let first = src.iter().copied().min().unwrap_or(0);
    let mut cols: Vec<Column> = Vec::new();
    for (i, cc) in t.columns.iter().enumerate() {
        if i == first {
            cols.push(col(name, MType::any()));
        } else if !src.contains(&i) {
            cols.push(cc.clone());
        }
    }
    let mut rows = Vec::with_capacity(t.rows.len());
    for r in &t.rows {
        interp.tick(1)?;
        let vals: Vec<Value> = src
            .iter()
            .map(|&i| r.get(i).cloned().unwrap_or(Value::Null))
            .collect();
        let combined = Value::store(interp.call(&combiner, vec![Value::list(vals)]))?;
        let mut row = Vec::with_capacity(cols.len());
        for (i, v) in r.iter().enumerate() {
            if i == first {
                row.push(combined.clone());
            } else if !src.contains(&i) {
                row.push(v.clone());
            }
        }
        rows.push(row);
    }
    make(interp, cols, rows)
}

fn transpose(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let t = tbl(arg(a, 0))?.clone();
    let n = t.rows.len();
    let cols = header(interp, arg(a, 1), n)?;
    let rows = (0..t.columns.len())
        .map(|c| {
            (0..cols.len())
                .map(|r| {
                    t.rows
                        .get(r)
                        .and_then(|row| row.get(c))
                        .cloned()
                        .unwrap_or(Value::Null)
                })
                .collect()
        })
        .collect();
    make(interp, cols, rows)
}
