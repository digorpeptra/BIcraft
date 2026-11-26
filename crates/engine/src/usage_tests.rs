//! The usage tests that need the engine's private items (`usage::glob_for_tests`, `report_measures::EXTENSIONS_PATH`); the others are the `report`
//! integration test (`tests/report/usage_tests.rs`), under the same module path.

use bicraft_report::{FieldBinding, FieldRef, Page, Report, Visual};
use serde_json::{Value, json};

use crate::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Sales(Amount, Cost, Spare, ProductKey, Date) → Product(ProductKey, Name, SortOrder); measures
/// [Total], [Margin], [Margin %] (in a visual), [Old] → [Total], [Older] → [Old], [Orphan].
fn session() -> Session {
    let mut s = Session::new();
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": "let x = 1 in x"}}),
        ),
        ("table.add", json!({"name": "Product"})),
        (
            "column.add",
            json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Cost", "data_type": "double"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Spare", "data_type": "double"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "ProductKey", "data_type": "int64"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Date", "data_type": "dateTime"}),
        ),
        (
            "column.add",
            json!({"table": "Product", "name": "ProductKey", "data_type": "int64"}),
        ),
        (
            "column.add",
            json!({"table": "Product", "name": "Name", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Product", "name": "SortOrder", "data_type": "int64"}),
        ),
        (
            "column.set",
            json!({"table": "Product", "name": "Name", "sort_by_column": "SortOrder"}),
        ),
        (
            "relationship.add",
            json!({"from": "Sales[ProductKey]", "to": "Product[ProductKey]", "name": "Sales_Product"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Margin", "expression": "[Total] - SUM(Sales[Cost])"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Margin %", "expression": "DIVIDE([Margin], [Total])"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Old", "expression": "[Total] * 2"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Older", "expression": "[Old] + 1"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Orphan", "expression": "1"}),
        ),
    ] {
        run(&mut s, id, p);
    }
    s.report = Some(Report {
        pages: vec![Page {
            name: "p1".into(),
            display_name: "Overview".into(),
            visuals: vec![Visual {
                name: "v1".into(),
                visual_type: Some("card".into()),
                fields: vec![FieldBinding {
                    display_name: None,
                    show_as: None,
                    role: "Values".into(),
                    field: FieldRef::Measure {
                        table: "Sales".into(),
                        measure: "Margin %".into(),
                    },
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    });
    s
}

fn names(list: &Value) -> Vec<String> {
    list.as_array()
        .into_iter()
        .flatten()
        .map(|e| match e.get("table") {
            Some(t) if e["kind"] != "table" => {
                format!("{}[{}]", t.as_str().unwrap(), e["name"].as_str().unwrap())
            }
            _ => e["name"].as_str().unwrap().to_string(),
        })
        .collect()
}

#[test]
fn allow_list_and_report_measures() {
    let mut s = session();
    let v = run(
        &mut s,
        "model.unused",
        json!({"allow": ["[Old*]", "Sales[spare]", "[Nothing]"]}),
    );
    assert_eq!(names(&v["unused"]["measures"]), ["Sales[Orphan]"]);
    assert!(!names(&v["unused"]["columns"]).contains(&"Sales[Spare]".to_string()));
    assert!(
        v["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap().contains("[Nothing]"))
    );
    assert!(s.execute("model.unused", &json!({"allow": [1]})).is_err());

    // a report-level measure keeps what it names
    let doc = json!({"name": "extension", "entities": [{"name": "Sales", "measures": [
        {"name": "Local", "dataType": "Double", "expression": "[Orphan] + SUM(Sales[Spare])"}]}]});
    s.report
        .as_mut()
        .unwrap()
        .resources
        .push(bicraft_report::Resource {
            path: crate::report_measures::EXTENSIONS_PATH.into(),
            bytes: serde_json::to_vec(&doc).unwrap(),
        });
    let v = run(&mut s, "model.unused", json!({}));
    assert_eq!(names(&v["unused"]["measures"]), ["Sales[Old]", "Sales[Older]"]);
    let u = run(&mut s, "model.usage", json!({"object": "Sales[Spare]"}));
    assert_eq!(u["direct"][0]["kind"], "report measure");
    assert_eq!(u["direct"][0]["name"], "Local");
    assert_eq!(v["reports"][0]["report_measures"], 1);
}

#[test]
fn glob_patterns() {
    use crate::usage::glob_for_tests as glob;
    assert!(glob("[old*]", "[Older]"));
    assert!(glob("*", ""));
    assert!(glob("S?les[*]", "Sales[Amount]"));
    assert!(!glob("[Old]", "[Older]"));
    assert!(glob("*a*b*c*", "xxaxxbxxcxx"));
    assert!(!glob("*a*b*c*d", "xxaxxbxxcxx"));
}
