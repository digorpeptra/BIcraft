//! End-to-end tests of `bicraft test` (CI quality gates, docs/ci-tests.md): exit codes, formats,
//! `--output`, paths relative to the current folder.

use std::process::Command;

use serde_json::Value;

#[path = "../../../testsupport/tempdir.rs"]
mod tempdir;
use tempdir::TempDir;

const MODEL: &str = r#"{
 "name": "Gates",
 "compatibilityLevel": 1567,
 "model": {
  "tables": [{
   "name": "Sales",
   "columns": [{"type": "calculatedTableColumn", "name": "Amount", "dataType": "double", "sourceColumn": "[Amount]"}],
   "partitions": [{"name": "Sales", "source": {"type": "calculated", "expression": "DATATABLE(\"Amount\", DOUBLE, {{10}, {32}})"}}],
   "measures": [{"name": "Total", "expression": "SUM(Sales[Amount])"}]
  }]
 }
}"#;

/// A folder with `proj/model.bim` and `proj/bicraft.tests.yaml` holding `tests`.
fn setup(name: &str, tests: &str) -> TempDir {
    let dir = TempDir::new(&format!("cli-citest-{name}"));
    let proj = dir.path().join("proj");
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::write(proj.join("model.bim"), MODEL).unwrap();
    std::fs::write(proj.join("bicraft.tests.yaml"), tests).unwrap();
    dir
}

fn bicraft(dir: &TempDir, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_bicraft"))
        .current_dir(dir.path())
        .arg("test")
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

const PASSING: &str = "version: 1\ndax:\n  - measure: Total\n    expect: 42\n";
const FAILING: &str = "version: 1\ndax:\n  - measure: Total\n    expect: 41\n  - name: Soft\n    measure: Total\n    expect: 0\n    severity: warning\n";

#[test]
fn passing_gates_exit_zero() {
    let dir = setup("pass", PASSING);
    let (code, out, err) = bicraft(&dir, &["proj/model.bim"]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(out.contains("PASS  total"), "{out}");
    assert!(out.contains("1 tests: 1 passed"), "{out}");
    let (code, out, _) = bicraft(&dir, &["proj/model.bim", "--format", "json"]);
    assert_eq!(code, 0);
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["passed"], true);
    // relative to the current folder
    assert_eq!(v["project"], "proj/model.bim");
    assert_eq!(v["tests_file"], "proj/bicraft.tests.yaml");
}

#[test]
fn failing_gates_exit_one_and_write_reports() {
    let dir = setup("fail", FAILING);
    let (code, out, _) = bicraft(
        &dir,
        &["proj/model.bim", "--format", "junit", "--output", "junit.xml"],
    );
    assert_eq!(code, 1, "{out}");
    // the text summary goes to stdout, the report to the file
    assert!(out.contains("FAIL  total") && out.contains("WARN  soft"), "{out}");
    let xml = std::fs::read_to_string(dir.path().join("junit.xml")).unwrap();
    assert!(
        xml.starts_with("<?xml") && xml.contains("failures=\"1\""),
        "{xml}"
    );
    let (code, _, _) = bicraft(
        &dir,
        &["proj/model.bim", "--format", "sarif", "--output", "out.sarif"],
    );
    assert_eq!(code, 1);
    let sarif: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.path().join("out.sarif")).unwrap()).unwrap();
    assert_eq!(sarif["version"], "2.1.0");
    assert_eq!(
        sarif["runs"][0]["results"][0]["locations"][0]["physicalLocation"]["artifactLocation"]["uri"],
        "proj/bicraft.tests.yaml"
    );
    // warnings gate with --fail-on warning; the error still fails
    let (code, _, _) = bicraft(&dir, &["proj/model.bim", "--fail-on", "warning"]);
    assert_eq!(code, 1);
    std::fs::write(
        dir.path().join("soft.yaml"),
        "version: 1\ndax:\n  - measure: Total\n    expect: 0\n    severity: warning\n",
    )
    .unwrap();
    assert_eq!(bicraft(&dir, &["proj/model.bim", "--tests", "soft.yaml"]).0, 0);
    assert_eq!(
        bicraft(
            &dir,
            &["proj/model.bim", "--tests", "soft.yaml", "--fail-on", "warning"]
        )
        .0,
        1
    );
}

#[test]
fn configuration_and_usage_errors_exit_two() {
    let dir = setup("config", "version: 1\ndax: [{measure: Total, expected: 1}]\n");
    let (code, _, err) = bicraft(&dir, &["proj/model.bim"]);
    assert_eq!(code, 2);
    assert!(err.contains("unknown key `expected`"), "{err}");
    for args in [
        vec!["proj/model.bim", "--format", "xml"],
        vec!["proj/model.bim", "--fail-on", "info"],
        vec!["proj/model.bim", "--nope"],
        vec!["proj/model.bim", "other"],
        vec!["missing/model.bim"],
        vec!["proj/model.bim", "--tests", "missing.yaml"],
    ] {
        assert_eq!(bicraft(&dir, &args).0, 2, "{args:?}");
    }
    let (code, out, _) = bicraft(&dir, &["--help"]);
    assert_eq!(code, 0);
    assert!(out.contains("Exit codes"));
}
