//! The Copilot container of the open file (`docs/formats/pbix-package.md` §2, "Copilot
//! containers"): PBIX parts `Copilot/…` and `VerifiedAnswers/…`, project folders
//! `<Name>.SemanticModel/Copilot/` and `VerifiedAnswers/`. bicraft does not edit it: it is kept
//! byte for byte on save, moved on conversion (text re-encoded between the package's UTF-16LE
//! and the project's UTF-8) and summarized by `file.inspect`.

use serde_json::{Value, json};

use crate::{Session, Source};

/// The container files of the open file in their project form (path relative to the package
/// root / semantic model folder, bytes). A project folder that cannot be read is reported in
/// `warnings`.
pub(crate) fn source_files(s: &Session, warnings: &mut Vec<String>) -> Vec<(String, Vec<u8>)> {
    match &s.source {
        Some(Source::Package { package, .. }) => bicraft_pbix::copilot_files(package),
        Some(Source::Project { project, .. }) => match bicraft_pbip::read_copilot_files(project) {
            Ok(files) => files,
            Err(e) => {
                warnings.push(format!("the Copilot folder was not carried over: {e}"));
                Vec::new()
            }
        },
        _ => Vec::new(),
    }
}

fn json_file(files: &[(String, Vec<u8>)], path: &str) -> Option<Value> {
    files
        .iter()
        .find(|(p, _)| p.eq_ignore_ascii_case(path))
        .and_then(|(_, b)| serde_json::from_slice(b).ok())
}

/// What `file.inspect` reports about the container: `null` when there is none. Counts and
/// flags only, never the instructions or prompts themselves.
pub(crate) fn summary(files: &[(String, Vec<u8>)]) -> Value {
    if files.is_empty() {
        return Value::Null;
    }
    let lower = |p: &str| p.to_ascii_lowercase();
    let instructions = files
        .iter()
        .find(|(p, _)| lower(p) == "copilot/instructions/instructions.md")
        .map(|(_, b)| String::from_utf8_lossy(b).chars().count());
    let schema = json_file(files, "Copilot/schema.json");
    let tables = schema
        .as_ref()
        .and_then(|v| v.get("tables"))
        .and_then(Value::as_array);
    let count = |key: &str| -> usize {
        tables.map_or(0, |t| {
            t.iter()
                .filter_map(|x| x.get(key).and_then(Value::as_array))
                .map(Vec::len)
                .sum()
        })
    };
    let prompts = json_file(files, "Copilot/examplePrompts.json")
        .and_then(|v| v.get("prompts").and_then(Value::as_array).map(Vec::len));
    let indexing = json_file(files, "Copilot/settings.json")
        .and_then(|v| v.get("indexingEnabled").and_then(Value::as_bool));
    let version = json_file(files, "Copilot/version.json")
        .and_then(|v| v.get("version").and_then(Value::as_str).map(str::to_string));
    let answers = files
        .iter()
        .filter(|(p, _)| {
            let p = lower(p);
            (p.starts_with("copilot/verifiedanswers/definitions/")
                || p.starts_with("verifiedanswers/definitions/"))
                && p.ends_with(".json")
        })
        .count();
    json!({
        "files": files.iter().map(|(p, b)| json!({"path": p, "bytes": b.len()})).collect::<Vec<_>>(),
        "version": version,
        "instructions_chars": instructions,
        "schema_tables": tables.map(Vec::len),
        "schema_columns": tables.map(|_| count("columns")),
        "schema_measures": tables.map(|_| count("measures")),
        "example_prompts": prompts,
        "indexing_enabled": indexing,
        "verified_answers": answers,
        "legacy_verified_answers": files.iter().any(|(p, _)| lower(p).starts_with("verifiedanswers/")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarizes_without_content() {
        assert_eq!(summary(&[]), Value::Null);
        let f = |p: &str, b: &str| (p.to_string(), b.as_bytes().to_vec());
        let files = vec![
            f("Copilot/version.json", r#"{"version":"1.0.0"}"#),
            f("Copilot/Instructions/instructions.md", "Réponds"),
            f(
                "Copilot/schema.json",
                r#"{"tables":[{"name":"T","columns":[{"name":"a"},{"name":"b"}],"measures":[{"name":"m"}]}]}"#,
            ),
            f("Copilot/examplePrompts.json", r#"{"prompts":["x","y"]}"#),
            f("Copilot/settings.json", r#"{"indexingEnabled":false}"#),
            f("Copilot/VerifiedAnswers/definitions/a.json", "{}"),
        ];
        let s = summary(&files);
        assert_eq!(s["version"], "1.0.0");
        assert_eq!(s["instructions_chars"], 7);
        assert_eq!(s["schema_tables"], 1);
        assert_eq!(s["schema_columns"], 2);
        assert_eq!(s["schema_measures"], 1);
        assert_eq!(s["example_prompts"], 2);
        assert_eq!(s["indexing_enabled"], false);
        assert_eq!(s["verified_answers"], 1);
        assert_eq!(s["legacy_verified_answers"], false);
        assert!(!s.to_string().contains("Réponds"));
    }
}
