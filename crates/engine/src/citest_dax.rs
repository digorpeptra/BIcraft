//! DAX assertions of `project.test`: each one runs as a `dax.evaluate` query over the open file's
//! data (bicraft's own evaluator, never Power BI Desktop) and its result is compared with the
//! expected value or table.
//!
//! - `measure` / `expression`: `EVALUATE ROW("Value", CALCULATE(<expr>, <filters>...))`;
//! - `query`: the first EVALUATE of the query.
//!
//! Comparison is typed: a number never equals a text, BLANK (`null`) never equals 0 or "". Numbers
//! match within `tolerance + relative_tolerance × |expected|`. A date column matches an expected
//! `YYYY-MM-DD` at midnight or the full ISO text. Data that cannot be read ("data not available":
//! no stored data, an M source bicraft cannot run) skips the assertion, or fails it with
//! `require_data: true`.

use std::cmp::Ordering;
use std::time::Instant;

use serde_json::{Value, json};

use crate::Session;
use crate::citest::{Case, Finding, Status, Target, elapsed};
use crate::citest_spec::{DaxTarget, DaxTest, Expected, TypeRule};

/// Most rows read back for a table assertion.
const MAX_ROWS: u64 = 100_000;
/// Most row differences listed per assertion.
const MAX_DIFFS: usize = 20;

pub(crate) fn run(s: &mut Session, tests: &[DaxTest]) -> Vec<Case> {
    tests.iter().map(|t| one(s, t)).collect()
}

/// `[Name]` for a bare measure name; a name already holding `[` is used as written.
fn measure_ref(m: &str) -> String {
    let m = m.trim();
    if m.contains('[') {
        m.to_string()
    } else {
        format!("[{}]", m.replace(']', "]]"))
    }
}

fn query_of(t: &DaxTest) -> String {
    let expr = match &t.target {
        DaxTarget::Query(q) => return q.clone(),
        DaxTarget::Measure(m) => measure_ref(m),
        DaxTarget::Expression(e) => e.clone(),
    };
    if t.filters.is_empty() {
        format!("EVALUATE ROW(\"Value\", {expr}\n)")
    } else {
        format!(
            "EVALUATE ROW(\"Value\", CALCULATE({expr}\n, {}\n))",
            t.filters.join("\n, ")
        )
    }
}

fn one(s: &mut Session, t: &DaxTest) -> Case {
    let kind = match t.target {
        DaxTarget::Measure(_) => "measure",
        DaxTarget::Expression(_) => "expression",
        DaxTarget::Query(_) => "query",
    };
    let mut case = Case::new("dax", t.id.clone(), t.name.clone(), kind).at(t.severity);
    let started = Instant::now();
    let query = query_of(t);
    let max_rows = match (&t.target, &t.expect) {
        (DaxTarget::Query(_), _) => MAX_ROWS,
        _ => 2,
    };
    let params = json!({"query": query, "max_rows": max_rows, "timeout_ms": t.timeout_ms});
    let result = crate::cache_abf::dax_evaluate(s, &params);
    case.elapsed_ms = elapsed(started);
    let here = Target::TestFile(t.name.clone());
    let v = match result {
        Ok(v) => v,
        Err(e) if e.contains(crate::preview::DATA_UNAVAILABLE) => {
            return if t.require_data {
                case.error(format!("required data is not loaded: {e}"))
            } else {
                case.skip(format!(
                    "skipped: the model's data is not loaded ({e}); set require_data: true to fail instead"
                ))
            };
        }
        Err(e) => {
            let mut c = case.error(e.clone());
            c.findings.push(Finding::new(t.severity, e, here));
            return c;
        }
    };
    let data_source = v.get("data_source").cloned().unwrap_or(Value::Null);
    let Some(result) = v.get("results").and_then(|r| r.get(0)) else {
        return case.error("the query returned no result");
    };
    let problems = check(t, result);
    let actual = summary(t, result);
    case.details = Some(json!({"query": query, "data_source": data_source, "actual": actual}));
    let findings: Vec<Finding> = problems
        .into_iter()
        .map(|m| Finding::new(t.severity, m, here.clone()))
        .collect();
    let mut c = case.conclude(findings, "differences");
    if c.status == Status::Passed {
        c.message = "as expected".into();
    }
    c
}

/// The actual value (scalar) or the first rows (table), for the report.
fn summary(t: &DaxTest, r: &Value) -> Value {
    match t.target {
        DaxTarget::Query(_) if !matches!(t.expect, Expected::Scalar(_)) => json!({
            "columns": r.get("columns").cloned().unwrap_or(Value::Null),
            "row_count": r.get("row_count").cloned().unwrap_or(Value::Null),
            "rows": r.get("rows").and_then(Value::as_array).map(|a| a.iter().take(MAX_DIFFS).cloned().collect::<Vec<_>>()),
        }),
        _ => first_cell(r).cloned().unwrap_or(Value::Null),
    }
}

fn columns(r: &Value) -> Vec<(String, String)> {
    r.get("columns")
        .and_then(Value::as_array)
        .map(|cols| {
            cols.iter()
                .map(|c| {
                    (
                        c.get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        c.get("type")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn rows(r: &Value) -> &[Value] {
    r.get("rows").and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

fn first_cell(r: &Value) -> Option<&Value> {
    let (name, _) = columns(r).into_iter().next()?;
    rows(r).first()?.get(&name)
}

/// Whether a returned column name matches an expected one: the same text (case-insensitive), or
/// the bracketed part (`Total` for `[Total]`, `Category` for `Product[Category]`).
fn column_matches(actual: &str, expected: &str) -> bool {
    if actual.eq_ignore_ascii_case(expected) {
        return true;
    }
    actual
        .rsplit_once('[')
        .and_then(|(_, rest)| rest.strip_suffix(']'))
        .is_some_and(|inner| inner.replace("]]", "]").eq_ignore_ascii_case(expected))
}

fn check(t: &DaxTest, r: &Value) -> Vec<String> {
    let cols = columns(r);
    let mut out = Vec::new();
    let scalar = !matches!(t.target, DaxTarget::Query(_)) || matches!(t.expect, Expected::Scalar(_));
    if scalar {
        let row_count = r.get("row_count").and_then(Value::as_u64).unwrap_or(0);
        if cols.len() != 1 || row_count != 1 {
            out.push(format!(
                "expected one value, the query returned {} column(s) and {row_count} row(s)",
                cols.len()
            ));
            return out;
        }
        let ty = cols.first().map(|c| c.1.as_str()).unwrap_or_default();
        let actual = first_cell(r).unwrap_or(&Value::Null);
        if let Some(rule) = t.type_rule
            && let Some(m) = type_problem(rule, ty, actual)
        {
            out.push(m);
        }
        if let Expected::Scalar(e) = &t.expect
            && !same(e, actual, ty, t)
        {
            out.push(format!("expected {}, got {}", show(e), show_typed(actual, ty)));
        }
        bounds(t, actual, &mut out);
        return out;
    }
    // a table
    if let Some(rule) = t.type_rule {
        for (name, ty) in &cols {
            for (i, row) in rows(r).iter().enumerate() {
                let v = row.get(name).unwrap_or(&Value::Null);
                if let Some(m) = type_problem(rule, ty, v) {
                    out.push(format!("row {}, {name}: {m}", i.saturating_add(1)));
                    break;
                }
            }
        }
    }
    let Expected::Table {
        columns: want_cols,
        rows: want_rows,
        row_count,
    } = &t.expect
    else {
        if t.min.is_some() || t.max.is_some() {
            out.push("min and max apply to a single value; this query returns a table".into());
        }
        return out;
    };
    if let Some(want) = want_cols {
        let names: Vec<&str> = cols.iter().map(|c| c.0.as_str()).collect();
        if want.len() != names.len() || !want.iter().zip(&names).all(|(w, a)| column_matches(a, w)) {
            out.push(format!("expected columns {}, got {}", json!(want), json!(names)));
            return out;
        }
    }
    let total = r.get("row_count").and_then(Value::as_u64).unwrap_or(0);
    if let Some(n) = row_count
        && *n != total
    {
        out.push(format!("expected {n} row(s), got {total}"));
    }
    let Some(want_rows) = want_rows else {
        return out;
    };
    if r.get("truncated").and_then(Value::as_bool) == Some(true) {
        out.push(format!(
            "the query returned more than {MAX_ROWS} rows; compare a smaller result"
        ));
        return out;
    }
    let got: Vec<Vec<Value>> = rows(r)
        .iter()
        .map(|row| {
            cols.iter()
                .map(|(n, _)| row.get(n).cloned().unwrap_or(Value::Null))
                .collect()
        })
        .collect();
    let mut want: Vec<Vec<Value>> = Vec::with_capacity(want_rows.len());
    for (i, w) in want_rows.iter().enumerate() {
        match w {
            Value::Array(cells) => {
                if cells.len() != cols.len() {
                    out.push(format!(
                        "expected row {} has {} value(s), the query returns {} column(s)",
                        i.saturating_add(1),
                        cells.len(),
                        cols.len()
                    ));
                    return out;
                }
                want.push(cells.clone());
            }
            Value::Object(map) => {
                let mut cells = vec![Value::Null; cols.len()];
                for (k, v) in map {
                    match cols.iter().position(|(n, _)| column_matches(n, k)) {
                        Some(j) => {
                            if let Some(c) = cells.get_mut(j) {
                                *c = v.clone();
                            }
                        }
                        None => {
                            out.push(format!(
                                "expected row {}: no column `{k}` in the result",
                                i.saturating_add(1)
                            ));
                            return out;
                        }
                    }
                }
                want.push(cells);
            }
            _ => {}
        }
    }
    if want.len() != got.len() {
        out.push(format!("expected {} row(s), got {}", want.len(), got.len()));
    }
    let types: Vec<&str> = cols.iter().map(|c| c.1.as_str()).collect();
    let mut got = got;
    if !t.ordered {
        want.sort_by(|a, b| row_cmp(a, b));
        got.sort_by(|a, b| row_cmp(a, b));
    }
    let mut diffs = 0usize;
    for (i, (w, g)) in want.iter().zip(&got).enumerate() {
        let equal = w.iter().zip(g).zip(&types).all(|((e, a), ty)| same(e, a, ty, t));
        if !equal {
            diffs = diffs.saturating_add(1);
            if diffs <= MAX_DIFFS {
                out.push(format!(
                    "row {}: expected {}, got {}",
                    i.saturating_add(1),
                    Value::Array(w.clone()),
                    Value::Array(g.clone())
                ));
            }
        }
    }
    if diffs > MAX_DIFFS {
        out.push(format!("{} more row difference(s)", diffs - MAX_DIFFS));
    }
    out
}

/// A total order over result rows, for unordered comparison.
fn row_cmp(a: &[Value], b: &[Value]) -> Ordering {
    for (x, y) in a.iter().zip(b) {
        let o = cell_cmp(x, y);
        if o != Ordering::Equal {
            return o;
        }
    }
    a.len().cmp(&b.len())
}

fn cell_cmp(a: &Value, b: &Value) -> Ordering {
    let rank = |v: &Value| match v {
        Value::Null => 0,
        Value::Number(_) => 1,
        Value::String(_) => 2,
        Value::Bool(_) => 3,
        _ => 4,
    };
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            let (x, y) = (x.as_f64().unwrap_or(0.0), y.as_f64().unwrap_or(0.0));
            x.total_cmp(&y)
        }
        (Value::String(x), Value::String(y)) => x.cmp(y),
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        _ => rank(a).cmp(&rank(b)),
    }
}

/// Typed equality of an expected and an actual cell.
fn same(expected: &Value, actual: &Value, ty: &str, t: &DaxTest) -> bool {
    match (expected, actual) {
        (Value::Null, Value::Null) => true,
        (Value::Number(e), Value::Number(a)) => {
            let (e, a) = (e.as_f64().unwrap_or(f64::NAN), a.as_f64().unwrap_or(f64::NAN));
            let tol = t.tolerance + t.relative_tolerance * e.abs();
            e == a || (e - a).abs() <= tol
        }
        (Value::String(e), Value::String(a)) if ty.eq_ignore_ascii_case("datetime") => {
            a == e || (e.len() == 10 && a.strip_prefix(e.as_str()) == Some("T00:00:00"))
        }
        (Value::String(e), Value::String(a)) => a == e,
        (Value::Bool(e), Value::Bool(a)) => a == e,
        _ => false,
    }
}

fn bounds(t: &DaxTest, actual: &Value, out: &mut Vec<String>) {
    if t.min.is_none() && t.max.is_none() {
        return;
    }
    let Some(x) = actual.as_f64() else {
        out.push(format!("min/max need a number, got {}", show(actual)));
        return;
    };
    if let Some(min) = t.min
        && x < min - t.tolerance
    {
        out.push(format!("expected at least {min}, got {x}"));
    }
    if let Some(max) = t.max
        && x > max + t.tolerance
    {
        out.push(format!("expected at most {max}, got {x}"));
    }
}

fn type_problem(rule: TypeRule, ty: &str, v: &Value) -> Option<String> {
    let ok = match rule {
        TypeRule::Blank => v.is_null(),
        _ if v.is_null() => true,
        TypeRule::Number => matches!(ty, "int64" | "double" | "decimal"),
        TypeRule::Integer => ty == "int64",
        TypeRule::Text => ty == "string",
        TypeRule::Boolean => ty == "boolean",
        TypeRule::Date => ty.eq_ignore_ascii_case("datetime"),
    };
    (!ok).then(|| format!("expected type {}, got {}", rule.name(), type_word(ty, v)))
}

fn type_word(ty: &str, v: &Value) -> String {
    if v.is_null() {
        return "BLANK".into();
    }
    match ty {
        "int64" => "integer".into(),
        "double" | "decimal" => "number".into(),
        "string" => "text".into(),
        "boolean" => "boolean".into(),
        t if t.eq_ignore_ascii_case("datetime") => "date".into(),
        other => other.into(),
    }
}

fn show(v: &Value) -> String {
    match v {
        Value::Null => "BLANK".into(),
        other => other.to_string(),
    }
}

fn show_typed(v: &Value, ty: &str) -> String {
    match v {
        Value::Null => "BLANK".into(),
        other => format!("{other} ({})", type_word(ty, v)),
    }
}
