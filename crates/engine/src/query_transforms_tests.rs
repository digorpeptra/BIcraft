//! `query.apply_step`'s row, column, text, number and date kinds (`query_transforms.rs`): each
//! step evaluates in `query.preview` to the table its M function describes; dates filter as
//! `#date` / `#datetime` literals and previews give a date/time column's M type.

use serde_json::{Value, json};

use super::*;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

const SALES: &str = "let\n    Source = #table(type table [Product = text, Qty = Int64.Type, City = text], {{\"Bike\", 2, \"Paris\"}, {\"Car\", 1, \"Lyon\"}, {\"Pen\", 9, \"Paris\"}}),\n    Kept = Table.SelectRows(Source, each [Qty] > 0)\nin\n    Kept";

fn session() -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Shop"}));
    run(&mut s, "table.import", json!({"name": "Sales", "m": SALES}));
    s
}

/// Adds `step` after the last step of Sales and previews the result.
fn step_preview(s: &mut Session, step: Value) -> (Value, Value) {
    let v = run(s, "query.apply_step", json!({"name": "Sales", "step": step}));
    let p = run(
        s,
        "query.preview",
        json!({"expression": v["expression"], "limit": 10}),
    );
    (v, p)
}

fn column(p: &Value, name: &str) -> Vec<Value> {
    let at = p["columns"]
        .as_array()
        .unwrap()
        .iter()
        .position(|c| c["name"] == name)
        .unwrap_or_else(|| panic!("no column {name}: {p}"));
    p["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r[at].clone())
        .collect()
}

#[test]
fn row_column_and_value_transforms_evaluate() {
    let mut s = session();
    let (v, p) = step_preview(&mut s, json!({"kind": "remove_top_rows", "count": 1}));
    assert_eq!(v["step"], "Removed Top Rows");
    assert_eq!(v["kind"], "remove_top_rows");
    assert_eq!(p["row_count"], 2);
    let (_, p) = step_preview(&mut s, json!({"kind": "keep_range", "first": 2, "count": 1}));
    assert_eq!(column(&p, "Product"), [json!("Car")]);
    let (_, p) = step_preview(&mut s, json!({"kind": "reverse_rows"}));
    assert_eq!(column(&p, "Qty"), [json!(9), json!(1), json!(2)]);
    let (_, p) = step_preview(&mut s, json!({"kind": "add_index", "start": 1}));
    assert_eq!(column(&p, "Index"), [json!(1), json!(2), json!(3)]);
    let (v, p) = step_preview(&mut s, json!({"kind": "duplicate_column", "column": "City"}));
    assert_eq!(v["step"], "Duplicated Column");
    assert_eq!(column(&p, "City - Copy")[1], json!("Lyon"));
    let (_, p) = step_preview(
        &mut s,
        json!({"kind": "merge_columns", "columns": ["Product", "City"], "delimiter": " @ "}),
    );
    assert_eq!(column(&p, "Merged")[0], json!("Bike @ Paris"));
    let (_, p) = step_preview(
        &mut s,
        json!({"kind": "conditional_column", "name": "Size", "rules": [
            {"column": "Qty", "op": "greater", "value": 5, "output": "big"}], "otherwise": "small"}),
    );
    assert_eq!(column(&p, "Size"), [json!("small"), json!("small"), json!("big")]);
    let (v, p) = step_preview(
        &mut s,
        json!({"kind": "column_op", "op": "upper", "columns": ["City"]}),
    );
    assert_eq!(v["step"], "Uppercased Text");
    assert!(
        v["expression"]
            .as_str()
            .unwrap()
            .contains("Table.TransformColumns(Kept,{{\"City\", Text.Upper, type text}})"),
        "{v}"
    );
    assert_eq!(column(&p, "City")[1], json!("LYON"));
    let (v, p) = step_preview(
        &mut s,
        json!({"kind": "column_op", "op": "first_chars", "count": 2, "columns": ["Product"], "add": true}),
    );
    assert_eq!(v["step"], "Inserted First Characters");
    assert_eq!(column(&p, "First Characters")[0], json!("Bi"));
    let (_, p) = step_preview(
        &mut s,
        json!({"kind": "column_op", "op": "multiply", "number": 10, "columns": ["Qty"]}),
    );
    assert_eq!(column(&p, "Qty")[2], json!(90));
    let (_, p) = step_preview(&mut s, json!({"kind": "fill_up", "columns": ["City"]}));
    assert_eq!(p["row_count"], 3);
    for bad in [
        json!({"kind": "remove_top_rows"}),
        json!({"kind": "remove_top_rows", "count": 0}),
        json!({"kind": "keep_range", "count": -1}),
        json!({"kind": "fill_down", "columns": []}),
        json!({"kind": "merge_columns", "columns": ["City"]}),
        json!({"kind": "conditional_column", "rules": []}),
        json!({"kind": "conditional_column", "rules": [{"column": "Qty", "op": "bad", "output": 1}]}),
        json!({"kind": "column_op", "op": "prefix", "columns": ["City"]}),
        json!({"kind": "column_op", "op": "upper", "columns": ["City", "Product"], "add": true}),
        json!({"kind": "column_op", "op": "nope", "columns": ["City"]}),
        json!({"kind": "column_op", "op": "divide", "number": 0, "columns": ["Qty"]}),
        json!({"kind": "column_op", "op": "round", "count": 99, "columns": ["Qty"]}),
        json!({"kind": "reverse_rows", "extra": 1}),
    ] {
        assert!(
            s.execute("query.apply_step", &json!({"name": "Sales", "step": bad}))
                .is_err(),
            "{bad}"
        );
    }
}

#[test]
fn dates_filter_as_date_literals_and_previews_say_the_date_kind() {
    let mut s = session();
    let m = "let Source = #table(type table [Day = date, At = datetime], {{#date(2024, 1, 31), #datetime(2024, 1, 31, 0, 0, 0)}, {#date(2024, 2, 1), #datetime(2024, 2, 1, 8, 30, 0)}}) in Source";
    let p = run(&mut s, "query.preview", json!({"expression": m, "limit": 10}));
    assert_eq!(p["columns"][0]["date_kind"], "date", "{p}");
    assert_eq!(p["columns"][1]["date_kind"], "datetime", "{p}");
    let v = run(
        &mut s,
        "query.apply_step",
        json!({"expression": m, "step": {"kind": "filter_values", "column": "Day", "values": [{"date": "2024-02-01"}]}}),
    );
    assert!(
        v["expression"]
            .as_str()
            .unwrap()
            .contains("[Day] = #date(2024, 2, 1)"),
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
        json!({"expression": m, "step": {"kind": "filter_rows", "column": "At", "op": "greater", "value": {"datetime": "2024-01-31T00:00:00"}}}),
    );
    let p = run(
        &mut s,
        "query.preview",
        json!({"expression": v["expression"], "limit": 10}),
    );
    assert_eq!(p["row_count"], 1);
    assert!(
        s.execute(
            "query.apply_step",
            &json!({"expression": m, "step": {"kind": "filter_values", "column": "Day", "values": [{"date": "2024-02-30"}]}})
        )
        .is_err()
    );
}

/// Each transform that asks values reads back from its step as the settings that wrote it, and
/// the gear's new settings write the step anew over the step it read.
#[test]
fn step_settings_read_back_and_rewrite_the_step() {
    let mut s = session();
    let cases = [
        json!({"kind": "filter_rows", "column": "City", "op": "equals", "value": "Paris"}),
        json!({"kind": "filter_rows", "column": "Qty", "op": "greater_or_equal", "value": 2.0}),
        json!({"kind": "filter_rows", "column": "City", "op": "not_contains", "value": "y"}),
        json!({"kind": "filter_rows", "column": "City", "op": "is_null"}),
        json!({"kind": "filter_rows", "column": "City", "op": "equals", "value": {"date": "2024-01-31"}}),
        json!({"kind": "replace_values", "columns": ["City"], "find": "Paris", "replace": "Lutèce"}),
        json!({"kind": "group_by", "keys": ["City"], "aggregations": [
            {"name": "Count", "op": "count"}, {"name": "Total", "op": "sum", "column": "Qty"}]}),
        json!({"kind": "merge", "other": "Sales", "keys": ["City"], "other_keys": ["City"],
            "join": "inner", "column": "Other"}),
        json!({"kind": "split_column", "column": "City", "delimiter": "a", "parts": 3}),
        json!({"kind": "remove_top_rows", "count": 2}),
        json!({"kind": "remove_bottom_rows", "count": 1}),
        json!({"kind": "keep_top_rows", "count": 5}),
        json!({"kind": "keep_bottom_rows", "count": 5}),
        json!({"kind": "keep_range", "first": 2, "count": 4}),
        json!({"kind": "remove_alternate_rows", "first": 2, "remove": 1, "keep": 3}),
        json!({"kind": "replace_errors", "columns": ["Qty", "City"], "value": 0.0}),
        json!({"kind": "add_index", "name": "Index", "start": 1, "step": 2}),
        json!({"kind": "merge_columns", "columns": ["Product", "City"], "delimiter": "-", "name": "M", "add": false}),
        json!({"kind": "merge_columns", "columns": ["Product", "City"], "delimiter": " ", "name": "M", "add": true}),
        json!({"kind": "conditional_column", "name": "Size", "rules": [
            {"column": "Qty", "op": "greater", "value": 5.0, "output": "big"},
            {"column": "City", "op": "is_null", "output": null}], "otherwise": "small"}),
        json!({"kind": "column_op", "op": "prefix", "text": "x-", "columns": ["City", "Product"], "add": false}),
        json!({"kind": "column_op", "op": "suffix", "text": "!", "columns": ["City"], "add": true, "name": "S"}),
        json!({"kind": "column_op", "op": "first_chars", "count": 2, "columns": ["City"], "add": false}),
        json!({"kind": "column_op", "op": "text_range", "start": 1, "count": 2, "columns": ["City"], "add": false}),
        json!({"kind": "column_op", "op": "before_delimiter", "text": ",", "columns": ["City"], "add": true, "name": "B"}),
        json!({"kind": "column_op", "op": "round", "count": 2, "columns": ["Qty"], "add": false}),
        json!({"kind": "column_op", "op": "multiply", "number": 1.5, "columns": ["Qty"], "add": false}),
        json!({"kind": "column_op", "op": "subtract", "number": -3.0, "columns": ["Qty"], "add": true, "name": "D"}),
    ];
    for step in cases {
        let v = run(&mut s, "query.apply_step", json!({"name": "Sales", "step": step}));
        let added = v["steps"].as_array().unwrap().last().unwrap().clone();
        assert_eq!(added["settings"], step, "{}", added["expression"]);
    }
    // steps BIcraft has no settings for: edited as M
    let v = run(&mut s, "query.steps", json!({"name": "Sales"}));
    assert_eq!(v["steps"][0]["settings"], Value::Null);
    let v = run(
        &mut s,
        "query.apply_step",
        json!({"name": "Sales", "step": {"kind": "column_op", "op": "upper", "columns": ["City"]}}),
    );
    assert_eq!(v["steps"][2]["settings"], Value::Null);
    // the gear: Kept's filter becomes another, still reading Source; the next step follows
    let v = run(
        &mut s,
        "query.apply_step",
        json!({"name": "Sales", "step": {"kind": "remove_top_rows", "count": 1}}),
    );
    let m = v["expression"].clone();
    let v = run(
        &mut s,
        "query.edit_step",
        json!({"expression": m, "step": "Kept", "settings": {"kind": "filter_rows", "column": "City", "op": "equals", "value": "Paris"}}),
    );
    assert!(
        v["expression"]
            .as_str()
            .unwrap()
            .contains("Kept = Table.SelectRows(Source, each [City] = \"Paris\")"),
        "{v}"
    );
    let p = run(&mut s, "query.preview", json!({"expression": v["expression"]}));
    assert_eq!(p["row_count"], 1, "Paris twice, then the top row removed");
    for bad in [
        json!({"expression": m, "step": "Kept", "settings": {"kind": "nope"}}),
        json!({"expression": m, "step": "Kept", "settings": {"kind": "remove_top_rows", "count": 1}, "formula": "1"}),
        json!({"expression": m, "step": "Source", "settings": {"kind": "remove_top_rows", "count": 1}}),
    ] {
        assert!(s.execute("query.edit_step", &bad).is_err(), "{bad}");
    }
}

#[test]
fn previews_read_the_other_queries_drafts() {
    let mut s = session();
    // the model's Sales, then the editor's draft of it
    let m = "let Source = Table.RowCount(Sales) in #table({\"n\"}, {{Source}})";
    let p = run(&mut s, "query.preview", json!({"expression": m}));
    assert_eq!(p["rows"], json!([[3]]));
    let draft = "let Source = #table({\"Product\"}, {{\"Bike\"}}) in Source";
    let p = run(
        &mut s,
        "query.preview",
        json!({"expression": m, "queries": [{"name": "Sales", "expression": draft}]}),
    );
    assert_eq!(p["rows"], json!([[1]]));
    // a new query of the editor, not in the model yet
    let q = "let Source = Query1 in Source";
    let p = run(
        &mut s,
        "query.preview",
        json!({"expression": q, "queries": [{"name": "Query1", "expression": draft}]}),
    );
    assert_eq!(p["rows"], json!([["Bike"]]));
    let p = run(
        &mut s,
        "query.profile",
        json!({"expression": q, "queries": [{"name": "Query1", "expression": draft}]}),
    );
    assert_eq!(p["rows"], 1);
    for bad in [
        json!({"expression": m, "queries": "Sales"}),
        json!({"expression": m, "queries": [{"name": "Sales"}]}),
        json!({"expression": m, "queries": [{"name": "Sales", "expression": draft, "x": 1}]}),
    ] {
        assert!(s.execute("query.preview", &bad).is_err(), "{bad}");
    }
}

#[test]
fn number_date_duration_and_text_operations_evaluate_and_read_back() {
    let mut s = session();
    let (v, p) = step_preview(
        &mut s,
        json!({"kind": "column_op", "op": "integer_divide", "number": 2, "columns": ["Qty"]}),
    );
    assert_eq!(v["step"], "Integer-Divided Column");
    assert_eq!(column(&p, "Qty"), [json!(1), json!(0), json!(4)]);
    let (_, p) = step_preview(
        &mut s,
        json!({"kind": "column_op", "op": "is_odd", "columns": ["Qty"], "add": true}),
    );
    assert_eq!(column(&p, "Is Odd"), [json!(false), json!(true), json!(true)]);
    let (_, p) = step_preview(
        &mut s,
        json!({"kind": "column_op", "op": "factorial", "columns": ["Qty"], "add": true, "name": "F"}),
    );
    assert_eq!(column(&p, "F")[0], json!(2));
    let (_, p) = step_preview(
        &mut s,
        json!({"kind": "column_op", "op": "between_delimiters", "text": "B", "end": "e",
            "columns": ["Product"], "add": true}),
    );
    assert_eq!(column(&p, "Text Between Delimiters")[0], json!("ik"));
    // the gear reads the operations with a value back
    for step in [
        json!({"kind": "column_op", "op": "integer_divide", "number": 2.0, "columns": ["Qty"], "add": false}),
        json!({"kind": "column_op", "op": "modulo", "number": 3.0, "columns": ["Qty"], "add": true, "name": "M"}),
        json!({"kind": "column_op", "op": "power", "number": 2.0, "columns": ["Qty"], "add": false}),
        json!({"kind": "column_op", "op": "between_delimiters", "text": "[", "end": "]",
            "columns": ["City"], "add": false}),
    ] {
        let v = run(&mut s, "query.apply_step", json!({"name": "Sales", "step": step}));
        let added = v["steps"].as_array().unwrap().last().unwrap().clone();
        assert_eq!(added["settings"], step, "{}", added["expression"]);
    }
    for bad in [
        json!({"kind": "column_op", "op": "integer_divide", "number": 0, "columns": ["Qty"]}),
        json!({"kind": "column_op", "op": "modulo", "columns": ["Qty"]}),
        json!({"kind": "column_op", "op": "percent_of", "number": 0, "columns": ["Qty"]}),
        json!({"kind": "column_op", "op": "power", "number": "2", "columns": ["Qty"]}),
        json!({"kind": "column_op", "op": "between_delimiters", "text": "[", "columns": ["City"]}),
        json!({"kind": "column_op", "op": "between_delimiters", "text": "[", "end": 5, "columns": ["City"]}),
        json!({"kind": "column_op", "op": "age", "columns": []}),
    ] {
        assert!(
            s.execute("query.apply_step", &json!({"name": "Sales", "step": bad}))
                .is_err(),
            "{bad}"
        );
    }
}
