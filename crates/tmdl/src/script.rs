//! TMDL scripts (Microsoft Learn, "TMDL scripts"): a command, then one or more objects indented
//! under it. Only `createOrReplace` exists: each object it declares replaces the model's object
//! of that type and name with all its descendants, or is created. A table's members can be
//! scripted on their own under `ref table T`, which names the table without replacing it.
//!
//! [`script`] writes such a script for model objects (Power BI Desktop's TMDL view "Script
//! TMDL"); [`read_script`] reads one into the objects it declares and a model holding their
//! definitions, which the engine merges into the open model (`tmdl.apply`).

use bicraft_model::{SemanticModel, Table};
use serde_json::{Map, Value};

use crate::Error;
use crate::write::{Entry, entry_text};

/// The only TMDL script command.
pub const CREATE_OR_REPLACE: &str = "createOrReplace";

/// A model object to script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scripted {
    /// The whole model: database, model properties and every object.
    Model,
    Table(String),
    Column {
        table: String,
        name: String,
    },
    Measure {
        table: String,
        name: String,
    },
    Hierarchy {
        table: String,
        name: String,
    },
    Partition {
        table: String,
        name: String,
    },
    Function(String),
    Expression(String),
    Relationship(String),
    Role(String),
    Perspective(String),
    Culture(String),
}

fn eq(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn items<'m>(model: &'m SemanticModel, key: &str) -> &'m [Value] {
    model
        .extra
        .get(key)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn item<'m>(
    model: &'m SemanticModel,
    key: &str,
    what: &str,
    name: &str,
) -> Result<&'m Map<String, Value>, String> {
    items(model, key)
        .iter()
        .filter_map(Value::as_object)
        .find(|o| o.get("name").and_then(Value::as_str).is_some_and(|n| eq(n, name)))
        .ok_or_else(|| format!("the model has no {what} `{name}`"))
}

fn table<'m>(model: &'m SemanticModel, name: &str) -> Result<&'m Table, String> {
    model
        .tables
        .iter()
        .find(|t| eq(&t.name, name))
        .ok_or_else(|| format!("the model has no table `{name}`"))
}

/// Writes a `createOrReplace` script of `objects`, in the order given; members of one table
/// given in a row share one `ref table` block. Lines end with `\n`, indented with tabs.
pub fn script(model: &SemanticModel, objects: &[Scripted]) -> Result<String, String> {
    let mut entries: Vec<String> = Vec::new();
    // members of the table being collected: (table index, members)
    let mut members: Option<Table> = None;
    let flush = |members: &mut Option<Table>, entries: &mut Vec<String>| {
        if let Some(t) = members.take() {
            entries.push(entry_text(&Entry::TableMembers(&t)));
        }
    };
    for o in objects {
        let member_table = match o {
            Scripted::Column { table: t, .. }
            | Scripted::Measure { table: t, .. }
            | Scripted::Hierarchy { table: t, .. }
            | Scripted::Partition { table: t, .. } => Some(table(model, t)?),
            _ => None,
        };
        if let Some(src) = member_table {
            if members.as_ref().is_some_and(|m| !eq(&m.name, &src.name)) {
                flush(&mut members, &mut entries);
            }
            let into = members.get_or_insert_with(|| Table {
                name: src.name.clone(),
                ..Default::default()
            });
            match o {
                Scripted::Column { name, .. } => {
                    let c = src
                        .columns
                        .iter()
                        .find(|c| eq(&c.name, name))
                        .ok_or_else(|| format!("table `{}` has no column `{name}`", src.name))?;
                    into.columns.push(c.clone());
                }
                Scripted::Measure { name, .. } => {
                    let m = src
                        .measures
                        .iter()
                        .find(|m| eq(&m.name, name))
                        .ok_or_else(|| format!("table `{}` has no measure `{name}`", src.name))?;
                    into.measures.push(m.clone());
                }
                Scripted::Partition { name, .. } => {
                    let p = src
                        .partitions
                        .iter()
                        .find(|p| eq(&p.name, name))
                        .ok_or_else(|| format!("table `{}` has no partition `{name}`", src.name))?;
                    into.partitions.push(p.clone());
                }
                Scripted::Hierarchy { name, .. } => {
                    let h = src
                        .extra
                        .get("hierarchies")
                        .and_then(Value::as_array)
                        .and_then(|a| {
                            a.iter()
                                .find(|h| h.get("name").and_then(Value::as_str).is_some_and(|n| eq(n, name)))
                        })
                        .ok_or_else(|| format!("table `{}` has no hierarchy `{name}`", src.name))?;
                    match into.extra.get_mut("hierarchies") {
                        Some(Value::Array(a)) => a.push(h.clone()),
                        _ => {
                            into.extra
                                .insert("hierarchies".into(), Value::Array(vec![h.clone()]));
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        flush(&mut members, &mut entries);
        match o {
            Scripted::Model => {
                let docs = crate::write_documents(model);
                let rank = |p: &str| match p {
                    "database.tmdl" => 0,
                    "model.tmdl" => 1,
                    _ => 2,
                };
                let mut docs: Vec<&(String, String)> = docs.iter().collect();
                docs.sort_by_key(|(p, _)| rank(p));
                for (_, text) in docs {
                    entries.push(text.clone());
                }
            }
            Scripted::Table(n) => entries.push(entry_text(&Entry::Table(table(model, n)?))),
            Scripted::Relationship(n) => {
                let r = model
                    .relationships
                    .iter()
                    .find(|r| eq(&r.name, n))
                    .ok_or_else(|| format!("the model has no relationship `{n}`"))?;
                entries.push(entry_text(&Entry::Relationship(r)));
            }
            Scripted::Expression(n) => {
                let x = model
                    .expressions
                    .iter()
                    .find(|x| eq(&x.name, n))
                    .ok_or_else(|| format!("the model has no shared expression `{n}`"))?;
                entries.push(entry_text(&Entry::Expression(x)));
            }
            Scripted::Role(n) => {
                let r = model
                    .roles
                    .iter()
                    .find(|r| eq(&r.name, n))
                    .ok_or_else(|| format!("the model has no role `{n}`"))?;
                entries.push(entry_text(&Entry::Role(r)));
            }
            Scripted::Function(n) => {
                entries.push(entry_text(&Entry::Item(
                    "function",
                    item(model, "functions", "function", n)?,
                )));
            }
            Scripted::Perspective(n) => entries.push(entry_text(&Entry::Item(
                "perspective",
                item(model, "perspectives", "perspective", n)?,
            ))),
            Scripted::Culture(n) => entries.push(entry_text(&Entry::Item(
                "cultureInfo",
                item(model, "cultures", "culture", n)?,
            ))),
            _ => {}
        }
    }
    flush(&mut members, &mut entries);
    let mut out = format!("{CREATE_OR_REPLACE}\n\n");
    for text in entries {
        for line in text.replace("\r\n", "\n").lines() {
            if line.is_empty() {
                out.push('\n');
            } else {
                out.push('\t');
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    // one blank line at the end at most
    while out.ends_with("\n\n") {
        out.pop();
    }
    Ok(out)
}

/// A member under a `ref table` entry of a script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptMember {
    /// `column`, `measure`, `hierarchy`, `partition`, … (lower case).
    pub kind: String,
    pub name: Option<String>,
    /// 1-based line in the script.
    pub line: usize,
}

/// An object declared (or a table referenced) under the script's command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptEntry {
    /// The TMDL object type, lower case (`table`, `function`, `cultureinfo`, …).
    pub kind: String,
    pub name: Option<String>,
    /// `ref table T`: only its members are replaced.
    pub is_ref: bool,
    pub line: usize,
    pub members: Vec<ScriptMember>,
}

/// A script read: its entries, and a model holding their definitions (a `ref table`'s members
/// in a table of that name, without the table's own properties).
#[derive(Debug, Clone)]
pub struct Script {
    pub entries: Vec<ScriptEntry>,
    pub model: SemanticModel,
}

/// Reads a TMDL script: `createOrReplace` followed by indented objects.
pub fn read_script(text: &str) -> Result<Script, Error> {
    const FILE: &str = "script";
    let syntax = |line: usize, message: String| Error::Syntax {
        file: FILE.to_string(),
        line,
        message,
    };
    let doc = crate::parse(FILE, text)?;
    let roots: Vec<&crate::Node> = doc.roots.iter().filter_map(|&i| doc.node(i)).collect();
    let Some(command) = roots.first() else {
        return Err(syntax(
            1,
            format!(
                "the script is empty: it starts with the command `{CREATE_OR_REPLACE}`, then the objects"
            ),
        ));
    };
    if command.kind != CREATE_OR_REPLACE.to_ascii_lowercase() || command.is_ref || command.name.is_some() {
        return Err(syntax(
            command.line,
            format!(
                "a TMDL script starts with a command; `{}` is not one (only `{CREATE_OR_REPLACE}` is supported)",
                command.keyword
            ),
        ));
    }
    if let Some(second) = roots.get(1) {
        return Err(syntax(
            second.line,
            format!(
                "only one command per script: `{}` is not indented under `{CREATE_OR_REPLACE}`",
                second.keyword
            ),
        ));
    }
    if command.children.is_empty() {
        return Err(syntax(
            command.line,
            format!("`{CREATE_OR_REPLACE}` has no object: indent the objects under it"),
        ));
    }
    let mut entries = Vec::new();
    let mut ref_lines = Vec::new();
    for &id in &command.children {
        let Some(n) = doc.node(id) else { continue };
        if n.name.is_none() && !matches!(n.kind.as_str(), "database" | "model") {
            return Err(syntax(
                n.line,
                format!(
                    "`{}` is not an object (a property cannot be indented under the command)",
                    n.keyword
                ),
            ));
        }
        let members: Vec<ScriptMember> = n
            .children
            .iter()
            .filter_map(|&c| doc.node(c))
            .filter(|c| c.name.is_some() && !c.is_ref)
            .map(|c| ScriptMember {
                kind: c.kind.clone(),
                name: c.name.clone(),
                line: c.line,
            })
            .collect();
        if n.is_ref && n.kind == "table" && !members.is_empty() {
            ref_lines.push(n.line);
        }
        entries.push(ScriptEntry {
            kind: n.kind.clone(),
            name: n.name.clone(),
            is_ref: n.is_ref,
            line: n.line,
            members,
        });
    }
    // `ref table T` with members reads as a partial `table T`
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut rewritten = String::with_capacity(text.len());
    for (i, line) in text.split_inclusive('\n').enumerate() {
        if ref_lines.contains(&i.saturating_add(1)) {
            let indent = line.len().saturating_sub(line.trim_start().len());
            let rest = line.trim_start();
            let after = rest.get(3..).unwrap_or_default().trim_start();
            rewritten.push_str(line.get(..indent).unwrap_or_default());
            rewritten.push_str(after);
        } else {
            rewritten.push_str(line);
        }
    }
    let model = crate::read_documents(&[(FILE.to_string(), rewritten)])?;
    Ok(Script { entries, model })
}
