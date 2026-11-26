//! `table.set {data_category, key_column}`: a table's data category, and "Mark as date table".
//!
//! Power BI Desktop marks a date table by giving the table the data category `Time` and its date
//! column `isKey`. The date column must be a date/time column whose values are unique, never
//! blank, contiguous (one row per day, no day missing) and all at the same time of day; Desktop
//! checks this against the data before it marks the table. bicraft makes the same checks when
//! the session has the column's data (stored PBIX / `cache.abf` data, or literal partitions) and
//! says so in `data_checked`; without data the table is marked unchecked, with a note.
//!
//! Leaving the `Time` category clears the key column (Desktop's "Mark as date table" off).

use bicraft_model::{DataType, SemanticModel, Table};
use bicraft_vertipaq::CellValue;
use serde_json::Value;

use crate::Session;
use crate::commands::model;
use crate::edit::{dax_column, table_index};

/// The `Time` data category: a date table.
pub(crate) const TIME: &str = "Time";
/// Rows read for the date checks.
const MAX_CHECK_ROWS: usize = 5_000_000;

/// What `table.set` does to the data category and the key column, decided (and checked against
/// the data) before the edit.
#[derive(Debug, Default)]
pub(crate) struct Plan {
    /// `Some(None)`: clear the category.
    pub data_category: Option<Option<String>>,
    /// The key column, by its model spelling (`Some(None)`: clear the key).
    pub key_column: Option<Option<String>>,
    pub notes: Vec<String>,
    /// The column checked against data (`None`: not checked).
    pub data_checked: Option<String>,
}

fn is_key(c: &bicraft_model::Column) -> bool {
    c.extra.get("isKey").and_then(Value::as_bool) == Some(true)
}

/// The table's data category (`dataCategory`, kept in `extra`).
pub(crate) fn data_category(t: &Table) -> Option<&str> {
    t.extra.get("dataCategory").and_then(Value::as_str)
}

/// Reads `data_category` / `key_column` of `table.set` and checks them against the model and the
/// data. Neither present: an empty plan.
pub(crate) fn plan(s: &Session, p: &Value, table: &str) -> Result<Plan, String> {
    let category = crate::params::change_str(p, "data_category")?;
    let key = crate::params::change_str(p, "key_column")?;
    let mut plan = Plan::default();
    if category.is_none() && key.is_none() {
        return Ok(plan);
    }
    let m = model(s)?;
    let t = m
        .tables
        .get(table_index(m, table)?)
        .ok_or("internal: table index out of range")?;
    if let Some(Some(c)) = &category
        && c.chars().any(char::is_control)
    {
        return Err("`data_category` must not contain control characters".into());
    }
    // The category as it will be.
    let next = match &category {
        Some(c) => c.clone(),
        None => data_category(t).map(str::to_string),
    };
    let time = next.as_deref().is_some_and(|c| c.eq_ignore_ascii_case(TIME));
    let key_name = match &key {
        Some(Some(k)) => {
            let c = t
                .columns
                .iter()
                .find(|c| !c.is_row_number() && c.name.eq_ignore_ascii_case(k))
                .ok_or_else(|| format!("table `{}` has no column `{k}`", t.name))?;
            Some(Some(c.name.clone()))
        }
        Some(None) => Some(None),
        None => None,
    };
    let key_after: Option<String> = match &key_name {
        Some(k) => k.clone(),
        None if time => t.columns.iter().find(|c| is_key(c)).map(|c| c.name.clone()),
        None => None,
    };
    if time {
        let Some(k) = &key_after else {
            return Err(format!(
                "a date table needs its date column: pass `key_column` (the column of `{}` holding one row per day)",
                t.name
            ));
        };
        let c = t
            .columns
            .iter()
            .find(|c| c.name == *k)
            .ok_or("internal: key column lost")?;
        if c.data_type != DataType::DateTime {
            return Err(format!(
                "{} is not a date column (data type {}); a date table is marked by a date/time column",
                dax_column(&t.name, &c.name),
                c.data_type.as_tmsl().unwrap_or("unknown")
            ));
        }
        match crate::preview::column_values(s, &t.name, &c.name, MAX_CHECK_ROWS) {
            Ok(Some(values)) => {
                check_dates(&values).map_err(|e| {
                    format!(
                        "{} cannot mark `{}` as a date table: {e}",
                        dax_column(&t.name, &c.name),
                        t.name
                    )
                })?;
                plan.data_checked = Some(dax_column(&t.name, &c.name));
            }
            Ok(None) => plan.notes.push(format!(
                "the values of {} were not checked (no data in the session): Power BI Desktop needs them \
                 unique, not blank, one per day without gaps and at the same time of day",
                dax_column(&t.name, &c.name)
            )),
            Err(e) => plan.notes.push(format!(
                "the values of {} could not be read to check them: {e}",
                dax_column(&t.name, &c.name)
            )),
        }
        // Desktop's other date columns of the table lose nothing; a second key is cleared below.
        plan.key_column = Some(Some(k.clone()));
    } else {
        let was_time = data_category(t).is_some_and(|c| c.eq_ignore_ascii_case(TIME));
        plan.key_column = match key_name {
            Some(k) => Some(k),
            // Leaving the date-table category clears the date key, as Desktop's "Mark as date
            // table" off does.
            None if was_time && category.is_some() => Some(None),
            None => None,
        };
    }
    plan.data_category = category;
    Ok(plan)
}

/// Applies a [`plan`] to the table (inside the edit).
pub(crate) fn apply(m: &mut SemanticModel, table: &str, plan: &Plan) -> Result<(), String> {
    let i = table_index(m, table)?;
    let t = m.tables.get_mut(i).ok_or("internal: table index out of range")?;
    if let Some(c) = &plan.data_category {
        match c {
            Some(c) => {
                let spelled = if c.eq_ignore_ascii_case(TIME) {
                    TIME.to_string()
                } else {
                    c.clone()
                };
                t.extra.insert("dataCategory".into(), Value::String(spelled));
            }
            None => {
                t.extra.remove("dataCategory");
            }
        }
    }
    if let Some(k) = &plan.key_column {
        for c in &mut t.columns {
            if c.is_row_number() {
                // The engine's row-number column is the key while no user column is
                // (engine-observed: `IsKey` 0 on it once a date column is the key).
                if k.is_some() {
                    c.extra.remove("isKey");
                } else {
                    c.extra.insert("isKey".into(), Value::Bool(true));
                }
                continue;
            }
            let key = k.as_deref().is_some_and(|k| c.name == k);
            if key {
                c.extra.insert("isKey".into(), Value::Bool(true));
            } else {
                c.extra.remove("isKey");
            }
        }
    }
    Ok(())
}

/// Days since 1899-12-30 and the time of day (fraction), or why the value is not a date.
fn day_of(v: &CellValue) -> Result<(i64, f64), String> {
    let x = match v {
        CellValue::DateTime(x) | CellValue::Real(x) => *x,
        CellValue::Int(n) => *n as f64,
        CellValue::Null => return Err("it has a blank value".into()),
        _ => return Err("it holds a value that is not a date".into()),
    };
    if !x.is_finite() {
        return Err("it holds a value that is not a date".into());
    }
    let day = x.floor();
    Ok((day as i64, x - day))
}

/// Desktop's date-table rules: unique, not blank, contiguous days, one time of day.
pub(crate) fn check_dates(values: &[CellValue]) -> Result<(), String> {
    if values.is_empty() {
        return Err("the table has no rows".into());
    }
    let mut days = Vec::with_capacity(values.len());
    let mut time: Option<f64> = None;
    for v in values {
        let (d, t) = day_of(v)?;
        match time {
            None => time = Some(t),
            Some(first) if (first - t).abs() > 1e-9 => {
                return Err("its values are not all at the same time of day".into());
            }
            Some(_) => {}
        }
        days.push(d);
    }
    days.sort_unstable();
    for w in days.windows(2) {
        if let [a, b] = w {
            if a == b {
                return Err("it has duplicate dates".into());
            }
            if b.saturating_sub(*a) != 1 {
                return Err("its dates are not contiguous (a day is missing)".into());
            }
        }
    }
    Ok(())
}
