//! The DAX query view's tabs saved with the file (G81, docs/formats/dax-queries.md): a PBIP project
//! keeps them in `<Name>.SemanticModel/DAXQueries/`, read by `file.open`, written by `file.save`,
//! `file.save_as` and `project.new`, the same bytes on every save; a PBIX keeps them for the
//! session with a warning. `dax.format` and `dax.cancel` through `Session::execute`.

use serde_json::{Value, json};

use bicraft_engine::Session;

use crate::tempdir::TempDir;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn shop() -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Shop"}));
    run(
        &mut s,
        "table.import",
        json!({"name": "Sales", "m": "#table(type table [Amount = number], {{10}, {5}})"}),
    );
    s
}

fn tabs() -> Value {
    json!({
        "tabs": [
            {"name": "Totals", "text": "// sum\nEVALUATE\n    ROW(\"T\", SUM(Sales[Amount]))\n"},
            {"name": "Query 2", "text": "EVALUATE\r\n    Sales\r\n"},
        ],
        "default_tab": "Query 2",
    })
}

fn files(dir: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    for name in ["Totals.dax", "Query 2.dax", ".pbi/daxQueries.json"] {
        out.push((
            name.to_string(),
            std::fs::read(dir.join(name)).unwrap_or_default(),
        ));
    }
    out
}

#[test]
fn a_project_saves_and_reopens_its_query_tabs_byte_for_byte() {
    let tmp = TempDir::new("engine-dax-queries-pbip");
    let mut s = shop();
    let r = run(&mut s, "dax.set_queries", tabs());
    assert_eq!(r["changed"], true);
    assert_eq!(r["saved_in"], "project");
    assert_eq!(s.snapshot()["dirty"]["queries"], true);
    let path = tmp.join("Shop.pbip");
    run(
        &mut s,
        "file.save_as",
        json!({"path": path, "format": "pbip-tmdl"}),
    );
    assert_eq!(s.snapshot()["dirty"]["queries"], false);
    let dir = tmp.join("Shop.SemanticModel").join("DAXQueries");
    assert_eq!(
        std::fs::read(dir.join("Query 2.dax")).unwrap(),
        b"EVALUATE\r\n    Sales\r\n"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join(".pbi/daxQueries.json")).unwrap(),
        "{\n  \"version\": \"1.0.0\",\n  \"tabOrder\": [\n    \"Totals\",\n    \"Query 2\"\n  ],\n  \"defaultTab\": \"Query 2\"\n}"
    );
    let first = files(&dir);

    // reopened: the same tabs; saved again with an edit elsewhere: the same bytes
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": path}));
    let q = run(&mut s, "dax.queries", json!({}));
    assert_eq!(q["tabs"], tabs()["tabs"]);
    assert_eq!(q["default_tab"], "Query 2");
    assert_eq!(q["dirty"], false);
    let snap = s.snapshot();
    assert_eq!(snap["dax_queries"]["tabs"], tabs()["tabs"]);
    let generation = snap["dax_queries"]["generation"].clone();
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
    );
    run(&mut s, "file.save", json!({}));
    assert_eq!(files(&dir), first);
    // the same tabs given again change nothing
    assert_eq!(run(&mut s, "dax.set_queries", tabs())["changed"], false);

    // a tab closed and one renamed: their files go
    run(
        &mut s,
        "dax.set_queries",
        json!({"tabs": [{"name": "Renamed", "text": "EVALUATE {1}"}]}),
    );
    run(&mut s, "file.save", json!({}));
    assert!(!dir.join("Totals.dax").exists());
    assert!(!dir.join("Query 2.dax").exists());
    assert_eq!(std::fs::read(dir.join("Renamed.dax")).unwrap(), b"EVALUATE {1}");
    let mut again = Session::new();
    run(&mut again, "file.open", json!({"path": path}));
    let snap = again.snapshot();
    assert_eq!(snap["dax_queries"]["tabs"][0]["name"], "Renamed");
    assert_eq!(snap["dax_queries"]["default_tab"], "Renamed");
    assert_ne!(snap["dax_queries"]["generation"], generation);

    // project.new from the open project carries them
    run(
        &mut again,
        "project.new",
        json!({"dir": tmp.join("copy"), "name": "Copy"}),
    );
    assert_eq!(
        std::fs::read(tmp.join("copy/Copy.SemanticModel/DAXQueries/Renamed.dax")).unwrap(),
        b"EVALUATE {1}"
    );
}

#[test]
fn a_package_keeps_its_query_tabs_for_the_session() {
    let tmp = TempDir::new("engine-dax-queries-pbix");
    let mut s = shop();
    let path = tmp.join("Shop.pbix");
    run(&mut s, "file.save_as", json!({"path": path, "format": "pbix"}));
    run(&mut s, "dax.set_queries", tabs());
    assert_eq!(s.snapshot()["dirty"]["queries"], false);
    let r = run(&mut s, "file.save", json!({"backup": false}));
    let warnings = r["warnings"].to_string();
    assert!(warnings.contains("kept for this session only"), "{r}");
    assert_eq!(run(&mut s, "dax.queries", json!({}))["saved_in"], "session");
    // saved as a project, they are written
    let pbip = tmp.join("Out.pbip");
    run(
        &mut s,
        "file.save_as",
        json!({"path": pbip, "format": "pbip-tmdl"}),
    );
    assert!(tmp.join("Out.SemanticModel/DAXQueries/Totals.dax").is_file());
}

#[test]
fn format_and_cancel_run_as_commands() {
    let mut s = shop();
    let r = run(
        &mut s,
        "dax.format",
        json!({"query": "evaluate filter(Sales,Sales[Amount]>5) order by Sales[Amount]"}),
    );
    assert_eq!(
        r["query"],
        "evaluate\n    filter(Sales, Sales[Amount] > 5)\norder by\n    Sales[Amount]"
    );
    let again = run(&mut s, "dax.format", json!({"query": r["query"]}));
    assert_eq!(again["changed"], false);
    assert!(s.execute("dax.format", &json!({"query": "EVALUATE ("})).is_err());
    assert_eq!(run(&mut s, "dax.cancel", json!({}))["running"], 0);
    // a cancel before a query does not stop it
    let rows = run(&mut s, "dax.evaluate", json!({"query": "EVALUATE Sales"}));
    assert_eq!(rows["results"][0]["row_count"], 2);
    assert!(s.runs_shared("dax.cancel", &json!({})));
    assert!(s.runs_shared("dax.format", &json!({"query": "1"})));
    assert!(!s.runs_shared("dax.set_queries", &json!({})));
}
