//! `relationship.autodetect`: Manage relationships › Autodetect (docs/formats/relationships.md,
//! "Autodetect"). Microsoft Learn ("Create and manage relationships in Power BI Desktop",
//! Autodetect) describes the rules: Desktop looks at the column names of the tables, sets
//! cardinality, cross filter direction and activity from the data, and creates nothing it cannot
//! match with confidence. BIcraft's candidates, in model order:
//!
//! - two columns of two different tables with the same name (ignoring case) and the same data
//!   type, the tables not related yet (one candidate per pair of tables: the first match);
//! - the session holds both columns' data, and at least one side holds unique, non-blank values:
//!   that side is the "one" side (many-to-one, single direction); both unique gives one-to-one,
//!   which cross filters in both directions; neither, no candidate (no many-to-many);
//! - the relationship, added active to the model with the candidates before it, makes no filter
//!   path ambiguous.
//!
//! Without `apply` the command only answers the candidates and the pairs it passed over (with
//! why); `apply: true` adds them (or those `only` names) in one undo step.

use std::collections::HashMap;

use bicraft_model::{DataType, Relationship, SemanticModel};
use serde_json::{Value, json};

use crate::Session;
use crate::commands::model;
use crate::edit::{Edit, apply, dax_column, new_guid};
use crate::params::{allow, opt_bool};
use crate::relationship_set::{ambiguous_pairs, check_unique};

/// Rows read per column for the uniqueness test.
const MAX_ROWS: usize = 5_000_000;
/// Most column pairs looked at (a model of many wide tables).
const MAX_PAIRS: usize = 200_000;
/// Most candidates and skipped pairs answered.
const MAX_LISTED: usize = 500;

/// One proposed relationship.
#[derive(Debug, Clone)]
struct Candidate {
    from_table: String,
    from_column: String,
    to_table: String,
    to_column: String,
    one_to_one: bool,
}

impl Candidate {
    fn from(&self) -> String {
        dax_column(&self.from_table, &self.from_column)
    }
    fn to(&self) -> String {
        dax_column(&self.to_table, &self.to_column)
    }
    fn json(&self) -> Value {
        json!({
            "from": self.from(),
            "to": self.to(),
            "from_cardinality": if self.one_to_one { "one" } else { "many" },
            "to_cardinality": "one",
            "cross_filter": if self.one_to_one { "both" } else { "single" },
            "active": true,
        })
    }
    fn relationship(&self, name: String) -> Relationship {
        Relationship {
            name,
            from_table: self.from_table.clone(),
            from_column: self.from_column.clone(),
            to_table: self.to_table.clone(),
            to_column: self.to_column.clone(),
            is_active: true,
            cross_filtering_behavior: self.one_to_one.then(|| "bothDirections".to_string()),
            from_cardinality: self.one_to_one.then(|| "one".to_string()),
            to_cardinality: None,
            ..Default::default()
        }
    }
}

/// Whether a column's values are unique and non-blank; `None` when the session has no data for
/// it or it has no rows (nothing to infer from).
type Unique = Result<Option<bool>, String>;

fn related(m: &SemanticModel, a: &str, b: &str) -> bool {
    m.relationships.iter().any(|r| {
        (r.from_table.eq_ignore_ascii_case(a) && r.to_table.eq_ignore_ascii_case(b))
            || (r.from_table.eq_ignore_ascii_case(b) && r.to_table.eq_ignore_ascii_case(a))
    })
}

/// The candidates and the skipped pairs (`{from, to, reason}`), for the model as it is now.
fn detect(s: &Session) -> Result<(Vec<Candidate>, Vec<Value>), String> {
    let m = model(s)?;
    let mut unique: HashMap<(String, String), Unique> = HashMap::new();
    let mut is_unique = |table: &str, column: &str| -> Unique {
        let key = (table.to_lowercase(), column.to_lowercase());
        if let Some(u) = unique.get(&key) {
            return u.clone();
        }
        let u = match crate::preview::column_values(s, table, column, MAX_ROWS) {
            Ok(Some(v)) if v.is_empty() => Ok(None),
            Ok(Some(v)) => Ok(Some(check_unique(&v).is_ok())),
            Ok(None) => Ok(None),
            Err(e) => Err(e),
        };
        unique.insert(key, u.clone());
        u
    };
    let mut scratch = m.clone();
    let mut out = Vec::new();
    let mut skipped = Vec::new();
    let mut pairs = 0usize;
    let tables: Vec<_> = m
        .tables
        .iter()
        .filter(|t| !t.extra.contains_key("calculationGroup"))
        .collect();
    for (i, a) in tables.iter().enumerate() {
        for b in tables.iter().skip(i.saturating_add(1)) {
            if related(&scratch, &a.name, &b.name) {
                continue;
            }
            'pair: for ca in a.columns.iter().filter(|c| !c.is_row_number()) {
                for cb in b.columns.iter().filter(|c| !c.is_row_number()) {
                    pairs = pairs.saturating_add(1);
                    if pairs > MAX_PAIRS {
                        return Err(format!(
                            "the model has more than {MAX_PAIRS} column pairs to compare; autodetect stopped"
                        ));
                    }
                    if !ca.name.eq_ignore_ascii_case(&cb.name)
                        || ca.data_type != cb.data_type
                        || ca.data_type == DataType::Unknown
                    {
                        continue;
                    }
                    let left = dax_column(&a.name, &ca.name);
                    let right = dax_column(&b.name, &cb.name);
                    let mut skip = |code: &str, reason: String| {
                        if skipped.len() < MAX_LISTED {
                            skipped.push(json!({"from": left, "to": right, "code": code, "reason": reason}));
                        }
                    };
                    let (ua, ub) = match (is_unique(&a.name, &ca.name), is_unique(&b.name, &cb.name)) {
                        (Ok(Some(x)), Ok(Some(y))) => (x, y),
                        (Err(e), _) | (_, Err(e)) => {
                            skip("unreadable", format!("the values could not be read: {e}"));
                            continue;
                        }
                        _ => {
                            skip(
                                "no_data",
                                "no data in the session to infer the cardinality from".into(),
                            );
                            continue;
                        }
                    };
                    let c = match (ua, ub) {
                        (false, false) => {
                            skip("no_unique_side", "neither column has unique values".into());
                            continue;
                        }
                        // b is the one side (a's values repeat, or both are unique)
                        (_, true) => Candidate {
                            from_table: a.name.clone(),
                            from_column: ca.name.clone(),
                            to_table: b.name.clone(),
                            to_column: cb.name.clone(),
                            one_to_one: ua,
                        },
                        (true, false) => Candidate {
                            from_table: b.name.clone(),
                            from_column: cb.name.clone(),
                            to_table: a.name.clone(),
                            to_column: ca.name.clone(),
                            one_to_one: false,
                        },
                    };
                    let before = ambiguous_pairs(&scratch);
                    scratch.relationships.push(c.relationship(String::new()));
                    match (before, ambiguous_pairs(&scratch)) {
                        (Some(old), Some(new)) if new.iter().all(|p| old.contains(p)) => {
                            if out.len() < MAX_LISTED {
                                out.push(c);
                            }
                            break 'pair;
                        }
                        (Some(_), Some(_)) => {
                            scratch.relationships.pop();
                            skip("ambiguous", "it would make filter paths ambiguous".into());
                        }
                        _ => {
                            scratch.relationships.pop();
                            skip(
                                "too_many_paths",
                                "the model has too many relationship paths to check it".into(),
                            );
                        }
                    }
                }
            }
        }
    }
    Ok((out, skipped))
}

/// The `{from, to}` pairs of `only` (lower-cased), or `None` for every candidate.
fn only(p: &Value) -> Result<Option<Vec<(String, String)>>, String> {
    let Some(v) = p.get("only") else { return Ok(None) };
    if v.is_null() {
        return Ok(None);
    }
    let list = v
        .as_array()
        .ok_or("parameter `only` must be a list of {from, to}")?;
    let mut out = Vec::with_capacity(list.len().min(MAX_LISTED));
    for e in list.iter().take(MAX_LISTED.saturating_add(1)) {
        let text = |k: &str| {
            e.get(k)
                .and_then(Value::as_str)
                .map(|x| x.trim().to_lowercase())
                .ok_or_else(|| format!("each `only` entry needs a string `{k}` (\"Table[Column]\")"))
        };
        out.push((text("from")?, text("to")?));
    }
    if out.len() > MAX_LISTED {
        return Err(format!("`only` lists more than {MAX_LISTED} relationships"));
    }
    Ok(Some(out))
}

/// `relationship.autodetect`.
pub(crate) fn relationship_autodetect(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["apply", "only"])?;
    let go = opt_bool(p, "apply")?.unwrap_or(false);
    let only = only(p)?;
    let (found, skipped) = detect(s)?;
    let chosen: Vec<Candidate> = match &only {
        None => found.clone(),
        Some(list) => {
            let mut v = Vec::new();
            for (f, t) in list {
                let c = found
                    .iter()
                    .find(|c| c.from().to_lowercase() == *f && c.to().to_lowercase() == *t)
                    .ok_or_else(|| format!("`{f}` → `{t}` is not an autodetected relationship now"))?;
                v.push(c.clone());
            }
            v
        }
    };
    let candidates: Vec<Value> = found.iter().map(Candidate::json).collect();
    if !go {
        return Ok(json!({
            "candidates": candidates,
            "skipped": skipped,
            "applied": false,
        }));
    }
    if chosen.is_empty() {
        return Err("autodetect found no relationship to add".into());
    }
    apply(s, p, move |m| {
        let mut added = Vec::with_capacity(chosen.len());
        for c in &chosen {
            let mut name = new_guid();
            while m.relationships.iter().any(|r| r.name.eq_ignore_ascii_case(&name)) {
                name = new_guid();
            }
            let mut v = c.json();
            if let Value::Object(o) = &mut v {
                o.insert("name".into(), json!(name));
            }
            m.relationships.push(c.relationship(name));
            added.push(v);
        }
        Ok(Edit::new(json!({
            "added": added,
            "skipped": skipped,
            "applied": true,
        })))
    })
}
