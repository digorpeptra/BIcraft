//! The `query.apply_step` kinds of the editor's row, column, text, number and date transforms
//! (`bicraft_data::m::transform_more`), read from `{kind, …}`, and the date literals of filters
//! (`{"date": "2024-01-31"}`, `{"datetime": "2024-01-31T10:30:00"}`).

use bicraft_data::m::transform::{FilterOp, Literal};
use bicraft_data::m::transform_more::{ColumnOp, MAX_ROWS, MAX_RULES, More, OpArgs, Rule};
use serde_json::Value;

use crate::params::{allow, opt_bool, opt_str, str_param};

/// The kinds read here.
pub(crate) const KINDS: [&str; 21] = [
    "remove_top_rows",
    "remove_bottom_rows",
    "keep_top_rows",
    "keep_bottom_rows",
    "keep_range",
    "remove_alternate_rows",
    "remove_blank_rows",
    "remove_errors",
    "keep_errors",
    "replace_errors",
    "fill_down",
    "fill_up",
    "reverse_rows",
    "transpose",
    "demote_headers",
    "reorder_columns",
    "duplicate_column",
    "add_index",
    "merge_columns",
    "conditional_column",
    "column_op",
];

fn digits(s: &str, at: std::ops::Range<usize>) -> Option<u32> {
    let d = s.get(at)?;
    if d.bytes().all(|b| b.is_ascii_digit()) {
        d.parse().ok()
    } else {
        None
    }
}

/// `YYYY-MM-DD` (and `THH:MM:SS` when `time`), checked as a calendar date and clock time.
fn parse_date(s: &str, time: bool) -> Option<(i32, u32, u32, u32, u32, u32)> {
    let s = s.trim();
    let (date, clock) = match s.split_once(['T', ' ']) {
        Some((d, c)) => (d, Some(c)),
        None => (s, None),
    };
    if date.len() != 10 || date.as_bytes().get(4) != Some(&b'-') || date.as_bytes().get(7) != Some(&b'-') {
        return None;
    }
    let y = i32::try_from(digits(date, 0..4)?).ok()?;
    let m = digits(date, 5..7)?;
    let d = digits(date, 8..10)?;
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    if y < 1 || d < 1 || d > days {
        return None;
    }
    let (h, mi, sec) = match clock {
        None => (0, 0, 0),
        Some(_) if !time => return None,
        Some(c) => {
            let c = c.split('.').next().unwrap_or(c);
            if c.len() != 8 || c.as_bytes().get(2) != Some(&b':') || c.as_bytes().get(5) != Some(&b':') {
                return None;
            }
            let (h, mi, sec) = (digits(c, 0..2)?, digits(c, 3..5)?, digits(c, 6..8)?);
            if h > 23 || mi > 59 || sec > 59 {
                return None;
            }
            (h, mi, sec)
        }
    };
    Some((y, m, d, h, mi, sec))
}

/// A `{"date": …}` or `{"datetime": …}` literal.
pub(crate) fn date_literal(v: &Value, key: &str) -> Result<Literal, String> {
    let bad = || {
        format!(
            "`{key}` must be text, a number, true/false, null, {{\"date\": \"YYYY-MM-DD\"}} or {{\"datetime\": \"YYYY-MM-DDTHH:MM:SS\"}}"
        )
    };
    let obj = v.as_object().filter(|o| o.len() == 1).ok_or_else(bad)?;
    if let Some(d) = obj.get("date") {
        let (y, m, d, ..) = d.as_str().and_then(|s| parse_date(s, false)).ok_or_else(bad)?;
        return Ok(Literal::Date(y, m, d));
    }
    if let Some(d) = obj.get("datetime") {
        let (y, m, d, h, mi, s) = d.as_str().and_then(|s| parse_date(s, true)).ok_or_else(bad)?;
        return Ok(Literal::DateTime(y, m, d, h, mi, s));
    }
    Err(bad())
}

fn count(v: &Value, key: &str, default: Option<u64>, min: u64) -> Result<u64, String> {
    match v.get(key) {
        None | Some(Value::Null) => default.ok_or_else(|| format!("`{key}` is required")),
        Some(x) => x
            .as_u64()
            .filter(|n| (min..=MAX_ROWS).contains(n))
            .ok_or_else(|| format!("`{key}` must be a whole number from {min} to {MAX_ROWS}")),
    }
}

fn int(v: &Value, key: &str, default: i64) -> Result<i64, String> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(x) => x
            .as_i64()
            .filter(|n| n.unsigned_abs() <= MAX_ROWS)
            .ok_or_else(|| format!("`{key}` must be a whole number from -{MAX_ROWS} to {MAX_ROWS}")),
    }
}

fn names(v: &Value, key: &str) -> Result<Vec<String>, String> {
    super::query_steps::list(v, key)
}

/// The transform of `{kind, …}` when `kind` is one of [`KINDS`].
pub(crate) fn more(v: &Value, kind: &str) -> Option<Result<More, String>> {
    if !KINDS.contains(&kind) {
        return None;
    }
    Some(read(v, kind))
}

fn read(v: &Value, kind: &str) -> Result<More, String> {
    let keys: &[&str] = match kind {
        "remove_top_rows" | "remove_bottom_rows" | "keep_top_rows" | "keep_bottom_rows" => &["kind", "count"],
        "keep_range" => &["kind", "first", "count"],
        "remove_alternate_rows" => &["kind", "first", "remove", "keep"],
        "remove_blank_rows" | "reverse_rows" | "transpose" | "demote_headers" => &["kind"],
        "remove_errors" | "keep_errors" | "fill_down" | "fill_up" | "reorder_columns" => &["kind", "columns"],
        "replace_errors" => &["kind", "columns", "value"],
        "duplicate_column" => &["kind", "column", "name"],
        "add_index" => &["kind", "name", "start", "step"],
        "merge_columns" => &["kind", "columns", "delimiter", "name", "add"],
        "conditional_column" => &["kind", "name", "rules", "otherwise"],
        _ => &[
            "kind", "op", "columns", "add", "name", "text", "end", "count", "start", "number",
        ],
    };
    allow(v, keys)?;
    Ok(match kind {
        "remove_top_rows" => More::RemoveTopRows(count(v, "count", None, 1)?),
        "remove_bottom_rows" => More::RemoveBottomRows(count(v, "count", None, 1)?),
        "keep_top_rows" => More::KeepTopRows(count(v, "count", None, 1)?),
        "keep_bottom_rows" => More::KeepBottomRows(count(v, "count", None, 1)?),
        "keep_range" => More::KeepRange {
            first: count(v, "first", Some(1), 1)?,
            count: count(v, "count", None, 1)?,
        },
        "remove_alternate_rows" => More::RemoveAlternateRows {
            first: count(v, "first", Some(1), 1)?,
            remove: count(v, "remove", Some(1), 1)?,
            keep: count(v, "keep", Some(1), 0)?,
        },
        "remove_blank_rows" => More::RemoveBlankRows,
        "remove_errors" => More::RemoveErrors(names(v, "columns")?),
        "keep_errors" => More::KeepErrors(names(v, "columns")?),
        "replace_errors" => More::ReplaceErrors {
            columns: names(v, "columns")?,
            value: super::query_steps::literal(v.get("value"), "value")?,
        },
        "fill_down" => More::FillDown(names(v, "columns")?),
        "fill_up" => More::FillUp(names(v, "columns")?),
        "reverse_rows" => More::ReverseRows,
        "transpose" => More::Transpose,
        "demote_headers" => More::DemoteHeaders,
        "reorder_columns" => More::ReorderColumns(names(v, "columns")?),
        "duplicate_column" => {
            let column = str_param(v, "column")?.to_string();
            More::DuplicateColumn {
                name: opt_str(v, "name")?.map_or_else(|| format!("{column} - Copy"), str::to_string),
                column,
            }
        }
        "add_index" => More::AddIndex {
            name: opt_str(v, "name")?.unwrap_or("Index").to_string(),
            start: int(v, "start", 0)?,
            step: int(v, "step", 1)?,
        },
        "merge_columns" => More::MergeColumns {
            columns: names(v, "columns")?,
            delimiter: opt_str(v, "delimiter")?.unwrap_or("").to_string(),
            name: opt_str(v, "name")?.unwrap_or("Merged").to_string(),
            add: opt_bool(v, "add")?.unwrap_or(false),
        },
        "conditional_column" => {
            let rules = match v.get("rules") {
                Some(Value::Array(a)) if !a.is_empty() && a.len() <= MAX_RULES => a,
                _ => {
                    return Err(format!(
                        "`rules` must be a list of 1 to {MAX_RULES} {{column, op, value, output}}"
                    ));
                }
            };
            let mut out = Vec::with_capacity(rules.len());
            for (i, r) in rules.iter().enumerate() {
                let at = |e: String| format!("rules[{i}]: {e}");
                allow(r, &["column", "op", "value", "output"]).map_err(at)?;
                let op = str_param(r, "op").map_err(at)?;
                let op = FilterOp::from_name(op).ok_or_else(|| {
                    at(format!(
                        "unknown `op` `{op}` (expected {})",
                        FilterOp::ALL.map(FilterOp::name).join(", ")
                    ))
                })?;
                out.push(Rule {
                    column: str_param(r, "column").map_err(at)?.to_string(),
                    op,
                    value: super::query_steps::literal(r.get("value"), "value").map_err(at)?,
                    output: super::query_steps::literal(r.get("output"), "output").map_err(at)?,
                });
            }
            More::ConditionalColumn {
                name: opt_str(v, "name")?.unwrap_or("Custom").to_string(),
                rules: out,
                otherwise: super::query_steps::literal(v.get("otherwise"), "otherwise")?,
            }
        }
        _ => {
            let number = match v.get("number") {
                None | Some(Value::Null) => None,
                Some(x) => Some(
                    x.as_f64()
                        .filter(|f| f.is_finite())
                        .ok_or("`number` must be a number")?,
                ),
            };
            let args = OpArgs {
                text: opt_str(v, "text")?.map(str::to_string),
                end: opt_str(v, "end")?.map(str::to_string),
                count: match v.get("count") {
                    None | Some(Value::Null) => None,
                    Some(_) => Some(count(v, "count", None, 0)?),
                },
                start: match v.get("start") {
                    None | Some(Value::Null) => None,
                    Some(_) => Some(count(v, "start", None, 0)?),
                },
                number,
            };
            More::ColumnOp {
                op: ColumnOp::from_name(str_param(v, "op")?, &args)?,
                columns: names(v, "columns")?,
                add: opt_bool(v, "add")?.unwrap_or(false),
                name: opt_str(v, "name")?.map(str::to_string),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn dates_are_checked() {
        assert_eq!(
            date_literal(&json!({"date": "2024-02-29"}), "v"),
            Ok(Literal::Date(2024, 2, 29))
        );
        assert!(date_literal(&json!({"date": "2023-02-29"}), "v").is_err());
        assert!(date_literal(&json!({"date": "2024-13-01"}), "v").is_err());
        assert!(date_literal(&json!({"date": "2024-01-01T10:00:00"}), "v").is_err());
        assert_eq!(
            date_literal(&json!({"datetime": "2024-01-01T10:20:30.5"}), "v"),
            Ok(Literal::DateTime(2024, 1, 1, 10, 20, 30))
        );
        assert_eq!(
            date_literal(&json!({"datetime": "2024-01-01"}), "v"),
            Ok(Literal::DateTime(2024, 1, 1, 0, 0, 0))
        );
        assert!(date_literal(&json!({"datetime": "2024-01-01T24:00:00"}), "v").is_err());
        assert!(date_literal(&json!({"date": "2024-01-01", "x": 1}), "v").is_err());
        assert!(date_literal(&json!({"date": "２０２４-01-01"}), "v").is_err());
    }
}
