//! Tests of the data preview (`visual.data`), on synthetic models with literal tables.

use serde_json::{Value, json};

use super::store::{ColData, Key};
use super::{Ir, Op, Out, eval, ole_to_iso};
use crate::Session;
use bicraft_vertipaq::CellValue;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Sales(Product, Amount, Qty) and Product(Product, Category) as M `#table` literals, related
/// many-to-one, with measures of every supported shape and one unsupported.
fn shop() -> Session {
    let mut s = Session::new();
    let sales = r#"#table({"Product", "Amount", "Qty"}, {{"Bike", 100, 1}, {"Bike", 50.5, 2}, {"Car", 1000, 1}, {"Ball", 5, 10}, {"Kite", 7, 1}})"#;
    let product =
        r#"#table({"Product", "Category"}, {{"Bike", "Wheels"}, {"Car", "Wheels"}, {"Ball", "Toys"}})"#;
    let steps = [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
        ),
        (
            "table.add",
            json!({"name": "Product", "partition": {"source_type": "m", "expression": product}}),
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
            "column.add",
            json!({"table": "Sales", "name": "Qty", "data_type": "int64"}),
        ),
        (
            "column.add",
            json!({"table": "Product", "name": "Product", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Product", "name": "Category", "data_type": "string"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Lines", "expression": "COUNTROWS(Sales)"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Per Line", "expression": "DIVIDE([Total], [Lines])"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Calc", "expression": "VAR t = [Total] RETURN (t - 5) * 2 + MAX(Sales[Qty])"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Hard", "expression": "RAND()"}),
        ),
        (
            "relationship.add",
            json!({"from": "Sales[Product]", "to": "Product[Product]"}),
        ),
        ("report.new", json!({})),
        ("page.add", json!({"display_name": "Main"})),
    ];
    for (id, p) in steps {
        run(&mut s, id, p);
    }
    s
}

fn add_visual(s: &mut Session, visual_type: &str, fields: Value) -> String {
    let r = run(
        s,
        "visual.add",
        json!({"page": "Main", "visual_type": visual_type, "x": 0, "y": 0, "width": 100, "height": 100,
               "fields": fields}),
    );
    r.get("visual")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| panic!("no visual name in {r}"))
}

/// The `data` of a visual in a fresh snapshot.
fn data(s: &Session, visual: &str) -> Value {
    find(&s.snapshot(), visual)
}

/// The `data` of a visual in `snap`.
fn find(snap: &Value, visual: &str) -> Value {
    let pages = snap["report"]["pages"].as_array().cloned().unwrap_or_default();
    for p in pages {
        for v in p["visuals"].as_array().cloned().unwrap_or_default() {
            if v["name"] == visual {
                return v["data"].clone();
            }
        }
    }
    panic!("visual {visual} not in the snapshot")
}

fn rows(d: &Value) -> Vec<Value> {
    assert_eq!(d["status"], "ok", "{d}");
    d["rows"].as_array().cloned().unwrap_or_default()
}

#[test]
fn card_with_a_simple_measure() {
    let mut s = shop();
    let v = add_visual(&mut s, "card", json!([{"role": "Values", "field": "[Total]"}]));
    let d = data(&s, &v);
    assert_eq!(rows(&d), vec![json!([1162.5])]);
    assert_eq!(d["columns"][0]["name"], "Total");
    assert_eq!(d["columns"][0]["kind"], "measure");
    assert_eq!(d["truncated"], false);
}

#[test]
fn group_by_a_column_of_the_same_table() {
    let mut s = shop();
    let v = add_visual(
        &mut s,
        "tableEx",
        json!([{"role": "Values", "field": "Sales[Product]"}, {"role": "Values", "field": "[Total]"},
               {"role": "Values", "field": "[Per Line]"}, {"role": "Values", "field": "Sum(Sales[Qty])"}]),
    );
    let d = data(&s, &v);
    // tables sort by their columns, ascending
    assert_eq!(
        rows(&d),
        vec![
            json!(["Ball", 5, 5, 10]),
            json!(["Bike", 150.5, 75.25, 3]),
            json!(["Car", 1000, 1000, 1]),
            json!(["Kite", 7, 7, 1]),
        ]
    );
    assert_eq!(d["columns"][3]["kind"], "aggregation");
    assert_eq!(d["columns"][0]["data_type"], "string");
}

/// Regression: the same aggregation in two roles of a scatter chart got a second queryRef
/// suffixed `1`, and Power BI Desktop 2.158 failed the visual's query. Both roles now share one
/// queryRef (one selected field), through `visual.add` and `visual.bind`.
#[test]
fn a_field_in_two_roles_shares_one_query_ref() {
    let mut s = shop();
    let v = add_visual(
        &mut s,
        "scatterChart",
        json!([{"role": "Category", "field": "Product[Category]"},
               {"role": "X", "field": "Sum(Sales[Amount])"},
               {"role": "Y", "field": "Sum(Sales[Qty])"},
               {"role": "Size", "field": "Sum(Sales[Qty])"}]),
    );
    assert_eq!(query_ref(&s, &v, 2), "Sum(Sales.Qty)");
    assert_eq!(query_ref(&s, &v, 3), "Sum(Sales.Qty)");
    run(
        &mut s,
        "visual.bind",
        json!({"page": "Main", "visual": v, "role": "Tooltips", "field": "Sum(Sales[Amount])"}),
    );
    assert_eq!(query_ref(&s, &v, 4), "Sum(Sales.Amount)");
    assert_eq!(data(&s, &v)["status"], "ok");
}

#[test]
fn group_through_a_many_to_one_relationship() {
    let mut s = shop();
    let v = add_visual(
        &mut s,
        "clusteredColumnChart",
        json!([{"role": "Category", "field": "Product[Category]"}, {"role": "Y", "field": "[Total]"}]),
    );
    // without a sortDefinition, by category (as Desktop); the unmatched "Kite" row is the blank
    // group, first
    assert_eq!(
        rows(&data(&s, &v)),
        vec![json!([null, 7]), json!(["Toys", 5]), json!(["Wheels", 1150.5])]
    );
}

#[test]
fn measures_with_vars_and_arithmetic() {
    let mut s = shop();
    let v = add_visual(&mut s, "card", json!([{"role": "Values", "field": "[Calc]"}]));
    // (1162.5 - 5) * 2 + 10
    assert_eq!(rows(&data(&s, &v)), vec![json!([2325])]);
}

#[test]
fn column_only_visuals_list_distinct_values() {
    let mut s = shop();
    let v = add_visual(
        &mut s,
        "slicer",
        json!([{"role": "Values", "field": "Product[Category]"}]),
    );
    assert_eq!(rows(&data(&s, &v)), vec![json!(["Toys"]), json!(["Wheels"])]);
}

#[test]
fn unsupported_measures_and_missing_data() {
    let mut s = shop();
    let hard = add_visual(&mut s, "card", json!([{"role": "Values", "field": "[Hard]"}]));
    let d = data(&s, &hard);
    assert_eq!(d["status"], "unsupported", "{d}");
    assert!(d["reason"].as_str().unwrap().contains("RAND"), "{d}");

    // a table without a partition has no data
    run(&mut s, "table.add", json!({"name": "Empty"}));
    run(
        &mut s,
        "column.add",
        json!({"table": "Empty", "name": "X", "data_type": "string"}),
    );
    let v = add_visual(&mut s, "slicer", json!([{"role": "Values", "field": "Empty[X]"}]));
    let d = data(&s, &v);
    assert_eq!(d["status"], "unavailable", "{d}");

    // visuals without fields carry no data
    let t = add_visual(&mut s, "textbox", json!([]));
    assert_eq!(data(&s, &t), Value::Null);
}

#[test]
fn results_are_cached_and_follow_model_edits() {
    let mut s = shop();
    let v = add_visual(&mut s, "card", json!([{"role": "Values", "field": "[Total]"}]));
    assert_eq!(rows(&data(&s, &v)), vec![json!([1162.5])]);
    run(
        &mut s,
        "measure.set",
        json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Qty])"}),
    );
    assert_eq!(rows(&data(&s, &v)), vec![json!([15])]);
    // a second snapshot at the same revision is served from the cache
    let a = s.snapshot();
    let b = s.snapshot();
    assert_eq!(a, b);
}

#[test]
fn rows_are_capped() {
    let mut s = shop();
    let v = add_visual(
        &mut s,
        "tableEx",
        json!([{"role": "Values", "field": "Sales[Amount]"}]),
    );
    let limits = crate::SnapshotLimits {
        data_rows_per_visual: 2,
        ..crate::SnapshotLimits::default()
    };
    let d = &find(&s.snapshot_with(&limits), &v);
    assert_eq!(d["rows"].as_array().unwrap().len(), 2);
    assert_eq!(d["truncated"], true);
    assert_eq!(d["total_rows"], 5);
}

#[test]
fn zero_budget_computes_nothing_new() {
    let mut s = shop();
    let v = add_visual(&mut s, "card", json!([{"role": "Values", "field": "[Total]"}]));
    let limits = crate::SnapshotLimits {
        data_budget_ms: 0,
        ..crate::SnapshotLimits::default()
    };
    assert_eq!(find(&s.snapshot_with(&limits), &v), Value::Null);
    // once computed, the cached result is served whatever the budget
    assert_eq!(rows(&data(&s, &v)), vec![json!([1162.5])]);
    assert_eq!(find(&s.snapshot_with(&limits), &v)["status"], "ok");
}

#[test]
fn blank_arithmetic_follows_dax() {
    let leaf = |i| Box::new(Ir::Leaf(i));
    let v = [Out::Blank, Out::Num(4.0)];
    assert_eq!(eval(&Ir::Bin(Op::Add, leaf(0), leaf(1)), &v, &[]), Out::Num(4.0));
    assert_eq!(eval(&Ir::Bin(Op::Add, leaf(0), leaf(0)), &v, &[]), Out::Blank);
    assert_eq!(eval(&Ir::Bin(Op::Mul, leaf(0), leaf(1)), &v, &[]), Out::Blank);
    assert_eq!(eval(&Ir::Bin(Op::Div, leaf(0), leaf(1)), &v, &[]), Out::Blank);
    assert_eq!(eval(&Ir::Divide(leaf(1), leaf(0), None), &v, &[]), Out::Blank);
    assert_eq!(
        eval(
            &Ir::Divide(leaf(1), Box::new(Ir::Const(0.0)), Some(Box::new(Ir::Const(-1.0)))),
            &v,
            &[]
        ),
        Out::Num(-1.0)
    );
    assert_eq!(eval(&Ir::Neg(leaf(1)), &v, &[]), Out::Num(-4.0));
}

#[test]
fn currency_kinds_round_fast_path_results() {
    use super::currency::Kind;
    let leaf = |i| Box::new(Ir::Leaf(i));
    // SUM of a Currency column (37.5005) and a row count (4)
    let v = [Out::Num(37.5005), Out::Num(4.0), Out::Blank];
    let k = [Kind::Currency, Kind::Int, Kind::Currency];
    assert_eq!(
        eval(&Ir::Divide(leaf(0), leaf(1), None), &v, &k),
        Out::Num(9.3751)
    );
    assert_eq!(
        eval(&Ir::Bin(Op::Div, leaf(0), leaf(1)), &v, &k),
        Out::Num(9.3751)
    );
    assert_eq!(
        eval(&Ir::Bin(Op::Mul, leaf(0), Box::new(Ir::Const(1.1111))), &v, &k),
        Out::Num(41.6668)
    );
    // Currency + double stays a double; Currency / Currency too
    assert_eq!(
        eval(&Ir::Bin(Op::Add, leaf(0), Box::new(Ir::Const(0.00001))), &v, &k),
        Out::Num(37.5005 + 0.00001)
    );
    assert_eq!(eval(&Ir::Bin(Op::Div, leaf(0), leaf(0)), &v, &k), Out::Num(1.0));
    // BLANK + Currency is Currency; without kinds nothing is rounded
    assert_eq!(
        eval(&Ir::Bin(Op::Add, leaf(2), leaf(0)), &v, &k),
        Out::Num(37.5005)
    );
    assert_eq!(
        eval(&Ir::Divide(leaf(0), leaf(1), None), &v, &[]),
        Out::Num(37.5005 / 4.0)
    );
    // nested: (SUM / 3) * 2 rounds at each Currency step, like the engine
    let third = Ir::Bin(Op::Div, leaf(0), Box::new(Ir::Const(3.0)));
    assert_eq!(
        eval(
            &Ir::Bin(Op::Mul, Box::new(third), Box::new(Ir::Const(2.0))),
            &v,
            &k
        ),
        Out::Num(25.0004)
    );
}

#[test]
fn dates_and_codes() {
    assert_eq!(ole_to_iso(45_658.0).as_deref(), Some("2025-01-01"));
    assert_eq!(ole_to_iso(45_658.5).as_deref(), Some("2025-01-01T12:00:00"));
    assert_eq!(ole_to_iso(f64::NAN), None);
    let c = ColData::from_values(vec![
        CellValue::Text("a".into()),
        CellValue::Null,
        CellValue::Text("A".into()),
        CellValue::Text("b".into()),
    ])
    .unwrap();
    // text keys are case-insensitive, blank is code 0
    assert_eq!(c.codes.plain(), &[1, 0, 1, 2]);
    assert_eq!(Key::of(&CellValue::Int(2)), Key::of(&CellValue::Real(2.0)));
    assert_eq!(Key::of(&CellValue::Null), None);
}

/// Regression: a top N filter on a column of the table a visual groups, beside a condition on
/// that table, was dropped by the fast path (every row shown, while the total row, from the
/// general evaluator, was the top N's): scoring the top N memoized the table's visible rows in the
/// filter context before the top N joined it.
#[test]
fn a_top_n_filter_beside_a_condition_keeps_its_rows() {
    let mut s = shop();
    let v = add_visual(
        &mut s,
        "tableEx",
        json!([{"role": "Values", "field": "Sales[Product]"}, {"role": "Values", "field": "[Total]"}]),
    );
    for f in [
        json!({"page": "Main", "visual": v, "field": "Sales[Qty]", "op": ">=", "value": 1}),
        json!({"page": "Main", "visual": v, "field": "Sales[Product]", "top": 2, "by": "[Total]"}),
    ] {
        run(&mut s, "visual.filter", f);
    }
    let d = data(&s, &v);
    assert_eq!(
        rows(&d),
        vec![json!(["Bike", 150.5]), json!(["Car", 1000])],
        "{d}"
    );
}

#[test]
fn ties_of_the_sort_follow_the_next_columns() {
    let mut s = shop();
    let log = r#"#table({"Day", "Name"}, {{1, "BRAVOX"}, {2, "x"}, {1, "ALPHA"}, {1, "Bravo"}})"#;
    run(
        &mut s,
        "table.add",
        json!({"name": "Log", "partition": {"source_type": "m", "expression": log}}),
    );
    run(
        &mut s,
        "column.add",
        json!({"table": "Log", "name": "Day", "data_type": "int64"}),
    );
    run(
        &mut s,
        "column.add",
        json!({"table": "Log", "name": "Name", "data_type": "string"}),
    );
    let v = add_visual(
        &mut s,
        "tableEx",
        json!([{"role": "Values", "field": "Log[Day]"}, {"role": "Values", "field": "Log[Name]"}]),
    );
    run(
        &mut s,
        "visual.set_sort",
        json!({"page": "Main", "visual": v, "field": "Log[Day]", "direction": "desc"}),
    );
    // rows of one day: by the next column, A to Z, case-insensitive (Desktop 2.158)
    assert_eq!(
        rows(&data(&s, &v)),
        vec![
            json!([2, "x"]),
            json!([1, "ALPHA"]),
            json!([1, "Bravo"]),
            json!([1, "BRAVOX"]),
        ]
    );
}

/// Sets the `active` flag of each projection of `role` (a drill hierarchy on a chart axis).
fn set_active(s: &mut Session, visual: &str, role: &str, flags: &[Option<bool>]) {
    if let Some(r) = s.report.as_mut() {
        for v in r.pages.iter_mut().flat_map(|p| p.visuals.iter_mut()) {
            if v.name != visual {
                continue;
            }
            // a visual added by a command holds its query only once saved
            if let Some(q) = bicraft_report::query::regenerate(v.visual_extra.get("query"), &v.fields) {
                v.visual_extra.insert("query".into(), q);
            }
            let ps = v
                .visual_extra
                .get_mut("query")
                .and_then(|q| q.get_mut("queryState"))
                .and_then(|q| q.get_mut(role))
                .and_then(|r| r.get_mut("projections"))
                .and_then(Value::as_array_mut)
                .unwrap();
            for (p, f) in ps.iter_mut().zip(flags) {
                if let (Some(o), Some(f)) = (p.as_object_mut(), f) {
                    o.insert("active".into(), json!(f));
                }
            }
        }
    }
    s.revision += 1;
}

fn visual_of(s: &Session, visual: &str) -> Value {
    let snap = s.snapshot();
    for p in snap["report"]["pages"].as_array().cloned().unwrap_or_default() {
        for v in p["visuals"].as_array().cloned().unwrap_or_default() {
            if v["name"] == visual {
                return v;
            }
        }
    }
    panic!("visual {visual} not in the snapshot")
}

#[test]
fn hierarchical_axis_is_queried_and_titled_at_its_drill_level() {
    let mut s = shop();
    let v = add_visual(
        &mut s,
        "clusteredBarChart",
        json!([{"role": "Category", "field": "Product[Category]"}, {"role": "Category", "field": "Product[Product]"},
               {"role": "Y", "field": "[Total]"}]),
    );
    run(
        &mut s,
        "visual.set_sort",
        json!({"page": "Main", "visual": v, "field": "[Total]", "direction": "desc"}),
    );
    // no drill state: the top level only, sorted by its own totals
    set_active(&mut s, &v, "Category", &[Some(true), Some(false)]);
    let o = visual_of(&s, &v);
    assert_eq!(
        rows(&o["data"]),
        vec![json!(["Wheels", 1150.5]), json!([null, 7]), json!(["Toys", 5])]
    );
    assert_eq!(o["auto_title"], "Total by Category");
    assert_eq!(o["fields"].as_array().map(Vec::len), Some(3));

    // drilled down: the second level
    set_active(&mut s, &v, "Category", &[Some(false), Some(true)]);
    let o = visual_of(&s, &v);
    assert_eq!(rows(&o["data"]).first(), Some(&json!(["Car", 1000])));
    assert_eq!(o["auto_title"], "Total by Product");

    // expanded to all levels, or no flags (a visual written by a tool): every field
    set_active(&mut s, &v, "Category", &[Some(true), Some(true)]);
    assert_eq!(visual_of(&s, &v)["auto_title"], "Total by Category and Product");
}

#[test]
fn tables_ignore_active_flags() {
    let mut s = shop();
    let v = add_visual(
        &mut s,
        "tableEx",
        json!([{"role": "Values", "field": "Product[Category]"}, {"role": "Values", "field": "Product[Product]"}]),
    );
    set_active(&mut s, &v, "Values", &[Some(true), Some(false)]);
    assert_eq!(
        rows(&data(&s, &v))
            .first()
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(2)
    );
}

/// The `queryRef` of the `i`-th projection of `visual` (in field order).
fn query_ref(s: &Session, visual: &str, i: usize) -> String {
    let r = s.report.as_ref().unwrap();
    let mut v = r.pages.iter().find_map(|p| p.visual(visual)).unwrap().clone();
    let q = bicraft_report::query::effective(&v).unwrap();
    v.visual_extra.insert("query".into(), q);
    crate::visual_series::projections(&v)[i].1.clone()
}

/// Sets the visual's `objects` (its query written out, as a saved visual has it).
fn set_objects(s: &mut Session, visual: &str, objects: Value) {
    let r = s.report.as_mut().unwrap();
    let v = r
        .pages
        .iter_mut()
        .flat_map(|p| p.visuals.iter_mut())
        .find(|v| v.name == visual)
        .unwrap();
    let q = bicraft_report::query::effective(v).unwrap();
    v.visual_extra.insert("query".into(), q);
    v.visual_extra.insert("objects".into(), objects);
}

/// Desktop 2.158 names a matrix color scale's input by `SelectRef` (the value's queryRef): the
/// scale is applied to the cells.
#[test]
fn color_scales_named_by_select_ref_are_applied() {
    let mut s = shop();
    let v = add_visual(
        &mut s,
        "pivotTable",
        json!([{"role": "Rows", "field": "Sales[Product]"}, {"role": "Values", "field": "[Total]"}]),
    );
    let r = query_ref(&s, &v, 1);
    let color = |c: &str| json!({"color": {"Literal": {"Value": format!("'{c}'")}}});
    set_objects(
        &mut s,
        &v,
        json!({"values": [{"properties": {"backColor": {"solid": {"color": {"expr": {"FillRule": {
            "Input": {"SelectRef": {"ExpressionName": r}},
            "FillRule": {"linearGradient2": {"min": color("#000000"), "max": color("#FFFFFF"),
                "nullColoringStrategy": {"strategy": {"Literal": {"Value": "'noColor'"}}}}}}}}}}},
            "selector": {"data": [{"dataViewWildcard": {"matchingOption": 1}}], "metadata": r}}]}),
    );
    let d = data(&s, &v);
    // rows: Ball 5, Bike 150.5, Car 1000, Kite 7
    let styles = d["cell_styles"].as_array().cloned().unwrap_or_default();
    let at = |row: u64| {
        styles
            .iter()
            .find(|x| x["r"] == row && x["c"] == 1)
            .map(|x| x["bg"].clone())
            .unwrap_or(Value::Null)
    };
    assert_eq!(at(0), "#000000", "{d}");
    assert_eq!(at(2), "#FFFFFF", "{d}");
    // the visual keeps Desktop's form
    let r = s.report.as_ref().unwrap();
    let props = crate::visual_props::extract_props(r.pages.iter().find_map(|p| p.visual(&v)).unwrap());
    assert!(props.get("conditional").is_none(), "{props}");
}

/// The whole-visual conditional formats (title text and colour, background, border) are
/// evaluated once in the visual's filter context (`data.visual_styles`); a chart's data label
/// colour by rule is evaluated per point (`cell_styles` `label`).
#[test]
fn container_and_label_conditional_formats_are_evaluated() {
    let mut s = shop();
    for (name, e) in [("Caption", r#""Top products""#), ("Hue", r##""#123456""##)] {
        run(
            &mut s,
            "measure.add",
            json!({"table": "Sales", "name": name, "expression": e}),
        );
    }
    let v = add_visual(
        &mut s,
        "clusteredColumnChart",
        json!([{"role": "Category", "field": "Sales[Product]"}, {"role": "Y", "field": "[Total]"}]),
    );
    run(
        &mut s,
        "visual.conditional",
        json!({"page": "Main", "visual": v, "conditional": [
            {"target": "title_text", "rule": {"field_value": {"field": "[Caption]"}}},
            {"target": "title_color", "rule": {"rules": {"field": "[Total]",
                "cases": [{"op": ">", "value": 1000, "color": "#00AA00"}], "else": "#AA0000"}}},
            {"target": "visual_background", "rule": {"field_value": {"field": "[Hue]"}}},
            {"target": "border_color", "rule": {"gradient": {"field": "[Total]",
                "min": {"color": "#000000", "value": 0}, "max": {"color": "#FFFFFF", "value": 2000}}}},
            {"target": "label_color", "rule": {"rules": {"field": "[Total]",
                "cases": [{"op": ">=", "value": 100, "color": "#FF0000"}], "else": "#0000FF"}}}]}),
    );
    let d = data(&s, &v);
    // the total over every product: 1162.5
    assert_eq!(
        d["visual_styles"],
        json!({"title_text": "Top products", "title_color": "#00AA00",
               "visual_background": "#123456", "border_color": "#949494"}),
        "{d}"
    );
    let labels: Vec<(u64, Value)> = d["cell_styles"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter(|x| x.get("label").is_some())
        .map(|x| (x["r"].as_u64().unwrap(), x["label"].clone()))
        .collect();
    let rows = rows(&d);
    assert_eq!(labels.len(), rows.len(), "{d}");
    for (r, c) in labels {
        let total = rows[r as usize][1].as_f64().unwrap();
        assert_eq!(c, if total >= 100.0 { "#FF0000" } else { "#0000FF" }, "{d}");
    }
}

/// `format.table`: the grid's row padding and font sizes (theme or visual), the visual's
/// column widths and sort.
#[test]
fn table_layout_reaches_the_snapshot() {
    let mut s = shop();
    let v = add_visual(
        &mut s,
        "tableEx",
        json!([{"role": "Values", "field": "Sales[Product]"}, {"role": "Values", "field": "[Total]"}]),
    );
    run(
        &mut s,
        "visual.set_sort",
        json!({"page": "Main", "visual": v, "field": "[Total]", "direction": "desc"}),
    );
    let r = query_ref(&s, &v, 0);
    set_objects(
        &mut s,
        &v,
        json!({"columnWidth": [{"properties": {"value": {"expr": {"Literal": {"Value": "96D"}}}}, "selector": {"metadata": r}}],
               "grid": [{"properties": {"rowPadding": {"expr": {"Literal": {"Value": "3D"}}}}}],
               "values": [{"properties": {"fontSize": {"expr": {"Literal": {"Value": "11D"}}}}}]}),
    );
    let snap = s.snapshot();
    let vis = snap["report"]["pages"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|p| p["visuals"].as_array().cloned().unwrap_or_default())
        .find(|x| x["name"] == v)
        .unwrap();
    let t = &vis["format"]["table"];
    assert_eq!(t["row_padding"], 3, "{t}");
    assert_eq!(t["font_size"], 11, "{t}");
    assert_eq!(t["column_widths"], json!([{"column": 0, "width": 96.0}]), "{t}");
    assert_eq!(t["sort"], json!([{"column": 1, "descending": true}]), "{t}");
}

/// G54: the axes', legend's, gauge's and KPI's whole-visual colours are evaluated once
/// (`data.visual_styles`), and Percent rule bounds are read over the points' range.
#[test]
fn whole_visual_colours_and_percent_rules_are_evaluated() {
    let mut s = shop();
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Hue", "expression": r##""#123456""##}),
    );
    let v = add_visual(
        &mut s,
        "clusteredColumnChart",
        json!([{"role": "Category", "field": "Sales[Product]"}, {"role": "Y", "field": "[Total]"}]),
    );
    run(
        &mut s,
        "visual.conditional",
        json!({"page": "Main", "visual": v, "conditional": [
            {"target": "category_axis_color", "rule": {"rules": {"field": "[Total]",
                "cases": [{"op": ">", "value": 1000, "color": "#00AA00"}], "else": "#AA0000"}}},
            {"target": "value_title_color", "rule": {"field_value": {"field": "[Hue]"}}},
            {"target": "legend_color", "rule": {"field_value": {"field": "[Hue]"}}},
            {"target": "data_color", "rule": {"rules": {"field": "[Total]",
                "cases": [{"op": ">=", "value": 50, "percent": true, "color": "#FF0000"}],
                "else": "#0000FF"}}}]}),
    );
    let d = data(&s, &v);
    // the total over every product: 1162.5
    assert_eq!(
        d["visual_styles"],
        json!({"category_axis_color": "#00AA00", "value_title_color": "#123456",
               "legend_color": "#123456"}),
        "{d}"
    );
    // 50 % of the range 5..1000 is 502.5: only the car's 1000 meets it
    let rows = rows(&d);
    let fills: Vec<(u64, Value)> = d["cell_styles"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter(|x| x.get("fill").is_some())
        .map(|x| (x["r"].as_u64().unwrap(), x["fill"].clone()))
        .collect();
    assert_eq!(fills.len(), rows.len(), "{d}");
    for (r, c) in fills {
        let total = rows[r as usize][1].as_f64().unwrap();
        assert_eq!(c, if total >= 502.5 { "#FF0000" } else { "#0000FF" }, "{d}");
    }
    let k = add_visual(
        &mut s,
        "kpi",
        json!([{"role": "Indicator", "field": "[Total]"}, {"role": "TrendLine", "field": "Sales[Product]"}]),
    );
    run(
        &mut s,
        "visual.conditional",
        json!({"page": "Main", "visual": k, "conditional": [
            {"target": "good_color", "rule": {"field_value": {"field": "[Hue]"}}},
            {"target": "goal_color", "rule": {"rules": {"field": "[Total]",
                "cases": [{"op": "<", "value": 0, "color": "#00AA00"}], "else": "#AA0000"}}}]}),
    );
    assert_eq!(
        data(&s, &k)["visual_styles"],
        json!({"good_color": "#123456", "goal_color": "#AA0000"})
    );
}
