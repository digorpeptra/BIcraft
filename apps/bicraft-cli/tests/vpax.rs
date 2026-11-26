//! `bicraft vpax` and the M9.3 read commands through `bicraft run`.

use std::process::Command;

use serde_json::Value;

#[path = "../../../testsupport/tempdir.rs"]
mod tempdir;
use tempdir::TempDir;

fn bicraft(args: &[&str]) -> (i32, Value) {
    let out = Command::new(env!("CARGO_BIN_EXE_bicraft"))
        .args(args)
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    (
        out.status.code().unwrap(),
        serde_json::from_str(&stdout).unwrap_or(Value::Null),
    )
}

#[test]
fn vpax_of_a_built_pbix() {
    let dir = TempDir::new("cli-vpax");
    let pbix = dir.join("v.pbix");
    let pbix = pbix.to_str().unwrap();
    let (code, out) = bicraft(&[
        "run",
        "--cmd",
        "model.new={\"name\": \"V\"}",
        "--cmd",
        "table.import={\"name\": \"T\", \"m\": \"let S = #table(type table [A = Int64.Type, B = text], {{1, \\\"x\\\"}, {2, \\\"y\\\"}, {3, \\\"x\\\"}}) in S\"}",
        "--cmd",
        &format!("file.save_as={{\"path\": {}}}", Value::from(pbix)),
        "--cmd",
        "model.storage_stats",
        "--cmd",
        "column.get={\"table\": \"T\", \"name\": \"B\"}",
    ]);
    assert_eq!(code, 0, "{out}");
    let stats = &out["results"][3];
    assert_eq!(stats["tables"][0]["rows"], 3);
    assert_eq!(out["results"][4]["data_type"], "string");

    let vpax = dir.join("v.vpax");
    let vpax = vpax.to_str().unwrap();
    let (code, out) = bicraft(&["vpax", pbix, vpax]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(out["model_bim"], true);
    // An existing file is kept unless --overwrite.
    assert_eq!(bicraft(&["vpax", pbix, vpax]).0, 1);
    let (code, out) = bicraft(&["vpax", pbix, vpax, "--overwrite", "--no-model-bim"]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(out["parts"], serde_json::json!(["DaxModel.json"]));
    assert_eq!(bicraft(&["vpax", pbix]).0, 2);
    assert_eq!(bicraft(&["vpax", pbix, vpax, "--bogus"]).0, 2);
}
