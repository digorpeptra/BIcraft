//! The Power Query editor's commands (ROADMAP M11 lot 12, docs/mcp.md "Power Query editor"):
//!
//! - `query.list`: the model's queries (tables loaded by an M partition, shared M expressions)
//!   with their M;
//! - `query.steps`: a query's applied steps (the variables of its `let`);
//! - `query.apply_step`, `query.edit_step`, `query.remove_step`: one step added after another
//!   (a transform of the editor's ribbon written as Power Query writes it), a step's formula or
//!   name changed, a step removed. They take the query's M (`expression`, the editor's draft) or
//!   a query of the model (`name`) and return the new M; they change nothing in the model, as
//!   the editor's steps stay in the editor until Close & Apply;
//! - `query.apply` (Close & Apply): the edited queries written back and their tables reloaded,
//!   as one edit (one undo step), all or nothing.
//!
//! The text edits are `bicraft_data::m::steps`'s: the user's text of the steps left alone is kept
//! byte for byte. The transforms' M is `bicraft_data::m::transform`'s.

use bicraft_data::Limits;
use bicraft_data::m::steps::{self, Steps};
use bicraft_data::m::transform::{Aggregate, ColumnType, FilterOp, JoinKind, Literal, Transform};
use serde_json::{Value, json};

use crate::Session;
use crate::edit::check_name;
use crate::mquery::{check_edit, is_m, query_names};
use crate::params::{allow, opt_bool, opt_str, str_param};

/// Longest M text, in bytes.
const MAX_EXPRESSION: usize = 1 << 20;
/// Most queries one `query.apply` writes.
const MAX_QUERIES: usize = 200;
/// Most steps listed in an answer.
const MAX_LISTED: usize = steps::MAX_STEPS;

fn limits() -> Limits {
    Limits::default()
}

/// The query's (kind, name, M): `expression` (a draft), else query or table `name`.
fn query_text(s: &Session, p: &Value) -> Result<(&'static str, String, String), String> {
    let (kind, name, text) = crate::query_preview::source(s, p)?;
    if text.len() > MAX_EXPRESSION {
        return Err(format!("the query is longer than {MAX_EXPRESSION} bytes"));
    }
    Ok((kind, name, text))
}

/// The steps of an answer.
fn steps_json(st: &Steps) -> Value {
    json!(
        st.steps
            .iter()
            .take(MAX_LISTED)
            .map(|x| {
                // the step settings gear: the transform that wrote the step, when it reads back
                let settings = bicraft_data::m::transform_parse::settings(&x.expression, &limits())
                    .map_or(Value::Null, |(_, v)| v);
                json!({"name": x.name, "expression": x.expression, "function": x.function,
                    "settings": settings})
            })
            .collect::<Vec<_>>()
    )
}

fn answer(s: &Session, name: &str, text: String, step: Option<&str>) -> Result<Value, String> {
    let st = steps::steps(&text, &limits())?;
    let connectors = crate::query_folding::needed_connectors(s, &text);
    Ok(json!({
        "name": name,
        "expression": text,
        "is_let": st.is_let,
        "steps": steps_json(&st),
        "result": st.result,
        "result_step": st.last().map(|x| x.name.clone()),
        "step": step,
        "connectors": connectors,
    }))
}

/// `query.list`: the model's queries and their M.
pub(crate) fn query_list(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &[])?;
    let model = s.model.as_ref().ok_or("no model is open (file.open first)")?;
    let mut out = Vec::new();
    for t in &model.tables {
        let Some(part) = t.partitions.iter().find(|x| is_m(&x.source_type)) else {
            continue;
        };
        let Some(m) = &part.expression else { continue };
        out.push(json!({
            "name": t.name,
            "source": "table",
            "expression": m,
            "partition": part.name,
            "partitions": t.partitions.len(),
            "hidden": t.is_hidden,
            "group": Value::Null,
            "parameter": false,
        }));
    }
    for e in &model.expressions {
        if !(e.kind.is_empty() || e.kind.eq_ignore_ascii_case("m")) {
            continue;
        }
        out.push(json!({
            "name": e.name,
            "source": "query",
            "expression": e.expression,
            "partition": Value::Null,
            "partitions": 0,
            "hidden": false,
            "group": e.extra.get("queryGroup").cloned().unwrap_or(Value::Null),
            "parameter": e.expression.contains("IsParameterQuery=true")
                || e.expression.contains("IsParameterQuery = true"),
        }));
    }
    Ok(json!({
        "queries": out,
        "data_root": s.data_root.as_ref().map(|r| r.display().to_string()),
    }))
}

/// `query.steps`: a query's applied steps.
pub(crate) fn query_steps(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &["name", "partition", "expression"])?;
    let (kind, name, text) = query_text(s, p)?;
    let mut v = answer(s, &name, text, None)?;
    v["source"] = json!(kind);
    Ok(v)
}

pub(crate) fn list(v: &Value, key: &str) -> Result<Vec<String>, String> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(a)) => {
            if a.len() > bicraft_data::m::transform::MAX_COLUMNS {
                return Err(format!("`{key}` holds too many names"));
            }
            a.iter()
                .map(|x| {
                    x.as_str()
                        .map(str::to_string)
                        .ok_or_else(|| format!("`{key}` must be a list of names"))
                })
                .collect()
        }
        Some(_) => Err(format!("`{key}` must be a list of names")),
    }
}

pub(crate) fn literal(v: Option<&Value>, key: &str) -> Result<Literal, String> {
    Ok(match v {
        None | Some(Value::Null) => Literal::Null,
        Some(Value::Bool(b)) => Literal::Logical(*b),
        Some(Value::Number(n)) => Literal::Number(
            n.as_f64()
                .filter(|f| f.is_finite())
                .ok_or_else(|| format!("`{key}` is not a number M can hold"))?,
        ),
        Some(Value::String(t)) => Literal::Text(t.clone()),
        Some(x @ Value::Object(_)) => crate::query_transforms::date_literal(x, key)?,
        Some(_) => return Err(format!("`{key}` must be text, a number, true/false or null")),
    })
}

fn objects<'a>(v: &'a Value, key: &str) -> Result<&'a [Value], String> {
    match v.get(key) {
        Some(Value::Array(a)) if a.len() <= bicraft_data::m::transform::MAX_COLUMNS => Ok(a),
        Some(Value::Array(_)) => Err(format!("`{key}` holds too many items")),
        _ => Err(format!("`{key}` must be a list")),
    }
}

/// A transform from `{kind, …}`.
pub(crate) fn transform(v: &Value) -> Result<Transform, String> {
    let kind = str_param(v, "kind")?;
    if let Some(m) = crate::query_transforms::more(v, kind) {
        return m.map(Transform::More);
    }
    let keys: &[&str] = match kind {
        "filter_rows" => &["kind", "column", "op", "value"],
        "filter_values" => &["kind", "column", "values", "exclude"],
        "remove_columns" | "keep_columns" | "remove_duplicates" => &["kind", "columns"],
        "rename_columns" => &["kind", "renames"],
        "change_type" => &["kind", "columns"],
        "promote_headers" | "custom" => &["kind"],
        "sort" => &["kind", "columns"],
        "replace_values" => &["kind", "columns", "find", "replace"],
        "group_by" => &["kind", "keys", "aggregations"],
        "merge" => &["kind", "other", "keys", "other_keys", "join", "column"],
        "expand" => &["kind", "column", "columns", "prefix"],
        "append" => &["kind", "others"],
        "add_column" => &["kind", "name", "formula"],
        "split_column" => &["kind", "column", "delimiter", "parts"],
        "unpivot" => &["kind", "columns", "other"],
        "pivot" => &["kind", "column", "values", "aggregate"],
        other => {
            return Err(format!(
                "unknown step kind `{other}` (expected filter_rows, filter_values, remove_columns, keep_columns, \
                 rename_columns, change_type, promote_headers, sort, remove_duplicates, \
                 replace_values, group_by, merge, expand, append, add_column, split_column, \
                 unpivot, pivot, custom or {})",
                crate::query_transforms::KINDS.join(", ")
            ));
        }
    };
    allow(v, keys)?;
    Ok(match kind {
        "filter_rows" => {
            let op = str_param(v, "op")?;
            let op = FilterOp::from_name(op).ok_or_else(|| {
                format!(
                    "unknown `op` `{op}` (expected {})",
                    FilterOp::ALL.map(FilterOp::name).join(", ")
                )
            })?;
            let value = literal(v.get("value"), "value")?;
            if op.takes_value()
                && value == Literal::Null
                && !matches!(op, FilterOp::Equals | FilterOp::NotEquals)
            {
                return Err(format!("`{}` needs a `value`", op.name()));
            }
            if matches!(
                op,
                FilterOp::Contains | FilterOp::NotContains | FilterOp::BeginsWith | FilterOp::EndsWith
            ) && !matches!(value, Literal::Text(_))
            {
                return Err(format!("`{}` compares text: `value` must be text", op.name()));
            }
            Transform::FilterRows {
                column: str_param(v, "column")?.to_string(),
                op,
                value,
            }
        }
        "filter_values" => {
            let values = objects(v, "values")?
                .iter()
                .enumerate()
                .map(|(i, x)| literal(Some(x), &format!("values[{i}]")))
                .collect::<Result<Vec<_>, _>>()?;
            if values.is_empty() {
                return Err("`values` must name at least one value".into());
            }
            Transform::FilterValues {
                column: str_param(v, "column")?.to_string(),
                values,
                exclude: opt_bool(v, "exclude")?.unwrap_or(false),
            }
        }
        "remove_columns" => Transform::RemoveColumns(list(v, "columns")?),
        "keep_columns" => Transform::KeepColumns(list(v, "columns")?),
        "remove_duplicates" => Transform::RemoveDuplicates(list(v, "columns")?),
        "rename_columns" => {
            let mut r = Vec::new();
            for (i, x) in objects(v, "renames")?.iter().enumerate() {
                allow(x, &["column", "new_name"]).map_err(|e| format!("renames[{i}]: {e}"))?;
                r.push((
                    str_param(x, "column")
                        .map_err(|e| format!("renames[{i}]: {e}"))?
                        .to_string(),
                    str_param(x, "new_name")
                        .map_err(|e| format!("renames[{i}]: {e}"))?
                        .to_string(),
                ));
            }
            Transform::RenameColumns(r)
        }
        "change_type" => {
            let mut r = Vec::new();
            for (i, x) in objects(v, "columns")?.iter().enumerate() {
                allow(x, &["column", "type"]).map_err(|e| format!("columns[{i}]: {e}"))?;
                let t = str_param(x, "type").map_err(|e| format!("columns[{i}]: {e}"))?;
                let ty = ColumnType::from_name(t).ok_or_else(|| {
                    format!(
                        "columns[{i}]: unknown type `{t}` (expected {})",
                        ColumnType::ALL.map(ColumnType::name).join(", ")
                    )
                })?;
                r.push((
                    str_param(x, "column")
                        .map_err(|e| format!("columns[{i}]: {e}"))?
                        .to_string(),
                    ty,
                ));
            }
            Transform::ChangeType(r)
        }
        "promote_headers" => Transform::PromoteHeaders,
        "custom" => Transform::Custom,
        "sort" => {
            let mut r = Vec::new();
            for (i, x) in objects(v, "columns")?.iter().enumerate() {
                allow(x, &["column", "descending"]).map_err(|e| format!("columns[{i}]: {e}"))?;
                r.push((
                    str_param(x, "column")
                        .map_err(|e| format!("columns[{i}]: {e}"))?
                        .to_string(),
                    opt_bool(x, "descending")
                        .map_err(|e| format!("columns[{i}]: {e}"))?
                        .unwrap_or(false),
                ));
            }
            Transform::Sort(r)
        }
        "replace_values" => Transform::ReplaceValues {
            columns: list(v, "columns")?,
            find: literal(v.get("find"), "find")?,
            replace: literal(v.get("replace"), "replace")?,
        },
        "group_by" => {
            let mut aggs = Vec::new();
            for (i, x) in objects(v, "aggregations")?.iter().enumerate() {
                allow(x, &["name", "op", "column"]).map_err(|e| format!("aggregations[{i}]: {e}"))?;
                let op = str_param(x, "op").map_err(|e| format!("aggregations[{i}]: {e}"))?;
                let a = Aggregate::from_name(op).ok_or_else(|| {
                    format!(
                        "aggregations[{i}]: unknown `op` `{op}` (expected {})",
                        Aggregate::ALL.map(Aggregate::name).join(", ")
                    )
                })?;
                let column = opt_str(x, "column").map_err(|e| format!("aggregations[{i}]: {e}"))?;
                if a.takes_column() && column.is_none() {
                    return Err(format!("aggregations[{i}]: `{op}` needs a `column`"));
                }
                aggs.push((
                    str_param(x, "name")
                        .map_err(|e| format!("aggregations[{i}]: {e}"))?
                        .to_string(),
                    a,
                    column.map(str::to_string),
                ));
            }
            Transform::GroupBy {
                keys: list(v, "keys")?,
                aggregations: aggs,
            }
        }
        "merge" => {
            let other = str_param(v, "other")?.to_string();
            let join = match opt_str(v, "join")? {
                None => JoinKind::LeftOuter,
                Some(j) => JoinKind::from_name(j).ok_or_else(|| {
                    format!(
                        "unknown `join` `{j}` (expected {})",
                        JoinKind::ALL.map(JoinKind::name).join(", ")
                    )
                })?,
            };
            Transform::Merge {
                column: opt_str(v, "column")?.map_or_else(|| other.clone(), str::to_string),
                other,
                keys: list(v, "keys")?,
                other_keys: list(v, "other_keys")?,
                kind: join,
            }
        }
        "expand" => Transform::Expand {
            column: str_param(v, "column")?.to_string(),
            columns: list(v, "columns")?,
            prefix: opt_bool(v, "prefix")?.unwrap_or(true),
        },
        "append" => Transform::Append(list(v, "others")?),
        "add_column" => Transform::AddColumn {
            name: str_param(v, "name")?.to_string(),
            formula: str_param(v, "formula")?.to_string(),
        },
        "split_column" => Transform::SplitColumn {
            column: str_param(v, "column")?.to_string(),
            delimiter: str_param(v, "delimiter")?.to_string(),
            parts: match v.get("parts") {
                None | Some(Value::Null) => 2,
                Some(x) => x
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .filter(|n| (1..=bicraft_data::m::transform::MAX_PARTS).contains(n))
                    .ok_or_else(|| {
                        format!(
                            "`parts` must be a whole number from 1 to {}",
                            bicraft_data::m::transform::MAX_PARTS
                        )
                    })?,
            },
        },
        "unpivot" => Transform::Unpivot {
            columns: list(v, "columns")?,
            other: opt_bool(v, "other")?.unwrap_or(true),
        },
        "pivot" => Transform::Pivot {
            column: str_param(v, "column")?.to_string(),
            values: str_param(v, "values")?.to_string(),
            aggregate: match opt_str(v, "aggregate")? {
                None => None,
                Some(a) if a.eq_ignore_ascii_case("none") => None,
                Some(a) => Some(Aggregate::from_name(a).ok_or_else(|| {
                    format!(
                        "unknown `aggregate` `{a}` (expected none, {})",
                        Aggregate::ALL.map(Aggregate::name).join(", ")
                    )
                })?),
            },
        },
        _ => return Err(format!("unknown step kind `{kind}`")),
    })
}

/// `query.apply_step`: one transform added after a step.
pub(crate) fn query_apply_step(s: &Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &["name", "partition", "expression", "after", "step", "step_name"],
    )?;
    let (_, name, text) = query_text(s, p)?;
    let t = transform(p.get("step").ok_or("`step` ({kind, …}) is required")?)?;
    let st = steps::steps(&text, &limits())?;
    let after = match opt_str(p, "after")? {
        Some(a) => st
            .steps
            .get(st.find(a).ok_or_else(|| format!("the query has no step `{a}`"))?)
            .ok_or("internal: step index out of range")?,
        None => st.last().ok_or("the query has no steps")?,
    };
    let step_name = match opt_str(p, "step_name")? {
        Some(n) => {
            steps::check_name(n)?;
            n.to_string()
        }
        None => st.free_name(&t.step_name()),
    };
    let m = t.m(&after.name)?;
    let out = steps::insert_step(&text, Some(&after.name), &step_name, &m, &limits())?;
    let mut v = answer(s, &name, out, Some(&step_name))?;
    v["kind"] = json!(t.kind());
    Ok(v)
}

/// `query.edit_step`: a step's formula, or its name.
pub(crate) fn query_edit_step(s: &Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "name",
            "partition",
            "expression",
            "step",
            "formula",
            "new_name",
            "settings",
        ],
    )?;
    let (_, name, text) = query_text(s, p)?;
    let step = str_param(p, "step")?;
    let settings = p.get("settings").filter(|v| !v.is_null());
    let formula = match settings {
        Some(_) if p.get("formula").is_some_and(|v| !v.is_null()) => {
            return Err("give `formula` or `settings`, not both".into());
        }
        // the step settings gear: the step written anew from its settings, reading what it read
        Some(v) => Some(from_settings(&text, step, v)?),
        None => opt_str(p, "formula")?.map(str::to_string),
    };
    let new_name = opt_str(p, "new_name")?;
    if formula.is_none() && new_name.is_none() {
        return Err("query.edit_step needs `formula`, `settings` or `new_name`".into());
    }
    let formula = formula.as_deref();
    let mut out = text;
    let mut current = step.to_string();
    if let Some(f) = formula {
        if f.len() > MAX_EXPRESSION {
            return Err(format!("the formula is longer than {MAX_EXPRESSION} bytes"));
        }
        out = steps::set_step(&out, &current, f, &limits())?;
    }
    if let Some(n) = new_name {
        out = steps::rename_step(&out, &current, n, &limits())?;
        current = n.to_string();
    }
    answer(s, &name, out, Some(&current))
}

/// The M of step `step` written from `settings` (`{kind, …}` as `query.apply_step`'s `step`),
/// reading the step it read: the one its settings name, else the step before it.
fn from_settings(text: &str, step: &str, settings: &Value) -> Result<String, String> {
    let t = transform(settings)?;
    let st = steps::steps(text, &limits())?;
    let i = st
        .find(step)
        .ok_or_else(|| format!("the query has no step `{step}`"))?;
    let current = st.steps.get(i).ok_or("internal: step index out of range")?;
    let input = match bicraft_data::m::transform_parse::settings(&current.expression, &limits()) {
        Some((prev, _)) => prev,
        None => st
            .steps
            .get(
                i.checked_sub(1)
                    .ok_or_else(|| format!("step `{step}` reads no step before it"))?,
            )
            .ok_or("internal: step index out of range")?
            .name
            .clone(),
    };
    t.m(&input)
}

/// `query.remove_step`.
pub(crate) fn query_remove_step(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &["name", "partition", "expression", "step"])?;
    let (_, name, text) = query_text(s, p)?;
    let step = str_param(p, "step")?;
    let out = steps::remove_step(&text, step, &limits())?;
    answer(s, &name, out, None)
}

/// The M of query `name` at step `step` (`query.preview {step}`).
pub(crate) fn at_step(text: &str, step: &str) -> Result<String, String> {
    steps::at_step(text, step, &limits())
}

/// The M of a blank query (Home › New source › Blank query).
pub const BLANK: &str = "let\n    Source = \"\"\nin\n    Source";

/// The names a new or renamed query may not take: `names` (the editor's queries, its drafts
/// included) when given, else the model's queries; and the model's tables that no M query loads
/// (calculated tables), whose names queries share.
fn taken_names(s: &Session, p: &Value) -> Result<Vec<String>, String> {
    let model = s.model.as_ref().ok_or("no model is open (file.open first)")?;
    let mut out: Vec<String> = match p.get("names") {
        None | Some(Value::Null) => model
            .tables
            .iter()
            .map(|t| t.name.clone())
            .chain(model.expressions.iter().map(|e| e.name.clone()))
            .collect(),
        Some(_) => list(p, "names")?,
    };
    out.extend(
        model
            .tables
            .iter()
            .filter(|t| !t.partitions.iter().any(|x| is_m(&x.source_type)))
            .map(|t| t.name.clone()),
    );
    Ok(out)
}

fn is_taken(taken: &[String], name: &str) -> bool {
    taken.iter().any(|t| t.eq_ignore_ascii_case(name))
}

/// The editor's name for a new query: `Query1`, `Query2`… (blank), `Sales (2)`, `Sales (3)`…
/// (a duplicate or a reference of `Sales`).
fn free_query_name(taken: &[String], of: Option<&str>) -> Result<String, String> {
    for n in 1..=10_000u32 {
        let name = match of {
            None => format!("Query{n}"),
            Some(o) => format!("{o} ({})", n.saturating_add(1)),
        };
        if !is_taken(taken, &name) {
            return Ok(name);
        }
    }
    Err("no free query name".into())
}

/// `query.new`: a new query's name and M for the editor's draft (Blank query, Duplicate,
/// Reference). Nothing in the model changes; `query.apply` loads it.
pub(crate) fn query_new(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &["kind", "of", "expression", "names"])?;
    let kind = opt_str(p, "kind")?.unwrap_or("blank");
    let taken = taken_names(s, p)?;
    let (name, m) = match kind {
        "blank" => {
            if p.get("of").is_some_and(|v| !v.is_null()) {
                return Err("a blank query is not made `of` another".into());
            }
            (free_query_name(&taken, None)?, BLANK.to_string())
        }
        "duplicate" | "reference" => {
            let of = str_param(p, "of")?;
            check_name("query", of)?;
            let m = if kind == "duplicate" {
                match opt_str(p, "expression")? {
                    Some(x) => x.to_string(),
                    None => crate::query_preview::source(s, &json!({"name": of}))?.2,
                }
            } else {
                if p.get("expression").is_some_and(|v| !v.is_null()) {
                    return Err("a reference reads the query by name: no `expression`".into());
                }
                format!(
                    "let\n    Source = {}\nin\n    Source",
                    bicraft_data::m::transform::ident(of)
                )
            };
            if m.len() > MAX_EXPRESSION {
                return Err(format!("the query is longer than {MAX_EXPRESSION} bytes"));
            }
            // the text must be M
            steps::steps(&m, &limits())?;
            (free_query_name(&taken, Some(of))?, m)
        }
        other => {
            return Err(format!(
                "unknown `kind` `{other}` (expected blank, duplicate or reference)"
            ));
        }
    };
    Ok(json!({"name": name, "expression": m, "kind": kind}))
}

/// `query.rename`: a query renamed in the editor's drafts, the references of the other drafts
/// following. Nothing in the model changes; `query.apply {renamed}` renames it there.
pub(crate) fn query_rename(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &["name", "new_name", "queries"])?;
    let name = str_param(p, "name")?;
    let new_name = str_param(p, "new_name")?;
    check_name("query", new_name)?;
    let drafts = objects(p, "queries")?;
    if drafts.len() > MAX_QUERIES * 10 {
        return Err("`queries` holds too many queries".into());
    }
    let mut parsed: Vec<(String, String)> = Vec::with_capacity(drafts.len());
    for (i, q) in drafts.iter().enumerate() {
        allow(q, &["name", "expression"]).map_err(|e| format!("queries[{i}]: {e}"))?;
        let n = str_param(q, "name").map_err(|e| format!("queries[{i}]: {e}"))?;
        let m = str_param(q, "expression").map_err(|e| format!("queries[{i}]: {e}"))?;
        if m.len() > MAX_EXPRESSION {
            return Err(format!(
                "queries[{i}]: the query is longer than {MAX_EXPRESSION} bytes"
            ));
        }
        parsed.push((n.to_string(), m.to_string()));
    }
    if !parsed.iter().any(|(n, _)| n == name) {
        return Err(format!("`queries` holds no query `{name}`"));
    }
    let mut taken: Vec<String> = parsed
        .iter()
        .filter(|(n, _)| n != name)
        .map(|(n, _)| n.clone())
        .collect();
    taken.extend(taken_names(s, &json!({"names": []}))?);
    if !name.eq_ignore_ascii_case(new_name) && is_taken(&taken, new_name) {
        return Err(format!("a query or table named `{new_name}` already exists"));
    }
    let mut changed = Vec::new();
    let mut not_rewritten = Vec::new();
    if name != new_name {
        for (n, m) in &parsed {
            match crate::partition::rewrite(m, name, new_name) {
                Some((_, 0)) => {}
                Some((out, k)) => {
                    let n = if n == name { new_name } else { n.as_str() };
                    changed.push(json!({"name": n, "expression": out, "references": k}));
                }
                None => not_rewritten.push(json!(n)),
            }
        }
    }
    Ok(json!({"renamed": name, "name": new_name, "queries": changed,
        "not_rewritten": not_rewritten}))
}

/// One query of `query.apply`.
struct Edited {
    name: String,
    m: String,
}

/// The `renamed` of `query.apply`: (model name, new name), in order.
fn renames(p: &Value) -> Result<Vec<(String, String)>, String> {
    let Some(v) = p.get("renamed").filter(|v| !v.is_null()) else {
        return Ok(Vec::new());
    };
    let a = v
        .as_array()
        .filter(|a| a.len() <= MAX_QUERIES)
        .ok_or_else(|| format!("`renamed` must be a list of at most {MAX_QUERIES} {{name, new_name}}"))?;
    let mut out = Vec::with_capacity(a.len());
    for (i, x) in a.iter().enumerate() {
        allow(x, &["name", "new_name"]).map_err(|e| format!("renamed[{i}]: {e}"))?;
        let from = str_param(x, "name").map_err(|e| format!("renamed[{i}]: {e}"))?;
        let to = str_param(x, "new_name").map_err(|e| format!("renamed[{i}]: {e}"))?;
        check_name("query", to).map_err(|e| format!("renamed[{i}]: {e}"))?;
        out.push((from.to_string(), to.to_string()));
    }
    Ok(out)
}

/// Renames model query or table `from` to `to`; the M of the other queries follows.
fn rename_query(s: &mut Session, from: &str, to: &str) -> Result<Value, String> {
    let (kind, actual) = target(s, from)?;
    match kind {
        Target::New => Err(format!("the model has no query `{from}` to rename")),
        Target::Query => {
            crate::partition::expression_rename(s, &json!({"name": actual, "new_name": to, "update_m": true}))
        }
        Target::Table { .. } => {
            if let Some(m) = s.model.as_ref()
                && m.expressions.iter().any(|e| e.name.eq_ignore_ascii_case(to))
            {
                return Err(format!("a shared expression named `{to}` already exists"));
            }
            let v = crate::edit::table_rename(s, &json!({"name": actual, "new_name": to}))?;
            if let Some(m) = s.model.as_mut() {
                crate::partition::follow_rename(m, &actual, to);
            }
            s.dirty = true;
            Ok(v)
        }
    }
}

/// Removes model query or table `name`, refused while another M query reads it.
fn remove_query(s: &mut Session, name: &str) -> Result<(), String> {
    let (kind, actual) = target(s, name)?;
    match kind {
        Target::New => Err(format!("the model has no query `{name}` to delete")),
        Target::Query => crate::partition::expression_remove(s, &json!({"name": actual})).map(|_| ()),
        Target::Table { .. } => {
            let users = s
                .model
                .as_ref()
                .map(|m| crate::partition::m_users(m, &actual))
                .unwrap_or_default();
            let users: Vec<String> = users
                .iter()
                .filter(|u| u["table"] != json!(actual))
                .map(|u| {
                    u["table"]
                        .as_str()
                        .or(u["name"].as_str())
                        .unwrap_or_default()
                        .to_string()
                })
                .collect();
            if !users.is_empty() {
                return Err(format!(
                    "query `{actual}` is read by {}: change those queries first",
                    users.join(", ")
                ));
            }
            crate::model_props::table_remove(s, &json!({"name": actual})).map(|_| ())
        }
    }
}

/// `query.apply` (Close & Apply): the queries written back, their tables reloaded.
pub(crate) fn query_apply(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["queries", "renamed", "removed", "data_root", "refresh"])?;
    let renamed = renames(p)?;
    let removed = list(p, "removed")?;
    if removed.len() > MAX_QUERIES {
        return Err(format!("`removed` holds at most {MAX_QUERIES} queries"));
    }
    let empty = Vec::new();
    let list = match p.get("queries") {
        None | Some(Value::Null) if !(renamed.is_empty() && removed.is_empty()) => &empty,
        other => other
            .and_then(Value::as_array)
            .ok_or("`queries` must be a list of {name, expression}")?,
    };
    if (list.is_empty() && renamed.is_empty() && removed.is_empty()) || list.len() > MAX_QUERIES {
        return Err(format!(
            "`queries` holds 1 to {MAX_QUERIES} queries (or name `renamed` or `removed` ones)"
        ));
    }
    let refresh = opt_bool(p, "refresh")?.unwrap_or(true);
    let mut edited: Vec<Edited> = Vec::with_capacity(list.len());
    for (i, q) in list.iter().enumerate() {
        allow(q, &["name", "expression"]).map_err(|e| format!("queries[{i}]: {e}"))?;
        let name = str_param(q, "name").map_err(|e| format!("queries[{i}]: {e}"))?;
        let m = str_param(q, "expression").map_err(|e| format!("queries[{i}]: {e}"))?;
        if m.len() > MAX_EXPRESSION {
            return Err(format!(
                "queries[{i}]: the query is longer than {MAX_EXPRESSION} bytes"
            ));
        }
        if m.trim().is_empty() {
            return Err(format!("queries[{i}]: the query is empty"));
        }
        if edited.iter().any(|e| e.name.eq_ignore_ascii_case(name)) {
            return Err(format!("query `{name}` is named twice"));
        }
        edited.push(Edited {
            name: name.to_string(),
            m: m.to_string(),
        });
    }
    if let Some(r) = removed
        .iter()
        .find(|r| edited.iter().any(|e| e.name.eq_ignore_ascii_case(r)))
    {
        return Err(format!("query `{r}` is both written and deleted"));
    }
    if s.model.is_none() {
        return Err("no model is open (file.open first)".into());
    }
    let saved = (
        s.model.clone(),
        s.model_error.clone(),
        s.dirty,
        s.data_root.clone(),
    );
    let run = |s: &mut Session| -> Result<Value, String> {
        // renamed first: the queries and deletions name the new names
        let mut done = Vec::new();
        for (from, to) in &renamed {
            rename_query(s, from, to).map_err(|e| format!("renaming `{from}`: {e}"))?;
            done.push(json!({"name": from, "new_name": to}));
        }
        let mut v = if edited.is_empty() {
            json!({"queries": [], "refreshed": refresh, "warnings": []})
        } else {
            apply_all(s, p, &edited, refresh)?
        };
        // deleted last: the queries that read them may have been changed above
        for r in &removed {
            remove_query(s, r).map_err(|e| format!("deleting `{r}`: {e}"))?;
        }
        v["renamed"] = json!(done);
        v["removed"] = json!(removed);
        Ok(v)
    };
    match run(s) {
        Ok(v) => Ok(v),
        Err(e) => {
            (s.model, s.model_error, s.dirty, s.data_root) = saved;
            Err(e)
        }
    }
}

#[derive(PartialEq, Eq)]
enum Target {
    Query,
    Table { partitions: usize },
    New,
}

fn target(s: &Session, name: &str) -> Result<(Target, String), String> {
    let m = s.model.as_ref().ok_or("no model is open (file.open first)")?;
    if let Some(e) = m.expressions.iter().find(|e| e.name.eq_ignore_ascii_case(name)) {
        if !(e.kind.is_empty() || e.kind.eq_ignore_ascii_case("m")) {
            return Err(format!("query `{}` is not Power Query (M)", e.name));
        }
        return Ok((Target::Query, e.name.clone()));
    }
    if let Some(t) = m.tables.iter().find(|t| t.name.eq_ignore_ascii_case(name)) {
        let ms = t.partitions.iter().filter(|x| is_m(&x.source_type)).count();
        if ms == 0 {
            return Err(format!(
                "table `{}` is not loaded by a Power Query (M) query",
                t.name
            ));
        }
        return Ok((
            Target::Table {
                partitions: t.partitions.len(),
            },
            t.name.clone(),
        ));
    }
    Ok((Target::New, name.to_string()))
}

fn apply_all(s: &mut Session, p: &Value, edited: &[Edited], refresh: bool) -> Result<Value, String> {
    let mut results = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut reload: Vec<String> = Vec::new();
    let mut changed_queries: Vec<String> = Vec::new();
    // the shared queries first: the tables' reload reads them
    for e in edited {
        let (kind, actual) = target(s, &e.name)?;
        if kind != Target::Query {
            continue;
        }
        let same = s
            .model
            .as_ref()
            .and_then(|m| m.expressions.iter().find(|x| x.name == actual))
            .is_some_and(|x| x.expression == e.m);
        if same {
            continue;
        }
        if refresh {
            // the tables that read it are reloaded below, so their rows follow the new text
            if let Some(m) = s.model.as_ref() {
                check_edit(
                    &format!("shared expression `{actual}`"),
                    &e.m,
                    &query_names(m),
                    false,
                )?;
            }
            if let Some(x) = s
                .model
                .as_mut()
                .and_then(|m| m.expressions.iter_mut().find(|x| x.name == actual))
            {
                x.expression.clone_from(&e.m);
            }
            s.dirty = true;
        } else {
            crate::partition::expression_set(s, &json!({"name": actual, "expression": e.m}))?;
        }
        changed_queries.push(actual.clone());
        results.push(json!({"name": actual, "source": "query", "changed": true}));
    }
    for e in edited {
        let (kind, actual) = target(s, &e.name)?;
        match kind {
            Target::Query => {}
            Target::Table { partitions } => {
                let current = s
                    .model
                    .as_ref()
                    .and_then(|m| m.tables.iter().find(|t| t.name == actual))
                    .and_then(|t| t.partitions.iter().find(|x| is_m(&x.source_type)))
                    .and_then(|x| x.expression.clone());
                if current.as_deref() == Some(e.m.as_str()) {
                    continue;
                }
                if refresh && partitions == 1 {
                    reload.push(actual.clone());
                    results.push(json!({"name": actual, "source": "table", "changed": true}));
                    // the text is written by the reload
                    set_partition_text(s, &actual, &e.m)?;
                } else {
                    let mut call = json!({"table": actual, "expression": e.m});
                    if let Some(part) = s
                        .model
                        .as_ref()
                        .and_then(|m| m.tables.iter().find(|t| t.name == actual))
                        .and_then(|t| t.partitions.iter().find(|x| is_m(&x.source_type)))
                    {
                        call["partition"] = json!(part.name);
                    }
                    crate::partition::partition_set(s, &call)?;
                    if refresh {
                        warnings.push(format!(
                            "table `{actual}` has {partitions} partitions: its query is written, its rows are not reloaded"
                        ));
                    }
                    results
                        .push(json!({"name": actual, "source": "table", "changed": true, "reloaded": false}));
                }
            }
            Target::New => {
                let mut call = json!({"name": actual, "m": e.m});
                if let Some(r) = p.get("data_root") {
                    call["data_root"] = r.clone();
                }
                let v = crate::pbix_struct::import(s, &call, false, false)
                    .map_err(|err| format!("query `{actual}`: {err}"))?;
                results.push(json!({"name": actual, "source": "table", "new": true, "rows": v["rows"], "columns": v["columns"]}));
            }
        }
    }
    if refresh {
        // tables that read an edited shared query
        if !changed_queries.is_empty()
            && let Some(m) = s.model.as_ref()
        {
            for t in &m.tables {
                if reload.iter().any(|r| r == &t.name) || t.partitions.len() != 1 {
                    continue;
                }
                let uses = t.partitions.iter().any(|x| {
                    is_m(&x.source_type)
                        && x.expression.as_deref().is_some_and(|text| {
                            changed_queries
                                .iter()
                                .any(|q| crate::partition::reaches(m, text, q))
                        })
                });
                if uses {
                    reload.push(t.name.clone());
                }
            }
        }
        for name in &reload {
            let v = reload_table(s, p, name).map_err(|err| format!("table `{name}`: {err}"))?;
            if let Some(r) = results.iter_mut().find(|r| r["name"] == json!(name)) {
                r["reloaded"] = json!(true);
                r["rows"] = v["rows"].clone();
                r["columns"] = v["columns"].clone();
            } else {
                results.push(
                    json!({"name": name, "source": "table", "changed": false, "reloaded": true,
                    "rows": v["rows"], "columns": v["columns"]}),
                );
            }
            if let Some(w) = v.get("warnings").and_then(Value::as_array) {
                warnings.extend(
                    w.iter()
                        .filter_map(Value::as_str)
                        .map(|x| format!("table `{name}`: {x}")),
                );
            }
        }
    }
    Ok(json!({"queries": results, "refreshed": refresh, "warnings": warnings}))
}

/// Writes `m` as table `name`'s M partition text (the reload checks and loads it).
fn set_partition_text(s: &mut Session, name: &str, m: &str) -> Result<(), String> {
    let t = s
        .model
        .as_mut()
        .and_then(|x| x.tables.iter_mut().find(|t| t.name == name))
        .ok_or_else(|| format!("no table named `{name}`"))?;
    let part = t
        .partitions
        .iter_mut()
        .find(|x| is_m(&x.source_type))
        .ok_or_else(|| format!("table `{name}` has no M partition"))?;
    part.expression = Some(m.to_string());
    s.dirty = true;
    Ok(())
}

/// Reloads table `name` from its (only) M partition: its columns follow the query's, its
/// partition keeps its name, mode and properties.
fn reload_table(s: &mut Session, p: &Value, name: &str) -> Result<Value, String> {
    let old = s
        .model
        .as_ref()
        .and_then(|x| x.tables.iter().find(|t| t.name == name))
        .and_then(|t| t.partitions.first().cloned())
        .ok_or_else(|| format!("no table named `{name}`"))?;
    let m = old
        .expression
        .clone()
        .ok_or_else(|| format!("table `{name}` has no query"))?;
    let mut call = json!({"name": name, "m": m});
    if let Some(r) = p.get("data_root") {
        call["data_root"] = r.clone();
    }
    let v = crate::pbix_struct::import(s, &call, true, true)?;
    if let Some(t) = s
        .model
        .as_mut()
        .and_then(|x| x.tables.iter_mut().find(|t| t.name == name))
    {
        t.partitions = vec![old];
    }
    Ok(v)
}
