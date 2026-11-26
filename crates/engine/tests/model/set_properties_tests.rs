//! `model.set_properties`: the same properties on several tables, columns and measures as one
//! undo step (the Properties pane's multi-selection); a refusal on one object changes nothing;
//! synonyms in the model culture's Q&A linguistic schema, shown by the snapshot; storage mode on
//! several tables; TMDL, TMSL and PBIX keep what it wrote.

use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Sales(Amount, Cost, Label, Code) with [Total] and [Count]; Product(Name).
fn shop() -> Session {
    let mut s = Session::new();
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.import",
            json!({"name": "Sales", "csv": "Amount,Cost,Label,Code\n1.5,1,a,1\n2,1,b,2\n"}),
        ),
        ("table.import", json!({"name": "Product", "csv": "Name\nX\n"})),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Count", "expression": "COUNTROWS(Sales)"}),
        ),
    ] {
        run(&mut s, id, p);
    }
    s
}

fn table(s: &Session, name: &str) -> Value {
    s.snapshot()["model"]["tables"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == name)
        .unwrap()
        .clone()
}

fn object(s: &Session, t: &str, kind: &str, name: &str) -> Value {
    table(s, t)[kind]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["name"] == name)
        .unwrap()
        .clone()
}

#[test]
fn several_columns_and_measures_in_one_undo_step() {
    let mut s = shop();
    let undo_before = run(&mut s, "edit.history", json!({}))["undo"]
        .as_array()
        .unwrap()
        .len();
    let v = run(
        &mut s,
        "model.set_properties",
        json!({
            "objects": [
                {"table": "sales", "column": "amount"},
                {"table": "Sales", "column": "Cost"},
                {"table": "Sales", "measure": "Total"},
                {"table": "Sales", "measure": "Count"}
            ],
            "props": {"description": "Money", "hidden": true, "display_folder": "Figures", "format_string": "0.0"}
        }),
    );
    assert_eq!(v["objects"][0], json!({"table": "Sales", "column": "Amount"}));
    for (kind, name) in [
        ("columns", "Amount"),
        ("columns", "Cost"),
        ("measures", "Total"),
        ("measures", "Count"),
    ] {
        let o = object(&s, "Sales", kind, name);
        assert_eq!(o["description"], "Money", "{name}");
        assert_eq!(o["hidden"], true, "{name}");
        assert_eq!(o["display_folder"], "Figures", "{name}");
        assert_eq!(o["format_string"], "0.0", "{name}");
    }
    // one undo step for the four objects
    let h = run(&mut s, "edit.history", json!({}));
    assert_eq!(h["undo"].as_array().unwrap().len(), undo_before + 1);
    run(&mut s, "edit.undo", json!({}));
    for (kind, name) in [("columns", "Amount"), ("measures", "Count")] {
        let o = object(&s, "Sales", kind, name);
        assert_eq!(o["hidden"], false, "{name}");
        assert_ne!(o["description"], "Money", "{name}");
    }
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(object(&s, "Sales", "measures", "Count")["hidden"], true);

    // columns only: summarize by, data category, sort by column
    run(
        &mut s,
        "model.set_properties",
        json!({
            "objects": [{"table": "Sales", "column": "Label"}, {"table": "Sales", "column": "Amount"}],
            "props": {"summarize_by": "none", "data_category": "City", "sort_by_column": "Code"}
        }),
    );
    for name in ["Label", "Amount"] {
        let o = object(&s, "Sales", "columns", name);
        assert_eq!(o["summarize_by"], "none", "{name}");
        assert_eq!(o["data_category"], "City", "{name}");
        assert_eq!(o["sort_by_column"], "Code", "{name}");
    }
    // tables: description and hidden
    run(
        &mut s,
        "model.set_properties",
        json!({"objects": [{"table": "Sales"}, {"table": "Product"}], "props": {"hidden": true, "description": null}}),
    );
    assert_eq!(table(&s, "Sales")["hidden"], true);
    assert_eq!(table(&s, "Product")["hidden"], true);
}

#[test]
fn a_refusal_changes_nothing_and_bad_params_are_errors() {
    let mut s = shop();
    let before = s.snapshot()["model"].clone();
    let undo = |s: &mut Session| {
        run(s, "edit.history", json!({}))["undo"]
            .as_array()
            .unwrap()
            .len()
    };
    let steps = undo(&mut s);
    // Cost then Code: Code cannot sort by itself, so Cost is not changed either
    let e = s
        .execute(
            "model.set_properties",
            &json!({
                "objects": [{"table": "Sales", "column": "Cost"}, {"table": "Sales", "column": "Code"}],
                "props": {"hidden": true, "sort_by_column": "Code"}
            }),
        )
        .unwrap_err();
    assert!(e.contains("Sales[Code]"), "{e}");
    assert_eq!(s.snapshot()["model"], before);
    assert_eq!(undo(&mut s), steps);
    for bad in [
        json!({}),
        json!({"objects": [], "props": {"hidden": true}}),
        json!({"objects": [{"table": "Sales"}], "props": {}}),
        json!({"objects": [{"table": "Nope"}], "props": {"hidden": true}}),
        json!({"objects": [{"table": "Sales", "column": "Nope"}], "props": {"hidden": true}}),
        json!({"objects": [{"table": "Sales", "measure": "Nope"}], "props": {"hidden": true}}),
        json!({"objects": [{"table": "Sales", "column": "Cost", "measure": "Total"}], "props": {"hidden": true}}),
        json!({"objects": [{"table": "Sales"}, {"table": "sales"}], "props": {"hidden": true}}),
        json!({"objects": [{"table": "Sales"}], "props": {"display_folder": "X"}}),
        json!({"objects": [{"table": "Sales", "measure": "Total"}], "props": {"summarize_by": "sum"}}),
        json!({"objects": [{"table": "Sales", "column": "Cost"}], "props": {"storage_mode": "dual"}}),
        json!({"objects": [{"table": "Sales"}], "props": {"colour": 1}}),
        json!({"objects": [{"table": "Sales"}], "props": {"hidden": "yes"}}),
        json!({"objects": [{"table": "Sales"}], "props": {"synonyms": "deals"}}),
        json!({"objects": [{"table": "Sales"}], "props": {"synonyms": [1]}}),
        json!({"objects": "Sales", "props": {"hidden": true}}),
        json!({"objects": [{"table": "Sales"}], "props": {"hidden": true}, "extra": 1}),
        json!({"objects": [7], "props": {"hidden": true}}),
    ] {
        assert!(s.execute("model.set_properties", &bad).is_err(), "{bad}");
    }
    assert_eq!(s.snapshot()["model"], before);
}

#[test]
fn synonyms_are_written_in_the_model_cultures_schema() {
    let mut s = shop();
    run(
        &mut s,
        "model.set_properties",
        json!({"objects": [{"table": "Sales"}], "props": {"synonyms": ["sales", "deals", " Deals "]}}),
    );
    run(
        &mut s,
        "model.set_properties",
        json!({"objects": [{"table": "Sales", "column": "Amount"}, {"table": "Sales", "measure": "Total"}],
               "props": {"synonyms": ["value"]}}),
    );
    assert_eq!(table(&s, "Sales")["synonyms"], json!(["sales", "deals"]));
    assert_eq!(
        object(&s, "Sales", "columns", "Amount")["synonyms"],
        json!(["value"])
    );
    assert_eq!(
        object(&s, "Sales", "measures", "Total")["synonyms"],
        json!(["value"])
    );
    assert!(object(&s, "Sales", "columns", "Cost").get("synonyms").is_none());
    let m = s.model.as_ref().unwrap();
    let c = &m.extra["cultures"][0];
    assert_eq!(c["name"], "en-US");
    assert_eq!(c["linguisticMetadata"]["contentType"], "json");
    let entities = &c["linguisticMetadata"]["content"]["Entities"];
    assert_eq!(
        entities["sales"],
        json!({"Definition": {"Binding": {"ConceptualEntity": "Sales"}},
               "Terms": [{"sales": {}}, {"deals": {}}]})
    );
    assert_eq!(
        entities["sales.amount"]["Definition"]["Binding"],
        json!({"ConceptualEntity": "Sales", "ConceptualProperty": "Amount"})
    );
    // a rename follows (rename_refs), and the synonyms stay on the object
    run(
        &mut s,
        "column.rename",
        json!({"table": "Sales", "name": "Amount", "new_name": "Net"}),
    );
    assert_eq!(
        object(&s, "Sales", "columns", "Net")["synonyms"],
        json!(["value"])
    );
    // an empty list clears them
    run(
        &mut s,
        "model.set_properties",
        json!({"objects": [{"table": "Sales"}], "props": {"synonyms": []}}),
    );
    assert_eq!(table(&s, "Sales")["synonyms"], json!([]));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(table(&s, "Sales")["synonyms"], json!(["sales", "deals"]));
}

#[test]
fn a_generated_term_left_out_is_marked_deleted() {
    let mut s = shop();
    run(&mut s, "culture.add", json!({"name": "en-US"}));
    // a schema as a file brings it: JSON text, with a generated and a suggested term
    let content = json!({"Version": "1.0.0", "Language": "en-US", "Entities": {"product": {
        "Definition": {"Binding": {"ConceptualEntity": "Product"}}, "State": "Generated",
        "Terms": [{"product": {"State": "Generated"}}, {"item": {"State": "Suggested", "Weight": 0.8}}]}}});
    {
        let m = s.model.as_mut().unwrap();
        let cultures = m.extra.get_mut("cultures").unwrap().as_array_mut().unwrap();
        let c = cultures.iter_mut().find(|c| c["name"] == "en-US").unwrap();
        c["linguisticMetadata"] = json!({"content": content.to_string(), "contentType": "json"});
    }
    assert_eq!(table(&s, "Product")["synonyms"], json!(["product"]));
    run(
        &mut s,
        "model.set_properties",
        json!({"objects": [{"table": "Product"}], "props": {"synonyms": ["article"]}}),
    );
    assert_eq!(table(&s, "Product")["synonyms"], json!(["article"]));
    let m = s.model.as_ref().unwrap();
    let c = m.extra["cultures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "en-US")
        .unwrap();
    // still JSON text, as it was read
    let text = c["linguisticMetadata"]["content"].as_str().unwrap();
    let v: Value = serde_json::from_str(text).unwrap();
    assert_eq!(
        v["Entities"]["product"]["Terms"],
        json!([{"article": {}}, {"product": {"State": "Deleted"}},
               {"item": {"State": "Suggested", "Weight": 0.8}}])
    );
}

const SQL: &str =
    "let Source = Sql.Database(\"srv.example\", \"db\"), T = Source{[Schema=\"dbo\",Item=\"T\"]}[Data] in T";

#[test]
fn storage_mode_of_several_tables() {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Shop"}));
    for table in ["A", "B"] {
        run(
            &mut s,
            "table.add",
            json!({"name": table, "partition": {"source_type": "m", "expression": SQL}}),
        );
        run(
            &mut s,
            "column.add",
            json!({"table": table, "name": "K", "data_type": "int64"}),
        );
    }
    run(
        &mut s,
        "model.set_properties",
        json!({"objects": [{"table": "A"}, {"table": "B"}], "props": {"storage_mode": "directQuery"}}),
    );
    assert_eq!(table(&s, "A")["storage_mode"], "directQuery");
    assert_eq!(table(&s, "B")["storage_mode"], "directQuery");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(table(&s, "A")["storage_mode"], "import");
    assert_eq!(table(&s, "B")["storage_mode"], "import");
}

#[test]
fn what_it_writes_is_saved_to_tmdl_tmsl_and_pbix() {
    let tmp = TempDir::new("engine-set-properties-save");
    for (format, file) in [
        ("pbip-tmdl", "Tmdl.pbip"),
        ("pbip-tmsl", "Tmsl.pbip"),
        ("pbix", "shop.pbix"),
    ] {
        let mut s = shop();
        run(
            &mut s,
            "model.set_properties",
            json!({
                "objects": [{"table": "Sales", "column": "Amount"}, {"table": "Sales", "measure": "Total"}],
                "props": {"description": "Money", "hidden": true, "display_folder": "Figures",
                          "format_string": "0.0", "synonyms": ["value", "worth"]}
            }),
        );
        run(
            &mut s,
            "model.set_properties",
            json!({"objects": [{"table": "Sales"}], "props": {"synonyms": ["deals"]}}),
        );
        let path = tmp.path().join(file);
        run(&mut s, "file.save_as", json!({"path": path, "format": format}));
        let mut back = Session::new();
        run(&mut back, "file.open", json!({"path": path}));
        assert_eq!(back.model_error, None, "{format}");
        for (kind, name) in [("columns", "Amount"), ("measures", "Total")] {
            let o = object(&back, "Sales", kind, name);
            assert_eq!(o["description"], "Money", "{format} {name}");
            assert_eq!(o["hidden"], true, "{format} {name}");
            assert_eq!(o["display_folder"], "Figures", "{format} {name}");
            assert_eq!(o["format_string"], "0.0", "{format} {name}");
            assert_eq!(o["synonyms"], json!(["value", "worth"]), "{format} {name}");
        }
        assert_eq!(table(&back, "Sales")["synonyms"], json!(["deals"]), "{format}");
        // and it can be edited after the round trip
        run(
            &mut back,
            "model.set_properties",
            json!({"objects": [{"table": "Sales"}], "props": {"synonyms": ["orders"]}}),
        );
        assert_eq!(table(&back, "Sales")["synonyms"], json!(["orders"]), "{format}");
    }
}
