//! `query.profile`: the Power Query editor's data profiling (View › Column quality, Column
//! distribution, Column profile; docs/mcp.md "Power Query editor"), as Microsoft Learn's "Using
//! the data profiling tools" describes it:
//!
//! - column quality: the share of valid, error and empty values of each column;
//! - column distribution: each column's distinct and unique counts and its most frequent values
//!   (the editor's small histogram);
//! - column profile: one column's statistics and value distribution.
//!
//! Profiling reads the query's first 1000 rows, or the whole result with `full` ("Column
//! profiling based on the entire data set"). The query is evaluated by BIcraft's M evaluator as
//! `query.preview` evaluates it (the same sources, connectors and roles); nothing is loaded or
//! changed. Error values are counted per column and profiled as errors, not as blanks.

use std::collections::HashMap;

use bicraft_data::ColumnData;
use bicraft_vertipaq::CellValue;
use serde_json::{Value, json};

use crate::Session;
use crate::params::{allow, opt_bool, opt_str};

/// Rows profiled by default (the editor's "top 1000 rows").
pub const TOP_ROWS: usize = 1_000;
/// Values listed in each column's distribution.
const DISTRIBUTION: usize = 30;
/// Values listed in a column profile's value distribution.
const PROFILE_VALUES: usize = 100;

/// `query.profile`; see the module docs.
pub(crate) fn query_profile(s: &Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "name",
            "partition",
            "expression",
            "step",
            "column",
            "full",
            "data_root",
            "allow_connectors",
            "timeout_ms",
            "queries",
        ],
    )?;
    let full = opt_bool(p, "full")?.unwrap_or(false);
    let column = opt_str(p, "column")?;
    let ev = crate::query_preview::evaluate(s, p, if full { None } else { Some(TOP_ROWS) }, true)?;
    let cols = ev.data.columns();
    let total = cols.first().map_or(0, |c| c.values.len());
    let rows = if full { total } else { total.min(TOP_ROWS) };
    let profiled: Vec<Profiled> = cols
        .iter()
        .enumerate()
        .map(|(i, c)| Profiled::of(c, rows, ev.errors.get(i).copied().unwrap_or(0)))
        .collect();
    let columns: Vec<Value> = profiled.iter().map(Profiled::summary).collect();
    let one = match column {
        None => Value::Null,
        Some(n) => {
            let at = cols
                .iter()
                .position(|c| c.name == n)
                .or_else(|| cols.iter().position(|c| c.name.eq_ignore_ascii_case(n)))
                .ok_or_else(|| format!("the query has no column `{n}` at this step"))?;
            match (cols.get(at), profiled.get(at)) {
                (Some(c), Some(pr)) => pr.profile(c, rows),
                _ => return Err(format!("the query has no column `{n}` at this step")),
            }
        }
    };
    Ok(json!({
        "name": ev.name,
        "step": ev.step,
        "source": ev.kind,
        "full": full,
        "rows": rows,
        "truncated": !full && total > TOP_ROWS,
        "columns": columns,
        "column": one,
        "connectors": ev.connectors,
        "elapsed_ms": u64::try_from(ev.started.elapsed().as_millis()).unwrap_or(u64::MAX),
    }))
}

/// A column's counts.
struct Profiled {
    name: String,
    data_type: &'static str,
    count: usize,
    errors: usize,
    empty: usize,
    distinct: usize,
    unique: usize,
    /// (value, count), most frequent first; ties in the order they first appear.
    values: Vec<(Value, usize)>,
}

fn is_empty(v: &CellValue) -> bool {
    match v {
        CellValue::Null => true,
        CellValue::Text(t) => t.is_empty(),
        _ => false,
    }
}

fn share(n: usize, of: usize) -> f64 {
    if of == 0 {
        0.0
    } else {
        // a share to the tenth of a percent, as the editor shows it
        ((n as f64 / of as f64) * 1000.0).round() / 1000.0
    }
}

impl Profiled {
    fn of(c: &ColumnData, rows: usize, errors: usize) -> Self {
        let cells = c.values.get(..rows.min(c.values.len())).unwrap_or_default();
        let count = cells.len();
        let errors = errors.min(count);
        // errors were loaded as blanks: they are not empty values
        let nulls = cells.iter().filter(|v| is_empty(v)).count();
        let empty = nulls.saturating_sub(errors);
        let mut seen: HashMap<String, usize> = HashMap::new();
        let mut values: Vec<(Value, usize)> = Vec::new();
        let mut blanks_left = errors;
        for v in cells {
            // the blanks the errors became are left out of the values (the first ones: the
            // evaluator does not say which)
            if matches!(v, CellValue::Null) && blanks_left > 0 {
                blanks_left = blanks_left.saturating_sub(1);
                continue;
            }
            let j = crate::preview::cell_json(v);
            let key = j.to_string();
            match seen.get(&key) {
                Some(&i) => {
                    if let Some(x) = values.get_mut(i) {
                        x.1 = x.1.saturating_add(1);
                    }
                }
                None => {
                    seen.insert(key, values.len());
                    values.push((j, 1));
                }
            }
        }
        let distinct = values.len();
        let unique = values.iter().filter(|x| x.1 == 1).count();
        // a stable sort keeps the first-seen order among ties
        values.sort_by_key(|x| std::cmp::Reverse(x.1));
        Self {
            name: c.name.clone(),
            data_type: c.data_type.as_tmsl().unwrap_or("unknown"),
            count,
            errors,
            empty,
            distinct,
            unique,
            values,
        }
    }

    fn valid(&self) -> usize {
        self.count.saturating_sub(self.errors).saturating_sub(self.empty)
    }

    fn values_json(&self, n: usize) -> Value {
        json!(
            self.values
                .iter()
                .take(n)
                .map(|(v, k)| json!({"value": v, "count": k}))
                .collect::<Vec<_>>()
        )
    }

    fn summary(&self) -> Value {
        json!({
            "name": self.name,
            "data_type": self.data_type,
            "count": self.count,
            "valid": self.valid(),
            "errors": self.errors,
            "empty": self.empty,
            "valid_share": share(self.valid(), self.count),
            "error_share": share(self.errors, self.count),
            "empty_share": share(self.empty, self.count),
            "distinct": self.distinct,
            "unique": self.unique,
            "distribution": self.values_json(DISTRIBUTION),
        })
    }

    /// The column profile: statistics and value distribution.
    fn profile(&self, c: &ColumnData, rows: usize) -> Value {
        let cells = c.values.get(..rows.min(c.values.len())).unwrap_or_default();
        let mut stats = json!({
            "count": self.count,
            "errors": self.errors,
            "empty": self.empty,
            "distinct": self.distinct,
            "unique": self.unique,
        });
        let nums: Vec<f64> = cells
            .iter()
            .filter_map(|v| match v {
                CellValue::Int(n) => Some(*n as f64),
                CellValue::Real(x) => Some(*x),
                CellValue::Decimal(n) => Some(*n as f64 / 10_000.0),
                _ => None,
            })
            .filter(|x| x.is_finite())
            .collect();
        let numeric = matches!(c.data_type.as_tmsl(), Some("int64" | "double" | "decimal"));
        if numeric {
            let n = nums.len();
            let min = nums.iter().copied().fold(f64::INFINITY, f64::min);
            let max = nums.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let sum: f64 = nums.iter().sum();
            let mean = if n == 0 { 0.0 } else { sum / n as f64 };
            // the sample standard deviation
            let sd = if n < 2 {
                0.0
            } else {
                (nums.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1) as f64).sqrt()
            };
            let whole: Vec<f64> = nums.iter().copied().filter(|x| x.fract() == 0.0).collect();
            stats["min"] = if n == 0 { Value::Null } else { num(min) };
            stats["max"] = if n == 0 { Value::Null } else { num(max) };
            stats["average"] = if n == 0 { Value::Null } else { num(mean) };
            stats["standard_deviation"] = if n == 0 { Value::Null } else { num(sd) };
            stats["zero"] = json!(nums.iter().filter(|x| **x == 0.0).count());
            stats["even"] = json!(whole.iter().filter(|x| (*x % 2.0) == 0.0).count());
            stats["odd"] = json!(whole.iter().filter(|x| (*x % 2.0) != 0.0).count());
        } else {
            // text, dates and true/false: the smallest and largest values, ordinal
            let mut present = self
                .values
                .iter()
                .map(|x| &x.0)
                .filter(|v| !v.is_null() && v.as_str() != Some(""));
            let first = present.next().cloned();
            let (min, max) = present.fold((first.clone(), first), |(lo, hi), v| {
                let lo = match lo {
                    Some(l) if order(&l, v).is_le() => Some(l),
                    _ => Some(v.clone()),
                };
                let hi = match hi {
                    Some(h) if order(&h, v).is_ge() => Some(h),
                    _ => Some(v.clone()),
                };
                (lo, hi)
            });
            stats["min"] = min.unwrap_or(Value::Null);
            stats["max"] = max.unwrap_or(Value::Null);
            if c.data_type.as_tmsl() == Some("string") {
                stats["empty_string"] = json!(
                    cells
                        .iter()
                        .filter(|v| matches!(v, CellValue::Text(t) if t.is_empty()))
                        .count()
                );
            }
        }
        json!({
            "name": self.name,
            "data_type": self.data_type,
            "statistics": stats,
            "values": self.values_json(PROFILE_VALUES),
            "values_listed": self.values.len().min(PROFILE_VALUES),
        })
    }
}

fn num(x: f64) -> Value {
    serde_json::Number::from_f64(x).map_or(Value::Null, Value::Number)
}

/// Ordinal order of two profiled values of one column (text by code point, as M's `<`).
fn order(a: &Value, b: &Value) -> std::cmp::Ordering {
    match (a, b) {
        (Value::String(x), Value::String(y)) => x.cmp(y),
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        (Value::Number(x), Value::Number(y)) => x
            .as_f64()
            .partial_cmp(&y.as_f64())
            .unwrap_or(std::cmp::Ordering::Equal),
        _ => a.to_string().cmp(&b.to_string()),
    }
}
