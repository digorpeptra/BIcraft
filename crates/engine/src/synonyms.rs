//! Synonyms of tables, columns and measures (the Properties pane's Synonyms, used by Q&A): the
//! terms of the entity bound to the object in the Q&A linguistic schema of the model's culture
//! (`cultures[].linguisticMetadata.content.Entities`, JSON), docs/formats/model-objects.md
//! "Synonyms".
//!
//! An entity is bound by `Definition.Binding` (`ConceptualEntity` the table,
//! `ConceptualProperty` the column or measure); its `Terms` are `[{term: {State?, …}}]`. The
//! synonyms shown are the terms not `Suggested` or `Deleted`. Setting them keeps each listed term
//! that exists (a `Deleted` one comes back), adds the others as `{term: {}}`, marks the generated
//! terms left out `Deleted` (so they are not generated again), keeps the suggestions and drops the
//! other terms left out. An object with no entity gets one keyed as Desktop generates keys
//! (`sales.amount`); a model culture with no schema gets a JSON one.

use std::collections::HashMap;

use bicraft_model::SemanticModel;
use serde_json::{Map, Value, json};

use crate::rename_refs::{entity_binding, ling_pair};

/// Most synonyms an object takes.
pub(crate) const MAX_TERMS: usize = 100;
/// Longest synonym.
pub(crate) const MAX_TERM_CHARS: usize = 200;
/// The culture of a model that names none.
const DEFAULT_CULTURE: &str = "en-US";
/// The version written in a new schema (the JSON linguistic schema's).
const SCHEMA_VERSION: &str = "1.0.0";

/// `(table, column or measure)`, lower case: an object's key in [`all`].
pub(crate) type Key = (String, Option<String>);

fn culture_name(m: &SemanticModel) -> String {
    m.culture
        .clone()
        .filter(|c| !c.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_CULTURE.to_string())
}

fn is_culture(c: &Value, name: &str) -> bool {
    c.get("name")
        .and_then(Value::as_str)
        .is_some_and(|n| n.eq_ignore_ascii_case(name))
}

/// The content of a culture's linguistic metadata as a JSON value (JSON text parsed).
fn content(c: &Value) -> Option<Value> {
    match c.get("linguisticMetadata")?.get("content")? {
        Value::String(t) => serde_json::from_str::<Value>(t).ok(),
        v => Some(v.clone()),
    }
}

/// A term entry's text and state.
fn term(entry: &Value) -> Option<(&str, Option<&str>)> {
    let (text, v) = entry.as_object()?.iter().next()?;
    Some((text.as_str(), v.get("State").and_then(Value::as_str)))
}

fn shown(state: Option<&str>) -> bool {
    !matches!(state, Some(s) if s.eq_ignore_ascii_case("Suggested") || s.eq_ignore_ascii_case("Deleted"))
}

/// The synonyms of every bound object of the model culture's schema.
pub(crate) fn all(m: &SemanticModel) -> HashMap<Key, Vec<String>> {
    let mut out = HashMap::new();
    let name = culture_name(m);
    let Some(c) = m
        .extra
        .get("cultures")
        .and_then(Value::as_array)
        .and_then(|a| a.iter().find(|c| is_culture(c, &name)))
    else {
        return out;
    };
    let Some(content) = content(c) else { return out };
    let Some(entities) = content.get("Entities").and_then(Value::as_object) else {
        return out;
    };
    for v in entities.values() {
        let Some((entity, property)) = entity_binding(v) else {
            continue;
        };
        let terms: Vec<String> = v
            .get("Terms")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(term)
            .filter(|(_, st)| shown(*st))
            .map(|(t, _)| t.to_string())
            .take(MAX_TERMS)
            .collect();
        out.entry((entity.to_lowercase(), property.map(|p| p.to_lowercase())))
            .or_insert(terms);
    }
    out
}

/// Checks a list of synonyms (`synonyms` parameter) and returns it trimmed, without blanks and
/// without repeats (case-insensitive).
pub(crate) fn parse(v: &Value) -> Result<Vec<String>, String> {
    let list = v
        .as_array()
        .ok_or("parameter `synonyms` must be a list of strings")?;
    if list.len() > MAX_TERMS {
        return Err(format!("at most {MAX_TERMS} synonyms per object"));
    }
    let mut out: Vec<String> = Vec::new();
    for x in list {
        let t = x
            .as_str()
            .ok_or("parameter `synonyms` must be a list of strings")?
            .trim();
        if t.chars().count() > MAX_TERM_CHARS {
            return Err(format!("a synonym is at most {MAX_TERM_CHARS} characters"));
        }
        if !t.is_empty() && !out.iter().any(|o| o.eq_ignore_ascii_case(t)) {
            out.push(t.to_string());
        }
    }
    Ok(out)
}

fn binds(v: &Value, table: &str, property: Option<&str>) -> bool {
    entity_binding(v).is_some_and(|(e, p)| {
        e.eq_ignore_ascii_case(table)
            && match (p.as_deref(), property) {
                (None, None) => true,
                (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
                _ => false,
            }
    })
}

/// The terms of an entity after setting its synonyms to `wanted`.
fn new_terms(old: &[Value], wanted: &[String]) -> Vec<Value> {
    let mut out = Vec::new();
    for w in wanted {
        let found = old
            .iter()
            .find(|e| term(e).is_some_and(|(t, _)| t.eq_ignore_ascii_case(w)));
        match found {
            Some(e) => {
                let mut e = e.clone();
                // a deleted (or suggested) term listed again is the author's
                if let Some(inner) = e
                    .as_object_mut()
                    .and_then(|o| o.values_mut().next())
                    .and_then(Value::as_object_mut)
                    && inner
                        .get("State")
                        .and_then(Value::as_str)
                        .is_some_and(|s| !shown(Some(s)))
                {
                    inner.remove("State");
                }
                out.push(e);
            }
            None => out.push(json!({ w.as_str(): {} })),
        }
    }
    for e in old {
        let Some((t, state)) = term(e) else { continue };
        if wanted.iter().any(|w| w.eq_ignore_ascii_case(t)) {
            continue;
        }
        match state {
            Some(s) if s.eq_ignore_ascii_case("Suggested") || s.eq_ignore_ascii_case("Deleted") => {
                out.push(e.clone());
            }
            Some(s) if s.eq_ignore_ascii_case("Generated") => {
                let mut e = e.clone();
                if let Some(inner) = e
                    .as_object_mut()
                    .and_then(|o| o.values_mut().next())
                    .and_then(Value::as_object_mut)
                {
                    inner.insert("State".into(), json!("Deleted"));
                }
                out.push(e);
            }
            _ => {}
        }
    }
    out
}

/// Sets the synonyms of `table` (with `property`: its column or measure, named as in the model) to
/// `wanted`, in the schema of the model's culture.
pub(crate) fn set(
    m: &mut SemanticModel,
    table: &str,
    property: Option<&str>,
    wanted: &[String],
) -> Result<(), String> {
    let culture = culture_name(m);
    let cultures = m
        .extra
        .entry("cultures")
        .or_insert_with(|| Value::Array(Vec::new()));
    let list = cultures
        .as_array_mut()
        .ok_or("the model's `cultures` is not a list")?;
    let at = match list.iter().position(|c| is_culture(c, &culture)) {
        Some(i) => i,
        None => {
            list.push(json!({ "name": culture }));
            list.len().saturating_sub(1)
        }
    };
    let c = list
        .get_mut(at)
        .and_then(Value::as_object_mut)
        .ok_or("internal: culture entry")?;
    let (mut doc, as_text) = match c.get("linguisticMetadata") {
        None | Some(Value::Null) => (
            json!({"Version": SCHEMA_VERSION, "Language": culture, "Entities": {}}),
            false,
        ),
        Some(lm) => {
            let json_type = lm
                .get("contentType")
                .and_then(Value::as_str)
                .is_none_or(|t| t.eq_ignore_ascii_case("json"));
            let parsed = match lm.get("content") {
                Some(Value::String(t)) => serde_json::from_str::<Value>(t).ok().map(|v| (v, true)),
                Some(v @ Value::Object(_)) => Some((v.clone(), false)),
                _ => None,
            };
            match parsed {
                Some((v, text)) if json_type && v.is_object() => (v, text),
                _ => {
                    return Err(format!(
                        "the Q&A linguistic schema of culture `{culture}` is not JSON (a legacy XML \
                         schema): its synonyms cannot be written"
                    ));
                }
            }
        }
    };
    let root = doc.as_object_mut().ok_or("internal: schema")?;
    let entities = root
        .entry("Entities")
        .or_insert_with(|| Value::Object(Map::new()));
    let entities = entities
        .as_object_mut()
        .ok_or("the linguistic schema's `Entities` is not an object")?;
    let key = match entities
        .iter()
        .find(|(_, v)| binds(v, table, property))
        .map(|(k, _)| k.clone())
    {
        Some(k) => k,
        None => {
            if wanted.is_empty() {
                return Ok(());
            }
            let base = ling_pair(table, property);
            let mut k = base.clone();
            let mut n = 2u32;
            while entities.contains_key(&k) {
                k = format!("{base}_{n}");
                n = n.saturating_add(1);
                if n > 10_000 {
                    return Err("no free key in the linguistic schema".into());
                }
            }
            let mut binding = Map::new();
            binding.insert("ConceptualEntity".into(), json!(table));
            if let Some(p) = property {
                binding.insert("ConceptualProperty".into(), json!(p));
            }
            entities.insert(
                k.clone(),
                json!({"Definition": {"Binding": Value::Object(binding)}, "Terms": []}),
            );
            k
        }
    };
    let entity = entities
        .get_mut(&key)
        .and_then(Value::as_object_mut)
        .ok_or("internal: entity")?;
    let old: Vec<Value> = entity
        .get("Terms")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    entity.insert("Terms".into(), Value::Array(new_terms(&old, wanted)));
    let content = if as_text {
        Value::String(serde_json::to_string(&doc).map_err(|e| e.to_string())?)
    } else {
        doc
    };
    let lm = c
        .entry("linguisticMetadata")
        .or_insert_with(|| Value::Object(Map::new()));
    if !lm.is_object() {
        *lm = Value::Object(Map::new());
    }
    if let Some(o) = lm.as_object_mut() {
        o.insert("content".into(), content);
        o.entry("contentType").or_insert_with(|| json!("json"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terms_listed_kept_added_and_generated_ones_marked_deleted() {
        let old = vec![
            json!({"sales": {"State": "Generated"}}),
            json!({"revenue": {}}),
            json!({"turnover": {"State": "Suggested", "Weight": 0.5}}),
            json!({"income": {"State": "Deleted"}}),
        ];
        let got = new_terms(&old, &["Income".into(), "deals".into()]);
        assert_eq!(
            got,
            vec![
                json!({"income": {}}),
                json!({"deals": {}}),
                json!({"sales": {"State": "Deleted"}}),
                json!({"turnover": {"State": "Suggested", "Weight": 0.5}}),
            ]
        );
    }

    #[test]
    fn parse_trims_and_dedupes() {
        assert_eq!(
            parse(&json!([" a ", "A", "", "b"])).unwrap(),
            vec!["a".to_string(), "b".to_string()]
        );
        assert!(parse(&json!("a")).is_err());
        assert!(parse(&json!([1])).is_err());
    }
}
