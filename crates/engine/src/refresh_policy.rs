//! `table.set_refresh_policy` and `table.remove_refresh_policy`: a table's incremental refresh
//! policy, TMSL `refreshPolicy` (docs/formats/model-objects.md, "Incremental refresh policy").
//! The rules come from Microsoft Learn ("Configure incremental refresh for Power BI semantic
//! models", "Advanced incremental refresh and real-time data with the XMLA endpoint"):
//!
//! - the model needs the `RangeStart` and `RangeEnd` parameters (names case-sensitive), of type
//!   Date/Time;
//! - the table's Power Query query filters on them (a `Table.SelectRows` step that uses both),
//!   else Desktop's switch stays off;
//! - *Archive data starting* is the store period (`rollingWindowPeriods` of
//!   `rollingWindowGranularity`), *Incrementally refresh data starting* the refresh period
//!   (`incrementalPeriods` of `incrementalGranularity`), in days, months, quarters or years;
//! - *Only refresh complete days* is `incrementalPeriodsOffset: -1`; *Get the latest data in real
//!   time with DirectQuery* is `mode: hybrid` and turns it on too;
//! - *Detect data changes* names a date/time column (not the one the range filters): its maximum
//!   per period is the `pollingExpression`;
//! - `sourceExpression` is the table's query; the policy applies when the service refreshes the
//!   model: Desktop, and BIcraft, keep the table's partition as it is.
//!
//! Compatibility levels: 1550 for an import policy, 1565 for a hybrid one (raised when lower).

use bicraft_data::Limits;
use bicraft_data::m::analyze_all;
use bicraft_model::{DataType, SemanticModel, Table};
use serde_json::{Map, Value, json};

use crate::Session;
use crate::edit::{Edit, apply, dax_column, dax_table, table_index};
use crate::mquery::is_m;
use crate::params::{allow, opt_bool, opt_str, str_param};

/// The period units, as TMSL spells the granularities.
pub(crate) const UNITS: &[&str] = &["day", "month", "quarter", "year"];
/// Most periods a policy keeps.
const MAX_PERIODS: u64 = 100_000;
/// Compatibility levels a policy needs (import, hybrid).
const LEVEL_IMPORT: u32 = 1550;
const LEVEL_HYBRID: u32 = 1565;

/// `{n, unit}` of parameter `key`.
fn period(p: &Value, key: &str) -> Result<(u64, &'static str), String> {
    let v = p.get(key).ok_or_else(|| {
        format!("missing parameter `{key}` ({{n, unit}}, e.g. {{\"n\": 5, \"unit\": \"year\"}})")
    })?;
    let o = v
        .as_object()
        .ok_or_else(|| format!("parameter `{key}` must be an object {{n, unit}}"))?;
    if let Some(k) = o.keys().find(|k| !["n", "unit"].contains(&k.as_str())) {
        return Err(format!("unknown key `{k}` in `{key}` (expected n, unit)"));
    }
    let n = o
        .get("n")
        .and_then(Value::as_u64)
        .filter(|n| (1..=MAX_PERIODS).contains(n))
        .ok_or_else(|| format!("`{key}.n` must be a whole number from 1 to {MAX_PERIODS}"))?;
    let unit = o
        .get("unit")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing `{key}.unit` (day, month, quarter or year)"))?;
    let u = unit.trim().to_ascii_lowercase();
    let u = u.strip_suffix('s').unwrap_or(&u);
    let unit =
        UNITS.iter().copied().find(|x| *x == u).ok_or_else(|| {
            format!("unknown unit `{unit}` in `{key}` (expected day, month, quarter or year)")
        })?;
    Ok((n, unit))
}

/// Approximate days of `n` units (only to compare the two periods).
fn days(n: u64, unit: &str) -> u64 {
    n.saturating_mul(match unit {
        "month" => 30,
        "quarter" => 91,
        "year" => 365,
        _ => 1,
    })
}

/// The text without spaces, for matching `meta [...]` records.
fn squeezed(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Why `name` is not a usable Date/Time parameter (`None`: it is).
fn parameter_problem(m: &SemanticModel, name: &str) -> Option<String> {
    let add = format!(
        "add it with expression.add {{name: \"{name}\", expression: \"#datetime(2024, 1, 1, 0, 0, 0) meta \
         [IsParameterQuery=true, Type=\\\"DateTime\\\", IsParameterQueryRequired=true]\"}} (Power Query \
         editor › Manage parameters)"
    );
    let Some(e) = m.expressions.iter().find(|e| e.name == name) else {
        return Some(
            match m.expressions.iter().find(|e| e.name.eq_ignore_ascii_case(name)) {
                Some(e) => format!(
                    "the parameter is named `{}`: incremental refresh needs exactly `{name}` (the name is \
                 case-sensitive)",
                    e.name
                ),
                None => format!("the model has no `{name}` parameter: {add}"),
            },
        );
    };
    let text = squeezed(&e.expression);
    if !is_m(&e.kind) || !text.contains("IsParameterQuery=true") {
        return Some(format!("`{name}` is a query, not a parameter: {add}"));
    }
    if !(text.contains("Type=\"DateTime\"") || text.contains("Type=\"Date/Time\"")) {
        return Some(format!(
            "`{name}` must be a Date/Time parameter (meta [Type=\"DateTime\"]): change its type"
        ));
    }
    None
}

/// The table's query: its M partition's text, or the policy's `sourceExpression` when the
/// service already applied it (policy-range partitions).
fn query_of(t: &Table) -> Option<String> {
    if let Some(v) = t
        .extra
        .get("refreshPolicy")
        .and_then(|p| p.get("sourceExpression"))
    {
        match v {
            Value::String(s) => return Some(s.clone()),
            Value::Array(a) => {
                let lines: Vec<&str> = a.iter().filter_map(Value::as_str).collect();
                return Some(lines.join("\n"));
            }
            _ => {}
        }
    }
    match t.partitions.as_slice() {
        [p] if is_m(&p.source_type) => p.expression.clone(),
        _ => None,
    }
}

/// Whether M `query` filters on both parameters in a `Table.SelectRows` step.
fn filters_on_range(query: &str) -> (bool, bool, bool) {
    let mut start = false;
    let mut end = false;
    let mut select = false;
    for a in analyze_all(vec![query.to_string()], &Limits::default()) {
        for r in a.references {
            match r.name.as_str() {
                "RangeStart" => start = true,
                "RangeEnd" => end = true,
                "Table.SelectRows" => select = true,
                _ => {}
            }
        }
    }
    (start, end, select)
}

/// An M field name: plain when it is a simple identifier, else quoted (`#"..."`).
fn field(name: &str) -> String {
    if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        name.to_string()
    } else {
        format!("#\"{}\"", name.replace('"', "\"\""))
    }
}

/// The policy's polling expression for "Detect data changes": the column's maximum (BIcraft's
/// own text: the latest value of the column, or a fixed date when it has none).
fn polling(table: &str, column: &str) -> String {
    let q = |s: &str| format!("#\"{}\"", s.replace('"', "\"\""));
    format!(
        "let\n    MaxValue = List.Max({}[{}]),\n    Result = if MaxValue = null then #datetime(1901, 1, 1, 0, 0, 0) else MaxValue\nin\n    Result",
        q(table),
        field(column)
    )
}

/// `table.set_refresh_policy`.
pub(crate) fn table_set_refresh_policy(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "table",
            "store",
            "refresh",
            "detect_changes",
            "only_complete_periods",
            "realtime",
        ],
    )?;
    let table = str_param(p, "table")?;
    let store = period(p, "store")?;
    let refresh = period(p, "refresh")?;
    let detect = opt_str(p, "detect_changes")?;
    let realtime = opt_bool(p, "realtime")?.unwrap_or(false);
    let complete = opt_bool(p, "only_complete_periods")?;
    if realtime && complete == Some(false) {
        return Err(
            "realtime needs only_complete_periods: the DirectQuery partition starts where the last \
             complete period ends"
                .into(),
        );
    }
    let complete = complete.unwrap_or(false) || realtime;
    apply(s, p, |m| {
        let ti = table_index(m, table)?;
        let t = m.tables.get(ti).ok_or("internal: table index out of range")?;
        let name = t.name.clone();
        let mut missing = Vec::new();
        for param in ["RangeStart", "RangeEnd"] {
            if let Some(why) = parameter_problem(m, param) {
                missing.push(why);
            }
        }
        let mode = crate::storage_mode::table_mode(m, t);
        if !(mode == "import" || (mode == "hybrid" && t.extra.contains_key("refreshPolicy"))) {
            missing.push(format!(
                "{} is in {mode} mode: incremental refresh applies to Import tables (set it to Import \
                 with table.set_storage_mode)",
                dax_table(&name)
            ));
        }
        let query = query_of(t);
        match query.as_deref() {
            None => missing.push(format!(
                "{} has no Power Query query to filter (incremental refresh needs one M partition)",
                dax_table(&name)
            )),
            Some(q) => {
                let (start, end, select) = filters_on_range(q);
                if !(start && end && select) {
                    let lacking: Vec<&str> = [("RangeStart", start), ("RangeEnd", end)]
                        .into_iter()
                        .filter(|x| !x.1)
                        .map(|x| x.0)
                        .collect();
                    missing.push(format!(
                        "the query of {} has no filter step on {}: add one, e.g. Table.SelectRows(Source, \
                         each [OrderDate] >= RangeStart and [OrderDate] < RangeEnd) (an equal sign on \
                         one bound only)",
                        dax_table(&name),
                        if lacking.is_empty() {
                            "RangeStart and RangeEnd (Table.SelectRows)".to_string()
                        } else {
                            lacking.join(" and ")
                        }
                    ));
                }
            }
        }
        let detect_col = match detect {
            None => None,
            Some(c) => match t
                .columns
                .iter()
                .filter(|x| !x.is_row_number())
                .find(|x| x.name.eq_ignore_ascii_case(c))
            {
                None => {
                    missing.push(format!(
                        "no column `{c}` in {} to detect data changes",
                        dax_table(&name)
                    ));
                    None
                }
                Some(col) if col.data_type != DataType::DateTime => {
                    missing.push(format!(
                        "{} is not a date/time column: detecting data changes needs one",
                        dax_column(&name, &col.name)
                    ));
                    None
                }
                Some(col) if col.expression.is_some() => {
                    missing.push(format!(
                        "{} is a calculated column: detecting data changes reads a column of the source",
                        dax_column(&name, &col.name)
                    ));
                    None
                }
                Some(col) => Some(col.name.clone()),
            },
        };
        if !missing.is_empty() {
            return Err(format!(
                "incremental refresh of {} cannot be set: {}",
                dax_table(&name),
                missing.join("; ")
            ));
        }
        let source = query.unwrap_or_default();
        let mut notes = Vec::new();
        if days(refresh.0, refresh.1) > days(store.0, store.1) {
            notes.push(format!(
                "the refresh period ({} {}) is longer than the archive period ({} {}): the refresh \
                 period is normally the most recent part of the archive",
                refresh.0, refresh.1, store.0, store.1
            ));
        }
        if let Some(c) = &detect_col
            && source.contains(&format!("[{c}]"))
            && source.contains("RangeStart")
        {
            notes.push(format!(
                "{} should not be the column the RangeStart/RangeEnd filter uses",
                dax_column(&name, c)
            ));
        }
        if realtime {
            notes.push(
                "real-time data (a DirectQuery partition) needs a Premium, Premium Per User or Fabric \
                 capacity in the service"
                    .into(),
            );
        }
        notes.push(
            "the policy applies when the Power BI service refreshes the published model; the table \
             keeps its partition here"
                .into(),
        );
        let mut policy = Map::new();
        policy.insert("policyType".into(), json!("basic"));
        if realtime {
            policy.insert("mode".into(), json!("hybrid"));
        }
        policy.insert("rollingWindowGranularity".into(), json!(store.1));
        policy.insert("rollingWindowPeriods".into(), json!(store.0));
        policy.insert("incrementalGranularity".into(), json!(refresh.1));
        policy.insert("incrementalPeriods".into(), json!(refresh.0));
        if complete {
            policy.insert("incrementalPeriodsOffset".into(), json!(-1));
        }
        if let Some(c) = &detect_col {
            policy.insert("pollingExpression".into(), json!(polling(&name, c)));
        }
        policy.insert("sourceExpression".into(), json!(source));
        // the compatibility level the policy needs
        let need = if realtime { LEVEL_HYBRID } else { LEVEL_IMPORT };
        if let Some(l) = m.compatibility_level
            && l < need
        {
            m.compatibility_level = Some(need);
            notes.push(format!("the compatibility level was raised from {l} to {need}"));
        }
        let previous = m
            .tables
            .get_mut(ti)
            .ok_or("internal: table index out of range")?
            .extra
            .insert("refreshPolicy".into(), Value::Object(policy));
        let mut edit = Edit::new(json!({
            "table": name,
            "store": {"n": store.0, "unit": store.1},
            "refresh": {"n": refresh.0, "unit": refresh.1},
            "detect_changes": detect_col,
            "only_complete_periods": complete,
            "realtime": realtime,
            "replaced": previous.is_some(),
        }));
        edit.notes = notes;
        Ok(edit)
    })
}

/// `table.remove_refresh_policy`.
pub(crate) fn table_remove_refresh_policy(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["table"])?;
    let table = str_param(p, "table")?;
    apply(s, p, |m| {
        let ti = table_index(m, table)?;
        let t = m.tables.get_mut(ti).ok_or("internal: table index out of range")?;
        if t.extra.remove("refreshPolicy").is_none() {
            return Err(format!(
                "{} has no incremental refresh policy",
                dax_table(&t.name)
            ));
        }
        let mut edit = Edit::new(json!({"table": t.name}));
        if t.partitions.len() > 1 {
            edit.notes.push(format!(
                "{} keeps the {} partitions the service created from the policy",
                dax_table(&t.name),
                t.partitions.len()
            ));
        }
        Ok(edit)
    })
}

/// The snapshot's view of a table's policy (`None`: it has none).
pub(crate) fn summary(t: &Table) -> Option<Value> {
    let p = t.extra.get("refreshPolicy")?.as_object()?;
    let num = |k: &str| p.get(k).and_then(Value::as_i64);
    let text = |k: &str| p.get(k).and_then(Value::as_str);
    let detect = text("pollingExpression").and_then(|e| {
        // BIcraft's own polling text names the column as `[Column]`
        let at = e.find("List.Max(")?;
        let rest = e.get(at..)?;
        let open = rest.find('[')?;
        let close = rest.get(open..)?.find(']')?;
        rest.get(open + 1..open + close).map(|f| {
            let f = f
                .strip_prefix("#\"")
                .and_then(|f| f.strip_suffix('"'))
                .unwrap_or(f);
            f.replace("\"\"", "\"")
        })
    });
    Some(json!({
        "store": {"n": num("rollingWindowPeriods"), "unit": text("rollingWindowGranularity")},
        "refresh": {"n": num("incrementalPeriods"), "unit": text("incrementalGranularity")},
        "only_complete_periods": num("incrementalPeriodsOffset").is_some_and(|o| o < 0),
        "realtime": text("mode").is_some_and(|m| m.eq_ignore_ascii_case("hybrid")),
        "detect_changes": detect,
        "polling_expression": text("pollingExpression").is_some(),
    }))
}

/// The snapshot's `model.incremental_refresh`: whether the model has the `RangeStart` and
/// `RangeEnd` Date/Time parameters, and what is wrong with them.
pub(crate) fn parameters(m: &SemanticModel) -> Value {
    let problems: Vec<String> = ["RangeStart", "RangeEnd"]
        .into_iter()
        .filter_map(|n| parameter_problem(m, n))
        .collect();
    json!({"parameters": problems.is_empty(), "problems": problems})
}

/// Whether the table's query filters on `RangeStart` and `RangeEnd` (the dialog's switch). The
/// query is parsed only when the model has both parameters.
pub(crate) fn range_filtered(m: &SemanticModel, t: &Table) -> bool {
    ["RangeStart", "RangeEnd"]
        .into_iter()
        .all(|n| m.expressions.iter().any(|e| e.name == n))
        && query_of(t).is_some_and(|q| {
            let (a, b, c) = filters_on_range(&q);
            a && b && c
        })
}
