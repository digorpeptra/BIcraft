//! M9.7: partition sources and shared expressions, edited without evaluating M.
//!
//! - `partition.set` (alias `table.set_partition`): replace a partition's source expression in
//!   place. The table, its lineage tags, relationships, measures and the partition name stay, and
//!   so do an M table's columns. M is checked by `bicraft_data::m::analyze` (syntax, undefined
//!   names), never run, so external sources (Databricks, SQL, …) are fine; DAX is bound like any
//!   calculated table, and its new expression re-derives the table's columns, as Desktop does
//!   (evaluated over the session's data). `new_name` renames the table in the same command.
//! - `partition.add` / `partition.remove`.
//! - `expression.add` / `expression.set` / `expression.rename` / `expression.remove`: shared M
//!   expressions (queries and parameters). A rename rewrites the references of the other M
//!   queries (`m_rewritten`); a removal is refused while M uses the expression.
//!
//! A PBIX keeps the rows each partition's query loaded. Power BI Desktop never stores a model
//! whose query differs from its data (an edited query is applied, i.e. refreshed, or stays
//! pending in the editor), and the engine does not check: a metadata-only change would show
//! the old rows under the new query. So on a PBIX with data, `file.save` reloads a table whose
//! partitions changed (bicraft's M and DAX evaluators), and a source bicraft cannot run (a
//! connector) is refused up front, with the advice to edit a PBIP. Shared expressions that a
//! loaded table depends on are refused for the same reason; adding, renaming or removing one is
//! refused on a PBIX because its metadata writer edits expressions in place only.

use std::collections::HashSet;

use bicraft_data::Limits;
use bicraft_data::m::{LibraryName, analyze_all, identifier, library_name};
use bicraft_model::{Annotation, DataType, NamedExpression, Partition, SemanticModel, Table};
use serde_json::{Value, json};

use crate::edit::{Edit, apply, check_name, dax_table, new_guid, table_index};
use crate::mquery::{check_edit, is_m, query_names};
use crate::params::{allow, change_str, force, opt_bool, opt_str, str_param};
use crate::{Session, Source};

/// Partition modes TMSL knows.
const MODES: &[&str] = &["import", "directQuery", "dual", "default", "push"];
/// Most queries followed when looking for a connector behind a query.
const MAX_FOLLOWED: usize = 4096;

/// Whether the open file is a PBIX whose `DataModel` holds the rows.
pub(crate) fn pbix_with_data(s: &Session) -> bool {
    matches!(&s.source, Some(Source::Package { package, .. })
        if package.get(bicraft_pbix::DATA_MODEL).is_some()
            && package.get(bicraft_pbix::DATA_MODEL_SCHEMA).is_none())
}

pub(crate) const PBIX_ADVICE: &str = "save it as a PBIP project (file.save_as {path: \"out/Name.pbip\"}), edit the \
                           project, and refresh it in Power BI Desktop";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    M,
    Calculated,
}

impl Kind {
    fn tmsl(self) -> &'static str {
        match self {
            Kind::M => "m",
            Kind::Calculated => "calculated",
        }
    }

    fn of(source_type: &str) -> Option<Self> {
        if is_m(source_type) {
            Some(Kind::M)
        } else if source_type.eq_ignore_ascii_case("calculated") {
            Some(Kind::Calculated)
        } else {
            None
        }
    }
}

fn parse_kind(p: &Value) -> Result<Option<Kind>, String> {
    Ok(match opt_str(p, "kind")? {
        None => None,
        Some(k) if k.eq_ignore_ascii_case("m") => Some(Kind::M),
        Some(k) if k.eq_ignore_ascii_case("calculated") || k.eq_ignore_ascii_case("dax") => {
            Some(Kind::Calculated)
        }
        Some(k) => return Err(format!("unknown kind `{k}` (expected m or calculated)")),
    })
}

fn parse_mode(p: &Value) -> Result<Option<String>, String> {
    Ok(match opt_str(p, "mode")? {
        None => None,
        Some(m) => Some(
            MODES
                .iter()
                .find(|x| x.eq_ignore_ascii_case(m))
                .ok_or_else(|| format!("unknown mode `{m}` (expected {})", MODES.join(", ")))?
                .to_string(),
        ),
    })
}

fn expression_param(p: &Value) -> Result<&str, String> {
    let e = str_param(p, "expression")?;
    if e.trim().is_empty() {
        return Err("parameter `expression` must not be empty".into());
    }
    Ok(e)
}

/// Connector functions an M expression reaches, directly or through the model's queries.
pub(crate) fn connectors(m: &SemanticModel, expression: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut todo = vec![expression.to_string()];
    while !todo.is_empty() && seen.len() < MAX_FOLLOWED {
        let batch = std::mem::take(&mut todo);
        for a in analyze_all(batch, &Limits::default()) {
            for r in a.references {
                if !seen.insert(r.name.clone()) {
                    continue;
                }
                if library_name(&r.name) == LibraryName::Connector {
                    found.push(r.name);
                } else if let Some(text) = query_text(m, &r.name) {
                    todo.push(text.to_string());
                }
            }
        }
    }
    found
}

/// The M text of a query of the model (an M table's first partition or a shared expression).
fn query_text<'m>(m: &'m SemanticModel, name: &str) -> Option<&'m str> {
    if let Some(e) = m.expressions.iter().find(|e| e.name == name && is_m(&e.kind)) {
        return Some(&e.expression);
    }
    m.tables
        .iter()
        .find(|t| t.name == name)?
        .partitions
        .iter()
        .find(|p| is_m(&p.source_type))?
        .expression
        .as_deref()
}

/// On a PBIX with data: refuses an M source bicraft cannot run (see the module docs).
fn pbix_source_check(
    s: &Session,
    m: &SemanticModel,
    what: &str,
    kind: Kind,
    expression: &str,
) -> Result<(), String> {
    if !pbix_with_data(s) || kind != Kind::M {
        return Ok(());
    }
    let found = connectors(m, expression);
    if found.is_empty() {
        return Ok(());
    }
    Err(format!(
        "{what}: this PBIX stores the rows its queries loaded, so file.save reloads a table whose \
         source changed, and bicraft never runs external sources ({}); keeping the old rows under \
         the new query would be wrong (Power BI Desktop never does that). To change it, {PBIX_ADVICE}",
        found.join(", ")
    ))
}

const PBIX_NOTE: &str = "this PBIX stores data: file.save reloads the table with the new source \
                         (bicraft's Power Query or DAX evaluator)";

/// Finds the partition `name` of `t`, or its only partition.
fn partition_index(t: &Table, name: Option<&str>) -> Result<Option<usize>, String> {
    match name {
        Some(n) => t
            .partitions
            .iter()
            .position(|p| p.name.eq_ignore_ascii_case(n))
            .map(Some)
            .ok_or_else(|| format!("table `{}` has no partition named `{n}`", t.name)),
        None => match t.partitions.len() {
            0 => Ok(None),
            1 => Ok(Some(0)),
            _ => Err(format!(
                "table `{}` has {} partitions ({}): pass `partition`",
                t.name,
                t.partitions.len(),
                t.partitions
                    .iter()
                    .map(|p| p.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        },
    }
}

/// Whether `t` has data columns (not calculated, not the row number).
fn has_data_columns(t: &Table) -> bool {
    t.columns
        .iter()
        .any(|c| c.expression.is_none() && c.extra.get("type").and_then(Value::as_str) != Some("rowNumber"))
}

/// Checks a partition source: M (analyzed, never run) or DAX (bound by `apply`). Returns the M
/// diagnostics.
fn check_source(
    s: &Session,
    edit_what: &str,
    kind: Kind,
    expression: &str,
    force: bool,
) -> Result<Vec<Value>, String> {
    let m = crate::commands::model(s)?;
    if kind == Kind::M {
        let (diags, _) = check_edit(edit_what, expression, &query_names(m), force)?;
        pbix_source_check(s, m, edit_what, kind, expression)?;
        return Ok(diags);
    }
    Ok(Vec::new())
}

// ---------------------------------------------------------------------------------------------
// partition.*
// ---------------------------------------------------------------------------------------------

pub(crate) fn partition_set(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "table",
            "partition",
            "expression",
            "kind",
            "mode",
            "force",
            "new_name",
            "update_report",
            "update_dax",
        ],
    )?;
    let rename = rename_params(s, p)?;
    // the source and the rename are one command, so one undo step: a rename refused after the
    // source changed puts the model back
    let before = rename.as_ref().map(|_| (s.model.clone(), s.dirty));
    let mut source = p.clone();
    if let Some(o) = source.as_object_mut() {
        for k in ["new_name", "update_report", "update_dax"] {
            o.remove(k);
        }
    }
    let mut out = set_source(s, &source)?;
    if let Some(params) = rename {
        match crate::edit::table_rename(s, &params) {
            Ok(r) => {
                if let Some(o) = out.as_object_mut() {
                    o.insert("renamed".into(), r);
                }
            }
            Err(e) => {
                if let Some((model, dirty)) = before {
                    s.model = model;
                    s.dirty = dirty;
                }
                return Err(format!("nothing was changed: {e}"));
            }
        }
    }
    Ok(out)
}

/// `table.rename`'s params for `partition.set`'s `new_name` (`None` without one, or when it is
/// the table's name), checked before anything changes.
fn rename_params(s: &Session, p: &Value) -> Result<Option<Value>, String> {
    // `update_report` and `update_dax` only matter with `new_name`; the form may send their
    // defaults without one.
    let Some(new_name) = opt_str(p, "new_name")? else {
        return Ok(None);
    };
    check_name("table", new_name)?;
    let m = crate::commands::model(s)?;
    let i = table_index(m, str_param(p, "table")?)?;
    let name = m
        .tables
        .get(i)
        .map(|t| t.name.clone())
        .ok_or("internal: table index out of range")?;
    if name == new_name {
        return Ok(None);
    }
    if let Some((_, other)) = m
        .tables
        .iter()
        .enumerate()
        .find(|(j, t)| *j != i && t.name.eq_ignore_ascii_case(new_name))
    {
        return Err(format!("a table named `{}` already exists", other.name));
    }
    let mut params = json!({"name": name, "new_name": new_name});
    for k in ["update_report", "update_dax"] {
        if let Some(v) = opt_bool(p, k)? {
            params[k] = json!(v);
        }
    }
    Ok(Some(params))
}

/// A column a calculated table's expression gives: its name, type (`Unknown` when the values do
/// not tell) and source column (`Table[Column]` for a model column, else `[Name]`).
struct Derived {
    name: String,
    data_type: DataType,
    source: String,
}

/// Most bytes of a calculated table expression evaluated to find its columns.
const MAX_DERIVED_EXPRESSION: usize = 1 << 20;

/// The columns `expression` gives, from the session's DAX evaluator over its data (one row
/// kept): what the table's columns become, as Power BI Desktop re-derives them.
fn derived_columns(s: &Session, expression: &str) -> Result<Vec<Derived>, String> {
    if expression.len() > MAX_DERIVED_EXPRESSION {
        return Err(format!(
            "the expression is longer than {MAX_DERIVED_EXPRESSION} bytes"
        ));
    }
    let v = crate::preview::dax_evaluate(
        s,
        &json!({"query": format!("EVALUATE\n{expression}\n"), "max_rows": 1}),
    )?;
    let columns = v
        .pointer("/results/0/columns")
        .and_then(Value::as_array)
        .ok_or("the expression gave no table")?;
    let mut out: Vec<Derived> = Vec::with_capacity(columns.len());
    for c in columns {
        let Some(text) = c.get("name").and_then(Value::as_str) else {
            continue;
        };
        let data_type = DataType::from_tmsl(c.get("type").and_then(Value::as_str).unwrap_or(""));
        // `Table[Column]` or `[Name]`
        let (name, source) = match text.find('[') {
            Some(at) if text.ends_with(']') => (
                text.get(at.saturating_add(1)..text.len().saturating_sub(1))
                    .unwrap_or(text)
                    .to_string(),
                text.to_string(),
            ),
            _ => (text.to_string(), format!("[{text}]")),
        };
        if name.is_empty() || out.iter().any(|d| d.name.eq_ignore_ascii_case(&name)) {
            continue;
        }
        out.push(Derived {
            name,
            data_type,
            source,
        });
    }
    if out.is_empty() {
        return Err("the expression gave no column".into());
    }
    Ok(out)
}

/// What [`rederive`] changed: the added, removed and retyped columns' names.
type Rederived = (Vec<String>, Vec<String>, Vec<String>);

/// Gives table `ti` the columns `derived`: a column of the same source column (a field
/// parameter's `[Value1]`) or name keeps its properties (its type and source follow the
/// expression), others are added, data columns the expression no longer gives are removed with
/// their relationships, sort-by settings and hierarchy levels; calculated columns stay.
fn rederive(
    m: &mut SemanticModel,
    ti: usize,
    derived: &[Derived],
    expression: &str,
) -> Result<Rederived, String> {
    let t = m.tables.get_mut(ti).ok_or("internal: table index out of range")?;
    let table = t.name.clone();
    let (mut added, mut retyped, mut kept) = (Vec::new(), Vec::new(), Vec::<String>::new());
    let data = |c: &bicraft_model::Column| c.expression.is_none() && !is_row_number(c);
    for d in derived {
        let free =
            |c: &bicraft_model::Column| data(c) && !kept.iter().any(|k| k.eq_ignore_ascii_case(&c.name));
        // by source column first (a field parameter's `[Value1]` keeps its column's name), then
        // by name, as the save maps computed data onto declared columns
        let at = t
            .columns
            .iter()
            .position(|c| {
                free(c)
                    && c.source_column
                        .as_deref()
                        .is_some_and(|s| s.eq_ignore_ascii_case(&d.source))
            })
            .or_else(|| {
                t.columns
                    .iter()
                    .position(|c| free(c) && c.name.eq_ignore_ascii_case(&d.name))
            });
        match at.and_then(|i| t.columns.get_mut(i)) {
            Some(c) => {
                c.source_column = Some(d.source.clone());
                if d.data_type != DataType::Unknown && c.data_type != d.data_type {
                    c.data_type = d.data_type;
                    c.summarize_by =
                        Some(crate::desktop_defaults::default_summarize(d.data_type).to_string());
                    retyped.push(c.name.clone());
                }
                kept.push(c.name.clone());
            }
            None => {
                if t.columns.iter().any(|c| c.name.eq_ignore_ascii_case(&d.name)) {
                    // a calculated column of that name stays; the result column is not added
                    continue;
                }
                let mut c = crate::pbix_struct::new_column(&d.name, d.data_type, true, None);
                c.source_column = Some(d.source.clone());
                let at = t
                    .columns
                    .iter()
                    .position(|c| c.expression.is_some())
                    .unwrap_or(t.columns.len());
                t.columns.insert(at, c);
                kept.push(d.name.clone());
                added.push(d.name.clone());
            }
        }
    }
    let removed: Vec<String> = t
        .columns
        .iter()
        .filter(|c| data(c) && !kept.iter().any(|k| k.eq_ignore_ascii_case(&c.name)))
        .map(|c| c.name.clone())
        .collect();
    for name in &removed {
        crate::edit::remove_column_in(m, &table, name)?;
    }
    // a removal selects the other columns in the expression: the new one stays as written
    if let Some(p) = m.tables.get_mut(ti).and_then(|t| t.partitions.first_mut()) {
        p.expression = Some(expression.to_string());
    }
    Ok((added, removed, retyped))
}

fn is_row_number(c: &bicraft_model::Column) -> bool {
    c.extra.get("type").and_then(Value::as_str) == Some("rowNumber")
}

fn set_source(s: &mut Session, p: &Value) -> Result<Value, String> {
    let table = str_param(p, "table")?;
    let wanted = opt_str(p, "partition")?;
    let expression = expression_param(p)?;
    let kind_given = parse_kind(p)?;
    let mode = parse_mode(p)?;
    let force = force(p)?;
    let (table_name, index, kind, current_expression) = {
        let m = crate::commands::model(s)?;
        let t = m
            .tables
            .get(table_index(m, table)?)
            .ok_or("internal: table index out of range")?;
        let index = partition_index(t, wanted)?;
        let current_expression = index
            .and_then(|i| t.partitions.get(i))
            .and_then(|x| x.expression.clone());
        let current = index
            .and_then(|i| t.partitions.get(i))
            .map(|x| x.source_type.as_str());
        let current_kind = current.and_then(Kind::of);
        if let (Some(c), None) = (current, current_kind)
            && kind_given.is_none()
        {
            return Err(format!(
                "partition of table `{}` has source type `{c}`, which partition.set does not edit \
                 (pass kind: m or calculated to replace it)",
                t.name
            ));
        }
        let kind = kind_given.or(current_kind).unwrap_or(Kind::M);
        if current_kind.is_some_and(|c| c != kind) && has_data_columns(t) {
            return Err(format!(
                "changing table `{}` from {} to {} changes its columns: remove the table and add \
                 it again (table.add, table.import)",
                t.name,
                current_kind.map_or("?", Kind::tmsl),
                kind.tmsl()
            ));
        }
        if kind == Kind::Calculated && t.partitions.len() > 1 {
            return Err(format!(
                "table `{}` has several partitions; a calculated table has one",
                t.name
            ));
        }
        (t.name.clone(), index, kind, current_expression)
    };
    let what = format!("the partition of table `{table_name}`");
    let m_diagnostics = check_source(s, &what, kind, expression, force)?;
    // a calculated table's new expression gives its columns, as Desktop re-derives them
    let derived = (kind == Kind::Calculated && current_expression.as_deref() != Some(expression))
        .then(|| derived_columns(s, expression));
    let pbix = pbix_with_data(s);
    apply(s, p, |m| {
        let ti = table_index(m, &table_name)?;
        let t = m.tables.get_mut(ti).ok_or("internal: table index out of range")?;
        let was_m = t.partitions.iter().any(|x| is_m(&x.source_type));
        let (name, previous) = match index.and_then(|i| t.partitions.get_mut(i)) {
            Some(part) => {
                let previous = part.expression.replace(expression.to_string());
                part.source_type = kind.tmsl().into();
                if let Some(mode) = &mode {
                    part.mode = Some(mode.clone());
                }
                (part.name.clone(), previous)
            }
            None => {
                t.partitions.push(Partition {
                    name: t.name.clone(),
                    mode: Some(mode.clone().unwrap_or_else(|| "import".into())),
                    source_type: kind.tmsl().into(),
                    expression: Some(expression.to_string()),
                    ..Default::default()
                });
                (t.name.clone(), None)
            }
        };
        let changed = previous.as_deref() != Some(expression);
        let mut edit = Edit::new(json!({
            "table": table_name,
            "partition": name,
            "kind": kind.tmsl(),
            "mode": t.partitions.iter().find(|x| x.name == name).and_then(|x| x.mode.clone()),
            "changed": changed,
            "previous_expression": previous,
            "m_diagnostics": m_diagnostics,
        }));
        if kind == Kind::M {
            if !was_m {
                crate::desktop_defaults::m_table(t, expression);
            }
        } else {
            edit.check(
                format!("calculated table {}", dax_table(&table_name)),
                expression,
                None,
            );
            match &derived {
                Some(Ok(columns)) => {
                    let (added, removed, retyped) = rederive(m, ti, columns, expression)?;
                    if !removed.is_empty() {
                        edit = edit.breaking();
                    }
                    edit.insert(
                        "columns",
                        json!({"added": added, "removed": removed, "retyped": retyped}),
                    );
                }
                Some(Err(e)) => edit.notes.push(format!(
                    "the table's columns are kept as they are (its new expression could not be \
                     evaluated to find them: {e}): check them against the new expression"
                )),
                None => {}
            }
        }
        if pbix && changed {
            edit.notes.push(PBIX_NOTE.into());
        }
        Ok(edit)
    })
}

pub(crate) fn partition_add(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["table", "name", "expression", "kind", "mode", "force"])?;
    let table = str_param(p, "table")?;
    let expression = expression_param(p)?;
    let kind_given = parse_kind(p)?;
    let mode = parse_mode(p)?;
    let force = force(p)?;
    let (table_name, kind, name) = {
        let m = crate::commands::model(s)?;
        let t = m
            .tables
            .get(table_index(m, table)?)
            .ok_or("internal: table index out of range")?;
        let existing = t.partitions.first().map(|x| Kind::of(&x.source_type));
        let kind = match (kind_given, existing) {
            (Some(k), Some(Some(e))) if k != e => {
                return Err(format!(
                    "the partitions of table `{}` are {}: a {} partition cannot be added",
                    t.name,
                    e.tmsl(),
                    k.tmsl()
                ));
            }
            (_, Some(None)) => {
                return Err(format!(
                    "the partitions of table `{}` have a source type partition.add does not write",
                    t.name
                ));
            }
            (Some(k), _) => k,
            (None, Some(Some(e))) => e,
            (None, None) => Kind::M,
        };
        if kind == Kind::Calculated && !t.partitions.is_empty() {
            return Err(format!(
                "table `{}` is a calculated table: it has one partition (use partition.set)",
                t.name
            ));
        }
        let name = match opt_str(p, "name")? {
            Some(n) => {
                check_name("partition", n)?;
                if t.partitions.iter().any(|x| x.name.eq_ignore_ascii_case(n)) {
                    return Err(format!("table `{}` already has a partition named `{n}`", t.name));
                }
                n.to_string()
            }
            None => {
                let mut i = t.partitions.len().saturating_add(1);
                let mut n = if t.partitions.is_empty() {
                    t.name.clone()
                } else {
                    format!("{}-{i}", t.name)
                };
                while t.partitions.iter().any(|x| x.name.eq_ignore_ascii_case(&n)) {
                    i = i.saturating_add(1);
                    n = format!("{}-{i}", t.name);
                }
                n
            }
        };
        (t.name.clone(), kind, name)
    };
    let what = format!("partition `{name}` of table `{table_name}`");
    let m_diagnostics = check_source(s, &what, kind, expression, force)?;
    let pbix = pbix_with_data(s);
    apply(s, p, |m| {
        let ti = table_index(m, &table_name)?;
        let t = m.tables.get_mut(ti).ok_or("internal: table index out of range")?;
        let first = t.partitions.is_empty();
        t.partitions.push(Partition {
            name: name.clone(),
            mode: Some(mode.clone().unwrap_or_else(|| "import".into())),
            source_type: kind.tmsl().into(),
            expression: Some(expression.to_string()),
            ..Default::default()
        });
        let mut edit = Edit::new(
            json!({"table": table_name, "partition": name, "kind": kind.tmsl(),
            "m_diagnostics": m_diagnostics}),
        );
        if kind == Kind::Calculated {
            edit.check(
                format!("calculated table {}", dax_table(&table_name)),
                expression,
                None,
            );
        } else if first {
            crate::desktop_defaults::m_table(t, expression);
        }
        if pbix {
            edit.notes.push(PBIX_NOTE.into());
        }
        Ok(edit)
    })
}

pub(crate) fn partition_remove(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["table", "name"])?;
    let table = str_param(p, "table")?;
    let name = str_param(p, "name")?;
    apply(s, p, |m| {
        let ti = table_index(m, table)?;
        let t = m.tables.get_mut(ti).ok_or("internal: table index out of range")?;
        let i = partition_index(t, Some(name))?.ok_or("internal: no partition")?;
        if t.partitions.len() == 1 {
            return Err(format!(
                "`{name}` is the only partition of table `{}`: a table needs one (use \
                 partition.set to change it, or table.remove)",
                t.name
            ));
        }
        let removed = t.partitions.remove(i);
        Ok(Edit::new(json!({"table": t.name, "removed": removed.name})))
    })
}

// ---------------------------------------------------------------------------------------------
// expression.*
// ---------------------------------------------------------------------------------------------

fn expression_index(m: &SemanticModel, name: &str) -> Result<usize, String> {
    m.expressions
        .iter()
        .position(|e| e.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| format!("no shared expression named `{name}`"))
}

/// Query names are unique across tables and shared expressions (case-insensitive).
fn check_query_name_free(m: &SemanticModel, name: &str, except: Option<usize>) -> Result<(), String> {
    if let Some(t) = m.tables.iter().find(|t| t.name.eq_ignore_ascii_case(name)) {
        return Err(format!("a table named `{}` already exists", t.name));
    }
    if let Some((_, e)) = m
        .expressions
        .iter()
        .enumerate()
        .find(|(i, e)| Some(*i) != except && e.name.eq_ignore_ascii_case(name))
    {
        return Err(format!("a shared expression named `{}` already exists", e.name));
    }
    Ok(())
}

pub(crate) fn refuse_on_pbix(s: &Session, what: &str) -> Result<(), String> {
    if pbix_with_data(s) {
        return Err(format!(
            "{what} is not written into a PBIX's DataModel by bicraft (it edits shared \
             expressions in place only); {PBIX_ADVICE}"
        ));
    }
    Ok(())
}

/// The M objects that use `name` as a free name: `(kind, table, object name)`.
pub(crate) fn m_users(m: &SemanticModel, name: &str) -> Vec<Value> {
    let mut objects: Vec<(&str, Option<&str>, &str, &str)> = Vec::new();
    for t in &m.tables {
        for p in &t.partitions {
            if let (true, Some(e)) = (is_m(&p.source_type), p.expression.as_deref()) {
                objects.push(("partition", Some(&t.name), &p.name, e));
            }
        }
    }
    for e in &m.expressions {
        if is_m(&e.kind) && e.name != name {
            objects.push(("expression", None, &e.name, &e.expression));
        }
    }
    let analyses = analyze_all(
        objects.iter().map(|o| o.3.to_string()).collect(),
        &Limits::default(),
    );
    objects
        .iter()
        .zip(analyses)
        .filter(|(_, a)| a.references.iter().any(|r| r.name == name))
        .map(|(o, _)| json!({"kind": o.0, "table": o.1, "name": o.2}))
        .collect()
}

/// Rewrites the free references to `old` in `text` (not parsed: `None`).
pub(crate) fn rewrite(text: &str, old: &str, new: &str) -> Option<(String, usize)> {
    let a = analyze_all(vec![text.to_string()], &Limits::default())
        .into_iter()
        .next()?;
    if a.syntax.is_some() {
        return None;
    }
    let mut spans: Vec<(usize, usize)> = a
        .references
        .iter()
        .filter(|r| r.name == old)
        .filter_map(|r| r.span)
        .collect();
    if spans.is_empty() {
        return Some((text.to_string(), 0));
    }
    spans.sort_unstable();
    let replacement = identifier(new);
    let mut out = String::with_capacity(text.len());
    let mut at = 0usize;
    for (s, e) in &spans {
        out.push_str(text.get(at..*s)?);
        out.push_str(&replacement);
        at = *e;
    }
    out.push_str(text.get(at..)?);
    Some((out, spans.len()))
}

pub(crate) fn expression_add(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "name",
            "expression",
            "kind",
            "description",
            "query_group",
            "force",
        ],
    )?;
    let name = str_param(p, "name")?.to_string();
    check_name("shared expression", &name)?;
    let expression = expression_param(p)?;
    if let Some(k) = opt_str(p, "kind")?
        && !is_m(k)
    {
        return Err(format!(
            "unknown kind `{k}` (shared expressions are M: kind \"m\")"
        ));
    }
    let description = opt_str(p, "description")?.map(str::to_string);
    let group = opt_str(p, "query_group")?.map(str::to_string);
    let force = force(p)?;
    refuse_on_pbix(s, "adding a shared expression")?;
    let (diags, analysis) = {
        let m = crate::commands::model(s)?;
        check_query_name_free(m, &name, None)?;
        check_edit(
            &format!("shared expression `{name}`"),
            expression,
            &query_names(m),
            force,
        )?
    };
    apply(s, p, |m| {
        let mut e = NamedExpression {
            name: name.clone(),
            kind: "m".into(),
            expression: expression.to_string(),
            description,
            lineage_tag: Some(new_guid()),
            ..Default::default()
        };
        if let Some(g) = group {
            e.extra.insert("queryGroup".into(), json!(g));
        }
        crate::desktop_defaults::m_expression(&mut e.annotations, &analysis);
        m.expressions.push(e);
        Ok(Edit::new(json!({"expression": name, "m_diagnostics": diags,
            "result_type": analysis.result_type})))
    })
}

pub(crate) fn expression_set(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["name", "expression", "description", "query_group", "force"])?;
    let name = str_param(p, "name")?;
    let expression = match p.get("expression") {
        None | Some(Value::Null) => None,
        Some(_) => Some(expression_param(p)?),
    };
    let description = change_str(p, "description")?;
    let group = change_str(p, "query_group")?;
    if expression.is_none() && description.is_none() && group.is_none() {
        return Err("expression.set needs at least one of `expression`, `description`, `query_group`".into());
    }
    let force = force(p)?;
    let (actual, diags) = {
        let m = crate::commands::model(s)?;
        let e = m
            .expressions
            .get(expression_index(m, name)?)
            .ok_or("internal: expression index out of range")?;
        let diags = match expression {
            Some(x) if x != e.expression => {
                let (d, _) = check_edit(
                    &format!("shared expression `{}`", e.name),
                    x,
                    &query_names(m),
                    force,
                )?;
                if pbix_with_data(s) {
                    let users = loaded_users(m, &e.name);
                    if !users.is_empty() {
                        return Err(format!(
                            "shared expression `{}` feeds the stored rows of {}: on a PBIX the data \
                             would no longer match its query. To change it, {PBIX_ADVICE}",
                            e.name,
                            users.join(", ")
                        ));
                    }
                }
                d
            }
            _ => Vec::new(),
        };
        (e.name.clone(), diags)
    };
    apply(s, p, |m| {
        let i = expression_index(m, &actual)?;
        let e = m
            .expressions
            .get_mut(i)
            .ok_or("internal: expression index out of range")?;
        let previous = e.expression.clone();
        if let Some(x) = expression {
            x.clone_into(&mut e.expression);
        }
        if let Some(d) = description {
            e.description = d;
        }
        if let Some(g) = group {
            match g {
                Some(g) => {
                    e.extra.insert("queryGroup".into(), json!(g));
                }
                None => {
                    e.extra.remove("queryGroup");
                }
            }
        }
        Ok(Edit::new(
            json!({"expression": actual, "changed": previous != e.expression,
            "m_diagnostics": diags}),
        ))
    })
}

/// The tables whose M partitions use `name`, directly or through other queries.
fn loaded_users(m: &SemanticModel, name: &str) -> Vec<String> {
    m.tables
        .iter()
        .filter(|t| {
            t.partitions
                .iter()
                .any(|p| is_m(&p.source_type) && p.expression.as_deref().is_some_and(|e| reaches(m, e, name)))
        })
        .map(|t| format!("table `{}`", t.name))
        .collect()
}

/// Whether M `expression` uses query `name`, directly or through the model's queries.
pub(crate) fn reaches(m: &SemanticModel, expression: &str, name: &str) -> bool {
    let mut seen: HashSet<String> = HashSet::new();
    let mut todo = vec![expression.to_string()];
    while !todo.is_empty() && seen.len() < MAX_FOLLOWED {
        let batch = std::mem::take(&mut todo);
        for a in analyze_all(batch, &Limits::default()) {
            for r in a.references {
                if r.name == name {
                    return true;
                }
                if seen.insert(r.name.clone())
                    && let Some(text) = query_text(m, &r.name)
                {
                    todo.push(text.to_string());
                }
            }
        }
    }
    false
}

pub(crate) fn expression_rename(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["name", "new_name", "update_m"])?;
    let name = str_param(p, "name")?;
    let new_name = str_param(p, "new_name")?.to_string();
    check_name("shared expression", &new_name)?;
    let update_m = opt_bool(p, "update_m")?.unwrap_or(true);
    refuse_on_pbix(s, "renaming a shared expression")?;
    apply(s, p, |m| {
        let i = expression_index(m, name)?;
        check_query_name_free(m, &new_name, Some(i))?;
        let e = m
            .expressions
            .get_mut(i)
            .ok_or("internal: expression index out of range")?;
        let old = std::mem::replace(&mut e.name, new_name.clone());
        let (rewritten, not_rewritten) = if update_m && old != new_name {
            follow_rename(m, &old, &new_name)
        } else {
            (Vec::new(), Vec::new())
        };
        let translations = crate::translations::rename_expression(m, &old, &new_name);
        let mut edit = Edit::new(json!({"renamed": old, "to": new_name, "m_rewritten": rewritten,
            "m_not_rewritten": not_rewritten, "metadata_updated": translations}));
        if !update_m {
            edit.notes
                .push("update_m is false: M queries that use the old name are left as they are".into());
        }
        Ok(edit)
    })
}

/// Rewrites the references to query `old` in every M partition and shared expression of `m`:
/// (rewritten, not rewritten: texts that do not parse).
pub(crate) fn follow_rename(m: &mut SemanticModel, old: &str, new: &str) -> (Vec<Value>, Vec<Value>) {
    let mut rewritten = Vec::new();
    let mut not_rewritten = Vec::new();
    for t in &mut m.tables {
        for part in &mut t.partitions {
            if !is_m(&part.source_type) {
                continue;
            }
            let Some(text) = part.expression.as_mut() else {
                continue;
            };
            follow(
                text,
                old,
                new,
                json!({"kind": "partition", "table": t.name, "name": part.name}),
                &mut rewritten,
                &mut not_rewritten,
            );
        }
    }
    for x in &mut m.expressions {
        if is_m(&x.kind) {
            let id = json!({"kind": "expression", "name": x.name});
            follow(
                &mut x.expression,
                old,
                new,
                id,
                &mut rewritten,
                &mut not_rewritten,
            );
        }
    }
    (rewritten, not_rewritten)
}

/// Rewrites one M text for a rename, recording the outcome.
fn follow(
    text: &mut String,
    old: &str,
    new: &str,
    id: Value,
    done: &mut Vec<Value>,
    failed: &mut Vec<Value>,
) {
    if !text.contains(old) && !text.contains(&identifier(old)) {
        return;
    }
    match rewrite(text, old, new) {
        Some((_, 0)) => {}
        Some((out, n)) => {
            *text = out;
            let mut id = id;
            id["references"] = json!(n);
            done.push(id);
        }
        None => failed.push(id),
    }
}

pub(crate) fn expression_remove(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["name", "force"])?;
    let name = str_param(p, "name")?;
    let force = force(p)?;
    refuse_on_pbix(s, "removing a shared expression")?;
    let (actual, users) = {
        let m = crate::commands::model(s)?;
        let e = m
            .expressions
            .get(expression_index(m, name)?)
            .ok_or("internal: expression index out of range")?;
        (e.name.clone(), m_users(m, &e.name))
    };
    if !users.is_empty() && !force {
        return Err(format!(
            "shared expression `{actual}` is used by {} M quer{} ({}); pass \"force\": true to remove \
             it anyway",
            users.len(),
            if users.len() == 1 { "y" } else { "ies" },
            users
                .iter()
                .map(|u| u
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    apply(s, p, |m| {
        let i = expression_index(m, &actual)?;
        m.expressions.remove(i);
        Ok(Edit::new(json!({"removed": actual, "used_by": users})))
    })
}

/// Sets an annotation (kept in place when present).
pub(crate) fn set_annotation(list: &mut Vec<Annotation>, name: &str, value: &str) {
    match list.iter_mut().find(|a| a.name == name) {
        Some(a) => {
            if a.value != value {
                value.clone_into(&mut a.value);
            }
        }
        None => list.push(Annotation {
            name: name.into(),
            value: value.into(),
        }),
    }
}
