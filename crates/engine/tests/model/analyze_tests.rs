//! `model.analyze` through the session.

use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::Session;

const MODEL: &str = r#"{"name":"M","compatibilityLevel":1567,"model":{"tables":[{"name":"Sales",
    "columns":[{"name":"Amount","dataType":"double","sourceColumn":"Amount"}],
    "measures":[{"name":"Total","expression":"IFERROR(SUM(Sales[Amount]), 0)"}],
    "partitions":[{"name":"Sales","source":{"type":"m","expression":"x"}}]}]}}"#;

fn open(dir: &TempDir) -> Session {
    let path = dir.path().join("model.bim");
    std::fs::write(&path, MODEL).unwrap();
    let mut s = Session::new();
    s.execute("file.open", &json!({"path": path})).unwrap();
    s
}

fn ids(v: &Value) -> Vec<String> {
    v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["rule_id"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn default_rules_run_and_every_rule_is_evaluated() {
    let dir = TempDir::new("engine-analyze-default");
    let mut s = open(&dir);
    let rev = s.revision;
    let v = s.execute("model.analyze", &json!({})).unwrap();
    assert_eq!(s.revision, rev, "model.analyze is read-only");
    assert_eq!(v["summary"]["rules"], 71);
    assert_eq!(v["summary"]["evaluated"], 71, "{}", v["rules"]);
    let ids = ids(&v);
    assert!(ids.iter().any(|i| i == "AVOID_USING_THE_IFERROR_FUNCTION"));
    assert!(ids.iter().any(|i| i == "AVOID_FLOATING_POINT_DATA_TYPES"));
    assert!(v["findings"][0]["object"].is_string());
}

#[test]
fn extra_rule_files_inline_rules_and_filters() {
    let dir = TempDir::new("engine-analyze-extra");
    let mut s = open(&dir);
    let file = dir.path().join("mine.json");
    std::fs::write(
        &file,
        r#"[{"ID":"MY_TOTAL","Name":"No measure named Total","Severity":3,"Scope":"Measure",
            "Expression":"Name == \"Total\""}, {"Name": "invalid: no ID"}]"#,
    )
    .unwrap();
    let v = s
        .execute("model.analyze", &json!({"rules": file, "default_rules": false}))
        .unwrap();
    assert_eq!(ids(&v), ["MY_TOTAL"]);
    assert_eq!(v["rules"][0]["source"], file.to_string_lossy().as_ref());
    assert_eq!(v["invalid_rules"].as_array().unwrap().len(), 1);

    let v = s
        .execute(
            "model.analyze",
            &json!({"rules": [{"ID":"INLINE","Severity":1,"Scope":"Model","Expression":"Tables.Count = 1"}],
                    "rule_ids": ["inline", "AVOID_FLOATING_POINT_DATA_TYPES"]}),
        )
        .unwrap();
    let mut got = ids(&v);
    got.sort();
    assert_eq!(got, ["AVOID_FLOATING_POINT_DATA_TYPES", "INLINE"]);

    let v = s
        .execute(
            "model.analyze",
            &json!({"severity_min": "error", "max_findings": 0}),
        )
        .unwrap();
    assert!(
        v["rules"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["severity"] == "error")
    );
    assert_eq!(v["findings"].as_array().unwrap().len(), 0);

    for bad in [
        json!({"rules": 3}),
        json!({"rules": dir.path().join("missing.json")}),
        json!({"severity_min": 9}),
        json!({"rule_ids": "x"}),
        json!({"nope": 1}),
    ] {
        assert!(s.execute("model.analyze", &bad).is_err(), "{bad}");
    }
}

#[test]
fn analyze_needs_a_model() {
    let mut s = Session::new();
    assert!(s.execute("model.analyze", &json!({})).is_err());
}
