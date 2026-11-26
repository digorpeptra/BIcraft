//! `tmdl.script` and `tmdl.apply`: Power BI Desktop's TMDL view (Microsoft Learn, "TMDL view"
//! and "TMDL scripts"). `tmdl.script` writes model objects as a `createOrReplace` script;
//! `tmdl.apply` runs one against the open model: each object the script declares replaces the
//! model's object of that type and name, with all its descendants, or is added, and the members
//! under `ref table T` replace or add members of T. The whole script is one edit (one undo step):
//! a script that does not parse, names an unknown `ref` table, or brings DAX that does not
//! validate changes nothing, and the error gives `line L, column C` positions in the script.
//! `docs/formats/tmdl-view.md` has the details.

use bicraft_model::{SemanticModel, Table};
use bicraft_tmdl::{ScriptEntry, Scripted};
use serde_json::{Map, Value, json};

use crate::Session;
use crate::commands::model;
use crate::edit::{self, Edit};
use crate::params::{allow, force, str_param};

/// Largest script accepted.
const MAX_SCRIPT_BYTES: usize = 64 * 1024 * 1024;
/// Most objects one `tmdl.script` call scripts.
const MAX_OBJECTS: usize = 100_000;
/// Most diagnostics listed in an error.
const MAX_LISTED: usize = 20;

fn text(v: &Value, key: &str) -> Result<String, String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("each object needs `{key}` (a string)"))
}

/// The `objects` param of `tmdl.script`.
fn scripted(p: &Value) -> Result<Vec<Scripted>, String> {
    let list = match p.get("objects") {
        None | Some(Value::Null) => return Ok(vec![Scripted::Model]),
        Some(Value::Array(a)) => a,
        Some(_) => return Err("`objects` must be a list of {kind, name, table?}".into()),
    };
    if list.is_empty() {
        return Err(
            "`objects` is empty: name the objects to script, or leave it out for the whole model".into(),
        );
    }
    if list.len() > MAX_OBJECTS {
        return Err(format!("at most {MAX_OBJECTS} objects at once"));
    }
    let mut out = Vec::new();
    for o in list {
        if !o.is_object() {
            return Err("each object is {kind, name, table?}".into());
        }
        let kind = text(o, "kind")?.to_ascii_lowercase();
        let name = || text(o, "name");
        let member = |f: fn(String, String) -> Scripted| -> Result<Scripted, String> {
            Ok(f(text(o, "table")?, name()?))
        };
        out.push(match kind.as_str() {
            "model" => Scripted::Model,
            "table" | "calculation_group" | "calculated_table" => Scripted::Table(name()?),
            "column" => member(|table, name| Scripted::Column { table, name })?,
            "measure" => member(|table, name| Scripted::Measure { table, name })?,
            "hierarchy" => member(|table, name| Scripted::Hierarchy { table, name })?,
            "partition" => member(|table, name| Scripted::Partition { table, name })?,
            "function" => Scripted::Function(name()?),
            "expression" => Scripted::Expression(name()?),
            "relationship" => Scripted::Relationship(name()?),
            "role" => Scripted::Role(name()?),
            "perspective" => Scripted::Perspective(name()?),
            "culture" => Scripted::Culture(name()?),
            other => {
                return Err(format!(
                    "unknown object kind `{other}` (model, table, column, measure, hierarchy, partition, \
                     function, expression, relationship, role, perspective, culture)"
                ));
            }
        });
    }
    Ok(out)
}

/// `tmdl.script {objects?}`.
pub(crate) fn tmdl_script(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &["objects"])?;
    let objects = scripted(p)?;
    let m = model(s)?;
    let script = bicraft_tmdl::script(m, &objects)?;
    Ok(json!({"script": script, "count": objects.len()}))
}

// ---------------------------------------------------------------------------------------------
// tmdl.apply
// ---------------------------------------------------------------------------------------------

/// Column (1-based, in characters) of a line's first non-blank character.
fn first_column(lines: &[&str], line: usize) -> usize {
    lines.get(line.saturating_sub(1)).map_or(1, |l| {
        l.chars()
            .take_while(|c| c.is_whitespace())
            .count()
            .saturating_add(1)
    })
}

/// A DAX expression of the script, to validate, with where it starts in the script.
struct DaxCheck {
    object: String,
    expression: String,
    table: Option<String>,
    function: Option<String>,
    /// The line of the object's declaration.
    line: usize,
}

/// Where a DAX diagnostic (`dline`, `dcol` in the expression) is in the script: the expression's
/// first line is searched from the declaration's line down.
fn locate(lines: &[&str], c: &DaxCheck, dline: usize, dcol: usize) -> (usize, usize) {
    let first = c.expression.lines().next().unwrap_or_default().trim();
    let start = c.line.saturating_sub(1);
    let found = (!first.is_empty())
        .then(|| {
            lines
                .iter()
                .enumerate()
                .skip(start)
                .take(10_000)
                .find_map(|(i, l)| l.find(first).map(|b| (i, b)))
        })
        .flatten();
    let Some((i, byte)) = found else {
        return (c.line, first_column(lines, c.line));
    };
    if dline <= 1 {
        let col = lines
            .get(i)
            .and_then(|l| l.get(..byte))
            .map_or(0, |b| b.chars().count());
        return (i.saturating_add(1), col.saturating_add(dcol));
    }
    let l = i.saturating_add(dline);
    // multi-line TMDL expressions are indented below their declaration; the text read has that
    // indentation removed
    let indent = first_column(lines, l).saturating_sub(1);
    (l, indent.saturating_add(dcol))
}

fn name_eq(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn json_name(v: &Value) -> &str {
    v.get("name").and_then(Value::as_str).unwrap_or_default()
}

/// Replaces (by name) or appends `item` in the model collection `key`. Whether it replaced.
fn put_item(m: &mut SemanticModel, key: &str, item: Value) -> bool {
    let name = json_name(&item).to_string();
    match m.extra.get_mut(key) {
        Some(Value::Array(a)) => {
            if let Some(slot) = a.iter_mut().find(|v| name_eq(json_name(v), &name)) {
                *slot = item;
                true
            } else {
                a.push(item);
                false
            }
        }
        _ => {
            m.extra.insert(key.into(), Value::Array(vec![item]));
            false
        }
    }
}

fn replace_by_name<T: Clone>(list: &mut Vec<T>, item: &T, name: impl Fn(&T) -> &str) -> bool {
    let n = name(item).to_string();
    if let Some(slot) = list.iter_mut().find(|x| name_eq(name(x), &n)) {
        *slot = item.clone();
        true
    } else {
        list.push(item.clone());
        false
    }
}

/// The DAX of a table to validate: measures, calculated columns, a calculated table's partition.
fn table_checks(t: &Table, line: usize, out: &mut Vec<DaxCheck>, only: Option<(&str, &str)>) {
    let wanted = |kind: &str, name: &str| only.is_none_or(|(k, n)| k == kind && name_eq(n, name));
    for x in &t.measures {
        if wanted("measure", &x.name) {
            out.push(DaxCheck {
                object: format!("measure {}", edit::dax_measure(&x.name)),
                expression: x.expression.clone(),
                table: Some(t.name.clone()),
                function: None,
                line,
            });
        }
    }
    for c in &t.columns {
        if let Some(e) = &c.expression
            && wanted("column", &c.name)
        {
            out.push(DaxCheck {
                object: format!("calculated column {}", edit::dax_column(&t.name, &c.name)),
                expression: e.clone(),
                table: Some(t.name.clone()),
                function: None,
                line,
            });
        }
    }
    for p in &t.partitions {
        if p.source_type.eq_ignore_ascii_case("calculated")
            && let Some(e) = &p.expression
            && wanted("partition", &p.name)
        {
            out.push(DaxCheck {
                object: format!("calculated table `{}`", t.name),
                expression: e.clone(),
                table: None,
                function: None,
                line,
            });
        }
    }
}

/// The model collection a model-level TMDL object type goes into.
fn collection(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "function" => "functions",
        "perspective" => "perspectives",
        "cultureinfo" => "cultures",
        "datasource" => "dataSources",
        "querygroup" => "queryGroups",
        _ => return None,
    })
}

struct Merged {
    applied: Vec<Value>,
    checks: Vec<DaxCheck>,
    notes: Vec<String>,
}

/// Merges the script's objects into `next`.
fn merge(next: &mut SemanticModel, script: &bicraft_tmdl::Script, lines: &[&str]) -> Result<Merged, String> {
    let at = |line: usize| format!("line {line}, column {}", first_column(lines, line));
    let sm = &script.model;
    let mut out = Merged {
        applied: Vec::new(),
        checks: Vec::new(),
        notes: Vec::new(),
    };
    let action = |replaced: bool| if replaced { "replaced" } else { "added" };
    for e in &script.entries {
        let name = e.name.clone().unwrap_or_default();
        match (e.kind.as_str(), e.is_ref) {
            ("database" | "model", false) => {
                let mut whole = sm.clone();
                if e.kind == "model" {
                    whole.name.clone_from(&next.name);
                    whole.database_extra = next.database_extra.clone();
                    if whole.compatibility_level.is_none() {
                        whole.compatibility_level = next.compatibility_level;
                    }
                }
                *next = whole;
                for t in &next.tables {
                    table_checks(t, e.line, &mut out.checks, None);
                }
                out.applied
                    .push(json!({"kind": e.kind, "name": name, "action": "replaced"}));
            }
            ("table", false) => {
                let t = sm
                    .tables
                    .iter()
                    .find(|t| name_eq(&t.name, &name))
                    .ok_or_else(|| format!("{}: internal: table `{name}` was not read", at(e.line)))?;
                let replaced = replace_by_name(&mut next.tables, t, |t| &t.name);
                table_checks(t, e.line, &mut out.checks, None);
                out.applied
                    .push(json!({"kind": "table", "name": t.name, "action": action(replaced)}));
            }
            ("table", true) if !e.members.is_empty() => merge_members(next, sm, e, lines, &mut out)?,
            (_, true) => {}
            ("relationship", false) => {
                let r = sm
                    .relationships
                    .iter()
                    .find(|r| name_eq(&r.name, &name))
                    .ok_or_else(|| format!("{}: internal: relationship `{name}` was not read", at(e.line)))?;
                let replaced = replace_by_name(&mut next.relationships, r, |r| &r.name);
                out.applied
                    .push(json!({"kind": "relationship", "name": name, "action": action(replaced)}));
            }
            ("expression", false) => {
                let x = sm
                    .expressions
                    .iter()
                    .find(|x| name_eq(&x.name, &name))
                    .ok_or_else(|| format!("{}: internal: expression `{name}` was not read", at(e.line)))?;
                let replaced = replace_by_name(&mut next.expressions, x, |x| &x.name);
                out.applied
                    .push(json!({"kind": "expression", "name": name, "action": action(replaced)}));
            }
            ("role", false) => {
                let r = sm
                    .roles
                    .iter()
                    .find(|r| name_eq(&r.name, &name))
                    .ok_or_else(|| format!("{}: internal: role `{name}` was not read", at(e.line)))?;
                for tp in &r.table_permissions {
                    if let Some(f) = &tp.filter_expression {
                        out.checks.push(DaxCheck {
                            object: format!("row filter of role `{}` on `{}`", r.name, tp.table),
                            expression: f.clone(),
                            table: Some(tp.table.clone()),
                            function: None,
                            line: e.line,
                        });
                    }
                }
                let replaced = replace_by_name(&mut next.roles, r, |r| &r.name);
                out.applied
                    .push(json!({"kind": "role", "name": name, "action": action(replaced)}));
            }
            ("annotation", false) => {
                let a = sm
                    .annotations
                    .iter()
                    .find(|a| name_eq(&a.name, &name))
                    .ok_or_else(|| format!("{}: internal: annotation `{name}` was not read", at(e.line)))?;
                let replaced = replace_by_name(&mut next.annotations, a, |a| &a.name);
                out.applied
                    .push(json!({"kind": "annotation", "name": name, "action": action(replaced)}));
            }
            (kind, false) => {
                let Some(key) = collection(kind) else {
                    return Err(format!(
                        "{}: `{kind}` objects cannot be applied at the top of a script (tables, columns, \
                         measures, hierarchies and partitions under `ref table`, functions, shared \
                         expressions, relationships, roles, perspectives, cultures, data sources, query \
                         groups, annotations, the model or the database)",
                        at(e.line)
                    ));
                };
                let item = sm
                    .extra
                    .get(key)
                    .and_then(Value::as_array)
                    .and_then(|a| a.iter().find(|v| name_eq(json_name(v), &name)))
                    .cloned()
                    .ok_or_else(|| format!("{}: internal: {kind} `{name}` was not read", at(e.line)))?;
                if kind == "function" {
                    if let Err(m) = crate::udf::check_function_name(&name) {
                        return Err(format!("{}: {m}", at(e.line)));
                    }
                    if let Some(x) = item.get("expression").and_then(bicraft_dax::udf::expression_text) {
                        out.checks.push(DaxCheck {
                            object: format!("function `{name}`"),
                            expression: x,
                            table: None,
                            function: Some(name.clone()),
                            line: e.line,
                        });
                    }
                }
                let replaced = put_item(next, key, item);
                out.applied
                    .push(json!({"kind": kind, "name": name, "action": action(replaced)}));
            }
        }
    }
    let has_functions = next
        .extra
        .get("functions")
        .and_then(Value::as_array)
        .is_some_and(|a| !a.is_empty());
    if has_functions
        && let Some(l) = next.compatibility_level
        && l < crate::udf::UDF_COMPATIBILITY_LEVEL
    {
        next.compatibility_level = Some(crate::udf::UDF_COMPATIBILITY_LEVEL);
        out.notes.push(format!(
            "the compatibility level was raised from {l} to {} (user-defined functions need it)",
            crate::udf::UDF_COMPATIBILITY_LEVEL
        ));
    }
    Ok(out)
}

fn merge_members(
    next: &mut SemanticModel,
    sm: &SemanticModel,
    e: &ScriptEntry,
    lines: &[&str],
    out: &mut Merged,
) -> Result<(), String> {
    let at = |line: usize| format!("line {line}, column {}", first_column(lines, line));
    let name = e.name.clone().unwrap_or_default();
    let ti = next
        .tables
        .iter()
        .position(|t| name_eq(&t.name, &name))
        .ok_or_else(|| {
            format!(
                "{}: ref table `{name}`: the model has no table `{name}`",
                at(e.line)
            )
        })?;
    let src = sm
        .tables
        .iter()
        .find(|t| name_eq(&t.name, &name))
        .ok_or_else(|| format!("{}: internal: the members of `{name}` were not read", at(e.line)))?;
    for mem in &e.members {
        let mname = mem.name.clone().unwrap_or_default();
        let replaced = match mem.kind.as_str() {
            "measure" => {
                let x = src
                    .measures
                    .iter()
                    .find(|x| name_eq(&x.name, &mname))
                    .ok_or_else(|| format!("{}: internal: measure `{mname}` was not read", at(mem.line)))?;
                if let Some(other) = next
                    .tables
                    .iter()
                    .enumerate()
                    .find(|(i, t)| *i != ti && t.measures.iter().any(|m| name_eq(&m.name, &mname)))
                {
                    return Err(format!(
                        "{}: the model already has a measure [{mname}] in table `{}`: measure names are \
                         unique in a model",
                        at(mem.line),
                        other.1.name
                    ));
                }
                let t = edit::table_at(next, ti)?;
                let replaced = replace_by_name(&mut t.measures, x, |x| &x.name);
                table_checks(src, mem.line, &mut out.checks, Some(("measure", &mname)));
                replaced
            }
            "column" => {
                let c = src
                    .columns
                    .iter()
                    .find(|c| name_eq(&c.name, &mname))
                    .ok_or_else(|| format!("{}: internal: column `{mname}` was not read", at(mem.line)))?;
                let t = edit::table_at(next, ti)?;
                let replaced = replace_by_name(&mut t.columns, c, |c| &c.name);
                table_checks(src, mem.line, &mut out.checks, Some(("column", &mname)));
                replaced
            }
            "partition" => {
                let p = src
                    .partitions
                    .iter()
                    .find(|p| name_eq(&p.name, &mname))
                    .ok_or_else(|| format!("{}: internal: partition `{mname}` was not read", at(mem.line)))?;
                let t = edit::table_at(next, ti)?;
                let replaced = replace_by_name(&mut t.partitions, p, |p| &p.name);
                table_checks(src, mem.line, &mut out.checks, Some(("partition", &mname)));
                replaced
            }
            "hierarchy" => {
                let h = src
                    .extra
                    .get("hierarchies")
                    .and_then(Value::as_array)
                    .and_then(|a| a.iter().find(|h| name_eq(json_name(h), &mname)))
                    .cloned()
                    .ok_or_else(|| format!("{}: internal: hierarchy `{mname}` was not read", at(mem.line)))?;
                let t = edit::table_at(next, ti)?;
                match t.extra.get_mut("hierarchies") {
                    Some(Value::Array(a)) => {
                        if let Some(slot) = a.iter_mut().find(|v| name_eq(json_name(v), &mname)) {
                            *slot = h;
                            true
                        } else {
                            a.push(h);
                            false
                        }
                    }
                    _ => {
                        t.extra.insert("hierarchies".into(), Value::Array(vec![h]));
                        false
                    }
                }
            }
            other => {
                return Err(format!(
                    "{}: `{other}` cannot be replaced under `ref table`: script the whole table",
                    at(mem.line)
                ));
            }
        };
        out.applied.push(json!({
            "kind": mem.kind, "table": name, "name": mname,
            "action": if replaced { "replaced" } else { "added" },
        }));
    }
    Ok(())
}

/// `tmdl.apply {script, force?}`.
pub(crate) fn tmdl_apply(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["script", "force"])?;
    let text = str_param(p, "script")?;
    if text.len() > MAX_SCRIPT_BYTES {
        return Err(format!("the script is larger than {MAX_SCRIPT_BYTES} bytes"));
    }
    let force = force(p)?;
    let lines: Vec<&str> = text.lines().collect();
    let script = bicraft_tmdl::read_script(text).map_err(|e| match e {
        bicraft_tmdl::Error::Syntax { line, message, .. } => format!(
            "the script does not parse, nothing was applied: line {line}, column {}: {message}",
            first_column(&lines, line)
        ),
        other => format!("the script does not parse, nothing was applied: {other}"),
    })?;
    let mut next = model(s)?.clone();
    let merged = merge(&mut next, &script, &lines)?;
    // the DAX the script brings, against the model with every change made
    let mut problems: Vec<(usize, usize, String)> = Vec::new();
    let mut diagnostics = Vec::new();
    let mut validator = bicraft_dax::Validator::new(&next);
    for c in &merged.checks {
        let v = match &c.function {
            Some(name) => bicraft_dax::udf::validate_function(name, &c.expression, Some(&next)),
            None => validator.validate(&c.expression, c.table.as_deref()),
        };
        let mut located = Vec::new();
        for d in &v.diagnostics {
            let (line, column) = locate(&lines, c, d.line, d.column);
            if d.severity == bicraft_dax::Severity::Error {
                problems.push((line, column, format!("{}: {} [{}]", c.object, d.message, d.code)));
            }
            located.push(json!({
                "severity": d.severity, "code": d.code, "message": d.message,
                "line": line, "column": column,
            }));
        }
        if !located.is_empty() {
            diagnostics.push(json!({"object": c.object, "valid": v.valid, "diagnostics": located}));
        }
    }
    drop(validator);
    problems.sort();
    if !problems.is_empty() && !force {
        let listed: Vec<String> = problems
            .iter()
            .take(MAX_LISTED)
            .map(|(l, c, m)| format!("line {l}, column {c}: {m}"))
            .collect();
        let more = problems.len().saturating_sub(MAX_LISTED);
        return Err(format!(
            "invalid DAX, nothing was applied: {}{} (pass \"force\": true to apply anyway)",
            listed.join("; "),
            if more > 0 {
                format!("; and {more} more")
            } else {
                String::new()
            }
        ));
    }
    let mut result = Map::new();
    result.insert("applied".into(), Value::Array(merged.applied));
    let notes = merged.notes;
    let forced: Vec<String> = problems
        .iter()
        .map(|(l, c, m)| format!("applied with invalid DAX (force): line {l}, column {c}: {m}"))
        .collect();
    let mut out = edit::apply(s, &json!({}), move |m| {
        *m = next;
        let mut e = Edit::new(Value::Object(result)).breaking();
        e.notes.extend(notes);
        e.notes.extend(forced);
        Ok(e)
    })?;
    if let Some(o) = out.as_object_mut() {
        o.insert("diagnostics".into(), Value::Array(diagnostics));
    }
    Ok(out)
}
