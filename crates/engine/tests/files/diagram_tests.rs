//! Tests of `diagram.move` and the model diagram it edits (`diagram_layout.rs`,
//! docs/formats/diagram-layout.md): the snapshot's positions, undo, saving into a PBIP project
//! and a PBIX package, carrying it to a new project, and hostile parameters.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::Session;

fn temp(name: &str) -> TempDir {
    TempDir::new(&format!("engine-diagram-{name}"))
}

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Sales and Product, related, saved as a PBIP project in `dir`.
fn project(dir: &Path) -> (Session, PathBuf) {
    let mut s = Session::new();
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.import",
            json!({"name": "Sales", "csv": "ProductKey,Amount\n1,10\n2,20\n"}),
        ),
        (
            "table.import",
            json!({"name": "Product", "csv": "ProductKey,Name\n1,A\n2,B\n"}),
        ),
        (
            "relationship.add",
            json!({"from": "Sales[ProductKey]", "to": "Product[ProductKey]"}),
        ),
        ("report.new", json!({})),
    ] {
        run(&mut s, id, p);
    }
    let path = dir.join("shop.pbip");
    run(&mut s, "file.save_as", json!({"path": path}));
    (s, path)
}

fn layout_path(pbip: &Path) -> PathBuf {
    pbip.with_file_name("shop.SemanticModel")
        .join("diagramLayout.json")
}

fn nodes(s: &Session) -> Vec<(String, f64, f64)> {
    s.snapshot()["model"]["diagram"]["nodes"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|n| {
            (
                n["table"].as_str().unwrap().to_string(),
                n["x"].as_f64().unwrap(),
                n["y"].as_f64().unwrap(),
            )
        })
        .collect()
}

#[test]
fn a_move_is_saved_into_the_project_and_read_back() {
    let dir = temp("pbip");
    let (mut s, path) = project(dir.path());
    // a Desktop-shaped layout with a second diagram and a member BIcraft does not know
    let layout = json!({
        "version": "1.1.0",
        "diagrams": [
            {"ordinal": 0, "scrollPosition": {"x": 0, "y": 0}, "nodes": [
                {"location": {"x": 10, "y": 20}, "nodeIndex": "Product", "size": {"height": 104, "width": 234}, "zIndex": 2}
            ], "name": "All tables", "zoomValue": 100, "pinKeyFieldsToTop": false,
             "showExtraHeaderInfo": false, "hideKeyFieldsWhenCollapsed": false, "tablesLocked": false},
            {"ordinal": 1, "scrollPosition": {"x": 0, "y": 0}, "nodes": [], "name": "Sales only",
             "zoomValue": 80}
        ],
        "selectedDiagram": "All tables",
        "defaultDiagram": "All tables",
        "futureMember": [1, 2]
    });
    std::fs::write(layout_path(&path), layout.to_string()).unwrap();
    run(&mut s, "file.open", json!({"path": path}));
    assert_eq!(nodes(&s), vec![("Product".to_string(), 10.0, 20.0)]);
    assert_eq!(
        s.snapshot()["model"]["diagram"]["diagrams"],
        json!(["All tables", "Sales only"])
    );
    assert!(!s.dirty);

    // an existing node moves, a missing one is added
    let out = run(
        &mut s,
        "diagram.move",
        json!({"table": "Product", "x": 400.5, "y": 30}),
    );
    assert_eq!(out["diagram"], "All tables");
    run(&mut s, "diagram.move", json!({"table": "Sales", "x": 0, "y": 0}));
    assert!(s.dirty, "a moved table is an unsaved edit");
    assert_eq!(
        nodes(&s),
        vec![
            ("Product".to_string(), 400.5, 30.0),
            ("Sales".to_string(), 0.0, 0.0)
        ]
    );
    // a named diagram, several tables in one step
    let undo = |s: &mut Session| {
        run(s, "edit.history", json!({}))["undo"]
            .as_array()
            .unwrap()
            .len()
    };
    let before = undo(&mut s);
    let out = run(
        &mut s,
        "diagram.move",
        json!({"tables": [{"table": "Sales", "x": 5, "y": 6}, {"table": "Product", "x": 300, "y": 6}],
               "diagram": "Sales only"}),
    );
    assert_eq!(out["moved"].as_array().unwrap().len(), 2);
    assert_eq!(undo(&mut s), before + 1);

    run(&mut s, "file.save", json!({}));
    assert!(!s.dirty);
    let on_disk: Value = serde_json::from_str(&std::fs::read_to_string(layout_path(&path)).unwrap()).unwrap();
    let d0 = &on_disk["diagrams"][0];
    assert_eq!(d0["nodes"][0]["location"], json!({"x": 400.5, "y": 30}));
    assert_eq!(d0["nodes"][0]["zIndex"], 2, "the node's other members are kept");
    assert_eq!(d0["nodes"][1]["nodeIndex"], "Sales");
    assert_eq!(d0["nodes"][1]["size"]["width"], 234);
    assert_eq!(
        on_disk["diagrams"][1]["nodes"][0]["location"],
        json!({"x": 5, "y": 6})
    );
    assert_eq!(on_disk["diagrams"][1]["nodes"][1]["nodeIndex"], "Product");
    assert_eq!(on_disk["futureMember"], json!([1, 2]));
    let keys: Vec<&String> = on_disk.as_object().unwrap().keys().collect();
    assert_eq!(
        keys,
        [
            "version",
            "diagrams",
            "selectedDiagram",
            "defaultDiagram",
            "futureMember"
        ]
    );

    let mut again = Session::new();
    run(&mut again, "file.open", json!({"path": path}));
    assert_eq!(
        nodes(&again),
        vec![
            ("Product".to_string(), 400.5, 30.0),
            ("Sales".to_string(), 0.0, 0.0)
        ]
    );
}

#[test]
fn a_move_is_one_undo_step() {
    let dir = temp("undo");
    let (mut s, path) = project(dir.path());
    run(&mut s, "file.open", json!({"path": path}));
    assert!(nodes(&s).is_empty(), "a new project has an empty diagram");
    run(
        &mut s,
        "diagram.move",
        json!({"table": "Sales", "x": 100, "y": 50}),
    );
    run(
        &mut s,
        "diagram.move",
        json!({"table": "Sales", "x": 300, "y": 60}),
    );
    let undo = run(&mut s, "edit.history", json!({}))["undo"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(undo, 2);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(nodes(&s), vec![("Sales".to_string(), 100.0, 50.0)]);
    run(&mut s, "edit.undo", json!({}));
    assert!(nodes(&s).is_empty());
    assert!(!s.dirty, "back to the file as opened");
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(nodes(&s), vec![("Sales".to_string(), 100.0, 50.0)]);
    assert!(s.dirty);
    // saved, then undone: the file differs again
    run(&mut s, "file.save", json!({}));
    assert!(!s.dirty);
    run(&mut s, "edit.undo", json!({}));
    assert!(s.dirty, "the undone move is not on disk");
    // undone past the save: the diagram as opened, not the file's new one; saved again, the
    // file holds it
    assert!(nodes(&s).is_empty(), "{:?}", nodes(&s));
    run(&mut s, "file.save", json!({}));
    run(&mut s, "file.open", json!({"path": path}));
    assert!(nodes(&s).is_empty(), "{:?}", nodes(&s));
}

const SYNTHETIC_METADATA: &[u8] =
    include_bytes!("../../../metadata/tests/fixtures/synthetic_metadata.sqlitedb");

#[test]
fn a_move_is_saved_into_a_pbix_part_in_utf16() {
    let dir = temp("pbix");
    let path = dir.path().join("synthetic.pbix");
    let pkg = bicraft_pbix::synthetic::pbix(SYNTHETIC_METADATA).unwrap();
    std::fs::write(&path, pkg.to_bytes().unwrap()).unwrap();
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": path}));
    let table = s.model.as_ref().unwrap().tables[0].name.clone();
    run(&mut s, "diagram.move", json!({"table": table, "x": 120, "y": 80}));
    run(&mut s, "file.save", json!({"backup": false}));

    let pkg = bicraft_opc::Package::open(&path).unwrap();
    let part = pkg.get("DiagramLayout").unwrap();
    assert!(part.data.len() >= 2 && part.data[1] == 0, "UTF-16LE without BOM");
    let (text, _) = bicraft_opc::decode_text(&part.name, &part.data).unwrap();
    let v: Value = serde_json::from_str(&text).unwrap();
    let node = &v["diagrams"][0]["nodes"][0];
    assert_eq!(node["nodeIndex"], table.as_str());
    assert_eq!(node["location"], json!({"x": 120, "y": 80}));

    let mut again = Session::new();
    run(&mut again, "file.open", json!({"path": path}));
    assert_eq!(nodes(&again), vec![(table, 120.0, 80.0)]);
}

#[test]
fn a_moved_layout_goes_to_a_new_project() {
    let dir = temp("save-as");
    let (mut s, path) = project(dir.path());
    run(&mut s, "file.open", json!({"path": path}));
    run(
        &mut s,
        "diagram.move",
        json!({"table": "Product", "x": 7, "y": 8}),
    );
    let copy = dir.path().join("copy").join("other.pbip");
    std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
    run(&mut s, "file.save_as", json!({"path": copy}));
    let text = std::fs::read_to_string(
        copy.with_file_name("other.SemanticModel")
            .join("diagramLayout.json"),
    )
    .unwrap();
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["diagrams"][0]["nodes"][0]["nodeIndex"], "Product");
}

#[test]
fn hostile_params_are_refused_and_change_nothing() {
    let dir = temp("hostile");
    let (mut s, path) = project(dir.path());
    run(&mut s, "file.open", json!({"path": path}));
    let opened = s.diagram_layout.clone();
    for p in [
        json!({}),
        json!({"table": "Sales"}),
        json!({"table": "Sales", "x": "1", "y": 2}),
        json!({"table": "Sales", "x": 1, "y": null}),
        json!({"table": "Sales", "x": 1e300, "y": 2}),
        json!({"table": "Nope", "x": 1, "y": 2}),
        json!({"table": "Sales", "x": 1, "y": 2, "diagram": "Nope"}),
        json!({"table": "Sales", "x": 1, "y": 2, "extra": true}),
        json!({"table": 3, "x": 1, "y": 2}),
        json!({"tables": []}),
        json!({"tables": {}}),
        json!({"tables": [1]}),
        json!({"tables": [{"table": "Sales", "x": 1, "y": 2}, {"table": "Nope", "x": 1, "y": 2}]}),
        json!({"tables": [{"table": "Sales", "x": 1, "y": 2, "z": 3}]}),
        json!({"tables": [{"table": "Sales", "x": 1, "y": 2}], "table": "Sales"}),
        json!([1]),
        Value::Null,
    ] {
        assert!(s.execute("diagram.move", &p).is_err(), "{p}");
    }
    assert!(!s.dirty);
    assert_eq!(s.diagram_layout, opened);

    // a layout that is not a JSON object is left alone
    std::fs::write(layout_path(&path), "[1, 2]").unwrap();
    run(&mut s, "file.open", json!({"path": path}));
    assert!(s.snapshot()["model"].get("diagram").is_none());
    assert!(
        s.execute("diagram.move", &json!({"table": "Sales", "x": 1, "y": 2}))
            .is_err()
    );
    // nor are odd nodes and diagrams
    std::fs::write(
        layout_path(&path),
        r#"{"diagrams": [{"name": "A", "nodes": [1, {"nodeIndex": 2}, {"nodeIndex": "Sales", "location": {"x": "a"}}]}]}"#,
    )
    .unwrap();
    run(&mut s, "file.open", json!({"path": path}));
    assert!(nodes(&s).is_empty());
    run(&mut s, "diagram.move", json!({"table": "Sales", "x": 1, "y": 2}));
    assert_eq!(nodes(&s), vec![("Sales".to_string(), 1.0, 2.0)]);
    // no model
    let mut empty = Session::new();
    assert!(
        empty
            .execute("diagram.move", &json!({"table": "Sales", "x": 1, "y": 2}))
            .is_err()
    );
}

fn layouts(s: &Session) -> Vec<(String, bool, Vec<String>)> {
    s.snapshot()["model"]["diagram"]["layouts"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|d| {
            (
                d["name"].as_str().unwrap().to_string(),
                d["all"].as_bool().unwrap(),
                d["nodes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|n| n["table"].as_str().unwrap().to_string())
                    .collect(),
            )
        })
        .collect()
}

fn undo_len(s: &mut Session) -> usize {
    run(s, "edit.history", json!({}))["undo"]
        .as_array()
        .unwrap()
        .len()
}

#[test]
fn layout_tabs_are_added_renamed_and_deleted_one_undo_step_each() {
    let dir = temp("layouts");
    let (mut s, path) = project(dir.path());
    run(&mut s, "file.open", json!({"path": path}));
    // a new layout with Sales: after the layout of all tables, selected
    let out = run(
        &mut s,
        "diagram.add",
        json!({"name": "Sales view", "tables": ["Sales", "Sales"]}),
    );
    assert_eq!(out, json!({"diagram": "Sales view", "tables": 1}));
    assert_eq!(
        layouts(&s),
        vec![
            ("All tables".to_string(), true, vec![]),
            ("Sales view".to_string(), false, vec!["Sales".to_string()]),
        ]
    );
    assert_eq!(s.snapshot()["model"]["diagram"]["name"], "Sales view");
    assert!(s.dirty);
    assert_eq!(undo_len(&mut s), 1);
    // a move in the new layout adds Product to it alone
    run(
        &mut s,
        "diagram.move",
        json!({"table": "Product", "x": 400, "y": 60, "diagram": "Sales view"}),
    );
    assert_eq!(layouts(&s)[1].2, vec!["Sales", "Product"]);
    assert!(layouts(&s)[0].2.is_empty());
    // renamed: the selected name follows
    run(
        &mut s,
        "diagram.rename",
        json!({"name": "Sales view", "new_name": "Orders"}),
    );
    assert_eq!(s.snapshot()["model"]["diagram"]["name"], "Orders");
    assert_eq!(undo_len(&mut s), 3);
    // saved into diagramLayout.json, read back
    run(&mut s, "file.save", json!({}));
    let on_disk: Value = serde_json::from_str(&std::fs::read_to_string(layout_path(&path)).unwrap()).unwrap();
    assert_eq!(on_disk["diagrams"][1]["name"], "Orders");
    assert_eq!(on_disk["diagrams"][1]["ordinal"], 1);
    assert_eq!(on_disk["diagrams"][1]["nodes"][0]["size"]["width"], 234);
    assert_eq!(on_disk["selectedDiagram"], "Orders");
    assert_eq!(on_disk["defaultDiagram"], "All tables");
    let mut again = Session::new();
    run(&mut again, "file.open", json!({"path": path}));
    assert_eq!(layouts(&again), layouts(&s));
    // deleted (the name ignoring case): the selection goes back to the layout of all tables,
    // which cannot be deleted
    run(&mut s, "diagram.remove", json!({"name": "orders"}));
    assert_eq!(layouts(&s).len(), 1);
    assert_eq!(s.snapshot()["model"]["diagram"]["name"], "All tables");
    assert!(
        s.execute("diagram.remove", &json!({"name": "All tables"}))
            .is_err()
    );
    // each was one step: undo brings the layout back with its tables
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(layouts(&s)[1].0, "Orders");
    assert_eq!(layouts(&s)[1].2, vec!["Sales", "Product"]);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(layouts(&s)[1].0, "Sales view");
}

#[test]
fn layout_commands_refuse_hostile_params_and_change_nothing() {
    let dir = temp("layouts-hostile");
    let (mut s, path) = project(dir.path());
    run(&mut s, "file.open", json!({"path": path}));
    run(&mut s, "diagram.add", json!({"name": "Second"}));
    let before = s.diagram_layout.clone();
    for (id, p) in [
        ("diagram.add", json!({})),
        ("diagram.add", json!({"name": ""})),
        ("diagram.add", json!({"name": "   "})),
        ("diagram.add", json!({"name": "second"})),
        ("diagram.add", json!({"name": "all TABLES"})),
        ("diagram.add", json!({"name": "x".repeat(201)})),
        ("diagram.add", json!({"name": "X", "tables": ["Nope"]})),
        ("diagram.add", json!({"name": "X", "tables": [1]})),
        ("diagram.add", json!({"name": "X", "tables": "Sales"})),
        ("diagram.add", json!({"name": "X", "extra": 1})),
        ("diagram.add", json!({"name": 3})),
        ("diagram.rename", json!({"name": "Second"})),
        ("diagram.rename", json!({"name": "Nope", "new_name": "B"})),
        (
            "diagram.rename",
            json!({"name": "Second", "new_name": "All tables"}),
        ),
        ("diagram.rename", json!({"name": "Second", "new_name": ""})),
        ("diagram.remove", json!({"name": "Nope"})),
        ("diagram.remove", json!({"name": "All tables"})),
        ("diagram.remove", json!({})),
        ("diagram.remove", Value::Null),
    ] {
        assert!(s.execute(id, &p).is_err(), "{id} {p}");
    }
    assert_eq!(s.diagram_layout, before);
    // the same name in another case is a rename
    run(
        &mut s,
        "diagram.rename",
        json!({"name": "Second", "new_name": "SECOND"}),
    );
    // at most 100 layouts
    for k in 0..98 {
        run(&mut s, "diagram.add", json!({"name": format!("L{k}")}));
    }
    assert!(s.execute("diagram.add", &json!({"name": "One more"})).is_err());
    // no model
    let mut empty = Session::new();
    for id in ["diagram.add", "diagram.rename", "diagram.remove"] {
        assert!(empty.execute(id, &json!({"name": "A", "new_name": "B"})).is_err());
    }
}

/// Regression (G61): `table.rename` used to leave the layout's nodes naming the old table, so the
/// renamed table lost its place in every layout.
#[test]
fn a_renamed_table_keeps_its_place_in_every_layout() {
    let dir = temp("rename");
    let (mut s, path) = project(dir.path());
    run(&mut s, "file.open", json!({"path": path}));
    run(
        &mut s,
        "diagram.move",
        json!({"table": "Product", "x": 300, "y": 40}),
    );
    run(
        &mut s,
        "diagram.add",
        json!({"name": "Products", "tables": ["Product"]}),
    );
    run(
        &mut s,
        "table.rename",
        json!({"name": "Product", "new_name": "Item"}),
    );
    let l = layouts(&s);
    assert_eq!(l[0].2, vec!["Item"]);
    assert_eq!(l[1].2, vec!["Item"]);
    // the rename and its layout are one step
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(layouts(&s)[0].2, vec!["Product"]);
    run(&mut s, "edit.redo", json!({}));
    run(&mut s, "file.save", json!({}));
    let mut again = Session::new();
    run(&mut again, "file.open", json!({"path": path}));
    assert_eq!(layouts(&again)[0].2, vec!["Item"]);
    // a file with an empty layout keeps it as it is
    let dir2 = temp("rename-none");
    let (mut t, path2) = project(dir2.path());
    run(&mut t, "file.open", json!({"path": path2}));
    let opened = t.diagram_layout.clone();
    run(
        &mut t,
        "table.rename",
        json!({"name": "Sales", "new_name": "Orders"}),
    );
    assert_eq!(t.diagram_layout, opened);
}

#[test]
fn moving_a_calculation_item_is_one_undo_step_and_the_snapshot_follows_the_ordinals() {
    let dir = temp("calc-items");
    let (mut s, _) = project(dir.path());
    run(&mut s, "calculation_group.add", json!({"name": "Time"}));
    for n in ["Current", "YTD", "PY"] {
        run(
            &mut s,
            "calculation_item.add",
            json!({"group": "Time", "name": n, "expression": "SELECTEDMEASURE()"}),
        );
    }
    let items = |s: &Session| -> Vec<String> {
        s.snapshot()["model"]["tables"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == "Time")
            .unwrap()["calculation_items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(items(&s), ["Current", "YTD", "PY"]);
    let n = undo_len(&mut s);
    run(
        &mut s,
        "calculation_item.set",
        json!({"group": "Time", "name": "PY", "ordinal": 0}),
    );
    assert_eq!(items(&s), ["PY", "Current", "YTD"]);
    assert_eq!(undo_len(&mut s), n + 1);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(items(&s), ["Current", "YTD", "PY"]);
    // the Properties pane's details follow the same order, with each item's DAX
    let time = s.snapshot()["model"]["tables"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "Time")
        .cloned()
        .unwrap();
    let d = &time["calculation_item_details"];
    assert_eq!(d[2]["name"], "PY");
    assert_eq!(d[2]["expression"], "SELECTEDMEASURE()");
    assert_eq!(d[2]["format_string_expression"], "");
}

/// `diagram.remove_tables` (G61, Remove from layout): the cards leave a layout tab in one undo
/// step, the tables stay in the model; the layout of all tables and hostile params change nothing.
#[test]
fn tables_leave_a_layout_tab_in_one_undo_step() {
    let dir = temp("remove-tables");
    let (mut s, path) = project(dir.path());
    run(&mut s, "file.open", json!({"path": path}));
    run(
        &mut s,
        "diagram.add",
        json!({"name": "Both", "tables": ["Sales", "Product"]}),
    );
    let steps = undo_len(&mut s);
    let out = run(
        &mut s,
        "diagram.remove_tables",
        json!({"tables": ["Product", "Product"]}),
    );
    assert_eq!(out, json!({"removed": ["Product"], "diagram": "Both"}));
    assert_eq!(layouts(&s)[1].2, vec!["Sales"]);
    assert_eq!(undo_len(&mut s), steps + 1);
    assert_eq!(s.model.as_ref().unwrap().tables.len(), 2, "the table stays");
    let before = s.diagram_layout.clone();
    for p in [
        json!({"tables": ["Product"]}),
        json!({"tables": ["Sales"], "diagram": "All tables"}),
        json!({"tables": ["Sales"], "diagram": "Nope"}),
        json!({"tables": []}),
        json!({"tables": "Sales"}),
        json!({"tables": [1]}),
        json!({"tables": ["Sales"], "extra": 1}),
        json!({}),
        Value::Null,
    ] {
        assert!(s.execute("diagram.remove_tables", &p).is_err(), "{p}");
    }
    assert_eq!(s.diagram_layout, before);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(layouts(&s)[1].2, vec!["Sales", "Product"]);
    // saved without the card
    run(&mut s, "edit.redo", json!({}));
    run(&mut s, "file.save", json!({}));
    let text = std::fs::read_to_string(layout_path(&path)).unwrap();
    let on_disk: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(on_disk["diagrams"][1]["nodes"].as_array().map(Vec::len), Some(1));
}
