//! `query.dependencies`: the Power Query editor's View › Query dependencies (docs/mcp.md "Power
//! Query editor"): a graph of the queries, the data sources they read and the model tables they
//! load into, as Microsoft Learn's "Query dependencies" page describes the view.
//!
//! The queries are the model's (tables loaded by an M partition, shared queries and parameters),
//! or the editor's drafts (`queries`). Nothing is evaluated: references are the free names of
//! each query's M (`bicraft_data::m::analyze_all`) that name another query, sources the source
//! function calls of `bicraft_data::m::sources` (a file, a server, a URL), so a query that does
//! not parse has no edges and is listed in `unparsed`. Each node has a layer (sources first,
//! then each query after the queries it reads, the tables last), for a left-to-right drawing.

use std::collections::{BTreeMap, HashMap};

use bicraft_data::Limits;
use bicraft_data::m::analyze_all;
use bicraft_data::m::sources::{Form, Part, source_calls};
use serde_json::{Value, json};

use crate::Session;
use crate::mquery::is_m;
use crate::params::{allow, opt_bool, str_param};

/// Most queries in one graph.
const MAX_QUERIES: usize = 2_000;
/// Longest M text, in bytes.
const MAX_EXPRESSION: usize = 1 << 20;
/// Longest source label.
const MAX_LABEL: usize = 200;

/// One query of the graph.
struct Q {
    name: String,
    m: String,
    /// `table` (loaded into the model), `query` (a shared query) or `parameter`.
    kind: &'static str,
}

fn is_parameter(m: &str) -> bool {
    m.contains("IsParameterQuery=true") || m.contains("IsParameterQuery = true")
}

/// The model's queries.
fn model_queries(s: &Session) -> Result<Vec<Q>, String> {
    let model = s.model.as_ref().ok_or("no model is open (file.open first)")?;
    let mut out = Vec::new();
    for t in &model.tables {
        if let Some(m) = t
            .partitions
            .iter()
            .find(|x| is_m(&x.source_type))
            .and_then(|x| x.expression.clone())
        {
            out.push(Q {
                name: t.name.clone(),
                m,
                kind: "table",
            });
        }
    }
    for e in &model.expressions {
        if e.kind.is_empty() || e.kind.eq_ignore_ascii_case("m") {
            out.push(Q {
                name: e.name.clone(),
                m: e.expression.clone(),
                kind: if is_parameter(&e.expression) {
                    "parameter"
                } else {
                    "query"
                },
            });
        }
    }
    Ok(out)
}

/// The editor's drafts: `[{name, expression, load?}]` (load: default true for a table of the
/// model or a new query, false for a shared query of the model).
fn drafts(s: &Session, v: &Value) -> Result<Vec<Q>, String> {
    let a = v
        .as_array()
        .filter(|a| a.len() <= MAX_QUERIES)
        .ok_or_else(|| format!("`queries` must be a list of at most {MAX_QUERIES} {{name, expression}}"))?;
    let model = s.model.as_ref();
    let mut out: Vec<Q> = Vec::with_capacity(a.len());
    for (i, q) in a.iter().enumerate() {
        allow(q, &["name", "expression", "load"]).map_err(|e| format!("queries[{i}]: {e}"))?;
        let name = str_param(q, "name").map_err(|e| format!("queries[{i}]: {e}"))?;
        let m = str_param(q, "expression").map_err(|e| format!("queries[{i}]: {e}"))?;
        if m.len() > MAX_EXPRESSION {
            return Err(format!(
                "queries[{i}]: the query is longer than {MAX_EXPRESSION} bytes"
            ));
        }
        if out.iter().any(|x| x.name == name) {
            return Err(format!("query `{name}` is named twice"));
        }
        let shared = model.is_some_and(|m| m.expressions.iter().any(|e| e.name == name));
        let load = opt_bool(q, "load")
            .map_err(|e| format!("queries[{i}]: {e}"))?
            .unwrap_or(!shared);
        out.push(Q {
            name: name.to_string(),
            m: m.to_string(),
            kind: if is_parameter(m) {
                "parameter"
            } else if load {
                "table"
            } else {
                "query"
            },
        });
    }
    Ok(out)
}

/// A source call's label: its function and the text of its leading arguments.
fn source_label(function: &str, args: &[bicraft_data::m::sources::Arg]) -> String {
    let parts: Vec<String> = args
        .iter()
        .map(|a| match &a.form {
            Form::Text(t) | Form::StepText(_, t) => t.value.clone(),
            Form::Name { name, .. } => name.clone(),
            Form::Concat(ps) => ps
                .iter()
                .map(|p| match p {
                    Part::Text(t) => t.value.clone(),
                    Part::Name(n, _, _) => format!("[{n}]"),
                })
                .collect(),
            Form::Call(f) => format!("{f}(…)"),
            Form::Computed => "…".into(),
        })
        .collect();
    let text = if parts.is_empty() {
        function.to_string()
    } else {
        format!("{function}: {}", parts.join(", "))
    };
    if text.chars().count() > MAX_LABEL {
        let mut t: String = text.chars().take(MAX_LABEL).collect();
        t.push('…');
        t
    } else {
        text
    }
}

/// `query.dependencies`; see the module docs.
pub(crate) fn query_dependencies(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &["queries"])?;
    let qs = match p.get("queries") {
        None | Some(Value::Null) => model_queries(s)?,
        Some(v) => drafts(s, v)?,
    };
    if qs.len() > MAX_QUERIES {
        return Err(format!("the graph holds at most {MAX_QUERIES} queries"));
    }
    let analyses = analyze_all(qs.iter().map(|q| q.m.clone()).collect(), &Limits::default());
    let index: HashMap<&str, usize> = qs.iter().enumerate().map(|(i, q)| (q.name.as_str(), i)).collect();
    let mut nodes: Vec<Value> = Vec::new();
    let mut edges: Vec<Value> = Vec::new();
    let mut unparsed: Vec<Value> = Vec::new();
    // sources by label, in the order first met
    let mut sources: BTreeMap<String, (usize, String)> = BTreeMap::new();
    let mut source_order: Vec<String> = Vec::new();
    // query i reads queries reads[i]
    let mut reads: Vec<Vec<usize>> = vec![Vec::new(); qs.len()];
    let mut source_edges: Vec<(String, usize)> = Vec::new();
    for (i, (q, a)) in qs.iter().zip(&analyses).enumerate() {
        if let Some(e) = &a.syntax {
            unparsed.push(json!({"name": q.name, "error": e.message, "line": e.line, "column": e.column}));
            continue;
        }
        for r in &a.references {
            if let Some(&j) = index.get(r.name.as_str())
                && j != i
                && let Some(list) = reads.get_mut(i)
                && !list.contains(&j)
            {
                list.push(j);
            }
        }
        let calls = source_calls(&q.m).unwrap_or_default();
        for c in &calls {
            // a format reading another source call (`Csv.Document(File.Contents(…))`): the inner
            // call is the source
            let wraps = c.args.iter().any(|a| {
                matches!(&a.form, Form::Call(f) if calls.iter().any(|o| &o.function == f && o.start > c.start && o.end <= c.end))
            });
            if wraps {
                continue;
            }
            let label = source_label(&c.function, &c.args);
            if !sources.contains_key(&label) {
                sources.insert(label.clone(), (source_order.len(), c.function.clone()));
                source_order.push(label.clone());
            }
            if !source_edges.iter().any(|(l, k)| l == &label && *k == i) {
                source_edges.push((label, i));
            }
        }
    }
    // layers: a query after every query it reads (cycles are cut by the pass count)
    let mut layer: Vec<usize> = qs
        .iter()
        .enumerate()
        .map(|(i, _)| usize::from(source_edges.iter().any(|(_, k)| *k == i)))
        .collect();
    for _ in 0..qs.len().min(64) {
        let mut changed = false;
        for i in 0..qs.len() {
            let want = reads
                .get(i)
                .into_iter()
                .flatten()
                .filter_map(|j| layer.get(*j))
                .map(|l| l.saturating_add(1))
                .max()
                .unwrap_or(0);
            if let Some(l) = layer.get_mut(i)
                && want > *l
            {
                *l = want;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    // sources in layer 0: queries start at 1 when anything has a source
    let base = usize::from(!sources.is_empty());
    let top = layer.iter().map(|l| l.saturating_add(base)).max().unwrap_or(0);
    for label in &source_order {
        let function = sources.get(label).map(|x| x.1.clone()).unwrap_or_default();
        nodes.push(
            json!({"id": format!("source:{label}"), "kind": "source", "name": label,
            "function": function, "layer": 0}),
        );
    }
    for (i, q) in qs.iter().enumerate() {
        let l = layer.get(i).copied().unwrap_or(0).saturating_add(base);
        let l = if base == 1 { l.max(1) } else { l };
        nodes.push(
            json!({"id": format!("query:{}", q.name), "kind": q.kind, "name": q.name,
            "loaded": q.kind == "table", "layer": l}),
        );
    }
    let table_layer = top.saturating_add(1);
    for q in qs.iter().filter(|q| q.kind == "table") {
        nodes.push(
            json!({"id": format!("table:{}", q.name), "kind": "model_table", "name": q.name,
            "layer": table_layer}),
        );
    }
    for (label, i) in &source_edges {
        if let Some(q) = qs.get(*i) {
            edges.push(json!({"from": format!("source:{label}"), "to": format!("query:{}", q.name), "kind": "source"}));
        }
    }
    for (i, list) in reads.iter().enumerate() {
        for j in list {
            if let (Some(q), Some(o)) = (qs.get(i), qs.get(*j)) {
                edges.push(json!({"from": format!("query:{}", o.name), "to": format!("query:{}", q.name), "kind": "reference"}));
            }
        }
    }
    for q in qs.iter().filter(|q| q.kind == "table") {
        edges.push(
            json!({"from": format!("query:{}", q.name), "to": format!("table:{}", q.name), "kind": "load"}),
        );
    }
    Ok(json!({
        "nodes": nodes,
        "edges": edges,
        "layers": table_layer.saturating_add(1),
        "unparsed": unparsed,
    }))
}
