//! Tests of the Build pane's commands (`visual_fields.rs`): `visual.move_field`,
//! `visual.set_aggregation`, `visual.set_type`, `visual.unbind {index}`, and the snapshot fields
//! the app's Data and Build panes read (roles, visual types, hierarchy levels, display folders).

use serde_json::{Value, json};

use crate::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Runs a command that must fail and checks the report did not change.
fn fails(s: &mut Session, id: &str, params: Value) -> String {
    let before = s.report.clone();
    let err = s
        .execute(id, &params)
        .expect_err(&format!("{id} {params} should fail"));
    assert_eq!(s.report, before, "{id} changed the report although it failed");
    err
}

/// Sales(Amount, Qty, Product) with [Total], and a column chart: Category Product, Y [Total]
/// then Sum(Sales[Amount]).
fn session() -> (Session, String, String) {
    let mut s = Session::new();
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        ("table.add", json!({"name": "Sales"})),
        (
            "column.add",
            json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Qty", "data_type": "int64"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Product", "data_type": "string"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        ("report.new", json!({})),
    ] {
        run(&mut s, id, p);
    }
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    let v = run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "clusteredColumnChart", "x": 0, "y": 0, "width": 400,
               "height": 300, "fields": [{"role": "Category", "field": "Sales[Product]"},
                                         {"role": "Y", "field": "[Total]"},
                                         {"role": "Y", "field": "Sum(Sales[Amount])"}]}),
    );
    let visual = v["visual"].as_str().unwrap().to_string();
    (s, page, visual)
}

/// (role, field) of every binding of the visual, in order.
fn bindings(s: &Session, visual: &str) -> Vec<(String, String)> {
    let v = s.report.as_ref().unwrap().pages[0]
        .visuals
        .iter()
        .find(|v| v.name == visual)
        .unwrap();
    v.fields
        .iter()
        .map(|b| (b.role.clone(), crate::refs::field_text(&b.field)))
        .collect()
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter().map(|(r, f)| (r.to_string(), f.to_string())).collect()
}

#[test]
fn move_field_reorders_and_moves_between_roles_in_one_undo_step() {
    let (mut s, page, visual) = session();
    // reorder inside Y: Sum(Amount) first
    run(
        &mut s,
        "visual.move_field",
        json!({"page": page, "visual": visual, "role": "Y", "index": 1, "to_index": 0}),
    );
    assert_eq!(
        bindings(&s, &visual),
        pairs(&[
            ("Category", "Sales[Product]"),
            ("Y", "Sum(Sales[Amount])"),
            ("Y", "[Total]"),
        ])
    );
    // move a field to Tooltips (a role the visual does not use yet: Desktop's spelling)
    let v = run(
        &mut s,
        "visual.move_field",
        json!({"page": page, "visual": visual, "role": "y", "field": "[Total]", "to_role": "tooltips"}),
    );
    assert_eq!(v["role"], "Tooltips");
    assert_eq!(
        bindings(&s, &visual),
        pairs(&[
            ("Category", "Sales[Product]"),
            ("Y", "Sum(Sales[Amount])"),
            ("Tooltips", "[Total]"),
        ])
    );
    // one step each: undo puts [Total] back in Y
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(
        bindings(&s, &visual),
        pairs(&[
            ("Category", "Sales[Product]"),
            ("Y", "Sum(Sales[Amount])"),
            ("Y", "[Total]"),
        ])
    );
    // refused: a measure in a grouping role, nothing to move, past the end, both selectors
    let e = fails(
        &mut s,
        "visual.move_field",
        json!({"page": page, "visual": visual, "role": "Y", "field": "[Total]", "to_role": "Category"}),
    );
    assert!(e.contains("force"), "{e}");
    fails(
        &mut s,
        "visual.move_field",
        json!({"page": page, "visual": visual, "role": "Y", "index": 1, "to_index": 1}),
    );
    fails(
        &mut s,
        "visual.move_field",
        json!({"page": page, "visual": visual, "role": "Y", "index": 7}),
    );
    fails(
        &mut s,
        "visual.move_field",
        json!({"page": page, "visual": visual, "role": "Y", "index": 0, "to_index": 9}),
    );
    fails(
        &mut s,
        "visual.move_field",
        json!({"page": page, "visual": visual, "role": "Y", "index": 0, "field": "[Total]"}),
    );
    fails(
        &mut s,
        "visual.move_field",
        json!({"page": page, "visual": visual, "role": "Y"}),
    );
}

#[test]
fn unbind_by_position() {
    let (mut s, page, visual) = session();
    let v = run(
        &mut s,
        "visual.unbind",
        json!({"page": page, "visual": visual, "role": "Y", "index": 1}),
    );
    assert_eq!(v["removed"], json!(["Sum(Sales[Amount])"]));
    assert_eq!(
        bindings(&s, &visual),
        pairs(&[("Category", "Sales[Product]"), ("Y", "[Total]")])
    );
    fails(
        &mut s,
        "visual.unbind",
        json!({"page": page, "visual": visual, "role": "Y", "index": 1}),
    );
    fails(
        &mut s,
        "visual.unbind",
        json!({"page": page, "visual": visual, "role": "Y", "index": 0, "field": "[Total]"}),
    );
}

#[test]
fn set_aggregation_changes_how_a_column_is_summarized() {
    let (mut s, page, visual) = session();
    let v = run(
        &mut s,
        "visual.set_aggregation",
        json!({"page": page, "visual": visual, "role": "Y", "field": "Sum(Sales[Amount])", "aggregation": "Average"}),
    );
    assert_eq!(v["from"], "Sum(Sales[Amount])");
    assert_eq!(v["field"], "Avg(Sales[Amount])");
    assert_eq!(
        bindings(&s, &visual)[2],
        ("Y".into(), "Avg(Sales[Amount])".into())
    );
    // Count of a text column; then the column itself ("don't summarize") in a grouping role
    run(
        &mut s,
        "visual.set_aggregation",
        json!({"page": page, "visual": visual, "role": "Category", "index": 0, "aggregation": "Count", "force": true}),
    );
    assert_eq!(bindings(&s, &visual)[0].1, "Count(Sales[Product])");
    run(
        &mut s,
        "visual.set_aggregation",
        json!({"page": page, "visual": visual, "role": "Category", "index": 0, "aggregation": "none"}),
    );
    assert_eq!(bindings(&s, &visual)[0].1, "Sales[Product]");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(bindings(&s, &visual)[0].1, "Count(Sales[Product])");
    // a numeric aggregation of text: done, with a warning (as visual.bind)
    let v = run(
        &mut s,
        "visual.set_aggregation",
        json!({"page": page, "visual": visual, "role": "Category", "index": 0, "aggregation": "Sum"}),
    );
    assert!(!v["warnings"].as_array().unwrap().is_empty(), "{v}");
    run(&mut s, "edit.undo", json!({}));
    // refused: a measure, an unknown function, the same one
    let e = fails(
        &mut s,
        "visual.set_aggregation",
        json!({"page": page, "visual": visual, "role": "Y", "field": "[Total]", "aggregation": "Sum"}),
    );
    assert!(e.contains("measure"), "{e}");
    fails(
        &mut s,
        "visual.set_aggregation",
        json!({"page": page, "visual": visual, "role": "Y", "index": 1, "aggregation": "Nope"}),
    );
    fails(
        &mut s,
        "visual.set_aggregation",
        json!({"page": page, "visual": visual, "role": "Y", "index": 1, "aggregation": "Avg"}),
    );
    fails(
        &mut s,
        "visual.set_aggregation",
        json!({"page": page, "visual": visual, "role": "Y", "index": 1}),
    );
    fails(
        &mut s,
        "visual.set_aggregation",
        json!({"page": page, "visual": visual, "role": "Y", "index": 1, "aggregation": 3}),
    );
}

#[test]
fn set_type_moves_the_fields_to_the_new_roles() {
    let (mut s, page, visual) = session();
    // a table: every field goes to Values
    let v = run(
        &mut s,
        "visual.set_type",
        json!({"page": page, "visual": visual, "visual_type": "tableEx"}),
    );
    assert_eq!(v["from"], "clusteredColumnChart");
    assert_eq!(v["dropped"], json!([]));
    assert_eq!(
        bindings(&s, &visual),
        pairs(&[
            ("Values", "Sales[Product]"),
            ("Values", "[Total]"),
            ("Values", "Sum(Sales[Amount])"),
        ])
    );
    run(&mut s, "edit.undo", json!({}));
    // a card takes one value: the column has nowhere to go, the second value neither
    let v = run(
        &mut s,
        "visual.set_type",
        json!({"page": page, "visual": visual, "visual_type": "card"}),
    );
    assert_eq!(bindings(&s, &visual), pairs(&[("Values", "[Total]")]));
    assert_eq!(v["dropped"].as_array().unwrap().len(), 2, "{v}");
    run(&mut s, "edit.undo", json!({}));
    // a line chart has the column chart's roles: nothing moves
    let v = run(
        &mut s,
        "visual.set_type",
        json!({"page": page, "visual": visual, "visual_type": "lineChart"}),
    );
    assert_eq!(v["moved"], json!([]));
    let vis = &s.report.as_ref().unwrap().pages[0].visuals[0];
    assert_eq!(vis.visual_type.as_deref(), Some("lineChart"));
    // refused: the same type, a bad name, a group
    fails(
        &mut s,
        "visual.set_type",
        json!({"page": page, "visual": visual, "visual_type": "lineChart"}),
    );
    fails(
        &mut s,
        "visual.set_type",
        json!({"page": page, "visual": visual, "visual_type": "no such type!"}),
    );
    fails(
        &mut s,
        "visual.set_type",
        json!({"page": page, "visual": "nope", "visual_type": "card"}),
    );
    // a custom visual keeps the fields as they are, with a warning
    let v = run(
        &mut s,
        "visual.set_type",
        json!({"page": page, "visual": visual, "visual_type": "myCustomVisual1234"}),
    );
    assert!(!v["warnings"].as_array().unwrap().is_empty());
    assert_eq!(bindings(&s, &visual).len(), 3);
}

/// The Build pane shows a well per data role of each of BIcraft's own visuals (they are custom
/// visuals, absent from the built-in catalog): the snapshot carries their roles.
#[test]
fn bicraft_visuals_show_their_roles_in_the_snapshot() {
    let (mut s, page, _) = session();
    for b in crate::builtin_visuals::VISUALS {
        let v = run(
            &mut s,
            "visual.add",
            json!({"page": page, "visual_type": b.name, "x": 0, "y": 0, "width": 300, "height": 200}),
        );
        let name = v["visual"].as_str().unwrap().to_string();
        let snap = s.snapshot();
        let sv = snap["report"]["pages"][0]["visuals"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["name"] == name.as_str())
            .unwrap()
            .clone();
        let roles: Vec<&str> = sv["roles"]
            .as_array()
            .unwrap_or_else(|| panic!("{}: no roles in the snapshot", b.name))
            .iter()
            .map(|r| r["name"].as_str().unwrap())
            .collect();
        assert_eq!(roles, b.role_names(), "{}", b.name);
    }
}

#[test]
fn the_snapshot_has_what_the_panes_read() {
    let (mut s, _, visual) = session();
    run(
        &mut s,
        "hierarchy.add",
        json!({"table": "Sales", "name": "Products", "levels": ["Product", "Qty"]}),
    );
    run(
        &mut s,
        "column.set",
        json!({"table": "Sales", "name": "Qty", "display_folder": "Numbers"}),
    );
    let snap = s.snapshot();
    let t = &snap["model"]["tables"][0];
    assert_eq!(t["hierarchy_levels"][0]["name"], "Products");
    assert_eq!(
        t["hierarchy_levels"][0]["levels"],
        json!([{"name": "Product", "column": "Product"}, {"name": "Qty", "column": "Qty"}])
    );
    let qty = t["columns"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "Qty")
        .unwrap();
    assert_eq!(qty["display_folder"], "Numbers");
    let v = &snap["report"]["pages"][0]["visuals"][0];
    assert_eq!(v["name"], visual.as_str());
    let roles: Vec<&str> = v["roles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    assert!(roles.starts_with(&["Category", "Y"]), "{roles:?}");
    assert_eq!(v["roles"][0]["kind"], "grouping");
    assert_eq!(v["roles"][1]["kind"], "measure");
    let types = snap["report"]["visual_types"].as_array().unwrap();
    for t in ["card", "tableEx", "pivotTable", "clusteredColumnChart", "slicer"] {
        assert!(types.iter().any(|x| x["name"] == t), "{t} missing");
    }
    assert!(!types.iter().any(|x| x["name"] == "textbox"));
}
