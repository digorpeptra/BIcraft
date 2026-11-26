//! `bicraft import SOURCE TARGET`: copies objects between files and saves the target.

use std::process::Command;

use serde_json::{Value, json};

#[path = "../../../testsupport/tempdir.rs"]
mod tempdir;
use tempdir::TempDir;

fn bicraft(args: &[&str]) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_bicraft"))
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap() + &String::from_utf8(out.stderr).unwrap(),
    )
}

const SOURCE: &str = r#"{"name":"Src","model":{"tables":[{"name":"Sales",
    "columns":[{"name":"Amount","dataType":"double","sourceColumn":"Amount"}],
    "measures":[{"name":"Total","expression":"SUM(Sales[Amount])"},
                {"name":"Twice","expression":"[Total] * 2"}]}]}}"#;
const TARGET: &str = r#"{"name":"Tgt","model":{"tables":[{"name":"Sales",
    "columns":[{"name":"Amount","dataType":"double","sourceColumn":"Amount"}],
    "measures":[{"name":"Total","expression":"1"}]}]}}"#;

fn measures(path: &str) -> Value {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    v["model"]["tables"][0]["measures"].clone()
}

#[test]
fn import_copies_saves_and_reports() {
    let dir = TempDir::new("cli-import");
    let src = dir.join("src.bim");
    let tgt = dir.join("tgt.bim");
    std::fs::write(&src, SOURCE).unwrap();
    std::fs::write(&tgt, TARGET).unwrap();
    let (s, t) = (src.to_str().unwrap(), tgt.to_str().unwrap());

    // conflict on Total: refused (exit 1), nothing written
    let (code, out) = bicraft(&["import", s, t, "--measure", "Total"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("already exist"), "{out}");
    assert_eq!(std::fs::read_to_string(&tgt).unwrap(), TARGET);

    // a dry run prints the plan and writes nothing
    let (code, out) = bicraft(&["import", s, t, "--measure", "Twice", "--dry-run", "--json"]);
    assert_eq!(code, 0, "{out}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["would_apply"], true, "{v}");
    assert_eq!(std::fs::read_to_string(&tgt).unwrap(), TARGET);

    // Twice uses the target's Total; the file is saved
    let (code, out) = bicraft(&["import", s, t, "--measure", "Twice"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("added") && out.contains("Twice") && out.contains("saved"),
        "{out}"
    );
    let names: Vec<Value> = measures(t)
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].clone())
        .collect();
    assert_eq!(names, [json!("Total"), json!("Twice")]);

    // rename on conflict
    let (code, out) = bicraft(&[
        "import",
        s,
        t,
        "--measure",
        "Total",
        "--on-conflict",
        "rename",
        "--json",
    ]);
    assert_eq!(code, 0, "{out}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["objects"][0]["name"], "Total (imported)", "{v}");

    // usage errors
    assert_eq!(bicraft(&["import", s]).0, 2);
    assert_eq!(bicraft(&["import", s, t, "--bogus"]).0, 2);
    assert_eq!(bicraft(&["import", s, t, "--measure"]).0, 2);
}
