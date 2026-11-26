//! Tests of the model editing commands (`model.new`, `table.*`, `column.*`, `measure.*`,
//! `relationship.*`, `role.*`).

use serde_json::{Value, json};

use bicraft_engine::Session;

/// A session with a new model: Sales(Amount, Product, Date) with [Total], Product(Product, Name),
/// 'Date Table'(Date).
fn session() -> Session {
    let mut s = Session::new();
    let steps = [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": "let x = 1 in x"}}),
        ),
        ("table.add", json!({"name": "Product"})),
        ("table.add", json!({"name": "Date Table"})),
        (
            "column.add",
            json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Product", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Date", "data_type": "dateTime"}),
        ),
        (
            "column.add",
            json!({"table": "Product", "name": "Product", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Product", "name": "Name", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Date Table", "name": "Date", "data_type": "dateTime"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
    ];
    for (id, params) in steps {
        s.execute(id, &params)
            .unwrap_or_else(|e| panic!("{id} {params}: {e}"));
    }
    s
}

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Runs a command that must fail, checks the model did not change, and returns the error.
fn fails(s: &mut Session, id: &str, params: Value) -> String {
    let before = s.model.clone();
    let dirty = s.dirty;
    let err = s
        .execute(id, &params)
        .expect_err(&format!("{id} {params} should fail"));
    assert_eq!(s.model, before, "{id} changed the model although it failed");
    assert_eq!(s.dirty, dirty);
    err
}

fn broken_names(v: &Value) -> Vec<String> {
    v["broken"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["name"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn model_new_starts_an_unsaved_model() {
    let mut s = Session::new();
    let v = run(&mut s, "model.new", json!({"name": "Shop"}));
    assert_eq!(v["compatibility_level"], 1606);
    assert_eq!(v["discarded_unsaved_changes"], false);
    let m = s.model.as_ref().unwrap();
    assert_eq!(m.culture.as_deref(), Some("en-US"));
    assert_eq!(m.compatibility_level, Some(1606));
    // What Desktop gives a new model; powerBI_V3 is what lets it refresh M partitions.
    assert_eq!(m.extra["defaultPowerBIDataSourceVersion"], "powerBI_V3");
    assert_eq!(m.extra["sourceQueryCulture"], "en-US");
    assert_eq!(
        m.extra["dataAccessOptions"],
        json!({"legacyRedirects": true, "returnErrorValuesAsNull": true})
    );
    assert_eq!(m.extra["valueFilterBehavior"], "independent");
    assert!(m.tables.is_empty());
    assert!(s.dirty);
    let info = run(&mut s, "file.inspect", json!({}));
    assert_eq!(info["path"], Value::Null);
    assert_eq!(info["dirty"], true);
    assert_eq!(info["details"]["type"], "new");

    run(&mut s, "table.add", json!({"name": "T"}));
    let v = run(&mut s, "model.new", json!({"name": "Other", "culture": "fr-FR"}));
    assert_eq!(v["discarded_unsaved_changes"], true);
    assert_eq!(s.model.as_ref().unwrap().culture.as_deref(), Some("fr-FR"));
    assert_eq!(s.model.as_ref().unwrap().extra["sourceQueryCulture"], "fr-FR");

    assert!(s.execute("model.new", &json!({})).is_err());
    assert!(s.execute("model.new", &json!({"name": " "})).is_err());
    assert!(
        s.execute("model.new", &json!({"name": "M", "culture": "en US"}))
            .is_err()
    );
}

#[test]
fn edits_need_an_open_model() {
    let mut s = Session::new();
    for (id, p) in [
        ("table.add", json!({"name": "T"})),
        (
            "measure.add",
            json!({"table": "T", "name": "M", "expression": "1"}),
        ),
        ("role.add", json!({"name": "R"})),
    ] {
        let e = s.execute(id, &p).unwrap_err();
        assert!(e.contains("no model is open"), "{e}");
    }
    assert!(s.model.is_none() && !s.dirty);
}

#[test]
fn unknown_and_mistyped_parameters_are_rejected() {
    let mut s = session();
    let e = fails(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "X", "expression": "1", "formatstring": "0"}),
    );
    assert!(e.contains("unknown parameter `formatstring`"), "{e}");
    let e = fails(&mut s, "table.add", json!({"name": "X", "hidden": "yes"}));
    assert!(e.contains("`hidden`"), "{e}");
    fails(&mut s, "table.add", json!(["X"]));
    fails(&mut s, "table.remove", json!({"name": 3}));
}

#[test]
fn table_add_checks_names_and_partitions() {
    let mut s = session();
    let e = fails(&mut s, "table.add", json!({"name": "SALES"}));
    assert!(e.contains("already exists"), "{e}");
    fails(&mut s, "table.add", json!({"name": ""}));

    // No partition: allowed, with a warning.
    let v = run(
        &mut s,
        "table.add",
        json!({"name": "Measures", "hidden": true, "description": "d"}),
    );
    assert!(v["warnings"][0].as_str().unwrap().contains("no partition"));
    let t = s.model.as_ref().unwrap().table("measures").unwrap();
    assert!(t.is_hidden && t.lineage_tag.is_some());
    assert_eq!(t.description.as_deref(), Some("d"));

    // A calculated table's DAX is validated against the model.
    let ok = run(
        &mut s,
        "table.add",
        json!({"name": "Big", "partition": {"source_type": "Calculated", "expression": "FILTER(Sales, Sales[Amount] > 10)"}}),
    );
    assert_eq!(ok["warnings"], json!([]));
    let p = &s.model.as_ref().unwrap().table("Big").unwrap().partitions[0];
    assert_eq!(
        (p.source_type.as_str(), p.mode.as_deref()),
        ("calculated", Some("import"))
    );

    let e = fails(
        &mut s,
        "table.add",
        json!({"name": "Bad", "partition": {"source_type": "calculated", "expression": "FILTER(Sales, Sales[Nope] > 1)"}}),
    );
    assert!(e.contains("invalid DAX") && e.contains("force"), "{e}");
    fails(
        &mut s,
        "table.add",
        json!({"name": "Bad", "partition": {"source_type": "sql", "expression": "x"}}),
    );
    fails(
        &mut s,
        "table.add",
        json!({"name": "Bad", "partition": {"source_type": "m"}}),
    );
    fails(&mut s, "table.add", json!({"name": "Bad", "partition": "m"}));

    let v = run(
        &mut s,
        "table.add",
        json!({"name": "Bad", "force": true, "partition": {"source_type": "calculated", "expression": "Sales[Nope]"}}),
    );
    assert!(v["warnings"][0].as_str().unwrap().contains("force"));
    assert_eq!(v["diagnostics"][0]["valid"], false);
    assert!(s.model.as_ref().unwrap().table("Bad").is_some());
}

#[test]
fn table_rename_follows_relationships_and_reports_broken_dax() {
    let mut s = session();
    run(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[Product]", "to": "Product[Product]"}),
    );
    run(&mut s, "role.add", json!({"name": "Reader"}));
    run(
        &mut s,
        "role.set_filter",
        json!({"role": "Reader", "table": "Product", "filter_expression": "Product[Name] = \"A\""}),
    );
    run(
        &mut s,
        "measure.add",
        json!({"table": "Product", "name": "Names", "expression": "COUNTROWS(Product)"}),
    );

    let e = fails(
        &mut s,
        "table.rename",
        json!({"name": "Product", "new_name": "sales"}),
    );
    assert!(e.contains("already exists"), "{e}");
    fails(&mut s, "table.rename", json!({"name": "Nope", "new_name": "X"}));

    // update_dax: false keeps the old DAX: the measure and the row filter then fail.
    let v = run(
        &mut s,
        "table.rename",
        json!({"name": "product", "new_name": "Item", "update_dax": false}),
    );
    let mut broken = broken_names(&v);
    broken.sort();
    assert_eq!(broken, ["Names", "Reader"]);
    assert!(v.get("dax_rewritten").is_none());
    run(&mut s, "edit.undo", json!({}));

    let v = run(
        &mut s,
        "table.rename",
        json!({"name": "product", "new_name": "Item"}),
    );
    assert_eq!(v["renamed"], "Product");
    // The DAX follows: nothing breaks.
    assert!(broken_names(&v).is_empty(), "{v}");
    let mut rewritten: Vec<&str> = v["dax_rewritten"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["kind"].as_str().unwrap())
        .collect();
    rewritten.sort_unstable();
    assert_eq!(rewritten, ["measure", "row filter"]);
    let m = s.model.as_ref().unwrap();
    assert_eq!(m.relationships[0].to_table, "Item");
    assert_eq!(m.roles[0].table_permissions[0].table, "Item");
    assert_eq!(
        m.roles[0].table_permissions[0].filter_expression.as_deref(),
        Some("'Item'[Name] = \"A\"")
    );
    // ITEM is a reserved word: the new name is quoted.
    assert_eq!(m.measure("Names").unwrap().1.expression, "COUNTROWS('Item')");

    // A case-only rename of the same table is allowed.
    run(
        &mut s,
        "table.rename",
        json!({"name": "item", "new_name": "ITEM"}),
    );
    assert!(s.model.as_ref().unwrap().tables.iter().any(|t| t.name == "ITEM"));
}

#[test]
fn table_remove_cascades() {
    let mut s = session();
    run(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[Product]", "to": "Product[Product]"}),
    );
    run(&mut s, "role.add", json!({"name": "R"}));
    run(
        &mut s,
        "role.set_filter",
        json!({"role": "R", "table": "Product", "filter_expression": "TRUE()"}),
    );
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "N", "expression": "COUNTROWS(Product)"}),
    );
    // [N] uses the table: refused, listing it.
    let e = fails(&mut s, "table.remove", json!({"name": "PRODUCT"}));
    assert!(e.contains("measure `Sales`[N]") && e.contains("force"), "{e}");
    let v = run(&mut s, "table.remove", json!({"name": "PRODUCT", "force": true}));
    assert_eq!(v["removed"], "Product");
    assert_eq!(v["removed_relationships"].as_array().unwrap().len(), 1);
    assert_eq!(v["removed_row_filters_from_roles"], json!(["R"]));
    assert_eq!(
        v["used_by"],
        json!([{"kind": "measure", "table": "Sales", "name": "N"}])
    );
    assert_eq!(broken_names(&v), ["N"]);
    // Left as it was.
    assert_eq!(
        s.model.as_ref().unwrap().measure("N").unwrap().1.expression,
        "COUNTROWS(Product)"
    );
    let m = s.model.as_ref().unwrap();
    assert!(m.relationships.is_empty() && m.roles[0].table_permissions.is_empty());
    fails(&mut s, "table.remove", json!({"name": "Product"}));
}

#[test]
fn table_set_changes_and_clears() {
    let mut s = session();
    run(
        &mut s,
        "table.set",
        json!({"name": "sales", "description": "Fact", "hidden": true}),
    );
    let t = s.model.as_ref().unwrap().table("Sales").unwrap();
    assert_eq!((t.description.as_deref(), t.is_hidden), (Some("Fact"), true));
    run(&mut s, "table.set", json!({"name": "Sales", "description": null}));
    assert_eq!(
        s.model.as_ref().unwrap().table("Sales").unwrap().description,
        None
    );
    fails(&mut s, "table.set", json!({"name": "Sales"}));
    fails(&mut s, "table.set", json!({"name": "Nope", "hidden": true}));
}

#[test]
fn column_add_data_and_calculated() {
    let mut s = session();
    let v = run(
        &mut s,
        "column.add",
        json!({"table": "sales", "name": "Qty", "data_type": "INT64", "summarize_by": "SUM",
               "format_string": "0", "display_folder": "Numbers", "hidden": true, "description": "d"}),
    );
    assert_eq!(v["calculated"], false);
    let c = s
        .model
        .as_ref()
        .unwrap()
        .table("Sales")
        .unwrap()
        .column("qty")
        .unwrap()
        .clone();
    assert_eq!(c.data_type, bicraft_model::DataType::Int64);
    assert_eq!(c.source_column.as_deref(), Some("Qty"));
    assert_eq!(c.summarize_by.as_deref(), Some("sum"));
    assert!(c.is_hidden && c.lineage_tag.is_some());

    run(
        &mut s,
        "column.add",
        json!({"table": "Sales", "name": "Double", "data_type": "double", "expression": "Sales[Amount] * 2"}),
    );
    let c = s
        .model
        .as_ref()
        .unwrap()
        .table("Sales")
        .unwrap()
        .column("Double")
        .unwrap()
        .clone();
    assert_eq!(
        (c.expression.as_deref(), c.source_column),
        (Some("Sales[Amount] * 2"), None)
    );

    // Uniqueness, case-insensitively, against columns and measures of the table.
    assert!(
        fails(
            &mut s,
            "column.add",
            json!({"table": "Sales", "name": "AMOUNT", "data_type": "double"})
        )
        .contains("already has")
    );
    assert!(
        fails(
            &mut s,
            "column.add",
            json!({"table": "Sales", "name": "total", "data_type": "double"})
        )
        .contains("measure")
    );
    // Same name in another table is fine.
    run(
        &mut s,
        "column.add",
        json!({"table": "Product", "name": "Amount", "data_type": "double"}),
    );

    fails(
        &mut s,
        "column.add",
        json!({"table": "Sales", "name": "X", "data_type": "text"}),
    );
    fails(&mut s, "column.add", json!({"table": "Sales", "name": "X"}));
    fails(
        &mut s,
        "column.add",
        json!({"table": "Nope", "name": "X", "data_type": "double"}),
    );
    fails(
        &mut s,
        "column.add",
        json!({"table": "Sales", "name": "X", "data_type": "double", "summarize_by": "median"}),
    );
    fails(
        &mut s,
        "column.add",
        json!({"table": "Sales", "name": "X", "data_type": "double", "expression": "1", "source_column": "X"}),
    );

    let e = fails(
        &mut s,
        "column.add",
        json!({"table": "Sales", "name": "X", "data_type": "double", "expression": "Sales[Nope] * 2"}),
    );
    assert!(
        e.contains("calculated column Sales[X]") && e.contains("unknown"),
        "{e}"
    );
    let v = run(
        &mut s,
        "column.add",
        json!({"table": "Sales", "name": "X", "data_type": "double", "expression": "Sales[Nope] * 2", "force": true}),
    );
    assert_eq!(v["warnings"].as_array().unwrap().len(), 1);
}

#[test]
fn column_set_renames_and_reports_broken_dax() {
    let mut s = session();
    run(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[Product]", "to": "Product[Product]"}),
    );
    run(
        &mut s,
        "column.add",
        json!({"table": "Product", "name": "Sort", "data_type": "int64"}),
    );
    s.model
        .as_mut()
        .unwrap()
        .table_mut("Product")
        .unwrap()
        .column_mut("Name")
        .unwrap()
        .sort_by_column = Some("Product".into());
    run(
        &mut s,
        "measure.add",
        json!({"table": "Product", "name": "Products", "expression": "DISTINCTCOUNT(Product[Product])"}),
    );

    let v = run(
        &mut s,
        "column.set",
        json!({"table": "product", "name": "PRODUCT", "new_name": "Key", "format_string": "@"}),
    );
    assert_eq!(v["renamed"], "Product");
    assert!(broken_names(&v).is_empty(), "{v}");
    assert_eq!(
        v["dax_rewritten"],
        json!([{"kind": "measure", "table": "Product", "name": "Products", "references": 1}])
    );
    assert_eq!(
        s.model
            .as_ref()
            .unwrap()
            .measure("Products")
            .unwrap()
            .1
            .expression,
        "DISTINCTCOUNT(Product[Key])"
    );
    let m = s.model.as_ref().unwrap();
    assert_eq!(m.relationships[0].to_column, "Key");
    let t = m.table("Product").unwrap();
    assert_eq!(t.column("Name").unwrap().sort_by_column.as_deref(), Some("Key"));
    assert_eq!(t.column("Key").unwrap().format_string.as_deref(), Some("@"));

    run(
        &mut s,
        "column.set",
        json!({"table": "Product", "name": "Key", "format_string": null, "summarize_by": "none"}),
    );
    let c = s
        .model
        .as_ref()
        .unwrap()
        .table("Product")
        .unwrap()
        .column("Key")
        .unwrap()
        .clone();
    assert_eq!((c.format_string, c.summarize_by.as_deref()), (None, Some("none")));

    // Case-only rename of itself is fine; clashing names are not.
    run(
        &mut s,
        "column.set",
        json!({"table": "Product", "name": "Key", "new_name": "KEY"}),
    );
    fails(
        &mut s,
        "column.set",
        json!({"table": "Product", "name": "KEY", "new_name": "name"}),
    );
    fails(
        &mut s,
        "column.set",
        json!({"table": "Product", "name": "KEY", "new_name": "Products"}),
    );
    fails(&mut s, "column.set", json!({"table": "Product", "name": "KEY"}));
    fails(
        &mut s,
        "column.set",
        json!({"table": "Product", "name": "Nope", "hidden": true}),
    );
    // Expressions belong to calculated columns, source columns to data columns.
    assert!(
        fails(
            &mut s,
            "column.set",
            json!({"table": "Product", "name": "KEY", "expression": "1"})
        )
        .contains("data column")
    );

    run(
        &mut s,
        "column.add",
        json!({"table": "Sales", "name": "Calc", "data_type": "double", "expression": "Sales[Amount]"}),
    );
    fails(
        &mut s,
        "column.set",
        json!({"table": "Sales", "name": "Calc", "source_column": "x"}),
    );
    fails(
        &mut s,
        "column.set",
        json!({"table": "Sales", "name": "Calc", "expression": "Sales[Nope]"}),
    );
    run(
        &mut s,
        "column.set",
        json!({"table": "Sales", "name": "Calc", "expression": "Sales[Nope]", "force": true}),
    );
    run(
        &mut s,
        "column.set",
        json!({"table": "Sales", "name": "Calc", "expression": "Sales[Amount] + 1", "data_type": "int64"}),
    );
}

#[test]
fn column_remove_cascades() {
    let mut s = session();
    run(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[Product]", "to": "Product[Product]"}),
    );
    {
        let t = s.model.as_mut().unwrap().table_mut("Product").unwrap();
        t.column_mut("Name").unwrap().sort_by_column = Some("Product".into());
        t.extra.insert(
            "hierarchies".into(),
            json!([{"name": "H", "levels": [
                {"name": "L1", "ordinal": 0, "column": "Product"},
                {"name": "L2", "ordinal": 1, "column": "Name"}]},
                {"name": "Only", "levels": [{"name": "L", "ordinal": 0, "column": "Product"}]}]),
        );
    }
    let v = run(
        &mut s,
        "column.remove",
        json!({"table": "Product", "name": "product"}),
    );
    assert_eq!(v["removed"], "Product");
    assert_eq!(v["removed_relationships"].as_array().unwrap().len(), 1);
    let t = s.model.as_ref().unwrap().table("Product").unwrap();
    assert_eq!(t.column("Name").unwrap().sort_by_column, None);
    assert_eq!(
        t.extra["hierarchies"],
        json!([{"name": "H", "levels": [{"name": "L2", "ordinal": 0, "column": "Name"}]}])
    );
    assert!(s.model.as_ref().unwrap().relationships.is_empty());

    let e = fails(
        &mut s,
        "column.remove",
        json!({"table": "Sales", "name": "Amount"}),
    );
    assert!(e.contains("[Total]"), "{e}");
    let v = run(
        &mut s,
        "column.remove",
        json!({"table": "Sales", "name": "Amount", "force": true}),
    );
    assert_eq!(broken_names(&v), ["Total"]);
    fails(
        &mut s,
        "column.remove",
        json!({"table": "Sales", "name": "Amount"}),
    );
}

#[test]
fn row_number_columns_are_hidden_and_protected() {
    let mut s = session();
    let mut rn = bicraft_model::Column {
        name: "RowNumber-2662979B".into(),
        data_type: bicraft_model::DataType::Int64,
        is_hidden: true,
        ..Default::default()
    };
    rn.extra.insert("type".into(), json!("rowNumber"));
    s.model
        .as_mut()
        .unwrap()
        .table_mut("Sales")
        .unwrap()
        .columns
        .insert(0, rn);

    let summary = run(&mut s, "model.summary", json!({}));
    let names: Vec<&str> = summary["tables"][0]["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Amount", "Product", "Date"]);

    for (id, p) in [
        (
            "column.set",
            json!({"table": "Sales", "name": "RowNumber-2662979B", "hidden": false}),
        ),
        (
            "column.remove",
            json!({"table": "Sales", "name": "rownumber-2662979b"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "RowNumber-2662979B", "data_type": "int64"}),
        ),
        (
            "column.set",
            json!({"table": "Sales", "name": "Amount", "new_name": "RowNumber-2662979B"}),
        ),
    ] {
        let e = fails(&mut s, id, p);
        assert!(e.contains("row-number"), "{id}: {e}");
    }
    fails(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[RowNumber-2662979B]", "to": "Product[Product]"}),
    );
    // Edits of user columns still work around it.
    run(
        &mut s,
        "column.set",
        json!({"table": "Sales", "name": "Amount", "hidden": true}),
    );
    run(&mut s, "column.remove", json!({"table": "Sales", "name": "Date"}));
    assert_eq!(s.model.as_ref().unwrap().table("Sales").unwrap().columns.len(), 3);
}

#[test]
fn measure_add_validates_and_keeps_names_unique() {
    let mut s = session();
    let v = run(
        &mut s,
        "measure.add",
        json!({"table": "sales", "name": "Twice", "expression": "[Total] * 2", "format_string": "0.00",
               "display_folder": "KPIs", "description": "d", "hidden": true}),
    );
    assert_eq!(v["table"], "Sales");
    assert_eq!(v["warnings"], json!([]));
    let (t, x) = s.model.as_ref().unwrap().measure("twice").unwrap();
    assert_eq!(t.name, "Sales");
    assert_eq!(x.format_string.as_deref(), Some("0.00"));
    assert!(x.is_hidden && x.lineage_tag.is_some());

    // Unique across the model, case-insensitively.
    let e = fails(
        &mut s,
        "measure.add",
        json!({"table": "Product", "name": "TOTAL", "expression": "1"}),
    );
    assert!(e.contains("unique"), "{e}");
    // Not the name of a column of its own table...
    let e = fails(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "amount", "expression": "1"}),
    );
    assert!(e.contains("column"), "{e}");
    // ...but a column of another table is fine.
    run(
        &mut s,
        "measure.add",
        json!({"table": "Product", "name": "Amount", "expression": "1"}),
    );

    let e = fails(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Bad", "expression": "SUM(Sales[Nope])"}),
    );
    assert!(e.contains("measure [Bad]") && e.contains("force"), "{e}");
    fails(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Bad", "expression": "SUM(1 +"}),
    );
    fails(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Bad", "expression": "  "}),
    );
    fails(
        &mut s,
        "measure.add",
        json!({"table": "Nope", "name": "Bad", "expression": "1"}),
    );
    assert!(s.model.as_ref().unwrap().measure("Bad").is_none());

    let v = run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Bad", "expression": "SUM(Sales[Nope])", "force": true}),
    );
    assert!(v["warnings"][0].as_str().unwrap().contains("force"));
    assert_eq!(v["diagnostics"][0]["valid"], false);
    assert!(s.model.as_ref().unwrap().measure("Bad").is_some());
}

#[test]
fn measure_set_rename_and_expression() {
    let mut s = session();
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Twice", "expression": "[Total] * 2"}),
    );
    let v = run(
        &mut s,
        "measure.set",
        json!({"name": "total", "new_name": "Revenue", "format_string": "0"}),
    );
    assert_eq!(v["renamed"], "Total");
    assert!(broken_names(&v).is_empty(), "{v}");
    assert_eq!(v["dax_rewritten"][0]["name"], "Twice");
    run(
        &mut s,
        "measure.set",
        json!({"name": "Twice", "expression": "[Revenue] * 2", "format_string": ""}),
    );
    let (_, x) = s.model.as_ref().unwrap().measure("Twice").unwrap();
    assert_eq!(
        (x.expression.as_str(), x.format_string.as_deref()),
        ("[Revenue] * 2", None)
    );

    let e = fails(
        &mut s,
        "measure.set",
        json!({"name": "Twice", "table": "Product", "hidden": true}),
    );
    assert!(e.contains("is in table `Sales`"), "{e}");
    run(
        &mut s,
        "measure.set",
        json!({"name": "Twice", "table": "SALES", "hidden": true}),
    );
    fails(
        &mut s,
        "measure.set",
        json!({"name": "Twice", "new_name": "revenue"}),
    );
    fails(
        &mut s,
        "measure.set",
        json!({"name": "Twice", "new_name": "Amount"}),
    );
    fails(&mut s, "measure.set", json!({"name": "Twice"}));
    fails(&mut s, "measure.set", json!({"name": "Nope", "hidden": true}));
    fails(
        &mut s,
        "measure.set",
        json!({"name": "Twice", "expression": "[Nope]"}),
    );
    run(
        &mut s,
        "measure.set",
        json!({"name": "Twice", "expression": "[Nope]", "force": true}),
    );
    // A case-only rename of itself is fine and not a rename for DAX purposes.
    let v = run(
        &mut s,
        "measure.set",
        json!({"name": "revenue", "new_name": "REVENUE"}),
    );
    assert_eq!(v["measure"], "REVENUE");
}

#[test]
fn measure_remove_and_move() {
    let mut s = session();
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Twice", "expression": "[Total] * 2"}),
    );
    run(&mut s, "table.add", json!({"name": "Measures"}));
    let v = run(
        &mut s,
        "measure.move",
        json!({"name": "TWICE", "to_table": "measures"}),
    );
    assert_eq!(
        (v["from_table"].as_str(), v["to_table"].as_str()),
        (Some("Sales"), Some("Measures"))
    );
    assert_eq!(broken_names(&v), Vec::<String>::new());
    assert_eq!(
        s.model.as_ref().unwrap().measure("Twice").unwrap().0.name,
        "Measures"
    );
    // The destination may not have a column with the measure's name.
    run(
        &mut s,
        "column.add",
        json!({"table": "Product", "name": "Twice", "data_type": "int64"}),
    );
    fails(
        &mut s,
        "measure.move",
        json!({"name": "Twice", "to_table": "Product"}),
    );
    fails(
        &mut s,
        "measure.move",
        json!({"name": "Twice", "to_table": "Nope"}),
    );
    fails(
        &mut s,
        "measure.move",
        json!({"name": "Nope", "to_table": "Sales"}),
    );

    let e = fails(&mut s, "measure.remove", json!({"name": "total"}));
    assert!(e.contains("[Twice]"), "{e}");
    let v = run(&mut s, "measure.remove", json!({"name": "total", "force": true}));
    assert_eq!(v["removed"], "Total");
    assert_eq!(broken_names(&v), ["Twice"]);
    fails(&mut s, "measure.remove", json!({"name": "Total"}));
}

#[test]
fn relationships_add_and_remove() {
    let mut s = session();
    let v = run(
        &mut s,
        "relationship.add",
        json!({"from": "sales[product]", "to": "'Product'[Product]", "cross_filter": "both"}),
    );
    let name = v["name"].as_str().unwrap().to_string();
    assert_eq!(name.len(), 36);
    assert_eq!(v["from"], "Sales[Product]");
    let r = s.model.as_ref().unwrap().relationships[0].clone();
    assert_eq!(
        (r.from_table.as_str(), r.to_column.as_str()),
        ("Sales", "Product")
    );
    assert_eq!(r.cross_filtering_behavior.as_deref(), Some("bothDirections"));
    assert!(r.is_active);

    let e = fails(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[Product]", "to": "Product[Product]"}),
    );
    assert!(e.contains("already links"), "{e}");
    let e = fails(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[Amount]", "to": "Product[Name]", "active": true}),
    );
    assert!(e.contains("active"), "{e}");
    let v = run(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[Amount]", "to": "Product[Name]", "active": false,
               "from_cardinality": "Many", "to_cardinality": "one"}),
    );
    assert!(v["warnings"][0].as_str().unwrap().contains("data types"));
    assert_ne!(v["name"], name.as_str());

    run(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[Date]", "to": "'Date Table'[Date]"}),
    );
    fails(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[Date]", "to": "Sales[Amount]"}),
    );
    fails(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[Nope]", "to": "Product[Name]"}),
    );
    fails(
        &mut s,
        "relationship.add",
        json!({"from": "Sales", "to": "Product[Name]"}),
    );
    fails(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[Date]", "to": "Product[Name]", "cross_filter": "sideways"}),
    );
    fails(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[Date]", "to": "Product[Name]", "to_cardinality": "few"}),
    );

    let v = run(
        &mut s,
        "relationship.remove",
        json!({"from": "SALES[Date]", "to": "'date table'[date]"}),
    );
    assert_eq!(v["to"], "'Date Table'[Date]");
    run(
        &mut s,
        "relationship.remove",
        json!({"name": name.to_uppercase()}),
    );
    assert_eq!(s.model.as_ref().unwrap().relationships.len(), 1);
    fails(&mut s, "relationship.remove", json!({"name": name}));
    fails(&mut s, "relationship.remove", json!({"from": "Sales[Date]"}));
    fails(&mut s, "relationship.remove", json!({}));
    fails(
        &mut s,
        "relationship.remove",
        json!({"name": "x", "from": "Sales[Date]", "to": "Product[Name]"}),
    );
}

#[test]
fn roles_and_row_filters() {
    let mut s = session();
    let v = run(&mut s, "role.add", json!({"name": "Readers"}));
    assert_eq!(v["model_permission"], "read");
    run(
        &mut s,
        "role.add",
        json!({"name": "Admins", "model_permission": "ADMINISTRATOR"}),
    );
    fails(&mut s, "role.add", json!({"name": "readers"}));
    fails(
        &mut s,
        "role.add",
        json!({"name": "X", "model_permission": "write"}),
    );

    run(
        &mut s,
        "role.set_filter",
        json!({"role": "readers", "table": "product", "filter_expression": "Product[Name] = \"A\""}),
    );
    let r = s.model.as_ref().unwrap().roles[0].clone();
    assert_eq!(r.table_permissions[0].table, "Product");
    // Replacing the filter keeps a single permission.
    run(
        &mut s,
        "role.set_filter",
        json!({"role": "Readers", "table": "Product", "filter_expression": "TRUE()"}),
    );
    assert_eq!(s.model.as_ref().unwrap().roles[0].table_permissions.len(), 1);

    let e = fails(
        &mut s,
        "role.set_filter",
        json!({"role": "Readers", "table": "Product", "filter_expression": "Product[Nope] = 1"}),
    );
    assert!(e.contains("row filter"), "{e}");
    fails(
        &mut s,
        "role.set_filter",
        json!({"role": "Nope", "table": "Product", "filter_expression": "TRUE()"}),
    );
    fails(
        &mut s,
        "role.set_filter",
        json!({"role": "Readers", "table": "Nope", "filter_expression": "TRUE()"}),
    );
    fails(
        &mut s,
        "role.set_filter",
        json!({"role": "Readers", "table": "Product"}),
    );
    run(
        &mut s,
        "role.set_filter",
        json!({"role": "Readers", "table": "Sales", "filter_expression": "Sales[Nope] = 1", "force": true}),
    );
    assert_eq!(s.model.as_ref().unwrap().roles[0].table_permissions.len(), 2);

    run(
        &mut s,
        "role.set_filter",
        json!({"role": "Readers", "table": "Product", "filter_expression": null}),
    );
    assert_eq!(s.model.as_ref().unwrap().roles[0].table_permissions.len(), 1);

    let summary = run(&mut s, "model.summary", json!({}));
    assert_eq!(summary["roles"][0]["row_filters"][0]["table"], "Sales");

    run(&mut s, "role.remove", json!({"name": "ADMINS"}));
    fails(&mut s, "role.remove", json!({"name": "Admins"}));
    assert_eq!(s.model.as_ref().unwrap().roles.len(), 1);
}

#[test]
fn dirty_tracks_successful_edits_only() {
    let tmp = crate::tempdir::TempDir::new("engine-dirty");
    let dir = tmp.path();
    let bim = dir.join("model.bim");
    std::fs::write(
        &bim,
        r#"{"name":"M","model":{"tables":[{"name":"Sales","columns":[{"name":"Amount","dataType":"double"}]}]}}"#,
    )
    .unwrap();
    let mut s = Session::new();
    let v = run(&mut s, "file.open", json!({"path": bim}));
    assert_eq!(v["dirty"], false);
    fails(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "T", "expression": "SUM(Sales[X])"}),
    );
    assert!(!s.dirty);
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "T", "expression": "SUM(Sales[Amount])"}),
    );
    assert_eq!(run(&mut s, "file.inspect", json!({}))["dirty"], true);
    // Reopening discards the edits.
    run(&mut s, "file.open", json!({"path": bim}));
    assert!(!s.dirty);
    assert!(s.model.as_ref().unwrap().measure("T").is_none());
}

#[test]
fn a_batch_of_edits_builds_a_model() {
    let mut s = Session::new();
    let steps: Vec<(String, Value)> = [
        ("model.new", json!({"name": "M"})),
        ("table.add", json!({"name": "T"})),
        (
            "column.add",
            json!({"table": "T", "name": "C", "data_type": "int64"}),
        ),
        (
            "measure.add",
            json!({"table": "T", "name": "Sum", "expression": "SUM(T[C])"}),
        ),
        (
            "measure.add",
            json!({"table": "T", "name": "Bad", "expression": "SUM(T[D])"}),
        ),
        (
            "measure.add",
            json!({"table": "T", "name": "Never", "expression": "1"}),
        ),
    ]
    .into_iter()
    .map(|(id, p)| (id.to_string(), p))
    .collect();
    let (index, err, done) = s.batch(&steps).unwrap_err();
    assert_eq!(index, 4);
    assert!(err.contains("invalid DAX"));
    assert_eq!(done.len(), 4);
    // Steps before the failure stay applied.
    let m = s.model.as_ref().unwrap();
    assert!(m.measure("Sum").is_some() && m.measure("Bad").is_none() && m.measure("Never").is_none());
}

/// A model with DAX in every place a rename must reach: KPIs, format string and detail rows
/// expressions, a calculation group, a field parameter, a calculated column, a row filter, and
/// an M partition (which must stay untouched).
fn g15_session() -> Session {
    let mut s = session();
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Twice", "expression": "VAR t = [Total] RETURN t * 2 -- [Total]"}),
    );
    run(
        &mut s,
        "column.add",
        json!({"table": "Sales", "name": "Big", "data_type": "boolean", "expression": "[Amount] > 100"}),
    );
    run(
        &mut s,
        "table.add",
        json!({"name": "Fields", "partition": {"source_type": "calculated",
            "expression": "{(\"Amount\", NAMEOF('Sales'[Amount]), 0), (\"Total\", NAMEOF([Total]), 1)}"}}),
    );
    run(&mut s, "role.add", json!({"name": "R"}));
    run(
        &mut s,
        "role.set_filter",
        json!({"role": "R", "table": "Sales", "filter_expression": "[Amount] > 0 && \"Sales[Amount]\" <> \"\""}),
    );
    let m = s.model.as_mut().unwrap();
    let sales = m.table_mut("Sales").unwrap();
    sales.partitions[0].expression = Some("let Sales = #table({\"Amount\"}, {{1}}) in Sales".into());
    let total = sales.measures.iter_mut().find(|x| x.name == "Total").unwrap();
    total.extra.insert(
        "kpi".into(),
        json!({"targetExpression": "[Total] * 1.1", "statusExpression": ["IF([Total] > 0,", "  1, -1)"],
               "trendExpression": "SUM(Sales[Amount])", "statusGraphic": "Shapes"}),
    );
    total.extra.insert(
        "formatStringDefinition".into(),
        json!({"expression": "IF([Total] > 1e6, \"#,0,,M\", \"#,0\")"}),
    );
    total.extra.insert(
        "detailRowsDefinition".into(),
        json!({"expression": "SELECTCOLUMNS(Sales, \"Amount\", Sales[Amount])"}),
    );
    m.tables.push(bicraft_model::Table {
        name: "Time Intel".into(),
        extra: serde_json::from_value(json!({"calculationGroup": {
            "calculationItems": [
                {"name": "Current", "expression": "SELECTEDMEASURE()"},
                {"name": "Doubled", "expression": "SELECTEDMEASURE() + [Total]",
                 "formatStringDefinition": {"expression": "IF(ISSELECTEDMEASURE([Total]), \"0\", \"0.0\")"}}
            ],
            "noSelectionExpression": {"expression": "[Total]"}
        }}))
        .unwrap(),
        ..Default::default()
    });
    s
}

#[test]
fn renames_rewrite_every_dax_expression() {
    let mut s = g15_session();
    let v = run(
        &mut s,
        "measure.rename",
        json!({"name": "TOTAL", "new_name": "Rev]enue"}),
    );
    assert!(broken_names(&v).is_empty(), "{v}");
    let kinds: Vec<&str> = v["dax_rewritten"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["kind"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        [
            "kpi target",
            "kpi status",
            "format string expression",
            "measure",
            "calculated table",
            "no selection expression",
            "calculation item",
            "calculation item format string"
        ]
    );
    let m = s.model.as_ref().unwrap();
    let (_, total) = m.measure("Rev]enue").unwrap();
    assert_eq!(total.extra["kpi"]["targetExpression"], "[Rev]]enue] * 1.1");
    // An expression stored as lines stays lines.
    assert_eq!(
        total.extra["kpi"]["statusExpression"],
        json!(["IF([Rev]]enue] > 0,", "  1, -1)"])
    );
    assert_eq!(
        m.measure("Twice").unwrap().1.expression,
        "VAR t = [Rev]]enue] RETURN t * 2 -- [Total]"
    );
    let ti = m.table("Time Intel").unwrap();
    assert_eq!(
        ti.extra["calculationGroup"]["calculationItems"][1]["formatStringDefinition"]["expression"],
        "IF(ISSELECTEDMEASURE([Rev]]enue]), \"0\", \"0.0\")"
    );

    let v = run(
        &mut s,
        "column.rename",
        json!({"table": "sales", "name": "amount", "new_name": "Net Amount"}),
    );
    assert!(broken_names(&v).is_empty(), "{v}");
    assert_eq!(v["dax_rewritten"].as_array().unwrap().len(), 6, "{v}");
    let m = s.model.as_ref().unwrap();
    let sales = m.table("Sales").unwrap();
    assert_eq!(
        sales.column("Big").unwrap().expression.as_deref(),
        Some("[Net Amount] > 100")
    );
    assert_eq!(
        m.roles[0].table_permissions[0].filter_expression.as_deref(),
        Some("[Net Amount] > 0 && \"Sales[Amount]\" <> \"\"")
    );
    assert_eq!(
        m.table("Fields").unwrap().partitions[0].expression.as_deref(),
        Some("{(\"Amount\", NAMEOF('Sales'[Net Amount]), 0), (\"Total\", NAMEOF([Rev]]enue]), 1)}")
    );

    let v = run(
        &mut s,
        "table.rename",
        json!({"name": "Sales", "new_name": "Fact"}),
    );
    assert!(broken_names(&v).is_empty(), "{v}");
    let m = s.model.as_ref().unwrap();
    let fact = m.table("Fact").unwrap();
    // M is not DAX: Desktop does not rewrite it either.
    assert_eq!(
        fact.partitions[0].expression.as_deref(),
        Some("let Sales = #table({\"Amount\"}, {{1}}) in Sales")
    );
    let (_, total) = m.measure("Rev]enue").unwrap();
    assert_eq!(total.expression, "SUM(Fact[Net Amount])");
    assert_eq!(total.extra["kpi"]["trendExpression"], "SUM(Fact[Net Amount])");
    assert_eq!(
        total.extra["detailRowsDefinition"]["expression"],
        "SELECTCOLUMNS(Fact, \"Amount\", Fact[Net Amount])"
    );
    assert_eq!(
        m.table("Fields").unwrap().partitions[0].expression.as_deref(),
        Some("{(\"Amount\", NAMEOF('Fact'[Net Amount]), 0), (\"Total\", NAMEOF([Rev]]enue]), 1)}")
    );
}

#[test]
fn rename_opt_out_and_unparsable_expressions() {
    let mut s = g15_session();
    let v = run(
        &mut s,
        "column.rename",
        json!({"table": "Sales", "name": "Amount", "new_name": "Net", "update_dax": false}),
    );
    assert!(v.get("dax_rewritten").is_none());
    assert!(v["warnings"][0].as_str().unwrap().contains("update_dax"));
    assert!(!broken_names(&v).is_empty());
    run(&mut s, "edit.undo", json!({}));

    // An expression that does not parse is left alone and listed.
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Broken", "expression": "SUM(Sales[Amount]", "force": true}),
    );
    let v = run(
        &mut s,
        "column.rename",
        json!({"table": "Sales", "name": "Amount", "new_name": "Net"}),
    );
    assert_eq!(
        v["dax_not_rewritten"],
        json!([{"kind": "measure", "table": "Sales", "name": "Broken",
                "reason": "the expression does not parse"}])
    );
    // A variable named like the new table name would capture a bare reference: left as it is.
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Var", "expression": "VAR Fact = 1 RETURN COUNTROWS(Product) + Fact"}),
    );
    let v = run(
        &mut s,
        "table.rename",
        json!({"name": "Product", "new_name": "Fact"}),
    );
    let skipped = &v["dax_not_rewritten"][0];
    assert_eq!(skipped["name"], "Var", "{v}");
    assert!(skipped["reason"].as_str().unwrap().contains("variable"), "{v}");
    assert_eq!(
        s.model.as_ref().unwrap().measure("Broken").unwrap().1.expression,
        "SUM(Sales[Amount]"
    );
    // A rename to the same name is not a rename.
    let v = run(
        &mut s,
        "measure.rename",
        json!({"name": "Total", "new_name": "Total"}),
    );
    assert!(v.get("dax_rewritten").is_none());
    fails(&mut s, "measure.rename", json!({"name": "Total"}));
    fails(
        &mut s,
        "column.rename",
        json!({"table": "Sales", "name": "Net", "new_name": "X", "format_string": "0"}),
    );
}

#[test]
fn removals_list_their_dependants() {
    let mut s = g15_session();
    let e = fails(&mut s, "measure.remove", json!({"name": "Total"}));
    for part in [
        "measure `Sales`[Twice]",
        "calculated table `Fields`",
        "calculation item `Time Intel`[Doubled]",
        "no selection expression",
    ] {
        assert!(e.contains(part), "{part}: {e}");
    }
    // The measure's own KPI and format string go with it: not dependants.
    assert!(!e.contains("kpi"), "{e}");
    let e = fails(
        &mut s,
        "column.remove",
        json!({"table": "Sales", "name": "Amount"}),
    );
    assert!(
        e.contains("row filter of role `R`") && e.contains("calculated column `Sales`[Big]"),
        "{e}"
    );
    // Expressions inside the removed table go with it; the others are listed.
    let e = fails(&mut s, "table.remove", json!({"name": "Sales"}));
    assert!(
        e.contains("calculated table `Fields`") && !e.contains("[Twice]"),
        "{e}"
    );
    // A column nothing uses goes without force.
    run(&mut s, "column.remove", json!({"table": "Sales", "name": "Date"}));
}

/// A session with name-keyed metadata around Sales: a perspective, translations, linguistic
/// metadata, a calculated table copying Sales, a variation and a user-defined function.
fn names_session() -> Session {
    let mut s = session();
    run(&mut s, "table.add", json!({"name": "Measures"}));
    let m = s.model.as_mut().unwrap();
    m.extra.insert(
        "perspectives".into(),
        json!([{"name": "Lite", "tables": [
            {"name": "Sales", "columns": [{"name": "Amount"}], "measures": [{"name": "Total"}]},
            {"name": "Product", "includeAll": true}
        ]}]),
    );
    m.extra.insert(
        "cultures".into(),
        json!([
            {"name": "en-US", "linguisticMetadata": {"contentType": "json", "content": {
                "Version": "1.0.0", "Language": "en-US",
                "Entities": {
                    "sales": {"Definition": {"Binding": {"ConceptualEntity": "Sales"}}},
                    "sales.amount": {"Definition": {"Binding": {"ConceptualEntity": "Sales", "ConceptualProperty": "Amount"}}},
                    "sales.total": {"Definition": {"Binding": {"ConceptualEntity": "Sales", "ConceptualProperty": "Total"}}},
                    "custom key": {"Definition": {"Binding": {"ConceptualEntity": "Sales", "ConceptualProperty": "Total"}}}
                },
                "Relationships": {"sales_has_amount": {"Roles": {
                    "sales": {"Target": {"Entity": "sales"}},
                    "sales.amount": {"Target": {"Entity": "sales.amount"}}
                }}}}}},
            {"name": "fr-FR", "translations": {"model": {"name": "Model", "tables": [
                {"name": "Sales", "translatedCaption": "Ventes",
                 "columns": [{"name": "Amount", "translatedCaption": "Montant"}],
                 "measures": [{"name": "Total", "translatedCaption": "Total FR"}]}
            ]}}}
        ]),
    );
    m.extra.insert(
        "functions".into(),
        json!([{"name": "Scaled", "expression": "(k : SCALAR NUMERIC VAL) => SUM(Sales[Amount]) * k + Sales[Total]"}]),
    );
    m.tables.push(bicraft_model::Table {
        name: "Copy".into(),
        columns: vec![bicraft_model::Column {
            name: "Amount".into(),
            data_type: bicraft_model::DataType::Double,
            source_column: Some("Sales[Amount]".into()),
            extra: serde_json::from_value(json!({"type": "calculatedTableColumn", "isNameInferred": true}))
                .unwrap(),
            ..Default::default()
        }],
        partitions: vec![bicraft_model::Partition {
            name: "Copy".into(),
            source_type: "calculated".into(),
            expression: Some("Sales".into()),
            ..Default::default()
        }],
        ..Default::default()
    });
    let date = m
        .table_mut("Sales")
        .unwrap()
        .columns
        .iter_mut()
        .find(|c| c.name == "Date")
        .unwrap();
    date.extra.insert(
        "variations".into(),
        json!([{"name": "Variation", "relationship": "r1",
                "defaultHierarchy": {"table": "Date Table", "hierarchy": "Date Hierarchy"}, "isDefault": true}]),
    );
    s
}

#[test]
fn renames_follow_perspectives_translations_and_source_columns() {
    let mut s = names_session();
    let v = run(
        &mut s,
        "table.rename",
        json!({"name": "sales", "new_name": "Fact Sales"}),
    );
    let kinds: Vec<&str> = v["metadata_updated"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["kind"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        [
            "perspective",
            "linguistic metadata",
            "translations",
            "calculated table column"
        ]
    );
    let functions: Vec<&str> = v["dax_rewritten"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|x| x["kind"] == "function")
        .map(|x| x["name"].as_str().unwrap())
        .collect();
    assert_eq!(functions, ["Scaled"]);
    run(
        &mut s,
        "table.rename",
        json!({"name": "Date Table", "new_name": "Calendar"}),
    );
    run(
        &mut s,
        "column.rename",
        json!({"table": "Fact Sales", "name": "amount", "new_name": "Net"}),
    );
    run(
        &mut s,
        "measure.rename",
        json!({"name": "Total", "new_name": "Revenue"}),
    );
    let m = s.model.as_ref().unwrap();
    assert_eq!(
        m.extra["perspectives"][0]["tables"][0],
        json!({"name": "Fact Sales", "columns": [{"name": "Net"}], "measures": [{"name": "Revenue"}]})
    );
    assert_eq!(m.extra["perspectives"][0]["tables"][1]["name"], "Product");
    let fr = &m.extra["cultures"][1]["translations"]["model"]["tables"][0];
    assert_eq!(fr["name"], "Fact Sales");
    assert_eq!(fr["columns"][0]["name"], "Net");
    assert_eq!(fr["measures"][0]["name"], "Revenue");
    // Entity keys generated from the names follow (`custom key` is not generated: it stays),
    // and so do the relationship roles targeting them.
    let content = &m.extra["cultures"][0]["linguisticMetadata"]["content"];
    let entities = &content["Entities"];
    let keys: Vec<&String> = entities.as_object().unwrap().keys().collect();
    assert_eq!(
        keys,
        ["fact_sales", "fact_sales.net", "fact_sales.revenue", "custom key"]
    );
    assert_eq!(
        entities["fact_sales"]["Definition"]["Binding"],
        json!({"ConceptualEntity": "Fact Sales"})
    );
    assert_eq!(
        entities["fact_sales.net"]["Definition"]["Binding"],
        json!({"ConceptualEntity": "Fact Sales", "ConceptualProperty": "Net"})
    );
    assert_eq!(
        entities["custom key"]["Definition"]["Binding"]["ConceptualProperty"],
        "Revenue"
    );
    let roles = &content["Relationships"]["sales_has_amount"]["Roles"];
    assert_eq!(roles["sales"]["Target"]["Entity"], "fact_sales");
    assert_eq!(roles["sales.amount"]["Target"]["Entity"], "fact_sales.net");
    let copy = m.table("Copy").unwrap();
    assert_eq!(copy.columns[0].source_column.as_deref(), Some("Fact Sales[Net]"));
    assert_eq!(copy.partitions[0].expression.as_deref(), Some("'Fact Sales'"));
    assert_eq!(
        m.extra["functions"][0]["expression"],
        "(k : SCALAR NUMERIC VAL) => SUM('Fact Sales'[Net]) * k + 'Fact Sales'[Revenue]"
    );
    let date = m
        .table("Fact Sales")
        .unwrap()
        .columns
        .iter()
        .find(|c| c.name == "Date")
        .unwrap();
    assert_eq!(
        date.extra["variations"][0]["defaultHierarchy"]["table"],
        "Calendar"
    );
}

#[test]
fn measure_move_rewrites_qualifiers_and_memberships() {
    let mut s = names_session();
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Twice", "expression": "Sales[Total] * 2 + [Total] + 'Sales'[Total]"}),
    );
    let v = run(
        &mut s,
        "measure.move",
        json!({"name": "Total", "to_table": "Measures"}),
    );
    assert_eq!(v["dax_rewritten"].as_array().unwrap().len(), 2, "{v}");
    assert_eq!(broken_names(&v), Vec::<String>::new());
    let m = s.model.as_ref().unwrap();
    assert_eq!(
        m.measure("Twice").unwrap().1.expression,
        "Measures[Total] * 2 + [Total] + 'Measures'[Total]"
    );
    assert_eq!(
        m.extra["functions"][0]["expression"],
        "(k : SCALAR NUMERIC VAL) => SUM(Sales[Amount]) * k + Measures[Total]"
    );
    let lite = &m.extra["perspectives"][0]["tables"];
    assert_eq!(lite[0], json!({"name": "Sales", "columns": [{"name": "Amount"}]}));
    assert_eq!(
        lite[2],
        json!({"name": "Measures", "measures": [{"name": "Total"}]})
    );
    let fr = &m.extra["cultures"][1]["translations"]["model"]["tables"];
    assert!(fr[0].get("measures").is_none());
    assert_eq!(
        fr[1],
        json!({"name": "Measures", "measures": [{"name": "Total", "translatedCaption": "Total FR"}]})
    );
    assert_eq!(
        m.extra["cultures"][0]["linguisticMetadata"]["content"]["Entities"]["measures.total"]["Definition"]["Binding"],
        json!({"ConceptualEntity": "Measures", "ConceptualProperty": "Total"})
    );
    // Opting out leaves the DAX alone (and reports it broken).
    let v = run(
        &mut s,
        "measure.move",
        json!({"name": "Total", "to_table": "Product", "update_dax": false}),
    );
    assert!(v.get("dax_rewritten").is_none());
    assert_eq!(broken_names(&v), ["Twice"]);
}

/// Calculated-table columns named after their source follow its renames, as the engine does
/// (`bicraft_engine::rename_inferred`): `C1 = Sales`, `C2 = SELECTCOLUMNS(Sales, Sales[Amount])`,
/// `C9 = FILTER(C1, …)`, with DAX on the inferred columns.
#[test]
fn inferred_calculated_table_columns_follow_renames() {
    let mut s = session();
    let inferred = |name: &str, source: &str| -> bicraft_model::Column {
        let mut c = bicraft_model::Column {
            name: name.into(),
            source_column: Some(source.into()),
            ..Default::default()
        };
        c.extra.insert("type".into(), json!("calculatedTableColumn"));
        c.extra.insert("isNameInferred".into(), json!(true));
        c
    };
    let calc = |name: &str, expr: &str, columns: Vec<bicraft_model::Column>| bicraft_model::Table {
        name: name.into(),
        columns,
        partitions: vec![bicraft_model::Partition {
            name: name.into(),
            source_type: "calculated".into(),
            expression: Some(expr.into()),
            ..Default::default()
        }],
        ..Default::default()
    };
    {
        let m = s.model.as_mut().unwrap();
        let mut c1 = calc(
            "C1",
            "Sales",
            vec![
                inferred("Amount", "Sales[Amount]"),
                inferred("Product", "Sales[Product]"),
            ],
        );
        c1.measures.push(bicraft_model::Measure {
            name: "M1".into(),
            expression: "SUM(C1[Amount])".into(),
            ..Default::default()
        });
        m.tables.push(c1);
        m.tables.push(calc(
            "C2",
            "SELECTCOLUMNS(Sales, Sales[Amount])",
            vec![inferred("Sales_Amount", "Sales[Amount]")],
        ));
        m.tables.push(calc(
            "C9",
            "FILTER(C1, C1[Amount] > 1)",
            vec![
                inferred("Amount", "C1[Amount]"),
                inferred("Product", "C1[Product]"),
            ],
        ));
    }
    let v = run(
        &mut s,
        "column.rename",
        json!({"table": "Sales", "name": "Amount", "new_name": "Net"}),
    );
    assert!(broken_names(&v).is_empty(), "{v}");
    let mut renamed: Vec<String> = v["inferred_renamed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| {
            format!(
                "{}[{}]->{}",
                x["table"].as_str().unwrap(),
                x["old"].as_str().unwrap(),
                x["new"].as_str().unwrap()
            )
        })
        .collect();
    renamed.sort();
    assert_eq!(
        renamed,
        [
            "C1[Amount]->Net",
            "C2[Sales_Amount]->Sales_Net",
            "C9[Amount]->Net"
        ]
    );
    let m = s.model.as_ref().unwrap();
    assert_eq!(m.measure("M1").unwrap().1.expression, "SUM(C1[Net])");
    let c9 = m.table("C9").unwrap();
    assert_eq!(
        c9.partitions[0].expression.as_deref(),
        Some("FILTER(C1, C1[Net] > 1)")
    );
    assert_eq!(c9.columns[0].source_column.as_deref(), Some("C1[Net]"));
    assert_eq!(c9.columns[0].extra["isNameInferred"], true);

    // Renaming an inferred column by hand makes its name explicit; its followers follow.
    let v = run(
        &mut s,
        "column.rename",
        json!({"table": "C1", "name": "Product", "new_name": "Prod"}),
    );
    assert!(broken_names(&v).is_empty(), "{v}");
    let m = s.model.as_ref().unwrap();
    let prod = m.table("C1").unwrap().column("Prod").unwrap();
    assert!(prod.extra.get("isNameInferred").is_none());
    assert_eq!(prod.source_column.as_deref(), Some("Sales[Product]"));
    assert!(m.table("C9").unwrap().column("Prod").is_some());

    // A table rename renames the `Table_Column` names only.
    let v = run(
        &mut s,
        "table.rename",
        json!({"name": "Sales", "new_name": "Orders"}),
    );
    assert_eq!(v["inferred_renamed"][0]["new"], "Orders_Net", "{v}");
    let m = s.model.as_ref().unwrap();
    assert_eq!(
        m.table("C2").unwrap().columns[0].source_column.as_deref(),
        Some("Orders[Net]")
    );
    assert!(m.table("C1").unwrap().column("Net").is_some());
}

/// Desktop 2.158 first-save diff (M9.7): a new boolean column's format is TMSL
/// `"TRUE";"TRUE";"FALSE"` (TMDL `"""TRUE"";""TRUE"";""FALSE"""`), not quoted twice; a new
/// model keeps auto date/time off explicitly (without `__PBI_TimeIntelligenceEnabled = 0`
/// Desktop added `LocalDateTable_…` tables at its first refresh).
#[test]
fn new_model_defaults_match_desktop_first_save() {
    let tmp = crate::tempdir::TempDir::new("engine-first-save");
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Kinds"}));
    run(
        &mut s,
        "table.import",
        json!({"name": "Flags", "m": "let Source = #table(type table [B = logical], {{true}, {false}}) in Source"}),
    );
    let m = s.model.as_ref().unwrap();
    assert!(
        m.annotations
            .iter()
            .any(|a| a.name == "__PBI_TimeIntelligenceEnabled" && a.value == "0")
    );
    let b = &m.tables[0].columns[0];
    assert_eq!(b.format_string.as_deref(), Some("\"TRUE\";\"TRUE\";\"FALSE\""));
    run(&mut s, "project.new", json!({"dir": tmp.path(), "name": "Kinds"}));
    let def = tmp.path().join("Kinds.SemanticModel").join("definition");
    let table = std::fs::read_to_string(def.join("tables").join("Flags.tmdl")).unwrap();
    assert!(
        table.contains("formatString: \"\"\"TRUE\"\";\"\"TRUE\"\";\"\"FALSE\"\"\"\r\n"),
        "{table}"
    );
    let model = std::fs::read_to_string(def.join("model.tmdl")).unwrap();
    assert!(
        model.contains("annotation __PBI_TimeIntelligenceEnabled = 0"),
        "{model}"
    );
}
