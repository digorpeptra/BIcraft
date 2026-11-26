//! `parameter.what_if` and `parameter.fields` (Modeling › New parameter): the tables, columns,
//! measure and metadata they add, their slicer, one undo step, refusals that change nothing, and
//! the preview of a visual bound to a field parameter following its slicer.

use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Sales(Product, Amount) with [Total] and [Lines], and a report of one page.
fn shop() -> Session {
    let mut s = Session::new();
    let sales = r#"#table({"Product", "Amount"}, {{"Bike", 100}, {"Bike", 50}, {"Car", 1000}, {"Ball", 5}})"#;
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Product", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Lines", "expression": "COUNTROWS(Sales)"}),
        ),
        ("report.new", json!({})),
    ] {
        run(&mut s, id, p);
    }
    s
}

fn page(s: &Session) -> String {
    s.report.as_ref().unwrap().pages[0].name.clone()
}

fn add_visual(s: &mut Session, visual_type: &str, fields: Value) -> String {
    let page = page(s);
    let r = run(
        s,
        "visual.add",
        json!({"page": page, "visual_type": visual_type, "x": 500, "y": 300, "width": 300, "height": 200,
               "fields": fields}),
    );
    r["visual"].as_str().unwrap().to_string()
}

fn visual_json(snap: &Value, visual: &str) -> Value {
    for p in snap["report"]["pages"].as_array().cloned().unwrap_or_default() {
        for v in p["visuals"].as_array().cloned().unwrap_or_default() {
            if v["name"] == visual {
                return v;
            }
        }
    }
    panic!("visual {visual} not in the snapshot")
}

fn data(s: &Session, visual: &str) -> Value {
    visual_json(&s.snapshot(), visual)["data"].clone()
}

fn rows(s: &Session, visual: &str) -> Vec<Value> {
    let d = data(s, visual);
    assert_eq!(d["status"], "ok", "{d}");
    d["rows"].as_array().cloned().unwrap_or_default()
}

fn ext_prop(c: &bicraft_model::Column) -> Value {
    c.extra["extendedProperties"][0].clone()
}

#[test]
fn what_if_adds_a_series_table_a_measure_and_a_slicer_in_one_undo_step() {
    let mut s = shop();
    let before = (s.model.clone(), s.report.clone());
    let r = run(
        &mut s,
        "parameter.what_if",
        json!({"name": "Discount", "min": 0, "max": 50, "increment": 5, "default": 10}),
    );
    assert_eq!(r["expression"], "GENERATESERIES(0, 50, 5)");
    assert_eq!(r["data_type"], "whole");
    assert_eq!(r["measure"]["name"], "Discount Value");
    assert_eq!(
        r["measure"]["expression"],
        "SELECTEDVALUE('Discount'[Discount], 10)"
    );
    let m = s.model.as_ref().unwrap();
    let t = m.table("Discount").unwrap();
    assert_eq!(t.partitions[0].source_type, "calculated");
    assert_eq!(t.columns.len(), 1);
    let c = &t.columns[0];
    assert_eq!(c.name, "Discount");
    assert_eq!(c.source_column.as_deref(), Some("[Value]"));
    assert_eq!(c.data_type, bicraft_model::DataType::Int64);
    assert_eq!(
        ext_prop(c),
        json!({"type": "json", "name": "ParameterMetadata", "value": {"version": 0}})
    );
    assert_eq!(t.measures[0].name, "Discount Value");
    // the slicer: on the page, bound to the column, a single-value slider
    let slicer = r["slicer"]["visual"].as_str().unwrap().to_string();
    let v = s.report.as_ref().unwrap().pages[0]
        .visuals
        .iter()
        .find(|v| v.name == slicer)
        .unwrap()
        .clone();
    assert_eq!(v.visual_type.as_deref(), Some("slicer"));
    assert_eq!(
        v.visual_extra["objects"]["data"][0]["properties"]["mode"]["expr"]["Literal"]["Value"],
        "'Single'"
    );
    // the series evaluates: 11 values
    assert_eq!(rows(&s, &slicer).len(), 11);
    // one undo step takes the table, the measure and the slicer away
    run(&mut s, "edit.undo", json!({}));
    assert_eq!((s.model.clone(), s.report.clone()), before);
}

#[test]
fn what_if_measure_reads_the_slicer_value_else_its_default() {
    let mut s = shop();
    let r = run(
        &mut s,
        "parameter.what_if",
        json!({"name": "Rate", "min": 0.5, "max": 2, "increment": 0.5, "default": 1}),
    );
    assert_eq!(r["data_type"], "decimal");
    assert_eq!(r["expression"], "GENERATESERIES(0.5, 2, 0.5)");
    let slicer = r["slicer"]["visual"].as_str().unwrap().to_string();
    let card = add_visual(
        &mut s,
        "card",
        json!([{"role": "Values", "field": "[Rate Value]"}]),
    );
    assert_eq!(rows(&s, &card), vec![json!([1])]);
    let page = page(&s);
    run(
        &mut s,
        "view.interact",
        json!({"action": "slicer_toggle", "page": page, "visual": slicer, "value": 1.5}),
    );
    assert_eq!(rows(&s, &card), vec![json!([1.5])]);
}

#[test]
fn what_if_refuses_bad_ranges_and_taken_names_and_changes_nothing() {
    let mut s = shop();
    let before = (s.model.clone(), s.report.clone(), s.revision);
    for p in [
        json!({"name": "P", "min": 10, "max": 0, "increment": 1}),
        json!({"name": "P", "min": 0, "max": 10, "increment": 0}),
        json!({"name": "P", "min": 0, "max": 10, "increment": -1}),
        json!({"name": "P", "min": 0, "max": 1e12, "increment": 1}),
        json!({"name": "P", "min": 0, "max": 1, "increment": 0.5, "data_type": "whole"}),
        json!({"name": "P", "min": 0, "max": 1, "increment": 1, "data_type": "money"}),
        json!({"name": "Sales", "min": 0, "max": 1, "increment": 1}),
        json!({"name": "Total", "min": 0, "max": 1, "increment": 1, "page": "nowhere"}),
        json!({"name": " ", "min": 0, "max": 1, "increment": 1}),
        json!({"name": "P", "min": "0", "max": 1, "increment": 1}),
    ] {
        assert!(s.execute("parameter.what_if", &p).is_err(), "{p}");
        assert_eq!(
            (s.model.clone(), s.report.clone()),
            (before.0.clone(), before.1.clone()),
            "{p}"
        );
    }
    // a measure name taken elsewhere: `Lines Value`
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Lines Value", "expression": "1"}),
    );
    let e = s
        .execute(
            "parameter.what_if",
            &json!({"name": "Lines", "min": 0, "max": 1, "increment": 1}),
        )
        .unwrap_err();
    assert!(e.contains("Lines Value"), "{e}");
    assert!(s.model.as_ref().unwrap().table("Lines").is_none());
}

#[test]
fn what_if_without_a_report_adds_no_slicer_and_add_slicer_needs_one() {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "M"}));
    let r = run(
        &mut s,
        "parameter.what_if",
        json!({"name": "N", "min": 1, "max": 3, "increment": 1}),
    );
    assert!(r.get("slicer").is_none(), "{r}");
    assert_eq!(r["measure"]["expression"], "SELECTEDVALUE('N'[N])");
    assert!(
        s.execute(
            "parameter.what_if",
            &json!({"name": "O", "min": 1, "max": 3, "increment": 1, "add_slicer": true})
        )
        .is_err()
    );
}

#[test]
fn fields_adds_the_tuple_table_its_columns_metadata_and_slicer() {
    let mut s = shop();
    let before = (s.model.clone(), s.report.clone());
    let r = run(
        &mut s,
        "parameter.fields",
        json!({"name": "Metric", "fields": ["[Total]", {"field": "Sales[Product]", "label": "Item \"name\""}, "Sales[Lines]"]}),
    );
    assert_eq!(
        r["expression"],
        "{\n    (\"Total\", NAMEOF('Sales'[Total]), 0),\n    (\"Item \"\"name\"\"\", NAMEOF('Sales'[Product]), 1),\n    (\"Lines\", NAMEOF('Sales'[Lines]), 2)\n}"
    );
    assert_eq!(r["columns"], json!(["Metric", "Metric Fields", "Metric Order"]));
    let t = s.model.as_ref().unwrap().table("Metric").unwrap().clone();
    let [label, fields, order] = [&t.columns[0], &t.columns[1], &t.columns[2]];
    assert_eq!(label.source_column.as_deref(), Some("[Value1]"));
    assert!(!label.is_hidden);
    assert_eq!(
        label.extra["relatedColumnDetails"],
        json!({"groupByColumns": [{"groupingColumn": "Metric Fields"}]})
    );
    assert_eq!(label.sort_by_column.as_deref(), Some("Metric Order"));
    assert!(fields.is_hidden && order.is_hidden);
    assert_eq!(
        ext_prop(fields),
        json!({"type": "json", "name": "ParameterMetadata", "value": {"version": 3, "kind": 2}})
    );
    assert_eq!(order.data_type, bicraft_model::DataType::Int64);
    // the slicer lists the labels in order
    let slicer = r["slicer"]["visual"].as_str().unwrap().to_string();
    let listed: Vec<Value> = rows(&s, &slicer).into_iter().map(|r| r[0].clone()).collect();
    assert_eq!(
        listed,
        vec![json!("Total"), json!("Item \"name\""), json!("Lines")]
    );
    run(&mut s, "edit.undo", json!({}));
    assert_eq!((s.model.clone(), s.report.clone()), before);
}

#[test]
fn fields_refuses_aggregations_unknown_and_repeated_fields() {
    let mut s = shop();
    let before = (s.model.clone(), s.report.clone());
    for p in [
        json!({"name": "P", "fields": []}),
        json!({"name": "P", "fields": "Sales[Product]"}),
        json!({"name": "P", "fields": ["Sum(Sales[Amount])"]}),
        json!({"name": "P", "fields": ["Sales[Nope]"]}),
        json!({"name": "P", "fields": ["[Total]", "Sales[Total]"]}),
        json!({"name": "P", "fields": [{"field": "[Total]", "label": "a\nb"}]}),
        json!({"name": "P", "fields": [{"field": "[Total]", "extra": 1}]}),
        json!({"name": "P", "fields": [3]}),
        json!({"name": "Sales", "fields": ["[Total]"]}),
        json!({"name": "P", "fields": ["[Total]"], "page": "nowhere"}),
    ] {
        assert!(s.execute("parameter.fields", &p).is_err(), "{p}");
        assert_eq!((s.model.clone(), s.report.clone()), before, "{p}");
    }
}

#[test]
fn a_visual_bound_to_a_field_parameter_shows_the_fields_its_slicer_chooses() {
    let mut s = shop();
    let r = run(
        &mut s,
        "parameter.fields",
        json!({"name": "Metric", "fields": ["[Total]", "[Lines]"]}),
    );
    let slicer = r["slicer"]["visual"].as_str().unwrap().to_string();
    let table = add_visual(
        &mut s,
        "tableEx",
        json!([{"role": "Values", "field": "Sales[Product]"}, {"role": "Values", "field": "Metric[Metric]"}]),
    );
    // nothing chosen: every field of the parameter, in its order
    assert_eq!(
        rows(&s, &table),
        vec![
            json!(["Ball", 5, 1]),
            json!(["Bike", 150, 2]),
            json!(["Car", 1000, 1])
        ]
    );
    let page = page(&s);
    run(
        &mut s,
        "view.interact",
        json!({"action": "slicer_toggle", "page": page, "visual": slicer, "value": "Lines"}),
    );
    assert_eq!(
        rows(&s, &table),
        vec![json!(["Ball", 1]), json!(["Bike", 2]), json!(["Car", 1])]
    );
    // both chosen again (Ctrl+click): both, still in the parameter's order
    run(
        &mut s,
        "view.interact",
        json!({"action": "slicer_toggle", "page": page, "visual": slicer, "value": "Total", "ctrl": true}),
    );
    assert_eq!(rows(&s, &table)[0], json!(["Ball", 5, 1]));
    // a card bound to the parameter alone
    let card = add_visual(
        &mut s,
        "card",
        json!([{"role": "Values", "field": "Metric[Metric]"}]),
    );
    run(
        &mut s,
        "view.interact",
        json!({"action": "slicer_toggle", "page": page, "visual": slicer, "value": "Total"}),
    );
    assert_eq!(rows(&s, &card), vec![json!([1155])]);
}

/// The saved `visual.json` of visual `name` under `dir`.
fn saved_visual(dir: &std::path::Path, name: &str) -> Value {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else if p.file_name().is_some_and(|f| f == "visual.json") {
                let v: Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
                if v["name"] == name {
                    return v;
                }
            }
        }
    }
    panic!("no visual.json for {name}")
}

/// G64: Power BI Desktop 2.159.1231.0 shows a visual bound only to a field parameter's label
/// column as plain text; it needs the parameter's fields as the role's projections and the
/// role's `fieldParameters` entry, which BIcraft now writes as Desktop saves them. The file reads
/// back as one binding of the label column, so the preview still follows the slicer.
#[test]
fn a_visual_bound_to_a_field_parameter_saves_desktops_field_parameters() {
    let dir = TempDir::new("bicraft-params-fp");
    let mut s = shop();
    let r = run(
        &mut s,
        "parameter.fields",
        json!({"name": "Metric", "fields": ["[Total]", {"field": "[Lines]", "label": "Rows"}]}),
    );
    let slicer = r["slicer"]["visual"].as_str().unwrap().to_string();
    let table = add_visual(
        &mut s,
        "tableEx",
        json!([{"role": "Values", "field": "Sales[Product]"}, {"role": "Values", "field": "Metric[Metric]"}]),
    );
    let out = dir.path().join("Shop.pbip");
    run(&mut s, "file.save_as", json!({"path": out.to_string_lossy()}));
    let measure =
        |m: &str| json!({"Measure": {"Expression": {"SourceRef": {"Entity": "Sales"}}, "Property": m}});
    let values = saved_visual(dir.path(), &table)["visual"]["query"]["queryState"]["Values"].clone();
    assert_eq!(
        values["fieldParameters"],
        json!([{"parameterExpr": {"Column": {"Expression": {"SourceRef": {"Entity": "Metric"}}, "Property": "Metric"}},
                "index": 1, "length": 2}]),
        "{values}"
    );
    assert_eq!(
        values["projections"][1],
        json!({"field": measure("Total"), "queryRef": "Sales.Total", "nativeQueryRef": "Total", "displayName": "Total"})
    );
    assert_eq!(values["projections"][2]["field"], measure("Lines"));
    assert_eq!(values["projections"][2]["displayName"], "Rows");
    // the slicer lists the labels: no fieldParameters there
    let sv = saved_visual(dir.path(), &slicer);
    assert!(!sv.to_string().contains("fieldParameters"), "{sv}");
    // read back: one binding of the label column, and the preview follows the slicer
    let mut t = Session::new();
    run(&mut t, "file.open", json!({"path": out.to_string_lossy()}));
    let v = t.report.as_ref().unwrap().pages[0]
        .visuals
        .iter()
        .find(|v| v.name == table)
        .unwrap()
        .clone();
    assert_eq!(v.fields.len(), 2);
    assert_eq!(
        v.fields[1].field,
        bicraft_report::FieldRef::Column {
            table: "Metric".into(),
            column: "Metric".into()
        }
    );
    let page = page(&t);
    run(
        &mut t,
        "view.interact",
        json!({"action": "slicer_toggle", "page": page, "visual": slicer, "value": "Rows"}),
    );
    assert_eq!(
        rows(&t, &table),
        vec![json!(["Ball", 1]), json!(["Bike", 2]), json!(["Car", 1])]
    );
    // a later edit of the parameter's fields follows into the visual
    run(
        &mut t,
        "partition.set",
        json!({"table": "Metric", "expression": "{(\"Rows\", NAMEOF('Sales'[Lines]), 0)}"}),
    );
    let q = t.report.as_ref().unwrap().pages[0]
        .visuals
        .iter()
        .find(|v| v.name == table)
        .unwrap()
        .visual_extra["query"]
        .clone();
    let values = &q["queryState"]["Values"];
    assert_eq!(values["fieldParameters"][0]["length"], 1, "{values}");
    assert_eq!(values["projections"][1]["field"], measure("Lines"));
}

#[test]
fn a_field_parameter_survives_a_save_and_reopen() {
    let dir = TempDir::new("bicraft-params");
    let mut s = shop();
    run(
        &mut s,
        "parameter.fields",
        json!({"name": "Metric", "fields": ["[Total]", "Sales[Product]"]}),
    );
    run(
        &mut s,
        "parameter.what_if",
        json!({"name": "Discount", "min": 0, "max": 10, "increment": 1}),
    );
    let out = dir.path().join("Shop.pbip");
    run(&mut s, "file.save_as", json!({"path": out.to_string_lossy()}));
    let mut t = Session::new();
    run(&mut t, "file.open", json!({"path": out.to_string_lossy()}));
    let m = t.model.as_ref().unwrap();
    let metric = m.table("Metric").unwrap();
    assert_eq!(
        ext_prop(&metric.columns[1]),
        json!({"type": "json", "name": "ParameterMetadata", "value": {"version": 3, "kind": 2}})
    );
    assert_eq!(
        metric.columns[0].extra["relatedColumnDetails"],
        json!({"groupByColumns": [{"groupingColumn": "Metric Fields"}]})
    );
    let discount = m.table("Discount").unwrap();
    assert_eq!(
        ext_prop(&discount.columns[0]),
        json!({"type": "json", "name": "ParameterMetadata", "value": {"version": 0}})
    );
}
