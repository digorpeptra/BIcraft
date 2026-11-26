//! `relationship.set`: changes a relationship in place. The relationship keeps its position and
//! its name (a GUID for relationships Desktop made) unless `new_name` renames it, so a saved file
//! shows a property change, not a removal and a new relationship. A rename follows into the
//! column variations that name the relationship (auto date/time); DAX names relationships by
//! their columns (`USERELATIONSHIP`) and reports do not name them, so nothing else changes.
//!
//! Checks, as Power BI Desktop makes them when the relationship dialog is confirmed:
//!
//! - one active relationship between two tables, and no new ambiguous filter path (two
//!   different active paths a filter can take from one table to another);
//! - cardinalities: many-to-one, one-to-one or many-to-many (`one`-to-`many` is the same
//!   relationship with its ends swapped and is refused with that advice);
//! - security filtering in both directions needs cross filtering in both directions;
//! - one-to-one cross filters in both directions: an explicit single direction is refused, and a
//!   change that leaves a one-to-one relationship single-direction makes it both (Desktop 2.158
//!   does not open a file with a single-direction one-to-one relationship);
//! - an end that becomes a "one" side (or a "one" end moved to another column with `new_from` /
//!   `new_to`) must hold unique, non-blank values when the session has the column's data (stored
//!   PBIX/`cache.abf` data or literal partitions); without data the result says it was not checked.
//!
//! The checks are [`crate::relationship_review::review`]'s, shared with `relationship.add`;
//! `check: true` answers them without changing the model (the Edit relationship dialog).
//!
//! On a PBIX, `file.save` writes activity, cross filtering, security filtering and referential
//! integrity into the `Relationship` row; a cardinality or an end change rebuilds the
//! relationship's storage (its `R$` indexes) under the same name through the structural writer.

use std::collections::HashMap;

use bicraft_model::{Relationship, SemanticModel};
use bicraft_vertipaq::CellValue;
use serde_json::{Value, json};

use crate::Session;
use crate::commands::model;
use crate::edit::{Edit, apply, check_name, dax_column, parse_column_ref};
use crate::params::{allow, opt_bool, opt_str};
use crate::relationship_review::{Review, cards, is_both, resolve_column, review, set_flags};
/// Path-search steps of the ambiguity check before it gives up (with a note).
const MAX_PATH_STEPS: usize = 200_000;

/// Finds the relationship `p` names: `name`, or `from` + `to`.
pub(crate) fn find(m: &SemanticModel, p: &Value) -> Result<usize, String> {
    let name = opt_str(p, "name")?;
    let from = opt_str(p, "from")?;
    let to = opt_str(p, "to")?;
    match (name, from, to) {
        (Some(n), None, None) => m
            .relationships
            .iter()
            .position(|r| r.name.eq_ignore_ascii_case(n))
            .ok_or_else(|| format!("no relationship named `{n}`")),
        (None, Some(f), Some(t)) => {
            let (ft, fc) = parse_column_ref(f)?;
            let (tt, tc) = parse_column_ref(t)?;
            let matches: Vec<usize> = m
                .relationships
                .iter()
                .enumerate()
                .filter(|(_, r)| {
                    r.from_table.eq_ignore_ascii_case(&ft)
                        && r.from_column.eq_ignore_ascii_case(&fc)
                        && r.to_table.eq_ignore_ascii_case(&tt)
                        && r.to_column.eq_ignore_ascii_case(&tc)
                })
                .map(|(i, _)| i)
                .collect();
            match matches.as_slice() {
                [i] => Ok(*i),
                [] => Err(format!("no relationship from {f} to {t}")),
                _ => Err(format!(
                    "several relationships go from {f} to {t}; name one with `name`"
                )),
            }
        }
        _ => Err("pass either `name`, or both `from` and `to`".into()),
    }
}

fn cardinality(key: &str, p: &Value) -> Result<Option<&'static str>, String> {
    match opt_str(p, key)? {
        None => Ok(None),
        Some(c) if c.eq_ignore_ascii_case("one") => Ok(Some("one")),
        Some(c) if c.eq_ignore_ascii_case("many") => Ok(Some("many")),
        Some(c) => Err(format!("unknown {key} `{c}` (expected one or many)")),
    }
}

fn cross_filter(p: &Value) -> Result<Option<&'static str>, String> {
    match opt_str(p, "cross_filter")? {
        None => Ok(None),
        Some(c) if c.eq_ignore_ascii_case("single") || c.eq_ignore_ascii_case("oneDirection") => {
            Ok(Some("oneDirection"))
        }
        Some(c) if c.eq_ignore_ascii_case("both") || c.eq_ignore_ascii_case("bothDirections") => {
            Ok(Some("bothDirections"))
        }
        Some(c) => Err(format!("unknown cross_filter `{c}` (expected single or both)")),
    }
}

fn security_filter(p: &Value) -> Result<Option<&'static str>, String> {
    match opt_str(p, "security_filtering")? {
        None => Ok(None),
        Some(c) if c.eq_ignore_ascii_case("single") || c.eq_ignore_ascii_case("oneDirection") => {
            Ok(Some("oneDirection"))
        }
        Some(c) if c.eq_ignore_ascii_case("both") || c.eq_ignore_ascii_case("bothDirections") => {
            Ok(Some("bothDirections"))
        }
        Some(c) if c.eq_ignore_ascii_case("none") => Ok(Some("none")),
        Some(c) => Err(format!(
            "unknown security_filtering `{c}` (expected single, both or none)"
        )),
    }
}

fn security_of(r: &Relationship) -> Option<&str> {
    r.extra.get("securityFilteringBehavior").and_then(Value::as_str)
}

/// Directed filter edges of the active relationships: the one side filters the many side, and
/// back with cross filtering in both directions. Nodes are lower-cased table names.
fn filter_edges(m: &SemanticModel) -> HashMap<String, Vec<String>> {
    let mut edges: HashMap<String, Vec<String>> = HashMap::new();
    for r in m.relationships.iter().filter(|r| r.is_active) {
        let (f, t) = (r.from_table.to_lowercase(), r.to_table.to_lowercase());
        if f == t {
            continue;
        }
        let (fc, tc) = cards(r);
        let both = is_both(r.cross_filtering_behavior.as_deref());
        // The side that filters the other by default: the "one" side (to, for many-to-many).
        let (src, dst) = if fc == "one" && tc == "many" {
            (f.clone(), t.clone())
        } else {
            (t.clone(), f.clone())
        };
        edges.entry(src.clone()).or_default().push(dst.clone());
        if both {
            edges.entry(dst).or_default().push(src);
        }
    }
    edges
}

/// Pairs of tables `(from, to)` a filter reaches by two different simple paths. `None` when the
/// search ran out of steps.
pub(crate) fn ambiguous_pairs(m: &SemanticModel) -> Option<Vec<(String, String)>> {
    let edges = filter_edges(m);
    let mut steps = 0usize;
    let mut out = Vec::new();
    let mut sources: Vec<&String> = edges.keys().collect();
    sources.sort();
    for s in sources {
        let mut counts: HashMap<&str, u8> = HashMap::new();
        let mut path: Vec<&str> = vec![s.as_str()];
        // Explicit DFS stack of (node, next edge index).
        let mut stack: Vec<(&str, usize)> = vec![(s.as_str(), 0)];
        while let Some((node, i)) = stack.last_mut().map(|(n, i)| (*n, i)) {
            steps = steps.saturating_add(1);
            if steps > MAX_PATH_STEPS {
                return None;
            }
            let next = edges.get(node).and_then(|v| v.get(*i));
            *i = i.saturating_add(1);
            match next {
                None => {
                    stack.pop();
                    path.pop();
                }
                Some(n) => {
                    if path.contains(&n.as_str()) {
                        continue;
                    }
                    let c = counts.entry(n.as_str()).or_insert(0);
                    *c = c.saturating_add(1);
                    path.push(n.as_str());
                    stack.push((n.as_str(), 0));
                }
            }
        }
        let mut targets: Vec<&str> = counts.iter().filter(|(_, c)| **c >= 2).map(|(t, _)| *t).collect();
        targets.sort_unstable();
        out.extend(targets.into_iter().map(|t| (s.clone(), t.to_string())));
    }
    Some(out)
}

/// Unique, non-blank values ("one" side). `Err` names the first problem.
pub(crate) fn check_unique(values: &[CellValue]) -> Result<(), String> {
    let mut seen = std::collections::HashSet::with_capacity(values.len());
    for (i, v) in values.iter().enumerate() {
        let key = match v {
            CellValue::Null => return Err(format!("has a blank value (row {})", i.saturating_add(1))),
            CellValue::Text(s) => format!("t{}", s.to_lowercase()),
            CellValue::Int(n) => format!("n{}", *n as f64),
            CellValue::Real(x) | CellValue::DateTime(x) => format!("n{x}"),
            CellValue::Decimal(d) => format!("n{}", *d as f64 / 10_000.0),
            CellValue::Bool(b) => format!("b{b}"),
        };
        if !seen.insert(key) {
            return Err(format!("has duplicate values (row {})", i.saturating_add(1)));
        }
    }
    Ok(())
}

/// `model.validate`'s relationship findings: relationships Power BI Desktop does not open. Today a
/// one-to-one relationship with single-direction cross filtering (Desktop 2.158 refuses the PBIX
/// or PBIP; `relationship.set` with `cross_filter: "both"` repairs it).
pub(crate) fn relationship_problems(m: &SemanticModel) -> Vec<Value> {
    m.relationships
        .iter()
        .filter(|r| {
            let (fc, tc) = cards(r);
            fc == "one" && tc == "one" && !is_both(r.cross_filtering_behavior.as_deref())
        })
        .map(|r| {
            json!({
                "name": r.name,
                "from": dax_column(&r.from_table, &r.from_column),
                "to": dax_column(&r.to_table, &r.to_column),
                "message": "a one-to-one relationship must cross filter in both directions; Power BI \
                            Desktop does not open the file (fix: relationship.set with cross_filter \"both\")",
            })
        })
        .collect()
}

/// `relationship.set`.
pub(crate) fn relationship_set(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "name",
            "from",
            "to",
            "active",
            "cross_filter",
            "from_cardinality",
            "to_cardinality",
            "security_filtering",
            "assume_referential_integrity",
            "new_name",
            "new_from",
            "new_to",
            "check",
        ],
    )?;
    let new_name = opt_str(p, "new_name")?;
    if let Some(n) = new_name {
        check_name("relationship", n)?;
    }
    let active = opt_bool(p, "active")?;
    let check = opt_bool(p, "check")?.unwrap_or(false);
    let cross = cross_filter(p)?;
    let from_card = cardinality("from_cardinality", p)?;
    let to_card = cardinality("to_cardinality", p)?;
    let security = security_filter(p)?;
    let rely = opt_bool(p, "assume_referential_integrity")?;
    let new_from = opt_str(p, "new_from")?;
    let new_to = opt_str(p, "new_to")?;
    if !check
        && new_name.is_none()
        && active.is_none()
        && cross.is_none()
        && from_card.is_none()
        && to_card.is_none()
        && security.is_none()
        && rely.is_none()
        && new_from.is_none()
        && new_to.is_none()
    {
        return Err(
            "relationship.set needs at least one of `new_name`, `new_from`, `new_to`, `active`, \
             `cross_filter`, `from_cardinality`, `to_cardinality`, `security_filtering`, \
             `assume_referential_integrity`"
                .into(),
        );
    }
    // The relationship as it will be, reviewed (data, activity, paths) before the edit.
    let current = model(s)?;
    let index = find(current, p)?;
    let old = current
        .relationships
        .get(index)
        .cloned()
        .ok_or("internal: relationship index out of range")?;
    let mut new = old.clone();
    if let Some(n) = new_name {
        if let Some((_, other)) = current
            .relationships
            .iter()
            .enumerate()
            .find(|(i, r)| *i != index && r.name.eq_ignore_ascii_case(n))
        {
            return Err(format!("a relationship named `{}` already exists", other.name));
        }
        new.name = n.to_string();
    }
    if let Some(f) = new_from {
        let (t, c) = resolve_column(current, f)?;
        new.from_table = t;
        new.from_column = c;
    }
    if let Some(f) = new_to {
        let (t, c) = resolve_column(current, f)?;
        new.to_table = t;
        new.to_column = c;
    }
    if let Some(a) = active {
        new.is_active = a;
    }
    if let Some(c) = cross {
        new.cross_filtering_behavior = (c != "oneDirection").then(|| c.to_string());
    }
    if let Some(c) = from_card {
        new.from_cardinality = (c != "many").then(|| c.to_string());
    }
    if let Some(c) = to_card {
        new.to_cardinality = (c != "one").then(|| c.to_string());
    }
    set_flags(&mut new, rely, security);
    let mut notes = Vec::new();
    let (fc, tc) = cards(&new);
    // One-to-one cross filters in both directions in Power BI Desktop, which refuses to open a
    // file with a single-direction one: refuse single, and turn an existing one into both.
    if fc == "one" && tc == "one" && !is_both(new.cross_filtering_behavior.as_deref()) {
        if cross == Some("oneDirection") {
            return Err(crate::edit::ONE_TO_ONE_SINGLE.into());
        }
        new.cross_filtering_behavior = Some("bothDirections".into());
        notes.push(
            "a one-to-one relationship cross filters in both directions: cross_filter set to both".into(),
        );
    }
    let (new, verdict) = review(
        s,
        Review {
            current,
            new,
            index: Some(index),
            old: Some(&old),
            active_given: active,
            read_both: check,
        },
    );
    if check {
        return Ok(verdict.answer(&new));
    }
    if let Some(e) = verdict.refusal() {
        return Err(e);
    }
    notes.extend(verdict.warnings.iter().map(|f| f.message.clone()));
    let data_checked = verdict.data_checked;
    apply(s, p, move |m| {
        let r = m
            .relationships
            .get_mut(index)
            .ok_or("internal: relationship index out of range")?;
        *r = new.clone();
        let renamed = old.name != new.name;
        if renamed {
            rename_in_variations(m, &old.name, &new.name);
        }
        let (fc, tc) = cards(&new);
        let rely = new
            .extra
            .get("relyOnReferentialIntegrity")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let mut result = json!({
            "name": new.name,
            "from": dax_column(&new.from_table, &new.from_column),
            "to": dax_column(&new.to_table, &new.to_column),
            "active": new.is_active,
            "cross_filter": if is_both(new.cross_filtering_behavior.as_deref()) { "both" } else { "single" },
            "from_cardinality": fc,
            "to_cardinality": tc,
            "security_filtering": security_of(&new).unwrap_or("oneDirection"),
            "assume_referential_integrity": rely,
            "changed": old != new,
            "data_checked": data_checked,
        });
        if renamed && let Value::Object(o) = &mut result {
            o.insert("renamed".into(), json!(old.name));
        }
        let mut edit = Edit::new(result);
        edit.notes = notes;
        Ok(edit)
    })
}

// Points the column variations that name relationship `old` to `new`.
fn rename_in_variations(m: &mut SemanticModel, old: &str, new: &str) {
    for c in m.tables.iter_mut().flat_map(|t| t.columns.iter_mut()) {
        let Some(Value::Array(list)) = c.extra.get_mut("variations") else {
            continue;
        };
        for v in list.iter_mut() {
            if let Some(r) = v.get_mut("relationship")
                && r.as_str().is_some_and(|x| x.eq_ignore_ascii_case(old))
            {
                *r = json!(new);
            }
        }
    }
}
