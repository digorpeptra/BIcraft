//! End-to-end tests of `bicraft-cli sources` and `bicraft-cli env` (docs/sources-env.md): saving,
//! dry runs, exit codes for CI pipelines.

use std::process::Command;

use serde_json::Value;

#[path = "../../../testsupport/tempdir.rs"]
mod tempdir;
use tempdir::TempDir;

const MODEL: &str = r#"{
 "name": "Envs",
 "compatibilityLevel": 1567,
 "model": {
  "expressions": [
   {"name": "Year", "kind": "m", "expression": "2025 meta [IsParameterQuery=true, Type=\"Number\", IsParameterQueryRequired=true]"}
  ],
  "tables": [{
   "name": "Sales",
   "columns": [{"name": "Amount", "dataType": "double", "sourceColumn": "Amount"}],
   "partitions": [{"name": "Sales", "source": {"type": "m",
     "expression": "let\n    Source = Csv.Document(File.Contents(\"C:\\Data\\Finance\\sales.csv\")),\n    Db = Sql.Database(\"dev-sql\", \"Fin\")\nin\n    Source"}}]
  }]
 }
}"#;

const PROFILES: &str = "version: 1\nprofiles:\n  prod:\n    parameters:\n      Year: 2026\n      Root: '\\\\files\\fin\\'\n    servers:\n      dev-sql: prod-sql\n    databases:\n      Fin: Fin\n  bad:\n    parameters:\n      Year: soon\n";

fn setup(name: &str) -> TempDir {
    let dir = TempDir::new(&format!("cli-sources-{name}"));
    std::fs::write(dir.path().join("model.bim"), MODEL).unwrap();
    std::fs::write(dir.path().join("bicraft.env.yaml"), PROFILES).unwrap();
    dir
}

fn bicraft(dir: &TempDir, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_bicraft-cli"))
        .current_dir(dir.path())
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

fn bim(dir: &TempDir) -> String {
    std::fs::read_to_string(dir.path().join("model.bim")).unwrap()
}

#[test]
fn list_rebase_and_profiles() {
    let dir = setup("flow");
    let (code, out, err) = bicraft(&dir, &["sources", "model.bim"]);
    assert_eq!(code, 0, "{err}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        v["sources"][1]["arguments"][0]["value"],
        "C:\\Data\\Finance\\sales.csv"
    );

    // a dry run writes nothing
    let before = bim(&dir);
    let (code, out, err) = bicraft(
        &dir,
        &[
            "sources",
            "rebase",
            "model.bim",
            "--from",
            "C:\\Data\\Finance",
            "--param",
            "Root",
            "--dry-run",
        ],
    );
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("Root & \\\"sales.csv\\\""), "{out}");
    assert_eq!(bim(&dir), before);

    // the rebase is saved
    let (code, out, err) = bicraft(
        &dir,
        &[
            "sources",
            "rebase",
            "model.bim",
            "--from",
            "C:\\Data\\Finance",
            "--param",
            "Root",
        ],
    );
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("\"saved\""), "{out}");
    assert!(bim(&dir).contains("Root & \\\"sales.csv\\\""));

    // check: the prod profile covers it now; apply saves
    let (code, out, _) = bicraft(&dir, &["env", "check", "model.bim", "prod"]);
    assert_eq!(code, 0, "{out}");
    let (code, _, err) = bicraft(&dir, &["env", "apply", "model.bim", "prod"]);
    assert_eq!(code, 0, "{err}");
    let text = bim(&dir);
    assert!(text.contains("2026 meta") && text.contains("prod-sql"), "{text}");
    assert!(text.contains("\\\\\\\\files\\\\fin\\\\"), "{text}");

    // a profile with errors: check exits 1, apply refuses (2) and writes nothing
    let (code, _, _) = bicraft(&dir, &["env", "check", "model.bim", "bad"]);
    assert_eq!(code, 1);
    let before = bim(&dir);
    let (code, _, err) = bicraft(&dir, &["env", "apply", "model.bim", "bad"]);
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("Number"), "{err}");
    assert_eq!(bim(&dir), before);
}

#[test]
fn usage_errors_exit_two() {
    let dir = setup("usage");
    for args in [
        vec!["sources"],
        vec!["sources", "model.bim", "extra"],
        vec!["sources", "rebase", "model.bim", "--nope"],
        vec!["sources", "rebase", "model.bim", "--server", "noequals"],
        vec!["sources", "rebase"],
        vec!["env"],
        vec!["env", "apply", "model.bim"],
        vec!["env", "apply", "model.bim", "nope"],
        vec!["env", "check", "model.bim", "prod", "--env-file", "missing.yaml"],
        vec!["env", "check", "missing.bim", "prod"],
    ] {
        assert_eq!(bicraft(&dir, &args).0, 2, "{args:?}");
    }
    for args in [vec!["sources", "--help"], vec!["env", "--help"]] {
        let (code, out, _) = bicraft(&dir, &args);
        assert_eq!(code, 0);
        assert!(out.contains("Exit codes"), "{out}");
    }
}
