//! What the engine passes to `bicraft-data` when it loads a partition's data (gap G21): the
//! model's culture, the M queries and parameters a partition references by name, and the
//! session's opt-in data root (`data_root`, the only folder Power Query `File.Contents` may
//! read). Also how text values come out of a load: as Analysis Services stores them
//! (`bicraft_vertipaq::system::TextStorage`), so the viewer and the structural save see the
//! values Desktop would.
//!
//! The data root is never inferred: it is `None` (no file access) until a command is given
//! `data_root` (`file.open`, `file.save`, `file.save_as`, `table.import`, `table.set_data`, or
//! `bicraft-cli view --data-root`).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use bicraft_data::{Limits, SourceOptions};
use bicraft_model::SemanticModel;
use bicraft_vertipaq::CellValue;
use bicraft_vertipaq::system::TextStorage;
use serde_json::Value;

/// Most shared queries considered for one partition.
const MAX_SHARED: usize = 4096;

/// A `data_root` parameter: absent → `None` (unchanged), `null` → `Some(None)` (no file access),
/// a path → `Some(Some(dir))`, which must be an existing folder (returned absolute).
pub(crate) fn data_root_param(p: &Value) -> Result<Option<Option<PathBuf>>, String> {
    match p.get("data_root") {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(s)) if s.trim().is_empty() => Ok(Some(None)),
        Some(Value::String(s)) => {
            let dir = std::path::absolute(s).map_err(|e| format!("data_root `{s}`: {e}"))?;
            if !dir.is_dir() {
                return Err(format!("data_root `{s}` is not an existing folder"));
            }
            Ok(Some(Some(dir)))
        }
        Some(_) => Err("parameter `data_root` must be a folder path (or null)".into()),
    }
}

/// The parameters without `data_root` (for code that checks its own allowed keys).
pub(crate) fn without_data_root(p: &Value) -> Value {
    match p {
        Value::Object(o) if o.contains_key("data_root") => {
            let mut o = o.clone();
            o.remove("data_root");
            Value::Object(o)
        }
        other => other.clone(),
    }
}

/// The root a command uses: its own `data_root` when given, else the session's.
pub(crate) fn effective_root(given: &Option<Option<PathBuf>>, session: Option<&Path>) -> Option<PathBuf> {
    match given {
        Some(r) => r.clone(),
        None => session.map(Path::to_path_buf),
    }
}

/// The culture M conversions use: the model's `sourceQueryCulture`, else its `culture`.
pub(crate) fn query_culture(m: &SemanticModel) -> Option<String> {
    m.extra
        .get("sourceQueryCulture")
        .and_then(Value::as_str)
        .filter(|c| !c.trim().is_empty())
        .map(str::to_string)
        .or_else(|| m.culture.clone())
}

/// Characters of an M regular identifier (dotted names such as `Sales.Region` included).
fn ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '.'
}

/// Whether M text `expr` mentions the query `name`: as `#"name"` or as a whole identifier.
fn mentions(expr: &str, name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let quoted = format!("#\"{}\"", name.replace('"', "\"\""));
    if expr.contains(&quoted) {
        return true;
    }
    if !name.chars().all(ident_char) {
        return false;
    }
    let mut from = 0;
    while let Some(at) = expr.get(from..).and_then(|rest| rest.find(name)) {
        let start = from.saturating_add(at);
        let end = start.saturating_add(name.len());
        let before = expr.get(..start).and_then(|s| s.chars().next_back());
        let after = expr.get(end..).and_then(|s| s.chars().next());
        if !before.is_some_and(|c| ident_char(c) || c == '"' || c == '#')
            && !after.is_some_and(|c| ident_char(c) || c == '"')
        {
            return true;
        }
        from = end;
    }
    false
}

/// The named M expressions `expr` (the partition of table `table`) may reference: the model's M
/// expressions (queries, parameters) and the single M partition of every other table, keeping
/// only those `expr` mentions, directly or through each other. Unrelated queries are left out,
/// so one the evaluator cannot parse never blocks another table.
pub(crate) fn shared_for(m: &SemanticModel, table: &str, expr: &str) -> Vec<(String, String)> {
    shared_with_drafts(m, table, expr, &[])
}

/// [`shared_for`] with the Power Query editor's `drafts` (name, M) in place of the model's
/// queries of the same names, and added when new: a preview reads the other queries as the
/// editor has them.
pub(crate) fn shared_with_drafts(
    m: &SemanticModel,
    table: &str,
    expr: &str,
    drafts: &[(String, String)],
) -> Vec<(String, String)> {
    let drafted = |n: &str| drafts.iter().any(|(d, _)| d.eq_ignore_ascii_case(n));
    let mut candidates: Vec<(String, String)> = drafts
        .iter()
        .filter(|(n, _)| !n.eq_ignore_ascii_case(table))
        .cloned()
        .collect();
    candidates.extend(
        m.expressions
            .iter()
            .filter(|e| (e.kind.is_empty() || e.kind.eq_ignore_ascii_case("m")) && !drafted(&e.name))
            .map(|e| (e.name.clone(), e.expression.clone())),
    );
    for t in &m.tables {
        if t.name.eq_ignore_ascii_case(table)
            || drafted(&t.name)
            || candidates.iter().any(|(n, _)| n == &t.name)
        {
            continue;
        }
        if let [p] = t.partitions.as_slice()
            && p.source_type.eq_ignore_ascii_case("m")
            && let Some(e) = &p.expression
        {
            candidates.push((t.name.clone(), e.clone()));
        }
    }
    candidates.truncate(MAX_SHARED);
    let mut used: HashSet<usize> = HashSet::new();
    let mut pending: Vec<&str> = vec![expr];
    while let Some(text) = pending.pop() {
        for (i, (name, e)) in candidates.iter().enumerate() {
            if !used.contains(&i) && mentions(text, name) {
                used.insert(i);
                pending.push(e);
            }
        }
    }
    candidates
        .into_iter()
        .enumerate()
        .filter(|(i, _)| used.contains(i))
        .map(|(_, c)| c)
        .collect()
}

/// `SourceOptions` for the M partition `expr` of table `table`.
pub(crate) fn source_options(
    model: Option<&SemanticModel>,
    table: &str,
    expr: &str,
    root: Option<&Path>,
    limits: Limits,
) -> SourceOptions {
    let now = crate::refresh::now().0;
    SourceOptions {
        limits,
        file_roots: root.map(Path::to_path_buf).into_iter().collect(),
        culture: model.and_then(query_culture),
        shared: model.map(|m| shared_for(m, table, expr)).unwrap_or_default(),
        strict_errors: false,
        connectors: Default::default(),
        // the clock functions read the time of the evaluation (an import, a preview load, a save
        // that materializes the data; `model.refresh` sets its start time); local time is the
        // process's local clock at that instant: UTC by default and on a server, as in the Power
        // BI service, the machine's time zone in the desktop app and the CLI (crate::clock)
        clock_utc_ms: Some(now),
        clock_local_offset_minutes: Some(crate::clock::offset_at_serial_ms(now)),
    }
}

/// A load error, with what to do when the query needs files and no data root was given.
pub(crate) fn describe(e: &bicraft_data::Error, root: Option<&Path>) -> String {
    let text = e.to_string();
    if root.is_none() && text.contains("file access is disabled") {
        format!(
            "{text}; this query reads local files: pass `data_root` (the folder they are in, or a parent) \
             to allow it"
        )
    } else {
        text
    }
}

/// Values as Analysis Services stores them in a column of `model`: texts deduplicated per the
/// model's culture (the first spelling wins), imported texts without trailing blanks.
pub(crate) fn stored_values(model: &SemanticModel, imported: bool, values: &[CellValue]) -> Vec<CellValue> {
    if !values.iter().any(|v| matches!(v, CellValue::Text(_))) {
        return values.to_vec();
    }
    let culture = model.culture.as_deref().unwrap_or("en-US");
    TextStorage::for_culture(culture, imported).canonicalize(values)
}

/// How relationship ends of `model` match text keys: the stored-text identity of its culture,
/// applied to stored values (so without import trimming, already done).
pub(crate) fn relationship_keys(model: &SemanticModel) -> TextStorage {
    TextStorage::for_culture(model.culture.as_deref().unwrap_or("en-US"), false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bicraft_model::{NamedExpression, Partition, Table};

    #[test]
    fn mentions_whole_names_only() {
        assert!(mentions("let a = Param in a", "Param"));
        assert!(mentions("Source = #\"My Query\"", "My Query"));
        assert!(mentions("Source = #\"Say \"\"hi\"\"\"", "Say \"hi\""));
        assert!(!mentions("let a = Params in a", "Param"));
        assert!(!mentions("let a = X.Param in a", "Param"));
        assert!(!mentions("\"Param\"", "Param"));
        assert!(!mentions("let My Query = 1 in 1", "My Query"));
        assert!(!mentions("", "x"));
    }

    #[test]
    fn shared_queries_are_the_referenced_ones() {
        let m = SemanticModel {
            culture: Some("fr-FR".into()),
            expressions: vec![
                NamedExpression {
                    name: "Rate".into(),
                    kind: "m".into(),
                    expression: "2 meta [IsParameterQuery = true]".into(),
                    ..Default::default()
                },
                NamedExpression {
                    name: "Broken".into(),
                    kind: "m".into(),
                    expression: "let in".into(),
                    ..Default::default()
                },
                NamedExpression {
                    name: "Base".into(),
                    kind: "m".into(),
                    expression: "#table({\"A\"}, {{Rate}})".into(),
                    ..Default::default()
                },
            ],
            tables: vec![Table {
                name: "Other".into(),
                partitions: vec![Partition {
                    source_type: "m".into(),
                    expression: Some("Base".into()),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let names = |v: Vec<(String, String)>| v.into_iter().map(|(n, _)| n).collect::<Vec<_>>();
        assert_eq!(names(shared_for(&m, "T", "Other")), ["Rate", "Base", "Other"]);
        assert_eq!(names(shared_for(&m, "Other", "Base")), ["Rate", "Base"]);
        assert!(shared_for(&m, "T", "#table({}, {})").is_empty());
        let o = source_options(Some(&m), "T", "Rate", None, Limits::default());
        assert_eq!(o.culture.as_deref(), Some("fr-FR"));
        assert!(o.file_roots.is_empty());
        assert_eq!(names(o.shared), ["Rate"]);
    }

    #[test]
    fn data_root_parameter() {
        assert_eq!(data_root_param(&serde_json::json!({})).unwrap(), None);
        assert_eq!(
            data_root_param(&serde_json::json!({"data_root": null})).unwrap(),
            Some(None)
        );
        assert!(data_root_param(&serde_json::json!({"data_root": 3})).is_err());
        let dir = std::env::temp_dir();
        let got = data_root_param(&serde_json::json!({"data_root": dir})).unwrap();
        assert!(matches!(got, Some(Some(p)) if p.is_dir()));
        let missing = dir.join("bicraft-no-such-folder-for-data-root");
        assert!(data_root_param(&serde_json::json!({"data_root": missing})).is_err());
        let v = without_data_root(&serde_json::json!({"data_root": "x", "a": 1}));
        assert_eq!(v, serde_json::json!({"a": 1}));
    }
}
