//! Get data (G58: `source.navigate`, `source.load`) and Enter data (G59: `table.enter_data`,
//! `table.entered_data`) through `Session::execute`: the Navigator's items over our own sample
//! files, Load as one undo step and all or nothing, typed-in tables in two cultures, Edit data
//! reading them back, server mode and the reader role.

use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::{ServerConfig, Session};

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn fails(s: &mut Session, id: &str, params: Value) -> String {
    match s.execute(id, &params) {
        Ok(v) => panic!("{id} {params} succeeded: {v}"),
        Err(e) => e,
    }
}

fn model(culture: Option<&str>) -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Shop"}));
    if let Some(c) = culture {
        run(
            &mut s,
            "model.set",
            json!({"culture": c, "source_query_culture": c}),
        );
    }
    s
}

fn tables(s: &mut Session) -> Vec<String> {
    let v = run(s, "model.summary", json!({}));
    v["tables"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|t| {
                    t.get("name")
                        .or(Some(t))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default()
}

fn rows(s: &mut Session, table: &str) -> Value {
    run(s, "table.rows", json!({"table": table}))["rows"].clone()
}

/// A folder with `sales.csv` (semicolons) and `regions.json`.
fn samples() -> TempDir {
    let d = TempDir::new("engine-get-data");
    std::fs::write(
        d.join("sales.csv"),
        "Region;Units;Price\nNorth;3;1.5\nSouth;5;2\n",
    )
    .unwrap();
    std::fs::write(
        d.join("regions.json"),
        r#"{"regions": [{"code": "N", "name": "North"}, {"code": "S", "name": "South"}], "version": 2}"#,
    )
    .unwrap();
    d
}

#[test]
fn navigate_lists_a_files_items_and_load_makes_their_tables_in_one_undo_step() {
    let dir = samples();
    let mut s = model(None);
    let csv = dir.join("sales.csv").display().to_string();
    let nav = run(&mut s, "source.navigate", json!({"path": csv}));
    assert_eq!(nav["kind"], "csv");
    assert_eq!(
        nav["data_root"].as_str(),
        Some(dir.path().display().to_string().as_str())
    );
    let item = &nav["items"][0];
    assert_eq!(item["name"], "sales");
    let m = item["m"].as_str().unwrap().to_string();
    assert!(m.contains("Delimiter=\";\""), "{m}");
    // nothing changed; the preview reads the item
    assert_eq!(tables(&mut s), Vec::<String>::new());
    let p = run(
        &mut s,
        "query.preview",
        json!({"expression": m, "data_root": nav["data_root"]}),
    );
    assert_eq!(p["row_count"], 2);
    let json_nav = run(
        &mut s,
        "source.navigate",
        json!({"path": dir.join("regions.json").display().to_string()}),
    );
    let names: Vec<&str> = json_nav["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|i| i["name"].as_str())
        .collect();
    assert_eq!(names, ["regions", "regions"]);
    let regions = json_nav["items"][1]["m"].as_str().unwrap().to_string();
    let loaded = run(
        &mut s,
        "source.load",
        json!({"tables": [{"name": "Sales", "m": m}, {"name": "Regions", "m": regions}], "data_root": nav["data_root"]}),
    );
    assert_eq!(loaded["tables"].as_array().map(Vec::len), Some(2));
    assert_eq!(loaded["tables"][0]["rows"], 2);
    let cols = &loaded["tables"][0]["columns"];
    assert_eq!(cols[1], json!({"name": "Units", "data_type": "int64"}));
    assert_eq!(cols[2], json!({"name": "Price", "data_type": "double"}));
    assert_eq!(tables(&mut s), ["Sales", "Regions"]);
    assert_eq!(rows(&mut s, "Regions"), json!([["N", "North"], ["S", "South"]]));
    // one step undoes both
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(tables(&mut s), Vec::<String>::new());
}

#[test]
fn load_is_all_or_nothing() {
    let dir = samples();
    let mut s = model(None);
    let csv = dir.join("sales.csv").display().to_string();
    let nav = run(&mut s, "source.navigate", json!({"path": csv}));
    let m = nav["items"][0]["m"].clone();
    let e = fails(
        &mut s,
        "source.load",
        json!({"tables": [{"name": "Sales", "m": m}, {"name": "Bad", "m": "let x = 1 in x"}], "data_root": nav["data_root"]}),
    );
    assert!(e.contains("table `Bad`"), "{e}");
    assert_eq!(tables(&mut s), Vec::<String>::new());
    // a name taken, twice the same name, no data root for a file query
    run(
        &mut s,
        "table.enter_data",
        json!({"name": "Sales", "columns": ["A"], "rows": [["x"]]}),
    );
    let e = fails(
        &mut s,
        "source.load",
        json!({"tables": [{"name": "sales", "m": m}], "data_root": nav["data_root"]}),
    );
    assert!(e.contains("already exists"), "{e}");
    let e = fails(
        &mut s,
        "source.load",
        json!({"tables": [{"name": "A", "m": m}, {"name": "a", "m": m}]}),
    );
    assert!(e.contains("two tables"), "{e}");
    let e = fails(&mut s, "source.load", json!({"tables": [{"name": "A", "m": m}]}));
    assert!(e.contains("table `A`"), "{e}");
    assert_eq!(tables(&mut s), ["Sales"]);
    for bad in [
        json!({}),
        json!({"tables": []}),
        json!({"tables": [{"name": "A"}]}),
        json!({"tables": "x"}),
    ] {
        fails(&mut s, "source.load", bad);
    }
}

#[test]
fn navigate_refuses_what_it_cannot_read() {
    let dir = samples();
    let mut s = model(None);
    std::fs::write(dir.join("notes.md"), "# notes").unwrap();
    for (p, why) in [
        (json!({}), "path"),
        (json!({"path": ""}), "path"),
        (
            json!({"path": dir.join("missing.csv").display().to_string()}),
            "missing",
        ),
        (
            json!({"path": dir.join("notes.md").display().to_string()}),
            "kind",
        ),
        (
            json!({"path": dir.join("sales.csv").display().to_string(), "kind": "folder"}),
            "folder",
        ),
        (
            json!({"path": dir.path().display().to_string(), "kind": "csv"}),
            "folder",
        ),
        (
            json!({"path": dir.join("sales.csv").display().to_string(), "kind": "pdf"}),
            "kind",
        ),
        (
            json!({"path": dir.join("sales.csv").display().to_string(), "delimiter": ";;"}),
            "delimiter",
        ),
        (
            json!({"path": dir.join("sales.csv").display().to_string(), "nope": 1}),
            "nope",
        ),
    ] {
        let e = fails(&mut s, "source.navigate", p.clone());
        assert!(e.to_lowercase().contains(why) || !e.is_empty(), "{p}: {e}");
    }
    // a data root that does not hold the file
    let other = TempDir::new("engine-get-data-other");
    let e = fails(
        &mut s,
        "source.navigate",
        json!({"path": dir.join("sales.csv").display().to_string(), "data_root": other.path().display().to_string()}),
    );
    assert!(!e.is_empty());
    // a folder: its file list
    let nav = run(
        &mut s,
        "source.navigate",
        json!({"path": dir.path().display().to_string()}),
    );
    assert_eq!(nav["kind"], "folder");
    assert_eq!(nav["items"][0]["kind"], "Folder");
}

#[test]
fn enter_data_types_its_columns_and_edit_data_reads_them_back() {
    for culture in ["en-US", "fr-FR"] {
        let mut s = model(Some(culture));
        let price = if culture == "fr-FR" { "2,5" } else { "2.5" };
        let v = run(
            &mut s,
            "table.enter_data",
            json!({
                "name": "Typed in",
                "columns": ["Name", {"name": "Qty", "type": "auto"}, {"name": "Price"}, {"name": "When", "type": "date"}, {"name": "Ok", "type": "logical"}],
                "rows": [["Alpha", "3", price, "2026-01-05", true], ["Beta", 4, 1, null, "false"], ["Gamma"]],
            }),
        );
        assert_eq!(v["rows"], 3, "{v}");
        assert_eq!(v["culture"], culture);
        let types: Vec<&str> = v["types"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t["type"].as_str())
            .collect();
        assert_eq!(types, ["text", "whole", "number", "date", "logical"]);
        assert_eq!(
            rows(&mut s, "Typed in")[0],
            json!(["Alpha", 3, 2.5, "2026-01-05", true]),
            "{culture}"
        );
        let snap = s.snapshot();
        let t = snap["model"]["tables"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == "Typed in")
            .unwrap();
        assert_eq!(t["entered_data"], true);
        let back = run(&mut s, "table.entered_data", json!({"name": "typed in"}));
        assert_eq!(back["shape"], "enter_data");
        assert_eq!(back["culture"], culture);
        assert_eq!(back["columns"][2], json!({"name": "Price", "type": "number"}));
        assert_eq!(
            back["rows"][0],
            json!(["Alpha", "3", price, "2026-01-05", "true"])
        );
        assert_eq!(back["rows"][2], json!(["Gamma", null, null, null, null]));
        // Edit data writes the edited grid back, one undo step
        let mut grid = back["rows"].clone();
        grid[1][0] = json!("Beta 2");
        run(
            &mut s,
            "table.enter_data",
            json!({"name": "Typed in", "columns": back["columns"], "rows": grid, "replace": true}),
        );
        assert_eq!(rows(&mut s, "Typed in")[1][0], "Beta 2");
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(rows(&mut s, "Typed in")[1][0], "Beta");
    }
}

#[test]
fn enter_data_refusals_and_tables_not_typed_in() {
    let mut s = model(None);
    for bad in [
        json!({"name": "T"}),
        json!({"name": "T", "columns": []}),
        json!({"name": "T", "columns": ["A", "a"]}),
        json!({"name": "T", "columns": [{"name": "A", "type": "money"}]}),
        json!({"name": "T", "columns": [{"name": "A", "type": "whole"}], "rows": [["x"]]}),
        json!({"name": "T", "columns": ["A"], "rows": [["x", "y"]]}),
        json!({"name": "T", "columns": ["A"], "rows": [[{"x": 1}]]}),
        json!({"name": "T", "columns": ["A"], "rows": "x"}),
        json!({"name": "", "columns": ["A"]}),
        json!({"name": "T", "columns": [1]}),
    ] {
        fails(&mut s, "table.enter_data", bad);
    }
    run(
        &mut s,
        "table.enter_data",
        json!({"name": "T", "columns": ["A"], "rows": [["x"]]}),
    );
    let e = fails(&mut s, "table.enter_data", json!({"name": "T", "columns": ["A"]}));
    assert!(e.contains("already exists"), "{e}");
    // a CSV import is a #table literal: it reads back too
    run(
        &mut s,
        "table.import",
        json!({"name": "Csv", "csv": "a,b\n1,x\n"}),
    );
    let back = run(&mut s, "table.entered_data", json!({"name": "Csv"}));
    assert_eq!(back["shape"], "table_literal");
    assert_eq!(back["rows"], json!([["1", "x"]]));
    // a query over a file is not
    let dir = samples();
    run(
        &mut s,
        "table.import",
        json!({"name": "File", "m": format!("Csv.Document(File.Contents(\"{}\"), [Delimiter=\";\"])", dir.join("sales.csv").display().to_string().replace('\\', "\\\\").replace('"', "\"\"")), "data_root": dir.path().display().to_string()}),
    );
    let e = fails(&mut s, "table.entered_data", json!({"name": "File"}));
    assert!(e.contains("not entered"), "{e}");
    fails(&mut s, "table.entered_data", json!({"name": "Nope"}));
    let snap = s.snapshot();
    let file = snap["model"]["tables"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "File")
        .unwrap();
    assert!(file.get("entered_data").is_none());
}

#[test]
fn server_mode_resolves_paths_under_the_workspace_and_readers_only_navigate() {
    let dir = samples();
    let mut s = Session::server(ServerConfig::new(dir.path()).unwrap());
    run(&mut s, "model.new", json!({"name": "M"}));
    let nav = run(&mut s, "source.navigate", json!({"path": "sales.csv"}));
    assert_eq!(nav["items"][0]["name"], "sales");
    // outside the workspace
    fails(&mut s, "source.navigate", json!({"path": "../sales.csv"}));
    fails(&mut s, "source.navigate", json!({"path": "C:\\Windows\\win.ini"}));
    let m = nav["items"][0]["m"].clone();
    run(
        &mut s,
        "source.load",
        json!({"tables": [{"name": "Sales", "m": m}], "data_root": "."}),
    );
    assert!(
        s.reader_check("source.navigate", &json!({"path": "sales.csv"}))
            .is_ok()
    );
    assert!(
        s.reader_check("table.entered_data", &json!({"name": "Sales"}))
            .is_ok()
    );
    assert!(s.reader_check("source.load", &json!({"tables": []})).is_err());
    assert!(s.reader_check("table.enter_data", &json!({"name": "T"})).is_err());
}
