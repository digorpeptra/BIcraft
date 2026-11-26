//! The report tests that need the engine's private items (`refs::field_text`); the others are
//! the `report` integration test (`tests/report/report_tests.rs`), under the same module path.

use bicraft_report::{FieldRef, Report};
use serde_json::{Value, json};

use crate::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Sales(Amount, Product, Date) with [Total], Product(Product, Name), 'Date Table'(Date), and a
/// blank report.
fn session() -> Session {
    let mut s = Session::new();
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        ("table.add", json!({"name": "Sales"})),
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
        ("report.new", json!({})),
    ] {
        run(&mut s, id, p);
    }
    s
}

fn report(s: &Session) -> &Report {
    s.report.as_ref().unwrap()
}

/// Adds a column chart with Category = Product[Name], Y = [Total]; returns (page, visual).
fn chart(s: &mut Session) -> (String, String) {
    let page = first_page(s);
    let v = run(
        s,
        "visual.add",
        json!({"page": page, "visual_type": "clusteredColumnChart", "x": 10, "y": 20, "width": 400,
               "height": 300, "fields": [{"role": "Category", "field": "Product[Name]"},
                                         {"role": "Y", "field": "[Total]"}]}),
    );
    (page, v["visual"].as_str().unwrap().to_string())
}

/// Errors `report.validate` finds in fields and structure (schema issues aside).
fn field_errors(s: &mut Session) -> usize {
    let v = run(s, "report.validate", json!({}));
    v["issues"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["severity"] == "error")
        .count()
}

#[test]
fn model_renames_report_and_optionally_rewrite_bindings() {
    let mut s = session();
    let (_, visual) = chart(&mut s);
    run(
        &mut s,
        "visual.bind",
        json!({"page": "Page 1", "visual": visual, "role": "Y", "field": "Sum(Sales[Amount])"}),
    );
    s.report.as_mut().unwrap().pages[0].filter_config = Some(json!({"filters": [{"filter": {
        "From": [{"Name": "p", "Entity": "Product"}],
        "Where": [{"Condition": {"In": {"Expressions": [{"Column": {"Expression": {"SourceRef": {"Source": "p"}}, "Property": "Name"}}]}}}]}}]}));
    s.report_dirty = false;

    // Without update_report: reported, not rewritten.
    let v = run(
        &mut s,
        "table.rename",
        json!({"name": "Product", "new_name": "Item"}),
    );
    assert_eq!(v["report"]["affected_binding_count"], 1, "{v}");
    assert_eq!(v["report"]["affected_bindings"][0]["field"], "Product[Name]");
    assert_eq!(v["report"]["other_reference_count"], 1);
    assert_eq!(v["report"]["updated"], false);
    assert!(v["report"]["hint"].as_str().unwrap().contains("update_report"));
    assert!(!s.report_dirty);
    // The binding and the page filter.
    assert_eq!(field_errors(&mut s), 2);

    // Rename back, then rename with update_report.
    run(
        &mut s,
        "table.rename",
        json!({"name": "Item", "new_name": "Product"}),
    );
    let v = run(
        &mut s,
        "table.rename",
        json!({"name": "Product", "new_name": "Item", "update_report": true}),
    );
    assert_eq!(v["report"]["updated"], true);
    assert!(s.report_dirty);
    let r = report(&s);
    assert_eq!(
        r.pages[0].visuals[0].fields[0].field,
        FieldRef::Column {
            table: "Item".into(),
            column: "Name".into()
        }
    );
    assert_eq!(
        r.pages[0].filter_config.as_ref().unwrap()["filters"][0]["filter"]["From"][0]["Entity"],
        "Item"
    );
    assert_eq!(field_errors(&mut s), 0);

    let v = run(
        &mut s,
        "column.set",
        json!({"table": "Item", "name": "Name", "new_name": "Label", "update_report": true}),
    );
    assert_eq!(v["report"]["affected_binding_count"], 1);
    assert_eq!(v["report"]["other_reference_count"], 1);
    let v = run(
        &mut s,
        "column.set",
        json!({"table": "Sales", "name": "Amount", "new_name": "Value", "update_report": true}),
    );
    assert_eq!(v["report"]["affected_bindings"][0]["field"], "Sum(Sales[Amount])");
    let v = run(
        &mut s,
        "measure.set",
        json!({"name": "Total", "new_name": "Revenue", "expression": "SUM(Sales[Value])", "update_report": true}),
    );
    assert_eq!(v["report"]["affected_binding_count"], 1);
    let fields: Vec<String> = report(&s).pages[0].visuals[0]
        .fields
        .iter()
        .map(|b| crate::refs::field_text(&b.field))
        .collect();
    assert_eq!(fields, ["Item[Label]", "[Revenue]", "Sum(Sales[Value])"]);
    assert_eq!(
        report(&s).pages[0].filter_config.as_ref().unwrap()["filters"][0]["filter"]["Where"][0]["Condition"]
            ["In"]["Expressions"][0]["Column"]["Property"],
        "Label"
    );
    assert_eq!(field_errors(&mut s), 0);

    // Moving a measure moves its bindings with update_report.
    let v = run(
        &mut s,
        "measure.move",
        json!({"name": "Revenue", "to_table": "Item", "update_report": true}),
    );
    assert_eq!(v["report"]["affected_binding_count"], 1);
    assert_eq!(
        report(&s).pages[0].visuals[0].fields[1].field,
        FieldRef::Measure {
            table: "Item".into(),
            measure: "Revenue".into()
        }
    );
    assert_eq!(field_errors(&mut s), 0);

    // An edit that touches no binding reports an empty list.
    let v = run(
        &mut s,
        "table.rename",
        json!({"name": "Date Table", "new_name": "Calendar"}),
    );
    assert_eq!(v["report"]["affected_binding_count"], 0);
    assert!(v["report"].get("hint").is_none());
}

fn first_page(s: &Session) -> String {
    report(s).pages[0].name.clone()
}
