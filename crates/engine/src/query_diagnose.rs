//! `query.diagnose` (the Power Query editor's Diagnose step, docs/mcp.md "Power Query editor"):
//! the query at one step, or at each step, evaluated as a refresh does by
//! `bicraft_data::m::folding::diagnose`, with BIcraft's own time, the rows and the requests sent
//! to its sources (never Desktop's figures).

use std::time::Instant;

use bicraft_data::Limits;
use bicraft_data::m::folding;
use bicraft_data::m::steps;
use serde_json::{Value, json};

use crate::Session;
use crate::params::{allow, opt_str};
use crate::query_folding::{PARAMS, Probing};
/// Most source requests listed per step in a diagnosis.
const MAX_REQUESTS: usize = 1_000;

/// `query.diagnose`: the query at one step (or each step) evaluated as a refresh does, with
/// BIcraft's own time, the rows and the requests sent to its sources.
pub(crate) fn query_diagnose(s: &Session, p: &Value) -> Result<Value, String> {
    let mut keys = PARAMS.to_vec();
    keys.extend(["step", "all_steps"]);
    allow(p, &keys)?;
    let pr = Probing::new(s, p)?;
    let names = pr.steps()?;
    let all = crate::params::opt_bool(p, "all_steps")?.unwrap_or(false);
    let chosen: Vec<String> = match (opt_str(p, "step")?, all) {
        (Some(_), true) => return Err("pass `step` or `all_steps`, not both".into()),
        (Some(st), false) => {
            if !names.iter().any(|n| n == st) {
                return Err(format!("query `{}` has no step `{st}`", pr.name));
            }
            vec![st.to_string()]
        }
        (None, true) => names,
        (None, false) => steps::steps(&pr.text, &Limits::default())?
            .last()
            .map(|x| vec![x.name.clone()])
            .ok_or("the query has no step")?,
    };
    let mut out = Vec::with_capacity(chosen.len());
    for st in &chosen {
        let text = crate::query_steps::at_step(&pr.text, st)?;
        let opts = pr.options(s, &text)?;
        let started = Instant::now();
        let d =
            folding::diagnose(&text, &opts).map_err(|e| crate::datasrc::describe(&e, pr.root.as_deref()))?;
        let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let requests: Vec<Value> = d
            .requests
            .iter()
            .take(MAX_REQUESTS)
            .map(|r| {
                json!({"connector": r.connector, "kind": r.kind, "text": r.text, "rows": r.rows,
                    "elapsed_ms": r.millis})
            })
            .collect();
        let (rows, columns, error) = match &d.result {
            Ok(t) => {
                let cols = t.columns();
                (
                    json!(cols.first().map_or(0, |c| c.values.len())),
                    json!(cols.len()),
                    Value::Null,
                )
            }
            Err(e) => (
                Value::Null,
                Value::Null,
                json!(crate::datasrc::describe(e, pr.root.as_deref())),
            ),
        };
        out.push(json!({
            "name": st,
            "elapsed_ms": elapsed,
            "rows": rows,
            "columns": columns,
            "error": error,
            "requests": requests,
            "requests_total": d.requests.len(),
        }));
    }
    Ok(json!({
        "name": pr.name,
        "steps": out,
        "allow_connectors": pr.allow_list,
        "elapsed_ms": u64::try_from(pr.started.elapsed().as_millis()).unwrap_or(u64::MAX),
    }))
}
