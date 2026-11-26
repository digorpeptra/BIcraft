//! M9.7: Power Query (M) without evaluation.
//!
//! - `m.validate`: syntax and undefined names of an M expression (`bicraft_data::m::analyze`);
//! - `m.format`: an expression or a query laid out by `bicraft_data::m::format` (M11.35);
//! - the M part of `model.validate`: every M partition and shared expression;
//! - `PBI_QueryOrder`, the model annotation in which Power BI Desktop lists its queries (the
//!   tables loaded by an M query, then the shared expressions and parameters, in the Power Query
//!   editor's order): kept up to date after every command that adds, removes or renames a query
//!   ([`QuerySnapshot`]), like Desktop: new queries at the end, removed ones dropped, a rename in
//!   place.

use std::collections::HashSet;

use bicraft_data::Limits;
use bicraft_data::m::{Analysis, FormatOptions, LibraryName, MAX_INDENT_WIDTH, analyze_all, library_name};
use bicraft_model::{Annotation, SemanticModel};
use serde_json::{Value, json};

use crate::Session;
use crate::params::{allow, opt_bool, opt_str, str_param};

/// The model annotation listing the queries.
pub(crate) const QUERY_ORDER: &str = "PBI_QueryOrder";

/// Whether a partition (TMSL `source.type`) or expression (`kind`) holds M.
pub(crate) fn is_m(source_type: &str) -> bool {
    source_type.eq_ignore_ascii_case("m")
}

/// The model's query names in Desktop's sense, in model order: tables with an M partition,
/// then the M shared expressions (parameters included); with their lineage tags.
fn queries(m: &SemanticModel) -> Vec<(String, Option<String>)> {
    let tables = m
        .tables
        .iter()
        .filter(|t| t.partitions.iter().any(|p| is_m(&p.source_type)))
        .map(|t| (t.name.clone(), t.lineage_tag.clone()));
    let expressions = m
        .expressions
        .iter()
        .filter(|e| is_m(&e.kind))
        .map(|e| (e.name.clone(), e.lineage_tag.clone()));
    tables.chain(expressions).collect()
}

/// Names an M expression of the model may use for another query: the tables loaded by M and
/// the shared expressions.
pub(crate) fn query_names(m: &SemanticModel) -> HashSet<String> {
    queries(m).into_iter().map(|(n, _)| n).collect()
}

// ---------------------------------------------------------------------------------------------
// Diagnostics
// ---------------------------------------------------------------------------------------------

/// Diagnostics of one analyzed M expression: the syntax error, undefined names (errors) and
/// unknown library functions (warnings). `known` are the other queries and parameters.
pub(crate) fn diagnostics(a: &Analysis, known: &HashSet<String>) -> Vec<Value> {
    let mut out = Vec::new();
    if let Some(e) = &a.syntax {
        out.push(json!({
            "severity": "error",
            "code": "m-syntax",
            "message": e.message,
            "line": e.line,
            "column": e.column,
        }));
        return out;
    }
    let mut seen = HashSet::new();
    for r in &a.references {
        if known.contains(&r.name) || !seen.insert(r.name.as_str()) {
            continue;
        }
        let library = r.name.contains('.') || r.name.starts_with('#');
        // `#"Sales.Fact"` names a query, never a library function.
        let quoted = r.span.is_some_and(|(s, e)| e.saturating_sub(s) != r.name.len());
        let (severity, code, message) = match library_name(&r.name) {
            LibraryName::Evaluated | LibraryName::Documented | LibraryName::Connector => continue,
            LibraryName::Unknown if library && !quoted => (
                "warning",
                "m-unknown-function",
                format!(
                    "`{}` is not a Power Query library function bicraft knows (nor a query or \
                     parameter of the model)",
                    r.name
                ),
            ),
            LibraryName::Unknown if r.name == "_" && r.line.is_none() => (
                "error",
                "m-undefined-name",
                "a `[field]` access outside `each` (or a function of `_`) has no record to read".to_string(),
            ),
            LibraryName::Unknown => (
                "error",
                "m-undefined-name",
                format!(
                    "`{}` is not defined: not a step of this query, a query or parameter of the \
                     model, nor a library function",
                    r.name
                ),
            ),
        };
        let mut d = json!({"severity": severity, "code": code, "message": message, "name": r.name});
        if let (Some(line), Some(column)) = (r.line, r.column) {
            d["line"] = json!(line);
            d["column"] = json!(column);
        }
        if let Some((start, end)) = r.span {
            d["span"] = json!({"start": start, "end": end});
        }
        out.push(d);
    }
    out
}

fn has_error(diags: &[Value]) -> bool {
    diags
        .iter()
        .any(|d| d.get("severity").and_then(Value::as_str) == Some("error"))
}

/// `m.validate {expression, shared_expressions?, names?}`.
pub(crate) fn m_validate(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &["expression", "shared_expressions", "names"])?;
    let expression = str_param(p, "expression")?;
    let use_model = opt_bool(p, "shared_expressions")?.unwrap_or(true);
    let mut known = HashSet::new();
    let model = if use_model { s.model.as_ref() } else { None };
    if let Some(m) = model {
        known = query_names(m);
    }
    match p.get("names") {
        None | Some(Value::Null) => {}
        Some(Value::Array(a)) => {
            for n in a {
                let n = n.as_str().ok_or("parameter `names` must be a list of strings")?;
                known.insert(n.to_string());
            }
        }
        Some(_) => return Err("parameter `names` must be a list of strings".into()),
    }
    let a = analyze_all(vec![expression.to_string()], &Limits::default())
        .into_iter()
        .next()
        .unwrap_or_default();
    let diags = diagnostics(&a, &known);
    let mut queries = Vec::new();
    let mut library = Vec::new();
    let mut seen = HashSet::new();
    for r in &a.references {
        if !seen.insert(r.name.as_str()) {
            continue;
        }
        if known.contains(&r.name) {
            queries.push(json!(r.name));
            continue;
        }
        let kind = match library_name(&r.name) {
            LibraryName::Evaluated => "evaluated",
            LibraryName::Documented => "documented",
            LibraryName::Connector => "connector",
            LibraryName::Unknown => continue,
        };
        library.push(json!({"name": r.name, "kind": kind}));
    }
    Ok(json!({
        "valid": !has_error(&diags),
        "syntax_valid": a.syntax.is_none(),
        "checked_against_model": model.is_some(),
        "diagnostics": diags,
        "steps": a.steps,
        "result_type": a.result_type,
        "references": {"queries": queries, "library": library},
    }))
}

// ---------------------------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------------------------

/// Widest `max_width` and `indent` `m.format` takes.
const MAX_FORMAT_WIDTH: u64 = 10_000;

fn opt_count(p: &Value, key: &str, max: u64) -> Result<Option<usize>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => match v.as_u64() {
            Some(n) if n <= max => usize::try_from(n).map(Some).map_err(|e| e.to_string()),
            _ => Err(format!(
                "parameter `{key}` must be a whole number from 0 to {max}"
            )),
        },
    }
}

/// The formatted text of `m.format`'s query, with where it came from.
struct Formatted {
    answer: Value,
    source: &'static str,
    name: String,
    text: String,
    changed: bool,
}

fn m_format_text(s: &Session, p: &Value) -> Result<Formatted, String> {
    allow(
        p,
        &[
            "expression",
            "name",
            "partition",
            "apply",
            "max_width",
            "indent",
            "tabs",
        ],
    )?;
    let (source, name, text) = crate::query_preview::source(s, p)?;
    let defaults = FormatOptions::default();
    let opts = FormatOptions {
        max_width: opt_count(p, "max_width", MAX_FORMAT_WIDTH)?.unwrap_or(defaults.max_width),
        indent_width: opt_count(p, "indent", MAX_INDENT_WIDTH as u64)?.unwrap_or(defaults.indent_width),
        use_tabs: opt_bool(p, "tabs")?.unwrap_or(false),
    };
    let formatted = bicraft_data::m::format(&text, &opts, &Limits::default()).map_err(|e| match e {
        bicraft_data::m::FormatError::Invalid(e) => format!("the M does not parse: {e}"),
        other => other.to_string(),
    })?;
    let changed = formatted != text;
    Ok(Formatted {
        answer: json!({
            "name": name,
            "source": source,
            "expression": formatted,
            "changed": changed,
            "applied": false,
        }),
        source,
        name,
        text: formatted,
        changed,
    })
}

/// `m.format` without `apply` (a shared run): the formatted text only.
pub(crate) fn m_format_shared(s: &Session, p: &Value) -> Result<Value, String> {
    if opt_bool(p, "apply")? == Some(true) {
        return Err("m.format {apply: true} changes the model: it cannot run as a shared read".into());
    }
    m_format_text(s, p).map(|f| f.answer)
}

/// `m.format {expression | name, partition?, apply?, max_width?, indent?, tabs?}`: with `apply`,
/// the named query or table partition takes the formatted text (`expression.set` or
/// `partition.set`, one undo step; nothing when it is already formatted).
pub(crate) fn m_format(s: &mut Session, p: &Value) -> Result<Value, String> {
    let apply = opt_bool(p, "apply")?.unwrap_or(false);
    let f = m_format_text(s, p)?;
    if !apply {
        return Ok(f.answer);
    }
    let mut answer = f.answer;
    match f.source {
        "expression" => {
            return Err("`apply` needs `name` (a query or a table of the model), not `expression`".into());
        }
        _ if !f.changed => {}
        "query" => {
            crate::partition::expression_set(s, &json!({"name": f.name, "expression": f.text}))?;
            answer["applied"] = json!(true);
        }
        _ => {
            let partition = match opt_str(p, "partition")? {
                Some(pn) => pn.to_string(),
                None => s
                    .model
                    .as_ref()
                    .and_then(|m| m.tables.iter().find(|t| t.name == f.name))
                    .and_then(|t| t.partitions.iter().find(|x| is_m(&x.source_type)))
                    .map(|x| x.name.clone())
                    .ok_or_else(|| format!("table `{}` has no Power Query (M) partition", f.name))?,
            };
            crate::partition::partition_set(
                s,
                &json!({"table": f.name, "partition": partition, "expression": f.text}),
            )?;
            answer["applied"] = json!(true);
        }
    }
    Ok(answer)
}

/// One M object of the model, for `model.validate`.
struct MObject<'m> {
    kind: &'static str,
    table: Option<&'m str>,
    name: &'m str,
    expression: &'m str,
}

/// The M part of `model.validate`: `(checked, invalid, objects with diagnostics)`.
pub(crate) fn model_m_validation(m: &SemanticModel) -> (usize, usize, Vec<Value>) {
    let mut objects = Vec::new();
    for t in &m.tables {
        for p in &t.partitions {
            if let (true, Some(e)) = (is_m(&p.source_type), p.expression.as_deref()) {
                objects.push(MObject {
                    kind: "partition",
                    table: Some(&t.name),
                    name: &p.name,
                    expression: e,
                });
            }
        }
    }
    for e in &m.expressions {
        if is_m(&e.kind) {
            objects.push(MObject {
                kind: "expression",
                table: None,
                name: &e.name,
                expression: &e.expression,
            });
        }
    }
    if objects.is_empty() {
        return (0, 0, Vec::new());
    }
    let known = query_names(m);
    let analyses = analyze_all(
        objects.iter().map(|o| o.expression.to_string()).collect(),
        &Limits::default(),
    );
    let mut invalid = 0;
    let mut out = Vec::new();
    for (o, a) in objects.iter().zip(&analyses) {
        let diags = diagnostics(a, &known);
        if diags.is_empty() {
            continue;
        }
        let valid = !has_error(&diags);
        if !valid {
            invalid += 1;
        }
        out.push(
            json!({"kind": o.kind, "table": o.table, "name": o.name, "valid": valid,
            "diagnostics": diags}),
        );
    }
    (objects.len(), invalid, out)
}

/// Analyzes one M expression of an edit and turns errors into a refusal (unless `force`):
/// returns the diagnostics and the analysis.
pub(crate) fn check_edit(
    what: &str,
    expression: &str,
    known: &HashSet<String>,
    force: bool,
) -> Result<(Vec<Value>, Analysis), String> {
    let a = analyze_all(vec![expression.to_string()], &Limits::default())
        .into_iter()
        .next()
        .unwrap_or_default();
    let diags = diagnostics(&a, known);
    if has_error(&diags) && !force {
        let errors: Vec<String> = diags
            .iter()
            .filter(|d| d.get("severity").and_then(Value::as_str) == Some("error"))
            .map(|d| {
                let at = match (d.get("line"), d.get("column")) {
                    (Some(l), Some(c)) => format!("{l}:{c}: "),
                    _ => String::new(),
                };
                format!(
                    "{at}{}",
                    d.get("message").and_then(Value::as_str).unwrap_or_default()
                )
            })
            .collect();
        return Err(format!(
            "invalid M in {what}, nothing was changed: {} (pass \"force\": true to apply anyway)",
            errors.join("; ")
        ));
    }
    Ok((diags, a))
}

// ---------------------------------------------------------------------------------------------
// PBI_QueryOrder
// ---------------------------------------------------------------------------------------------

/// The queries before a command, to update `PBI_QueryOrder` after it.
pub(crate) struct QuerySnapshot {
    id: String,
    queries: Vec<(String, Option<String>)>,
}

/// Commands that may add, remove or rename a query.
fn may_change_queries(id: &str) -> bool {
    id.starts_with("table.")
        || id.starts_with("partition.")
        || id.starts_with("expression.")
        || id == "sources.rebase"
}

pub(crate) fn before(s: &Session, id: &str) -> Option<QuerySnapshot> {
    if !may_change_queries(id) {
        return None;
    }
    let m = s.model.as_ref()?;
    Some(QuerySnapshot {
        id: id.to_string(),
        queries: queries(m),
    })
}

pub(crate) fn after(s: &mut Session, snap: Option<QuerySnapshot>) {
    let (Some(snap), Some(m)) = (snap, s.model.as_mut()) else {
        return;
    };
    sync_query_order(&snap.id, &snap.queries, m);
}

fn lower(s: &str) -> String {
    s.to_lowercase()
}

/// Updates `PBI_QueryOrder` from the queries before the command to those of `m`.
fn sync_query_order(id: &str, before: &[(String, Option<String>)], m: &mut SemanticModel) {
    let after = queries(m);
    let names = |q: &[(String, Option<String>)]| q.iter().map(|(n, _)| lower(n)).collect::<HashSet<_>>();
    let (had, has) = (names(before), names(&after));
    let mut renames: Vec<(String, String)> = Vec::new();
    for (old, tag) in before {
        let Some(tag) = tag else { continue };
        if let Some((new, _)) = after.iter().find(|(_, t)| t.as_deref() == Some(tag.as_str()))
            && new != old
        {
            renames.push((old.clone(), new.clone()));
        }
    }
    let mut removed: Vec<&String> = before
        .iter()
        .map(|(n, _)| n)
        .filter(|n| !has.contains(&lower(n)) && !renames.iter().any(|(o, _)| o == *n))
        .collect();
    let mut added: Vec<&String> = after
        .iter()
        .map(|(n, _)| n)
        .filter(|n| !had.contains(&lower(n)) && !renames.iter().any(|(_, x)| x == *n))
        .collect();
    // Objects without lineage tags: a rename command's one removal and one addition.
    if id.ends_with("rename")
        && let ([old], [new]) = (removed.as_slice(), added.as_slice())
    {
        renames.push(((*old).clone(), (*new).clone()));
        removed.clear();
        added.clear();
    }
    // A rename that only changes case (names compare case-insensitively, so it is neither a
    // removal nor an addition): the one spelling that disappeared and its new spelling.
    if id.ends_with("rename") && renames.is_empty() && removed.is_empty() && added.is_empty() {
        let gone: Vec<&String> = before
            .iter()
            .map(|(n, _)| n)
            .filter(|n| !after.iter().any(|(a, _)| a == *n))
            .collect();
        let new: Vec<&String> = after
            .iter()
            .map(|(n, _)| n)
            .filter(|n| !before.iter().any(|(b, _)| b == *n))
            .collect();
        if let ([old], [new]) = (gone.as_slice(), new.as_slice())
            && lower(old) == lower(new)
        {
            renames.push(((*old).clone(), (*new).clone()));
        }
    }
    if renames.is_empty() && removed.is_empty() && added.is_empty() {
        return;
    }
    let current = m.annotations.iter().position(|a| a.name == QUERY_ORDER);
    let mut list: Vec<String> = match current.and_then(|i| m.annotations.get(i)) {
        Some(a) => match serde_json::from_str(&a.value) {
            Ok(l) => l,
            // Not a JSON list of names: left as it is.
            Err(_) => return,
        },
        // Desktop writes the annotation once a model has queries; a model that had none gets
        // it, one that had queries without it is left alone.
        None if before.is_empty() => Vec::new(),
        None => return,
    };
    for (old, new) in &renames {
        for n in &mut list {
            if n.eq_ignore_ascii_case(old) {
                n.clone_from(new);
            }
        }
    }
    list.retain(|n| !removed.iter().any(|r| r.eq_ignore_ascii_case(n)));
    for a in added {
        if !list.iter().any(|n| n.eq_ignore_ascii_case(a)) {
            list.push(a.clone());
        }
    }
    let Ok(value) = serde_json::to_string(&list) else {
        return;
    };
    match current.and_then(|i| m.annotations.get_mut(i)) {
        Some(a) => {
            if a.value != value {
                a.value = value;
            }
        }
        None => m.annotations.push(Annotation {
            name: QUERY_ORDER.into(),
            value,
        }),
    }
}
