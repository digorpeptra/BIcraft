use serde_json::json;

use crate::tempdir::TempDir;
use bicraft_engine::{COMMANDS, Session, list_commands};

fn bim(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("model.bim");
    std::fs::write(
        &path,
        r#"{"name":"M","model":{"tables":[{"name":"Sales",
            "columns":[{"name":"Amount","dataType":"double"},
                       {"name":"Bad","dataType":"double","type":"calculated","expression":"Sales[Nope]"}],
            "measures":[{"name":"Total","expression":"SUM(Sales[Amount])"}]}]}}"#,
    )
    .unwrap();
    path
}

fn temp(name: &str) -> TempDir {
    TempDir::new(&format!("engine-{name}"))
}

#[test]
fn command_ids_are_unique_and_documented() {
    let mut ids: Vec<_> = COMMANDS.iter().map(|c| c.id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), COMMANDS.len());
    assert!(
        COMMANDS
            .iter()
            .all(|c| !c.label.is_empty() && !c.params.is_empty())
    );
    let dax: Vec<_> = list_commands(Some("dax")).iter().map(|c| c.id).collect();
    assert!(
        dax.contains(&"dax.validate") && dax.contains(&"dax.validate_query"),
        "{dax:?}"
    );
    assert!(list_commands(Some("no-such-command")).is_empty());
}

#[test]
fn open_validate_and_summarise_a_model() {
    let dir = temp("open");
    let mut s = Session::new();
    s.execute("file.open", &json!({"path": bim(&dir)})).unwrap();
    let summary = s.execute("model.summary", &json!({})).unwrap();
    assert_eq!(summary["tables"][0]["measures"][0], "Total");

    let ok = s
        .execute("dax.validate", &json!({"expression": "[Total] * 2"}))
        .unwrap();
    assert_eq!(ok["valid"], true);
    assert_eq!(ok["checked_against_model"], true);

    let report = s.execute("model.validate", &json!({})).unwrap();
    assert_eq!(report["invalid_count"], 1);
    assert_eq!(report["objects_with_diagnostics"][0]["name"], "Bad");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn errors_leave_the_session_unchanged() {
    let mut s = Session::new();
    assert!(s.execute("nope", &json!({})).is_err());
    assert!(s.execute("file.open", &json!({})).is_err());
    assert!(
        s.execute("file.open", &json!({"path": "/does/not/exist.pbix"}))
            .is_err()
    );
    assert!(s.source.is_none());
    assert!(s.execute("model.summary", &json!({})).is_err());
    // Validation works without a model (syntax and arity only).
    let v = s
        .execute("dax.validate", &json!({"expression": "SUM(T[C])"}))
        .unwrap();
    assert_eq!(v["checked_against_model"], false);
}

#[test]
fn batch_stops_at_the_first_error() {
    let mut s = Session::new();
    let steps = vec![
        ("dax.validate".to_string(), json!({"expression": "1"})),
        ("model.summary".to_string(), json!({})),
        ("dax.validate".to_string(), json!({"expression": "2"})),
    ];
    let (index, _, done) = s.batch(&steps).unwrap_err();
    assert_eq!(index, 1);
    assert_eq!(done.len(), 1);
}

#[test]
fn validate_dax_queries() {
    let mut s = Session::new();
    // Without a model: structure only.
    let v = s
        .execute("dax.validate_query", &json!({"query": "EVALUATE 1"}))
        .unwrap();
    assert_eq!(v["valid"], false);
    assert_eq!(v["checked_against_model"], false);
    assert_eq!(v["diagnostics"][0]["code"], "evaluate-needs-table");
    assert!(s.execute("dax.validate_query", &json!({})).is_err());
    assert!(
        s.execute(
            "dax.validate_query",
            &json!({"query": "EVALUATE {1}", "expression": "x"})
        )
        .is_err()
    );

    let dir = temp("query");
    s.execute("file.open", &json!({"path": bim(&dir)})).unwrap();
    let query = "DEFINE\n  MEASURE Sales[Double] = [Total] * 2\n  VAR v = 1\nEVALUATE\n  ROW(\"D\", [Double], \"V\", v)\nORDER BY [D] DESC";
    let v = s.execute("dax.validate_query", &json!({"query": query})).unwrap();
    assert_eq!(v["valid"], true, "{v}");
    assert_eq!(v["checked_against_model"], true);
    assert_eq!(v["evaluate_count"], 1);
    assert_eq!(
        v["definitions"],
        json!([{"kind": "measure", "table": "Sales", "name": "Double"}, {"kind": "var", "name": "v"}])
    );
    // The query measure was not added to the model.
    let summary = s.execute("model.summary", &json!({})).unwrap();
    assert_eq!(summary["tables"][0]["measures"], json!(["Total"]));

    let v = s
        .execute(
            "dax.validate_query",
            &json!({"query": "EVALUATE Sales ORDER BY Sales[Nope]"}),
        )
        .unwrap();
    assert_eq!(v["valid"], false);
    assert_eq!(v["diagnostics"][0]["code"], "unknown-column");
    assert_eq!(v["diagnostics"][0]["column"], 25);
    let _ = std::fs::remove_dir_all(&dir);
}
