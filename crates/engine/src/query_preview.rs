//! `query.preview`: the first rows of a Power Query (M) query, evaluated by BIcraft's M evaluator
//! without loading them into the model, as Power BI Desktop's Power Query editor previews a
//! query (docs/mcp.md "Query preview").
//!
//! The query is a shared expression of the model (a query or a parameter), a table's M
//! partition, or M text. It is evaluated as `Table.FirstN(<query>, limit + 1)`, so a source that
//! folds a row limit (the Databricks connector) reads only those rows, with the model's other
//! queries and parameters, its culture, and the session's (or the call's) `data_root` as the only
//! folder file functions may read. External connectors run only when the call names them in
//! `allow_connectors` (as `model.refresh` does) and the server allows network access; a run that
//! names them changes nothing either, but counts as writing for the server's roles (a reader
//! cannot open connections) and runs exclusively.

use std::time::Instant;

use bicraft_data::{Limits, SourceOptions, TableData};
use serde_json::{Value, json};

use crate::Session;
use crate::params::{allow, opt_str};

/// Rows previewed when `limit` is not given (Power Query's editor shows a thousand).
const DEFAULT_LIMIT: usize = 1_000;
/// Most rows previewed.
const MAX_LIMIT: usize = 10_000;
/// Longest M text, in bytes.
const MAX_EXPRESSION: usize = 1 << 20;
/// Default and largest evaluation time, in milliseconds.
const DEFAULT_TIMEOUT_MS: u64 = 60_000;
const MAX_TIMEOUT_MS: u64 = 600_000;
/// Name of the wrapped query's step.
const STEP: &str = "__bicraft_preview_source";

fn uint(p: &Value, key: &str, default: u64, min: u64, max: u64) -> Result<u64, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v
            .as_u64()
            .filter(|n| (min..=max).contains(n))
            .ok_or_else(|| format!("`{key}` must be a whole number from {min} to {max}")),
    }
}

/// The M to preview: (what it is, its name, its text).
pub(crate) fn source(s: &Session, p: &Value) -> Result<(&'static str, String, String), String> {
    let name = opt_str(p, "name")?;
    let expression = opt_str(p, "expression")?;
    let partition = opt_str(p, "partition")?;
    match (name, expression) {
        (Some(_), Some(_)) => {
            Err("pass `name` (a query or a table) or `expression` (M text), not both".into())
        }
        (None, None) => {
            Err("`name` (a query or a table of the model) or `expression` (M text) is required".into())
        }
        (None, Some(e)) => {
            if partition.is_some() {
                return Err("`partition` goes with `name` (a table)".into());
            }
            Ok(("expression", "Query".to_string(), e.to_string()))
        }
        (Some(n), None) => {
            let model = s.model.as_ref().ok_or("no model is open (file.open first)")?;
            if partition.is_none()
                && let Some(e) = model.expressions.iter().find(|e| e.name.eq_ignore_ascii_case(n))
            {
                if !(e.kind.is_empty() || e.kind.eq_ignore_ascii_case("m")) {
                    return Err(format!("query `{}` is not Power Query (M)", e.name));
                }
                return Ok(("query", e.name.clone(), e.expression.clone()));
            }
            let t = model
                .tables
                .iter()
                .find(|t| t.name.eq_ignore_ascii_case(n))
                .ok_or_else(|| format!("the model has no query or table `{n}`"))?;
            let part = match partition {
                Some(pn) => t
                    .partitions
                    .iter()
                    .find(|x| x.name.eq_ignore_ascii_case(pn))
                    .ok_or_else(|| format!("table `{}` has no partition `{pn}`", t.name))?,
                None => t
                    .partitions
                    .iter()
                    .find(|x| x.source_type.eq_ignore_ascii_case("m"))
                    .ok_or_else(|| format!("table `{}` has no Power Query (M) partition", t.name))?,
            };
            if !part.source_type.eq_ignore_ascii_case("m") {
                return Err(format!(
                    "partition `{}` of table `{}` is `{}`, not Power Query (M)",
                    part.name, t.name, part.source_type
                ));
            }
            let text = part
                .expression
                .clone()
                .ok_or_else(|| format!("partition `{}` has no expression", part.name))?;
            Ok(("table", t.name.clone(), text))
        }
    }
}

/// A query evaluated for `query.preview` or `query.profile`.
pub(crate) struct Evaluated {
    pub kind: &'static str,
    pub name: String,
    pub step: Option<String>,
    pub data: TableData,
    /// Each column's error values (loaded as blanks), when counted.
    pub errors: Vec<usize>,
    pub connectors: Vec<String>,
    pub started: Instant,
}

/// `query.preview`; see the module docs.
pub(crate) fn query_preview(s: &Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "name",
            "partition",
            "expression",
            "limit",
            "data_root",
            "allow_connectors",
            "timeout_ms",
            "step",
            "queries",
        ],
    )?;
    let limit = usize::try_from(uint(p, "limit", DEFAULT_LIMIT as u64, 1, MAX_LIMIT as u64)?)
        .unwrap_or(DEFAULT_LIMIT);
    let ev = evaluate(s, p, Some(limit), false)?;
    let cols = ev.data.columns();
    let total = cols.first().map_or(0, |c| c.values.len());
    let shown = total.min(limit);
    let columns: Vec<Value> = cols
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let mut v = json!({"name": c.name, "data_type": c.data_type.as_tmsl().unwrap_or("unknown")});
            // the M type of a date/time column (#date or #datetime literals in filters)
            if let Some(k) = ev.data.date_kind(i) {
                v["date_kind"] = json!(match k {
                    bicraft_data::DateKind::Date => "date",
                    bicraft_data::DateKind::Time => "time",
                    bicraft_data::DateKind::DateTime => "datetime",
                    bicraft_data::DateKind::DateTimeZone => "datetimezone",
                });
            }
            v
        })
        .collect();
    let rows: Vec<Value> = (0..shown)
        .map(|i| {
            Value::Array(
                cols.iter()
                    .map(|c| c.values.get(i).map_or(Value::Null, crate::preview::cell_json))
                    .collect(),
            )
        })
        .collect();
    Ok(json!({
        "name": ev.name,
        "step": ev.step,
        "source": ev.kind,
        "columns": columns,
        "rows": rows,
        "row_count": shown,
        "truncated": total > limit,
        "connectors": ev.connectors,
        "elapsed_ms": u64::try_from(ev.started.elapsed().as_millis()).unwrap_or(u64::MAX),
    }))
}

/// Most drafts one call passes (`queries`).
const MAX_DRAFTS: usize = 2_000;
/// Most bytes of all the drafts of one call.
const MAX_DRAFT_BYTES: usize = 64 << 20;

/// The editor's drafts of the other queries (`queries: [{name, expression}]`), read in place of
/// the model's queries of the same names.
fn drafts(p: &Value) -> Result<Vec<(String, String)>, String> {
    let list = match p.get("queries") {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(a)) if a.len() <= MAX_DRAFTS => a,
        Some(_) => {
            return Err(format!(
                "`queries` must be a list of at most {MAX_DRAFTS} {{name, expression}}"
            ));
        }
    };
    let mut out = Vec::with_capacity(list.len());
    let mut bytes = 0usize;
    for (i, q) in list.iter().enumerate() {
        let at = |e: String| format!("queries[{i}]: {e}");
        allow(q, &["name", "expression"]).map_err(at)?;
        let name = crate::params::str_param(q, "name").map_err(at)?;
        let m = crate::params::str_param(q, "expression").map_err(at)?;
        bytes = bytes.saturating_add(m.len());
        if m.len() > MAX_EXPRESSION || bytes > MAX_DRAFT_BYTES {
            return Err(at("the queries are too long".into()));
        }
        out.push((name.to_string(), m.to_string()));
    }
    Ok(out)
}

/// Evaluates the query of `p` (`name` | `expression`, `partition`, `step`, `data_root`,
/// `allow_connectors`, `timeout_ms`): its first `limit` rows and one more (so a caller can tell
/// there are more), or all of them (`None`, within the evaluator's row cap); with each column's
/// errors counted when `count_errors`.
pub(crate) fn evaluate(
    s: &Session,
    p: &Value,
    limit: Option<usize>,
    count_errors: bool,
) -> Result<Evaluated, String> {
    let timeout = uint(p, "timeout_ms", DEFAULT_TIMEOUT_MS, 1, MAX_TIMEOUT_MS)?;
    let (kind, name, text) = source(s, p)?;
    // the query at one of its applied steps (the Power Query editor's preview of a step)
    let step = opt_str(p, "step")?;
    let text = match step {
        Some(st) => crate::query_steps::at_step(&text, st)?,
        None => text,
    };
    if text.len() > MAX_EXPRESSION {
        return Err(format!("the query is longer than {MAX_EXPRESSION} bytes"));
    }
    let given_root = crate::datasrc::data_root_param(p)?;
    let root = crate::datasrc::effective_root(&given_root, s.data_root.as_deref());
    let allow_list = crate::refresh::string_list(p, "allow_connectors")?.unwrap_or_default();
    let (connectors, _dbx) = crate::refresh::connectors(s, &allow_list, false)?;
    let started = Instant::now();
    // intermediate tables may be larger than the preview: the default row cap applies to them
    let limits = Limits {
        max_millis: timeout,
        ..Limits::default()
    };
    let drafts = drafts(p)?;
    let opts = |text: &str| -> SourceOptions {
        let mut o = crate::datasrc::source_options(s.model.as_ref(), &name, text, root.as_deref(), limits);
        o.connectors = connectors.clone();
        if !drafts.is_empty()
            && let Some(m) = s.model.as_ref()
        {
            o.shared = crate::datasrc::shared_with_drafts(m, &name, text, &drafts);
        }
        o
    };
    // the first rows only: a source that folds a row limit reads no more
    let rows = |of: &str| match limit {
        Some(n) => format!("Table.FirstN({of}, {})", n.saturating_add(1)),
        None => of.to_string(),
    };
    let wrapped = format!("let\n    {STEP} =\n{text}\nin\n    {}", rows(STEP));
    // nested values (a merge's table column, records, lists) show as `[Table]`… as the editor
    // shows them; tried only when the rows do not load as they are
    let structured = format!(
        "let\n    {STEP} =\n{text}\nin\n    Table.TransformColumns({}, {{}}, (v) => \
         if v is table then \"[Table]\" else if v is record then \"[Record]\" else if v is list \
         then \"[List]\" else if v is function then \"[Function]\" else v)",
        rows(STEP)
    );
    let load = |m: &str| -> Result<(TableData, Vec<usize>), bicraft_data::Error> {
        if count_errors {
            bicraft_data::m::load_m_counting_errors(m, &opts(m))
        } else {
            bicraft_data::m::load_m_with(m, &opts(m)).map(|d| (d, Vec::new()))
        }
    };
    let (data, errors) = match load(&wrapped).or_else(|_| load(&structured)) {
        Ok(d) => d,
        // not a table, or an error of the query: the query's own answer says which
        Err(_) => load(&text).map_err(|e| {
            let msg = crate::datasrc::describe(&e, root.as_deref());
            if msg.contains("external source, not evaluated") && allow_list.is_empty() {
                format!("{msg} (pass allow_connectors, e.g. [\"databricks\"], to let bicraft run it)")
            } else {
                msg
            }
        })?,
    };
    Ok(Evaluated {
        kind,
        name,
        step: step.map(str::to_string),
        data,
        errors,
        connectors: allow_list,
        started,
    })
}
