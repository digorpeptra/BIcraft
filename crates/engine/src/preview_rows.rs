//! `table.rows`: one page of a table's rows for the app's Data view (Power BI Desktop's table
//! view; docs/mcp.md "Data view"), read from the same columns the DAX evaluator reads ([`super`]:
//! the PBIX `DataModel`, a PBIP's data, literal partitions), so a table of tens of millions of
//! rows is paged without building its rows.
//!
//! - Without a sort or a filter, a page is the rows `offset..offset + limit`: each cell is one
//!   code looked up in its column's dictionary (`O(limit × columns)`).
//! - A sort (one column, as the Data view's header sorts) ranks the column's dictionary once,
//!   then orders the rows with a counting sort over the ranks (two sequential passes over the
//!   codes, stable: equal values keep the table's order); a filter (per column: a list of values,
//!   a comparison, a text match, blank) is evaluated once per distinct value, then the rows are
//!   read in blocks of codes. The row order is kept in the session (a few of them) until the data
//!   or the model changes, so the next pages cost one lookup each. Both reserve their memory
//!   against the process' memory limit first (`bicraft-mem`; refused with the size needed).
//! - Calculated columns the data does not hold (added in BIcraft, or a literal table's) are
//!   evaluated by the DAX evaluator for the page's rows only (`preview_eval_rows.rs`); sorting or
//!   filtering on one computes it over the table (at most [`MAX_CALC_ROWS`] rows) and keeps it.
//!
//! Sorting and filtering are view state, as in Desktop: nothing in the model changes.

use std::cmp::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bicraft_vertipaq::CellValue;
use serde_json::{Map, Value, json};

use super::store::{ColData, Miss, model_table};
use super::{Ctx, PreviewLimits, cell_json, filters};
use crate::Session;
use crate::params::{allow, str_param};

/// Rows of a page when `limit` is not given.
pub(crate) const DEFAULT_LIMIT: usize = 100;
/// Most rows of a page.
pub(crate) const MAX_LIMIT: usize = 10_000;
/// Most columns asked by name.
const MAX_COLUMNS: usize = 4_096;
/// Most filter conditions.
const MAX_CONDITIONS: usize = 32;
/// Most values of one `in` condition.
const MAX_VALUES: usize = 10_000;
/// Longest text operand, in bytes.
const MAX_OPERAND: usize = 4_096;
/// Rows decoded at a time while sorting or filtering.
const CHUNK: usize = 1 << 16;
/// Most rows a calculated column is computed over for a sort or a filter.
pub(crate) const MAX_CALC_ROWS: usize = 2_000_000;
/// Row orders (and computed columns) kept per session.
const KEPT: usize = 4;
/// Longest time one call computes.
const BUDGET: Duration = Duration::from_secs(120);
/// Largest table paged (row indexes are 32-bit).
const MAX_TABLE_ROWS: usize = u32::MAX as usize;

/// What a session keeps between pages: row orders and calculated columns computed for a sort or
/// a filter, newest first, each with its memory reservation.
#[derive(Default)]
pub(super) struct Kept {
    orders: Vec<(String, Arc<Vec<u32>>, bicraft_mem::Reservation)>,
    calcs: Vec<(String, Arc<ColData>, bicraft_mem::Reservation)>,
}

impl Kept {
    fn order(&mut self, key: &str) -> Option<Arc<Vec<u32>>> {
        let i = self.orders.iter().position(|(k, _, _)| k == key)?;
        let e = self.orders.remove(i);
        let o = Arc::clone(&e.1);
        self.orders.insert(0, e);
        Some(o)
    }

    fn keep_order(&mut self, key: String, o: Arc<Vec<u32>>, r: bicraft_mem::Reservation) {
        self.orders.retain(|(k, _, _)| *k != key);
        self.orders.insert(0, (key, o, r));
        self.orders.truncate(KEPT);
    }

    fn calc(&mut self, key: &str) -> Option<Arc<ColData>> {
        self.calcs
            .iter()
            .find(|(k, _, _)| k == key)
            .map(|(_, c, _)| Arc::clone(c))
    }

    fn keep_calc(&mut self, key: String, c: Arc<ColData>, r: bicraft_mem::Reservation) {
        self.calcs.retain(|(k, _, _)| *k != key);
        self.calcs.insert(0, (key, c, r));
        self.calcs.truncate(KEPT);
    }

    /// Drops what was computed from older data or another model.
    pub(super) fn clear(&mut self) {
        self.orders.clear();
        self.calcs.clear();
    }

    /// Bytes reserved for what is kept.
    pub(super) fn bytes(&self) -> usize {
        let o: usize = self.orders.iter().map(|(_, _, r)| r.bytes()).sum();
        let c: usize = self.calcs.iter().map(|(_, _, r)| r.bytes()).sum();
        o.saturating_add(c)
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.orders.len() + self.calcs.len()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
    Contains,
    NotContains,
    StartsWith,
    NotStartsWith,
    EndsWith,
    NotEndsWith,
    In,
    NotIn,
    Blank,
    NotBlank,
}

impl Op {
    fn parse(s: &str) -> Option<Op> {
        Some(match s {
            "=" | "eq" => Op::Eq,
            "<>" | "!=" | "ne" => Op::Ne,
            ">" | "gt" => Op::Gt,
            ">=" | "ge" => Op::Ge,
            "<" | "lt" => Op::Lt,
            "<=" | "le" => Op::Le,
            "contains" => Op::Contains,
            "not_contains" => Op::NotContains,
            "starts_with" => Op::StartsWith,
            "not_starts_with" => Op::NotStartsWith,
            "ends_with" => Op::EndsWith,
            "not_ends_with" => Op::NotEndsWith,
            "in" => Op::In,
            "not_in" => Op::NotIn,
            "blank" => Op::Blank,
            "not_blank" => Op::NotBlank,
            _ => return None,
        })
    }
}

/// One test of a value.
#[derive(Debug, Clone)]
struct Alt {
    op: Op,
    values: Vec<Value>,
}

/// One filter condition on a column: the value passes one of its tests (one, or the `any` list
/// of the Data view's custom filter with Or).
#[derive(Debug, Clone)]
struct Cond {
    column: String,
    alts: Vec<Alt>,
}

const OPS: &str = "=, <>, >, >=, <, <=, contains, not_contains, starts_with, not_starts_with, ends_with, \
                   not_ends_with, in, not_in, blank, not_blank";

/// Most tests of one `any` condition.
const MAX_ALTS: usize = 8;

/// The test `{op, value | values}` of a condition.
fn parse_alt(o: &serde_json::Map<String, Value>) -> Result<Alt, String> {
    let values: Vec<Value> = match (o.get("value"), o.get("values")) {
        (Some(_), Some(_)) => {
            return Err("a `filter` condition takes `value` or `values`, not both".into());
        }
        (Some(v), None) => vec![v.clone()],
        (None, Some(Value::Array(a))) => {
            if a.len() > MAX_VALUES {
                return Err(format!(
                    "a `filter` condition lists more than {MAX_VALUES} values"
                ));
            }
            a.clone()
        }
        (None, Some(_)) => return Err("`values` must be a list".into()),
        (None, None) => Vec::new(),
    };
    for v in &values {
        operand_ok(v)?;
    }
    let op = match o.get("op") {
        None | Some(Value::Null) if o.contains_key("values") => Op::In,
        None | Some(Value::Null) => Op::Eq,
        Some(Value::String(s)) => {
            Op::parse(s).ok_or_else(|| format!("unknown filter `op` `{s}` (expected {OPS})"))?
        }
        Some(_) => return Err(format!("`op` must be one of {OPS}")),
    };
    match op {
        Op::Blank | Op::NotBlank if !values.is_empty() => {
            return Err("`blank` and `not_blank` take no value".into());
        }
        Op::Blank | Op::NotBlank | Op::In | Op::NotIn => {}
        _ if values.len() != 1 => return Err("this filter `op` takes one `value`".into()),
        _ => {}
    }
    Ok(Alt { op, values })
}

fn operand_ok(v: &Value) -> Result<(), String> {
    match v {
        Value::Null | Value::Bool(_) | Value::Number(_) => Ok(()),
        Value::String(s) if s.len() <= MAX_OPERAND => Ok(()),
        Value::String(_) => Err(format!("a filter value is longer than {MAX_OPERAND} bytes")),
        _ => Err("a filter value must be text, a number, true/false or null".into()),
    }
}

fn parse_filter(v: Option<&Value>) -> Result<Vec<Cond>, String> {
    let list = match v {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(a)) => a.as_slice(),
        Some(o @ Value::Object(_)) => std::slice::from_ref(o),
        Some(_) => return Err("`filter` must be a list of {column, op, value | values}".into()),
    };
    if list.len() > MAX_CONDITIONS {
        return Err(format!("`filter` has more than {MAX_CONDITIONS} conditions"));
    }
    let mut out = Vec::with_capacity(list.len());
    for c in list {
        let o = c
            .as_object()
            .ok_or("each `filter` condition must be an object {column, op, value | values}")?;
        for k in o.keys() {
            if !matches!(k.as_str(), "column" | "op" | "value" | "values" | "any") {
                return Err(format!("unknown key `{k}` in a `filter` condition"));
            }
        }
        let column = o
            .get("column")
            .and_then(Value::as_str)
            .ok_or("each `filter` condition needs a `column` (text)")?
            .to_string();
        let alts = match o.get("any") {
            None | Some(Value::Null) => vec![parse_alt(o)?],
            Some(Value::Array(list)) => {
                if ["op", "value", "values"].iter().any(|k| o.contains_key(*k)) {
                    return Err("a `filter` condition takes `any` or `op`/`value`, not both".into());
                }
                if list.is_empty() || list.len() > MAX_ALTS {
                    return Err(format!(
                        "`any` lists 1 to {MAX_ALTS} tests {{op, value | values}}"
                    ));
                }
                list.iter()
                    .map(|a| {
                        let a = a
                            .as_object()
                            .ok_or("each `any` test must be an object {op, value | values}")?;
                        if let Some(k) = a
                            .keys()
                            .find(|k| !matches!(k.as_str(), "op" | "value" | "values"))
                        {
                            return Err(format!("unknown key `{k}` in an `any` test"));
                        }
                        parse_alt(a)
                    })
                    .collect::<Result<Vec<_>, _>>()?
            }
            Some(_) => return Err("`any` must be a list of {op, value | values}".into()),
        };
        out.push(Cond { column, alts });
    }
    Ok(out)
}

/// The sort: a column and whether descending.
fn parse_sort(v: Option<&Value>) -> Result<Option<(String, bool)>, String> {
    let o = match v {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::String(s)) => return Ok(Some((s.clone(), false))),
        Some(Value::Object(o)) => o,
        Some(_) => return Err("`sort` must be {column, descending?: bool} or a column name".into()),
    };
    for k in o.keys() {
        if !matches!(k.as_str(), "column" | "descending" | "order") {
            return Err(format!("unknown key `{k}` in `sort`"));
        }
    }
    let column = o
        .get("column")
        .and_then(Value::as_str)
        .ok_or("`sort` needs a `column` (text)")?
        .to_string();
    let mut desc = match o.get("descending") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err("`sort.descending` must be true or false".into()),
    };
    match o.get("order") {
        None | Some(Value::Null) => {}
        Some(Value::String(s)) if s == "asc" || s == "ascending" => desc = false,
        Some(Value::String(s)) if s == "desc" || s == "descending" => desc = true,
        Some(_) => return Err("`sort.order` must be \"asc\" or \"desc\"".into()),
    }
    Ok(Some((column, desc)))
}

fn uint(p: &Value, key: &str, default: usize, max: usize, min: usize) -> Result<usize, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| (min..=max).contains(n))
            .ok_or_else(|| format!("`{key}` must be a whole number from {min} to {max}")),
    }
}

// --- values ---------------------------------------------------------------------------------

/// Type rank of a value for sorting: blank, booleans, numbers, dates, text.
fn type_rank(v: &CellValue) -> u8 {
    match v {
        CellValue::Null => 0,
        CellValue::Bool(_) => 1,
        CellValue::Int(_) | CellValue::Real(_) | CellValue::Decimal(_) => 2,
        CellValue::DateTime(_) => 3,
        CellValue::Text(_) => 4,
    }
}

fn num(v: &CellValue) -> Option<f64> {
    match v {
        CellValue::Int(n) => Some(*n as f64),
        CellValue::Real(x) | CellValue::DateTime(x) => Some(*x),
        CellValue::Decimal(n) => Some(*n as f64 / 10_000.0),
        CellValue::Bool(b) => Some(f64::from(u8::from(*b))),
        _ => None,
    }
}

/// The Data view's order: blank first, then by value (text without regard to case, then by
/// case).
fn compare(a: &CellValue, b: &CellValue) -> Ordering {
    type_rank(a).cmp(&type_rank(b)).then_with(|| match (a, b) {
        (CellValue::Text(x), CellValue::Text(y)) => {
            x.to_lowercase().cmp(&y.to_lowercase()).then_with(|| x.cmp(y))
        }
        (CellValue::Int(x), CellValue::Int(y)) => x.cmp(y),
        (CellValue::Decimal(x), CellValue::Decimal(y)) => x.cmp(y),
        _ => num(a).unwrap_or(0.0).total_cmp(&num(b).unwrap_or(0.0)),
    })
}

/// The text of a value as a filter reads it.
fn text_of(v: &CellValue) -> String {
    match cell_json(v) {
        Value::String(s) => s,
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// How a cell compares with an operand (`None`: not comparable, a blank against a value).
fn cmp_operand(v: &CellValue, x: &Value) -> Option<Ordering> {
    match (v, x) {
        (CellValue::Null, Value::Null) => Some(Ordering::Equal),
        (CellValue::Null, _) | (_, Value::Null) => None,
        (CellValue::Text(s), x) => {
            let t = match x {
                Value::String(t) => t.clone(),
                other => other.to_string(),
            };
            Some(s.to_lowercase().cmp(&t.to_lowercase()))
        }
        (CellValue::Bool(b), Value::Bool(c)) => Some(b.cmp(c)),
        (CellValue::Bool(b), Value::String(s)) => match s.to_ascii_lowercase().as_str() {
            "true" => Some(b.cmp(&true)),
            "false" => Some(b.cmp(&false)),
            _ => None,
        },
        (CellValue::DateTime(d), Value::String(s)) => {
            let t = filters::parse_as_of(s).ok()?;
            Some(d.total_cmp(&t))
        }
        (v, Value::Number(n)) => Some(num(v)?.total_cmp(&n.as_f64()?)),
        (v, Value::String(s)) => {
            let t: f64 = s.trim().parse().ok()?;
            Some(num(v)?.total_cmp(&t))
        }
        _ => None,
    }
}

/// Whether a value passes one of a condition's tests.
fn matches(v: &CellValue, c: &Cond) -> bool {
    c.alts.iter().any(|a| passes(v, a))
}

fn passes(v: &CellValue, c: &Alt) -> bool {
    let first = c.values.first();
    let lower = || {
        first.map(|x| match x {
            Value::String(s) => s.to_lowercase(),
            other => other.to_string().to_lowercase(),
        })
    };
    let eq = |x: &Value| cmp_operand(v, x) == Some(Ordering::Equal);
    let ord = || first.and_then(|x| cmp_operand(v, x));
    let blank = matches!(v, CellValue::Null) || matches!(v, CellValue::Text(s) if s.is_empty());
    match c.op {
        Op::Blank => blank,
        Op::NotBlank => !blank,
        Op::Eq => first.is_some_and(eq),
        Op::Ne => !first.is_some_and(eq),
        Op::In => c.values.iter().any(eq),
        Op::NotIn => !c.values.iter().any(eq),
        Op::Gt => ord() == Some(Ordering::Greater),
        Op::Ge => matches!(ord(), Some(Ordering::Greater | Ordering::Equal)),
        Op::Lt => ord() == Some(Ordering::Less),
        Op::Le => matches!(ord(), Some(Ordering::Less | Ordering::Equal)),
        Op::Contains
        | Op::NotContains
        | Op::StartsWith
        | Op::NotStartsWith
        | Op::EndsWith
        | Op::NotEndsWith => {
            let q = lower().unwrap_or_default();
            let t = text_of(v).to_lowercase();
            match c.op {
                Op::Contains => t.contains(&q),
                Op::NotContains => !t.contains(&q),
                Op::StartsWith => t.starts_with(&q),
                Op::NotStartsWith => !t.starts_with(&q),
                Op::EndsWith => t.ends_with(&q),
                _ => !t.ends_with(&q),
            }
        }
    }
}

/// The codes of rows `from..from + out.len()` (0, blank, past the end).
fn fill(c: &ColData, from: usize, out: &mut [u32]) {
    match c.codes.col() {
        bicraft_scan::Col::Plain(v) => {
            for (i, o) in out.iter_mut().enumerate() {
                *o = from.checked_add(i).and_then(|r| v.get(r)).copied().unwrap_or(0);
            }
        }
        bicraft_scan::Col::Packed(p) => p.fill(from, out),
    }
}

fn value_at(c: &ColData, row: u32) -> &CellValue {
    let code = usize::try_from(row)
        .ok()
        .and_then(|r| c.codes.get(r))
        .unwrap_or(0);
    usize::try_from(code)
        .ok()
        .and_then(|k| c.values.get(k))
        .unwrap_or(&CellValue::Null)
}

/// Per dictionary code, whether the value passes every condition of `conds` (one column's).
fn mask(c: &ColData, conds: &[&Cond]) -> Vec<bool> {
    c.values
        .iter()
        .map(|v| conds.iter().all(|k| matches(v, k)))
        .collect()
}

/// Per dictionary code, its rank in the Data view's order (equal values share a rank) and the
/// number of ranks.
fn ranks(c: &ColData) -> (Vec<u32>, usize) {
    let mut idx: Vec<u32> = (0..u32::try_from(c.values.len()).unwrap_or(u32::MAX)).collect();
    let get = |i: &u32| c.values.get(*i as usize).unwrap_or(&CellValue::Null);
    idx.sort_by(|a, b| compare(get(a), get(b)));
    let mut out = vec![0u32; c.values.len()];
    let mut rank = 0u32;
    let mut prev: Option<&CellValue> = None;
    for i in &idx {
        let v = get(i);
        if let Some(p) = prev
            && compare(p, v) != Ordering::Equal
        {
            rank = rank.saturating_add(1);
        }
        prev = Some(v);
        if let Some(slot) = out.get_mut(*i as usize) {
            *slot = rank;
        }
    }
    let card = if idx.is_empty() { 0 } else { rank as usize + 1 };
    (out, card)
}

/// One column's data for a page.
enum Data {
    Stored(Arc<ColData>),
    /// A calculated column the data does not hold: its expression.
    Calc(String),
    /// No data, with the reason.
    Missing(String),
}

/// The column's data, a calculated column without data as its expression.
fn data_of(
    ctx: &mut Ctx<'_>,
    table: &bicraft_model::Table,
    col: &bicraft_model::Column,
) -> Result<Data, String> {
    match ctx.column(&table.name, &col.name) {
        Ok(c) => Ok(Data::Stored(c)),
        Err(Miss::Unavailable(r)) => match &col.expression {
            Some(e) => Ok(Data::Calc(e.clone())),
            None => Ok(Data::Missing(r)),
        },
        Err(Miss::Error(e)) => Err(e),
    }
}

/// The data of a column a sort or a filter reads: stored, or a calculated column computed over
/// the table (kept in the session).
fn full_data(
    ctx: &mut Ctx<'_>,
    table: &bicraft_model::Table,
    col: &bicraft_model::Column,
    total: usize,
    stamp: &str,
) -> Result<Arc<ColData>, String> {
    match data_of(ctx, table, col)? {
        Data::Stored(c) => Ok(c),
        Data::Missing(r) => Err(format!("column '{}'[{}] has no data: {r}", table.name, col.name)),
        Data::Calc(expr) => {
            let key = format!(
                "{stamp}|{}|{}|{expr}",
                table.name.to_lowercase(),
                col.name.to_lowercase()
            );
            if let Some(c) = ctx.st.rows_kept.calc(&key) {
                return Ok(c);
            }
            if total > MAX_CALC_ROWS {
                return Err(format!(
                    "sorting or filtering on calculated column '{}'[{}] computes it over {total} rows \
                     (bicraft does it up to {MAX_CALC_ROWS}); save the file in Power BI Desktop to store its values",
                    table.name, col.name
                ));
            }
            let res = reserve(ctx, total.saturating_mul(24), "computing the calculated column")?;
            let rows: Vec<u32> = (0..u32::try_from(total).unwrap_or(u32::MAX)).collect();
            let values = super::general::rows_calc::calc_rows(ctx, &table.name, &expr, &rows, BUDGET)
                .map_err(|e| format!("calculated column '{}'[{}]: {e}", table.name, col.name))?;
            let values = super::general::rows_calc::as_declared(values, col.data_type);
            let c = Arc::new(ColData::from_stored(values).map_err(|m| match m {
                Miss::Unavailable(r) | Miss::Error(r) => r,
            })?);
            ctx.st.rows_kept.keep_calc(key, Arc::clone(&c), res);
            Ok(c)
        }
    }
}

/// Reserves `bytes` of the process' memory limit (always granted without a limit).
fn reserve(ctx: &Ctx<'_>, bytes: usize, what: &str) -> Result<bicraft_mem::Reservation, String> {
    ctx.st
        .memory
        .pool()
        .reserve(bicraft_mem::Class::Kept, bytes)
        .map_err(|e| format!("{what} needs {}: {e}", bicraft_mem::size_text(bytes)))
}

/// Calls `f(row, sort code)` for each row of the table that passes every filter mask, in table
/// order, reading the codes a block at a time.
fn scan(
    sort: Option<&ColData>,
    filters: &[(Arc<ColData>, Vec<bool>)],
    total: usize,
    started: Instant,
    mut f: impl FnMut(u32, u32),
) -> Result<(), String> {
    let mut bufs: Vec<Vec<u32>> = vec![vec![0u32; CHUNK.min(total.max(1))]; filters.len()];
    let mut sbuf = vec![
        0u32;
        if sort.is_some() {
            CHUNK.min(total.max(1))
        } else {
            0
        }
    ];
    let mut at = 0usize;
    while at < total {
        if crate::call_scope::cancelled() {
            return Err("cancelled".into());
        }
        if started.elapsed() > BUDGET {
            return Err(format!(
                "ordering the rows takes more than {} s",
                BUDGET.as_secs()
            ));
        }
        let len = CHUNK.min(total - at);
        for ((c, _), b) in filters.iter().zip(bufs.iter_mut()) {
            if let Some(b) = b.get_mut(..len) {
                fill(c, at, b);
            }
        }
        if let (Some(c), Some(b)) = (sort, sbuf.get_mut(..len)) {
            fill(c, at, b);
        }
        for i in 0..len {
            let pass = filters.iter().zip(bufs.iter()).all(|((_, m), b)| {
                b.get(i)
                    .and_then(|code| m.get(*code as usize))
                    .copied()
                    .unwrap_or(false)
            });
            if pass {
                let row = u32::try_from(at + i).unwrap_or(u32::MAX);
                f(row, sbuf.get(i).copied().unwrap_or(0));
            }
        }
        at += len;
    }
    Ok(())
}

/// The rows of the table in the asked order, the filter applied (`total` rows in the table): a
/// counting sort over the sort column's ranks (stable), or the rows kept in table order.
fn order(
    sort: Option<(&ColData, bool)>,
    filters: &[(Arc<ColData>, Vec<bool>)],
    total: usize,
) -> Result<Vec<u32>, String> {
    let started = Instant::now();
    let Some((col, desc)) = sort else {
        let mut kept = Vec::new();
        scan(None, filters, total, started, |row, _| kept.push(row))?;
        return Ok(kept);
    };
    let (rank, card) = ranks(col);
    let key = |code: u32| -> usize {
        let r = rank.get(code as usize).copied().unwrap_or(0) as usize;
        if desc {
            card.saturating_sub(1).saturating_sub(r)
        } else {
            r
        }
    };
    // how many rows each rank has, then where each rank's rows start
    let mut starts = vec![0usize; card.saturating_add(1)];
    let mut kept = 0usize;
    scan(Some(col), filters, total, started, |_, code| {
        if let Some(c) = starts.get_mut(key(code)) {
            *c = c.saturating_add(1);
        }
        kept = kept.saturating_add(1);
    })?;
    let mut sum = 0usize;
    for s in starts.iter_mut() {
        let c = *s;
        *s = sum;
        sum = sum.saturating_add(c);
    }
    let mut out = vec![0u32; kept];
    scan(Some(col), filters, total, started, |row, code| {
        if let Some(s) = starts.get_mut(key(code)) {
            if let Some(slot) = out.get_mut(*s) {
                *slot = row;
            }
            *s = s.saturating_add(1);
        }
    })?;
    Ok(out)
}

fn column_meta(c: &bicraft_model::Column, data: &Data) -> Value {
    let mut o = Map::new();
    o.insert("name".into(), json!(c.name));
    o.insert(
        "data_type".into(),
        json!(c.data_type.as_tmsl().unwrap_or("unknown")),
    );
    o.insert(
        "kind".into(),
        json!(if c.expression.is_some() {
            "calculated"
        } else {
            "data"
        }),
    );
    o.insert("format_string".into(), json!(c.format_string));
    o.insert("summarize_by".into(), json!(c.summarize_by));
    o.insert("data_category".into(), json!(c.data_category));
    o.insert("sort_by_column".into(), json!(c.sort_by_column));
    o.insert("hidden".into(), json!(c.is_hidden));
    match data {
        Data::Missing(r) => {
            o.insert("available".into(), json!(false));
            o.insert("reason".into(), json!(r));
        }
        Data::Calc(_) => {
            o.insert("available".into(), json!(true));
            o.insert("computed".into(), json!(true));
        }
        Data::Stored(_) => {
            o.insert("available".into(), json!(true));
        }
    }
    Value::Object(o)
}

/// What the data and the model the kept orders were computed from are.
fn stamp(ctx: &mut Ctx<'_>, s: &Session) -> String {
    let h = ctx.st.model_hash(s);
    format!("{}|{h}", ctx.st.generation)
}

/// `table.rows`; see the module docs.
pub(crate) fn table_rows(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &["table", "offset", "limit", "columns", "sort", "filter"])?;
    let started = Instant::now();
    let table = str_param(p, "table")?;
    let offset = uint(p, "offset", 0, MAX_TABLE_ROWS, 0)?;
    let limit = uint(p, "limit", DEFAULT_LIMIT, MAX_LIMIT, 1)?;
    let wanted: Option<Vec<String>> = match p.get("columns") {
        None | Some(Value::Null) => None,
        Some(Value::Array(a)) if a.len() <= MAX_COLUMNS => Some(
            a.iter()
                .map(|v| {
                    v.as_str()
                        .map(str::to_string)
                        .ok_or("`columns` must be a list of column names")
                })
                .collect::<Result<Vec<_>, _>>()?,
        ),
        Some(Value::Array(_)) => return Err(format!("`columns` lists more than {MAX_COLUMNS} names")),
        Some(_) => return Err("`columns` must be a list of column names".into()),
    };
    let sort = parse_sort(p.get("sort"))?;
    let conds = parse_filter(p.get("filter"))?;
    let model = s.model.as_ref().ok_or("no model is open (file.open first)")?;
    let t = model_table(model, table).ok_or_else(|| format!("the model has no table `{table}`"))?;
    let find = |name: &str| {
        t.columns
            .iter()
            .filter(|c| !c.is_row_number())
            .find(|c| c.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| format!("table `{}` has no column `{name}`", t.name))
    };
    let cols: Vec<&bicraft_model::Column> = match &wanted {
        None => t.columns.iter().filter(|c| !c.is_row_number()).collect(),
        Some(names) => names.iter().map(|n| find(n)).collect::<Result<_, _>>()?,
    };
    let sort_col = sort.as_ref().map(|(c, d)| find(c).map(|c| (c, *d))).transpose()?;
    let cond_cols: Vec<&bicraft_model::Column> =
        conds.iter().map(|c| find(&c.column)).collect::<Result<_, _>>()?;

    let limits = PreviewLimits {
        rows_per_visual: 0,
        table_rows: MAX_TABLE_ROWS,
        fallback_rows: MAX_TABLE_ROWS,
        budget: BUDGET,
        now: filters::clock(s),
        utc_offset: filters::utc_offset(s),
    };
    let mut st = s.preview.acquire(s);
    let mut ctx = Ctx {
        s,
        st: &mut st,
        model,
        limits: &limits,
    };
    let unavailable = |reason: String, cols: &[&bicraft_model::Column]| {
        let meta: Vec<Value> = cols
            .iter()
            .map(|c| column_meta(c, &Data::Missing(reason.clone())))
            .collect();
        json!({"table": t.name, "available": false, "reason": reason, "columns": meta,
               "rows": [], "offset": offset, "limit": limit, "row_count": 0, "total_rows": 0})
    };
    let total = match ctx.rows(&t.name) {
        Ok(n) => n,
        Err(Miss::Unavailable(r)) => return Ok(unavailable(r, &cols)),
        Err(Miss::Error(e)) => return Err(e),
    };
    if total > MAX_TABLE_ROWS {
        return Err(format!("table '{}' has more than {MAX_TABLE_ROWS} rows", t.name));
    }

    // the row order, when the view sorts or filters
    let mut cached = false;
    let ordered: Option<Arc<Vec<u32>>> = if sort_col.is_none() && conds.is_empty() {
        None
    } else {
        let stamp = stamp(&mut ctx, s);
        let spec = json!({
            "sort": sort_col.map(|(c, d)| json!([c.name.to_lowercase(), d])),
            "filter": conds.iter().zip(&cond_cols).map(|(c, col)| {
                let alts: Vec<Value> =
                    c.alts.iter().map(|a| json!([format!("{:?}", a.op), a.values])).collect();
                json!([col.name.to_lowercase(), alts])
            }).collect::<Vec<_>>(),
        });
        let key = format!("{stamp}|{}|{spec}", t.name.to_lowercase());
        if let Some(o) = ctx.st.rows_kept.order(&key) {
            cached = true;
            Some(o)
        } else {
            let sort_data = match sort_col {
                Some((c, d)) => Some((full_data(&mut ctx, t, c, total, &stamp)?, d)),
                None => None,
            };
            // one mask per filtered column (its conditions together)
            let mut masks: Vec<(Arc<ColData>, Vec<bool>)> = Vec::new();
            let mut done: Vec<String> = Vec::new();
            for col in &cond_cols {
                let lower = col.name.to_lowercase();
                if done.contains(&lower) {
                    continue;
                }
                let here: Vec<&Cond> = conds
                    .iter()
                    .filter(|c| c.column.eq_ignore_ascii_case(&col.name))
                    .collect();
                let data = full_data(&mut ctx, t, col, total, &stamp)?;
                let m = mask(&data, &here);
                masks.push((data, m));
                done.push(lower);
            }
            // the order itself, the codes of a block per column and the rank counts
            let need = total
                .saturating_mul(4)
                .saturating_add(CHUNK.saturating_mul(4).saturating_mul(masks.len() + 1));
            let work = reserve(&ctx, need, "ordering the rows")?;
            let o = order(sort_data.as_ref().map(|(c, d)| (c.as_ref(), *d)), &masks, total)?;
            drop(work);
            let keep = reserve(&ctx, o.len().saturating_mul(4), "keeping the row order")?;
            let o = Arc::new(o);
            ctx.st.rows_kept.keep_order(key, Arc::clone(&o), keep);
            Some(o)
        }
    };
    let row_count = ordered.as_ref().map_or(total, |o| o.len());
    let page: Vec<u32> = match &ordered {
        Some(o) => o
            .get(offset.min(o.len())..offset.saturating_add(limit).min(o.len()))
            .unwrap_or(&[])
            .to_vec(),
        None => {
            let end = offset.saturating_add(limit).min(total);
            (offset.min(end)..end)
                .filter_map(|r| u32::try_from(r).ok())
                .collect()
        }
    };

    // the cells
    let mut meta = Vec::with_capacity(cols.len());
    let mut cells: Vec<Vec<Value>> = vec![Vec::with_capacity(cols.len()); page.len()];
    let stamp = stamp(&mut ctx, s);
    for c in &cols {
        let data = data_of(&mut ctx, t, c)?;
        // a calculated column computed for a sort or a filter is read from what was kept
        let data = match data {
            Data::Calc(expr) => {
                let key = format!(
                    "{stamp}|{}|{}|{expr}",
                    t.name.to_lowercase(),
                    c.name.to_lowercase()
                );
                match ctx.st.rows_kept.calc(&key) {
                    Some(d) => Data::Stored(d),
                    None => Data::Calc(expr),
                }
            }
            d => d,
        };
        let values: Vec<Value> = match &data {
            Data::Stored(d) => page.iter().map(|r| cell_json(value_at(d, *r))).collect(),
            Data::Calc(expr) => {
                match super::general::rows_calc::calc_rows(&mut ctx, &t.name, expr, &page, BUDGET) {
                    Ok(v) => super::general::rows_calc::as_declared(v, c.data_type)
                        .iter()
                        .map(cell_json)
                        .collect(),
                    Err(e) => {
                        meta.push(column_meta(c, &Data::Missing(e)));
                        for row in cells.iter_mut() {
                            row.push(Value::Null);
                        }
                        continue;
                    }
                }
            }
            Data::Missing(_) => vec![Value::Null; page.len()],
        };
        meta.push(column_meta(c, &data));
        for (row, v) in cells.iter_mut().zip(values) {
            row.push(v);
        }
    }
    let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    Ok(json!({
        "table": t.name,
        "available": true,
        "columns": meta,
        "rows": cells,
        "offset": offset,
        "limit": limit,
        "row_count": row_count,
        "total_rows": total,
        "sort": sort_col.map(|(c, d)| json!({"column": c.name, "descending": d})),
        "filtered": !conds.is_empty(),
        "order_cached": cached,
        "elapsed_ms": elapsed,
    }))
}

#[cfg(test)]
#[path = "preview_rows_tests.rs"]
mod tests;
