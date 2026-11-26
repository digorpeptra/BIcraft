//! The Power Query editor's commands (`query_steps.rs`): the queries listed, steps read, added,
//! edited and removed on drafts without touching the model, the preview at a step, and Close &
//! Apply writing the queries back and reloading their tables as one undo step.

use serde_json::{Value, json};

use super::*;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// The Sales table's query, with a comment and spacing of its own.
const SALES: &str = "let\n    // typed in\n    Source = #table(type table [Product = text, Qty = Int64.Type, City = text], {{\"Bike\", 2, \"Paris\"}, {\"Car\", 1, \"Lyon\"}, {\"Pen\", 9, \"Paris\"}}),\n    Kept   = Table.SelectRows(Source, each [Qty] > 0)\nin\n    Kept";

/// A model with table Sales (its partition renamed) and shared query Cities.
fn session() -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Shop"}));
    run(&mut s, "table.import", json!({"name": "Sales", "m": SALES}));
    run(
        &mut s,
        "expression.add",
        json!({"name": "Cities", "expression": "#table(type table [City = text, Country = text], {{\"Paris\", \"FR\"}, {\"Lyon\", \"FR\"}})"}),
    );
    if let Some(t) = s
        .model
        .as_mut()
        .and_then(|m| m.tables.iter_mut().find(|t| t.name == "Sales"))
        && let Some(p) = t.partitions.first_mut()
    {
        p.name = "Sales-main".into();
    }
    s
}

fn partition_text(s: &Session, table: &str) -> String {
    s.model
        .as_ref()
        .and_then(|m| m.tables.iter().find(|t| t.name == table))
        .and_then(|t| t.partitions.first())
        .and_then(|p| p.expression.clone())
        .unwrap_or_default()
}

fn columns(s: &Session, table: &str) -> Vec<String> {
    s.model
        .as_ref()
        .and_then(|m| m.tables.iter().find(|t| t.name == table))
        .map(|t| {
            t.columns
                .iter()
                .filter(|c| !c.is_row_number())
                .map(|c| c.name.clone())
                .collect()
        })
        .unwrap_or_default()
}

fn step_names(v: &Value) -> Vec<String> {
    v["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["name"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn list_shows_tables_and_shared_queries_with_their_m() {
    let mut s = session();
    let v = run(&mut s, "query.list", json!({}));
    let q = v["queries"].as_array().unwrap();
    assert_eq!(q.len(), 2);
    assert_eq!(q[0]["name"], "Sales");
    assert_eq!(q[0]["source"], "table");
    assert_eq!(q[0]["partition"], "Sales-main");
    assert_eq!(q[0]["expression"], SALES);
    assert_eq!(q[1]["name"], "Cities");
    assert_eq!(q[1]["source"], "query");
}

#[test]
fn steps_of_a_query_by_name_or_text() {
    let mut s = session();
    let v = run(&mut s, "query.steps", json!({"name": "Sales"}));
    assert_eq!(step_names(&v), ["Source", "Kept"]);
    assert_eq!(v["result_step"], "Kept");
    assert_eq!(v["steps"][1]["function"], "Table.SelectRows");
    assert_eq!(v["source"], "table");
    let v = run(&mut s, "query.steps", json!({"name": "Cities"}));
    assert_eq!(step_names(&v), ["Source"]);
    assert_eq!(v["is_let"], false);
    assert!(
        s.execute("query.steps", &json!({"expression": "let A = in A"}))
            .is_err()
    );
}

#[test]
fn steps_are_added_on_the_draft_and_the_model_is_untouched() {
    let mut s = session();
    let before = s.revision;
    let v = run(
        &mut s,
        "query.apply_step",
        json!({"name": "Sales", "step": {"kind": "filter_rows", "column": "City", "op": "equals", "value": "Paris"}}),
    );
    assert_eq!(v["step"], "Filtered Rows");
    assert_eq!(v["kind"], "filter_rows");
    let m = v["expression"].as_str().unwrap().to_string();
    assert!(m.starts_with("let\n    // typed in\n    Source = #table("), "{m}");
    assert!(m.contains("    Kept   = Table.SelectRows(Source, each [Qty] > 0),\n    #\"Filtered Rows\" = Table.SelectRows(Kept, each [City] = \"Paris\")\nin\n    #\"Filtered Rows\""), "{m}");
    assert_eq!(partition_text(&s, "Sales"), SALES);
    assert_eq!(s.revision, before, "a step on a draft is read-only");
    // the next transforms chain on the draft
    let v = run(
        &mut s,
        "query.apply_step",
        json!({"expression": m, "step": {"kind": "remove_columns", "columns": ["City"]}}),
    );
    let v = run(
        &mut s,
        "query.apply_step",
        json!({"expression": v["expression"], "step": {"kind": "rename_columns", "renames": [{"column": "Qty", "new_name": "Quantity"}]}}),
    );
    let v = run(
        &mut s,
        "query.apply_step",
        json!({"expression": v["expression"], "step": {"kind": "change_type", "columns": [{"column": "Quantity", "type": "decimal"}]}}),
    );
    let v = run(
        &mut s,
        "query.apply_step",
        json!({"expression": v["expression"], "step": {"kind": "sort", "columns": [{"column": "Quantity", "descending": true}]}}),
    );
    assert_eq!(
        step_names(&v),
        [
            "Source",
            "Kept",
            "Filtered Rows",
            "Removed Columns",
            "Renamed Columns",
            "Changed Type",
            "Sorted Rows"
        ]
    );
    // the preview at each step
    let p = run(
        &mut s,
        "query.preview",
        json!({"expression": v["expression"], "step": "Kept"}),
    );
    assert_eq!(p["row_count"], 3);
    assert_eq!(p["step"], "Kept");
    let p = run(&mut s, "query.preview", json!({"expression": v["expression"]}));
    let cols: Vec<&str> = p["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(cols, ["Product", "Quantity"]);
    assert_eq!(p["rows"], json!([["Pen", 9], ["Bike", 2]]));
    assert!(
        s.execute(
            "query.preview",
            &json!({"expression": v["expression"], "step": "Nope"})
        )
        .is_err()
    );
}

#[test]
fn group_merge_append_and_promote_evaluate() {
    let mut s = session();
    let v = run(
        &mut s,
        "query.apply_step",
        json!({"name": "Sales", "step": {"kind": "merge", "other": "Cities", "keys": ["City"], "other_keys": ["City"]}}),
    );
    // the merge's table column previews as [Table]
    let p = run(&mut s, "query.preview", json!({"expression": v["expression"]}));
    assert_eq!(p["rows"][0], json!(["Bike", 2, "Paris", "[Table]"]), "{p}");
    let v = run(
        &mut s,
        "query.apply_step",
        json!({"expression": v["expression"], "step": {"kind": "expand", "column": "Cities", "columns": ["Country"]}}),
    );
    let v = run(
        &mut s,
        "query.apply_step",
        json!({"expression": v["expression"], "step": {"kind": "group_by", "keys": ["Cities.Country"],
            "aggregations": [{"name": "Total", "op": "sum", "column": "Qty"}, {"name": "Rows", "op": "count"}]}}),
    );
    assert_eq!(
        step_names(&v)[2..],
        ["Merged Queries", "Expanded Cities", "Grouped Rows"]
    );
    let p = run(&mut s, "query.preview", json!({"expression": v["expression"]}));
    assert_eq!(p["rows"], json!([["FR", 12, 3]]));
    let v = run(
        &mut s,
        "query.apply_step",
        json!({"name": "Sales", "step": {"kind": "append", "others": ["Sales"]}, "after": "Source"}),
    );
    // inserted after Source: Kept now reads the new step
    assert!(
        v["expression"]
            .as_str()
            .unwrap()
            .contains("Kept   = Table.SelectRows(#\"Appended Query\", each [Qty] > 0)")
    );
    let raw = "let Source = #table({\"Column1\"}, {{\"Name\"}, {\"x\"}}) in Source";
    let v = run(
        &mut s,
        "query.apply_step",
        json!({"expression": raw, "step": {"kind": "promote_headers"}}),
    );
    let p = run(&mut s, "query.preview", json!({"expression": v["expression"]}));
    assert_eq!(p["columns"][0]["name"], "Name");
}

#[test]
fn edit_rename_and_remove_steps() {
    let mut s = session();
    let v = run(
        &mut s,
        "query.edit_step",
        json!({"name": "Sales", "step": "Kept", "formula": "= Table.SelectRows(Source, each [Qty] > 5)"}),
    );
    assert_eq!(
        v["expression"],
        SALES.replace("each [Qty] > 0)", "each [Qty] > 5)")
    );
    let v = run(
        &mut s,
        "query.edit_step",
        json!({"expression": v["expression"], "step": "Source", "new_name": "Typed in"}),
    );
    assert_eq!(step_names(&v), ["Typed in", "Kept"]);
    assert!(
        v["expression"]
            .as_str()
            .unwrap()
            .contains("Table.SelectRows(#\"Typed in\", each [Qty] > 5)")
    );
    let v = run(
        &mut s,
        "query.remove_step",
        json!({"expression": v["expression"], "step": "Kept"}),
    );
    assert_eq!(step_names(&v), ["Typed in"]);
    assert_eq!(v["result_step"], "Typed in");
    for bad in [
        json!({"name": "Sales", "step": "Kept"}),
        json!({"name": "Sales", "step": "Nope", "formula": "1"}),
        json!({"name": "Sales", "step": "Kept", "formula": "Table.SelectRows("}),
        json!({"name": "Sales", "step": "Kept", "new_name": "Source"}),
    ] {
        assert!(s.execute("query.edit_step", &bad).is_err(), "{bad}");
    }
    assert!(
        s.execute("query.remove_step", &json!({"name": "Sales", "step": "Source"}))
            .is_err()
    );
    for bad in [
        json!({"name": "Sales"}),
        json!({"name": "Sales", "step": {"kind": "explode"}}),
        json!({"name": "Sales", "step": {"kind": "filter_rows", "column": "City", "op": "contains", "value": 3}}),
        json!({"name": "Sales", "step": {"kind": "remove_columns", "columns": []}}),
        json!({"name": "Sales", "step": {"kind": "group_by", "keys": [], "aggregations": [{"name": "S", "op": "sum"}]}}),
        json!({"name": "Sales", "step": {"kind": "sort", "columns": [{"column": "Qty", "extra": 1}]}}),
        json!({"name": "Sales", "step": {"kind": "custom"}, "after": "Nope"}),
        json!({"name": "Nope", "step": {"kind": "custom"}}),
    ] {
        assert!(s.execute("query.apply_step", &bad).is_err(), "{bad}");
    }
}

#[test]
fn close_and_apply_writes_reloads_and_undoes_in_one_step() {
    let mut s = session();
    let v = run(
        &mut s,
        "query.apply_step",
        json!({"name": "Sales", "step": {"kind": "remove_columns", "columns": ["City"]}}),
    );
    let v = run(
        &mut s,
        "query.apply_step",
        json!({"expression": v["expression"], "step": {"kind": "add_column", "name": "Double", "formula": "[Qty] * 2"}}),
    );
    let m = v["expression"].as_str().unwrap().to_string();
    let out = run(
        &mut s,
        "query.apply",
        json!({"queries": [{"name": "Sales", "expression": m}]}),
    );
    assert_eq!(out["refreshed"], true);
    assert_eq!(out["queries"][0]["reloaded"], true);
    assert_eq!(out["queries"][0]["rows"], 3);
    assert_eq!(partition_text(&s, "Sales"), m);
    assert_eq!(columns(&s, "Sales"), ["Product", "Qty", "Double"]);
    let part = &s.model.as_ref().unwrap().tables[0].partitions[0];
    assert_eq!(part.name, "Sales-main", "the partition keeps its name");
    let rows = run(&mut s, "table.rows", json!({"table": "Sales"}));
    assert_eq!(rows["rows"][2], json!(["Pen", 9, 18]));
    // one undo step back to the table as it was
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(partition_text(&s, "Sales"), SALES);
    assert_eq!(columns(&s, "Sales"), ["Product", "Qty", "City"]);
    // a query left as it was is skipped
    let out = run(
        &mut s,
        "query.apply",
        json!({"queries": [{"name": "Sales", "expression": SALES}]}),
    );
    assert_eq!(out["queries"], json!([]));
}

#[test]
fn close_and_apply_of_a_shared_query_reloads_its_users_and_new_queries_load() {
    let mut s = session();
    let merged = run(
        &mut s,
        "query.apply_step",
        json!({"name": "Sales", "step": {"kind": "merge", "other": "Cities", "keys": ["City"], "other_keys": ["City"]}}),
    );
    let merged = run(
        &mut s,
        "query.apply_step",
        json!({"expression": merged["expression"], "step": {"kind": "expand", "column": "Cities", "columns": ["Country"], "prefix": false}}),
    );
    run(
        &mut s,
        "query.apply",
        json!({"queries": [{"name": "Sales", "expression": merged["expression"]}]}),
    );
    assert_eq!(columns(&s, "Sales"), ["Product", "Qty", "City", "Country"]);
    let cities =
        "#table(type table [City = text, Country = text], {{\"Paris\", \"France\"}, {\"Lyon\", \"France\"}})";
    let out = run(
        &mut s,
        "query.apply",
        json!({"queries": [
            {"name": "Cities", "expression": cities},
            {"name": "Stock", "expression": "#table({\"Item\"}, {{\"Bike\"}})"}
        ]}),
    );
    let names: Vec<&str> = out["queries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|q| q["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Cities", "Stock", "Sales"], "{out}");
    let rows = run(&mut s, "table.rows", json!({"table": "Sales"}));
    assert_eq!(rows["rows"][0][3], "France");
    assert_eq!(columns(&s, "Stock"), ["Item"]);
    // all or nothing: a failing query leaves the model as it was
    let before = s.model.clone();
    let r = s.execute(
        "query.apply",
        &json!({"queries": [
            {"name": "Cities", "expression": "#table({\"City\"}, {{\"Nice\"}})"},
            {"name": "Sales", "expression": "let Source = Nope in Source"}
        ]}),
    );
    assert!(r.is_err());
    assert_eq!(s.model, before);
    for bad in [
        json!({}),
        json!({"queries": []}),
        json!({"queries": [{"name": "Sales"}]}),
        json!({"queries": [{"name": "Sales", "expression": " "}]}),
        json!({"queries": [{"name": "Sales", "expression": "1"}, {"name": "sales", "expression": "2"}]}),
        json!({"queries": [{"name": "Sales", "expression": "1", "x": 1}]}),
    ] {
        assert!(s.execute("query.apply", &bad).is_err(), "{bad}");
    }
}

#[test]
fn close_and_apply_without_refresh_writes_the_text_only() {
    let mut s = session();
    let m = SALES.replace("each [Qty] > 0)", "each [Qty] > 1)");
    let out = run(
        &mut s,
        "query.apply",
        json!({"queries": [{"name": "Sales", "expression": m}], "refresh": false}),
    );
    assert_eq!(out["queries"][0]["reloaded"], false);
    assert_eq!(partition_text(&s, "Sales"), m);
    assert_eq!(
        s.model.as_ref().unwrap().tables[0].partitions[0].name,
        "Sales-main"
    );
}

#[test]
fn filter_values_keeps_or_drops_the_values_ticked() {
    let mut s = session();
    let v = run(
        &mut s,
        "query.apply_step",
        json!({"name": "Sales", "step": {"kind": "filter_values", "column": "City", "values": ["Lyon", null]}}),
    );
    assert_eq!(v["step"], "Filtered Rows");
    assert_eq!(v["kind"], "filter_values");
    assert!(
        v["expression"]
            .as_str()
            .unwrap()
            .contains("Table.SelectRows(Kept, each ([City] = \"Lyon\" or [City] = null))"),
        "{v}"
    );
    let p = run(
        &mut s,
        "query.preview",
        json!({"expression": v["expression"], "limit": 10}),
    );
    assert_eq!(p["row_count"], 1);
    let v = run(
        &mut s,
        "query.apply_step",
        json!({"name": "Sales", "step": {"kind": "filter_values", "column": "Qty", "values": [9], "exclude": true}}),
    );
    assert!(v["expression"].as_str().unwrap().contains("each ([Qty] <> 9))"));
    for bad in [
        json!({"kind": "filter_values", "column": "City", "values": []}),
        json!({"kind": "filter_values", "column": "City"}),
        json!({"kind": "filter_values", "column": "City", "values": [[1]]}),
        json!({"kind": "filter_values", "values": ["a"]}),
        json!({"kind": "filter_values", "column": "City", "values": ["a"], "exclude": "yes"}),
    ] {
        assert!(
            s.execute("query.apply_step", &json!({"name": "Sales", "step": bad}))
                .is_err(),
            "{bad}"
        );
    }
}

#[test]
fn new_queries_are_blank_duplicates_or_references_with_the_editors_names() {
    let mut s = session();
    let before = s.model.clone();
    let v = run(&mut s, "query.new", json!({}));
    assert_eq!(v["name"], "Query1");
    assert_eq!(v["expression"], "let\n    Source = \"\"\nin\n    Source");
    let v = run(
        &mut s,
        "query.new",
        json!({"kind": "blank", "names": ["Sales", "query1"]}),
    );
    assert_eq!(v["name"], "Query2");
    let v = run(&mut s, "query.new", json!({"kind": "duplicate", "of": "Sales"}));
    assert_eq!(v["name"], "Sales (2)");
    assert_eq!(v["expression"], SALES);
    let v = run(
        &mut s,
        "query.new",
        json!({"kind": "duplicate", "of": "Sales", "expression": "let A = 1 in A", "names": ["Sales", "Sales (2)"]}),
    );
    assert_eq!(v["name"], "Sales (3)");
    assert_eq!(v["expression"], "let A = 1 in A");
    let v = run(
        &mut s,
        "query.new",
        json!({"kind": "reference", "of": "Order Lines"}),
    );
    assert_eq!(v["name"], "Order Lines (2)");
    assert_eq!(
        v["expression"],
        "let\n    Source = #\"Order Lines\"\nin\n    Source"
    );
    assert_eq!(s.model, before, "nothing in the model changes");
    for bad in [
        json!({"kind": "explode"}),
        json!({"kind": "duplicate"}),
        json!({"kind": "duplicate", "of": "Nope"}),
        json!({"kind": "duplicate", "of": "Sales", "expression": "let A = in A"}),
        json!({"kind": "reference", "of": "Sales", "expression": "1"}),
        json!({"kind": "blank", "of": "Sales"}),
        json!({"names": "Sales"}),
        json!({"x": 1}),
    ] {
        assert!(s.execute("query.new", &bad).is_err(), "{bad}");
    }
}

#[test]
fn rename_follows_in_the_drafts_and_close_and_apply_renames_and_deletes_in_one_undo_step() {
    let mut s = session();
    let merged = run(
        &mut s,
        "query.apply_step",
        json!({"name": "Sales", "step": {"kind": "merge", "other": "Cities", "keys": ["City"], "other_keys": ["City"]}}),
    );
    let merged = run(
        &mut s,
        "query.apply_step",
        json!({"expression": merged["expression"], "step": {"kind": "expand", "column": "Cities", "columns": ["Country"], "prefix": false}}),
    );
    let drafts = json!([
        {"name": "Sales", "expression": merged["expression"]},
        {"name": "Cities", "expression": "#table({\"City\"}, {{\"Paris\"}})"},
    ]);
    let v = run(
        &mut s,
        "query.rename",
        json!({"name": "Cities", "new_name": "Towns List", "queries": drafts}),
    );
    assert_eq!(v["name"], "Towns List");
    let q = v["queries"].as_array().unwrap();
    assert_eq!(q.len(), 1, "{v}");
    assert_eq!(q[0]["name"], "Sales");
    let sales = q[0]["expression"].as_str().unwrap().to_string();
    assert!(sales.contains("#\"Towns List\""), "{sales}");
    for bad in [
        json!({"name": "Cities", "new_name": "sales", "queries": drafts}),
        json!({"name": "Cities", "new_name": " x", "queries": drafts}),
        json!({"name": "Nope", "new_name": "X", "queries": drafts}),
        json!({"name": "Cities", "new_name": "X"}),
    ] {
        assert!(s.execute("query.rename", &bad).is_err(), "{bad}");
    }
    // Close & Apply: the shared query renamed (the model's M follows), Sales written
    let out = run(
        &mut s,
        "query.apply",
        json!({"renamed": [{"name": "Cities", "new_name": "Towns List"}],
            "queries": [{"name": "Sales", "expression": sales}]}),
    );
    assert_eq!(out["renamed"][0]["new_name"], "Towns List");
    let m = s.model.as_ref().unwrap();
    assert_eq!(m.expressions[0].name, "Towns List");
    assert_eq!(partition_text(&s, "Sales"), sales);
    assert_eq!(columns(&s, "Sales"), ["Product", "Qty", "City", "Country"]);
    run(&mut s, "edit.undo", json!({}));
    let m = s.model.as_ref().unwrap();
    assert_eq!(m.expressions[0].name, "Cities");
    assert_eq!(partition_text(&s, "Sales"), SALES);
    // a table query renamed: the model's other M follows
    run(
        &mut s,
        "expression.set",
        json!({"name": "Cities", "expression": "let Source = Sales in Source"}),
    );
    run(
        &mut s,
        "query.apply",
        json!({"renamed": [{"name": "Sales", "new_name": "Orders"}], "refresh": false}),
    );
    let m = s.model.as_ref().unwrap();
    assert!(m.tables.iter().any(|t| t.name == "Orders"));
    assert_eq!(m.expressions[0].expression, "let Source = Orders in Source");
    // a query another reads cannot go; once nothing reads it, it goes in the same step
    let before = s.model.clone();
    assert!(s.execute("query.apply", &json!({"removed": ["Orders"]})).is_err());
    assert_eq!(s.model, before);
    let out = run(&mut s, "query.apply", json!({"removed": ["Cities", "Orders"]}));
    assert_eq!(out["removed"], json!(["Cities", "Orders"]));
    let m = s.model.as_ref().unwrap();
    assert!(m.tables.iter().all(|t| t.name != "Orders"));
    assert!(m.expressions.is_empty());
    run(&mut s, "edit.undo", json!({}));
    assert!(
        s.model
            .as_ref()
            .unwrap()
            .tables
            .iter()
            .any(|t| t.name == "Orders")
    );
    for bad in [
        json!({"removed": ["Nope"]}),
        json!({"renamed": [{"name": "Nope", "new_name": "X"}]}),
        json!({"renamed": [{"name": "Orders"}]}),
        json!({"renamed": "Orders"}),
        json!({"removed": ["Cities"], "queries": [{"name": "Cities", "expression": "1"}]}),
    ] {
        assert!(s.execute("query.apply", &bad).is_err(), "{bad}");
    }
}
