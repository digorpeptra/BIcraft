//! Report, page and visual filters, slicer selections, sort-by-column and conditional formatting
//! in the data preview (`visual.data`), on a synthetic model with literal tables.

use bicraft_report::Report;
use serde_json::{Value, json};

use bicraft_engine::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Sales(Product, Amount, Qty) many-to-one Product(Product, Category, Rank).
fn shop() -> Session {
    let mut s = Session::new();
    let sales = r#"#table({"Product", "Amount", "Qty"}, {{"Bike", 100, 1}, {"Bike", 50.5, 2}, {"Car", 1000, 1}, {"Ball", 5, 10}, {"Kite", 7, 1}})"#;
    let product = r#"#table({"Product", "Category", "Rank"}, {{"Bike", "Wheels", 2}, {"Car", "Wheels", 2}, {"Ball", "Toys", 1}})"#;
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
            "column.add",
            json!({"table": "Product", "name": "Rank", "data_type": "int64"}),
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
            json!({"table": "Sales", "name": "Share", "expression": "DIVIDE([Total], CALCULATE([Total], ALLSELECTED(Sales[Product])))"}),
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

/// Edits the report directly (what Desktop would have written) and bumps the revision.
fn edit(s: &mut Session, f: impl FnOnce(&mut Report)) {
    if let Some(r) = s.report.as_mut() {
        f(r);
    }
    s.revision += 1;
}

/// The test page.
fn main(r: &mut Report) -> Option<&mut bicraft_report::Page> {
    r.pages.iter_mut().find(|p| p.display_name == "Main")
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

fn rows(d: &Value) -> Vec<Value> {
    assert_eq!(d["status"], "ok", "{d}");
    d["rows"].as_array().cloned().unwrap_or_default()
}

fn col(t: &str, c: &str) -> Value {
    json!({"Column": {"Expression": {"SourceRef": {"Entity": t}}, "Property": c}})
}

fn lit(v: &str) -> Value {
    json!({"Literal": {"Value": v}})
}

fn where_(cond: Value) -> Value {
    json!({"Version": 2, "From": [], "Where": [{"Condition": cond}]})
}

fn in_(t: &str, c: &str, values: &[&str]) -> Value {
    let vals: Vec<Value> = values.iter().map(|v| json!([lit(v)])).collect();
    json!({"In": {"Expressions": [col(t, c)], "Values": vals}})
}

fn config(filter: Value) -> Value {
    json!({"filters": [{"name": "f1", "field": {}, "type": "Categorical", "filter": filter}]})
}

/// Sets a slicer's selection (`objects.general.filter`).
fn select(s: &mut Session, slicer: &str, filter: Value) {
    edit(s, |r| {
        for v in r.pages.iter_mut().flat_map(|p| p.visuals.iter_mut()) {
            if v.name == slicer {
                v.visual_extra.insert(
                    "objects".into(),
                    json!({"general": [{"properties": {"filter": {"filter": filter}}}],
                           "data": [{"properties": {"mode": {"expr": {"Literal": {"Value": "'Dropdown'"}}}}}]}),
                );
            }
        }
    });
}

#[test]
fn visual_page_and_report_filters_apply() {
    let mut s = shop();
    let t = add_visual(
        &mut s,
        "tableEx",
        json!([{"role": "Values", "field": "Sales[Product]"}, {"role": "Values", "field": "[Total]"}]),
    );
    edit(&mut s, |r| {
        if let Some(v) = main(r).and_then(|p| p.visuals.get_mut(0)) {
            v.filter_config = Some(config(where_(in_(
                "Sales",
                "Product",
                &["'bike'", "'Car'", "'Kite'"],
            ))));
        }
    });
    let d = data(&s, &t);
    // text values match whatever their case, as in Power BI
    assert_eq!(
        rows(&d),
        vec![json!(["Bike", 150.5]), json!(["Car", 1000]), json!(["Kite", 7])]
    );
    assert_eq!(d["filters_applied"], 1);
    assert!(d.get("filters_ignored").is_none(), "{d}");

    // a page filter excluding a category (through the relationship), and a report filter
    edit(&mut s, |r| {
        if let Some(p) = main(r) {
            p.filter_config = Some(config(where_(
                json!({"Not": {"Expression": in_("Product", "Category", &["'Wheels'"])}}),
            )));
        }
        r.filter_config = Some(config(where_(json!({"Comparison": {"ComparisonKind": 2,
            "Left": col("Sales", "Amount"), "Right": lit("6D")}}))));
    });
    // Kite has no product row: its blank category is not Wheels; Ball (5) is under 6
    assert_eq!(rows(&data(&s, &t)), vec![json!(["Kite", 7])]);
}

#[test]
fn slicers_filter_the_other_visuals_of_their_page() {
    let mut s = shop();
    let slicer = add_visual(
        &mut s,
        "slicer",
        json!([{"role": "Values", "field": "Product[Category]"}]),
    );
    let card = add_visual(&mut s, "card", json!([{"role": "Values", "field": "[Total]"}]));
    let other = add_visual(&mut s, "card", json!([{"role": "Values", "field": "[Lines]"}]));
    let second = add_visual(
        &mut s,
        "slicer",
        json!([{"role": "Values", "field": "Product[Product]"}]),
    );
    select(&mut s, &slicer, where_(in_("Product", "Category", &["'Wheels'"])));
    assert_eq!(rows(&data(&s, &card)), vec![json!([1150.5])]);
    assert_eq!(rows(&data(&s, &other)), vec![json!([3])]);
    // a slicer is not filtered by its own selection, but by the other slicers
    let snap = s.snapshot();
    assert_eq!(
        rows(&visual_json(&snap, &slicer)["data"]),
        vec![json!(["Toys"]), json!(["Wheels"])]
    );
    assert_eq!(
        rows(&visual_json(&snap, &second)["data"]),
        vec![json!(["Bike"]), json!(["Car"])]
    );
    // the slicer's mode and selection are in the snapshot
    assert_eq!(
        visual_json(&snap, &slicer)["slicer"],
        json!({"mode": "dropdown", "selected": ["Wheels"]})
    );

    // "no filter" interaction: the slicer leaves that visual alone
    let (src, dst) = (slicer.clone(), other.clone());
    edit(&mut s, |r| {
        if let Some(p) = main(r) {
            p.extra.insert(
                "visualInteractions".into(),
                json!([{"source": src, "target": dst, "type": "NoFilter"}]),
            );
        }
    });
    assert_eq!(rows(&data(&s, &other)), vec![json!([5])]);
    assert_eq!(rows(&data(&s, &card)), vec![json!([1150.5])]);

    // an inverted selection ("select all" minus some values)
    select(
        &mut s,
        &slicer,
        where_(json!({"Not": {"Expression": in_("Product", "Category", &["'Wheels'"])}})),
    );
    assert_eq!(rows(&data(&s, &card)), vec![json!([12])]);
    assert_eq!(
        visual_json(&s.snapshot(), &slicer)["slicer"],
        json!({"mode": "dropdown", "selected": ["Wheels"], "inverted": true})
    );
}

#[test]
fn allselected_restores_the_outer_filters() {
    let mut s = shop();
    let slicer = add_visual(
        &mut s,
        "slicer",
        json!([{"role": "Values", "field": "Sales[Product]"}]),
    );
    let t = add_visual(
        &mut s,
        "tableEx",
        json!([{"role": "Values", "field": "Sales[Product]"}, {"role": "Values", "field": "[Share]"}]),
    );
    select(
        &mut s,
        &slicer,
        where_(in_("Sales", "Product", &["'Ball'", "'Kite'"])),
    );
    assert_eq!(
        rows(&data(&s, &t)),
        vec![json!(["Ball", 5.0 / 12.0]), json!(["Kite", 7.0 / 12.0])]
    );
}

#[test]
fn measure_filters_and_top_n() {
    let mut s = shop();
    let t = add_visual(
        &mut s,
        "tableEx",
        json!([{"role": "Values", "field": "Sales[Product]"}, {"role": "Values", "field": "Sum(Sales[Qty])"}]),
    );
    // [Total] > 6 on a measure the visual does not show
    let measure = json!({"Measure": {"Expression": {"SourceRef": {"Entity": "Sales"}}, "Property": "Total"}});
    edit(&mut s, |r| {
        if let Some(v) = main(r).and_then(|p| p.visuals.get_mut(0)) {
            v.filter_config = Some(config(where_(
                json!({"Comparison": {"ComparisonKind": 1, "Left": measure, "Right": lit("6D")}}),
            )));
        }
    });
    let d = data(&s, &t);
    assert_eq!(
        rows(&d),
        vec![json!(["Bike", 3]), json!(["Car", 1]), json!(["Kite", 1])]
    );
    assert_eq!(d["columns"].as_array().map(Vec::len), Some(2));

    // top 2 products by [Total]
    let top = json!({"Version": 2,
        "From": [{"Name": "subquery", "Type": 2, "Expression": {"Subquery": {"Query": {"Version": 2,
            "From": [{"Name": "s", "Entity": "Sales", "Type": 0}],
            "Select": [{"Column": {"Expression": {"SourceRef": {"Source": "s"}}, "Property": "Product"}, "Name": "field"}],
            "OrderBy": [{"Direction": 2, "Expression": {"Measure": {"Expression": {"SourceRef": {"Source": "s"}}, "Property": "Total"}}}],
            "Top": 2}}}}, {"Name": "s", "Entity": "Sales", "Type": 0}],
        "Where": [{"Condition": {"In": {"Expressions": [{"Column": {"Expression": {"SourceRef": {"Source": "s"}}, "Property": "Product"}}],
            "Table": {"SourceRef": {"Source": "subquery"}}}}}]});
    edit(&mut s, |r| {
        if let Some(v) = main(r).and_then(|p| p.visuals.get_mut(0)) {
            v.filter_config = Some(config(top));
        }
    });
    assert_eq!(rows(&data(&s, &t)), vec![json!(["Bike", 3]), json!(["Car", 1])]);
}

#[test]
fn unsupported_filters_are_named() {
    let mut s = shop();
    let card = add_visual(&mut s, "card", json!([{"role": "Values", "field": "[Total]"}]));
    edit(&mut s, |r| {
        if let Some(p) = main(r) {
            p.filter_config = Some(config(where_(
                json!({"In": {"Expressions": [{"HierarchyLevel": {}}],
                "Values": [[lit("1L")]]}}),
            )));
        }
    });
    let d = data(&s, &card);
    assert_eq!(rows(&d), vec![json!([1162.5])]);
    assert_eq!(d["filters_ignored"], true);
    assert!(
        d["filters_ignored_reason"]
            .as_str()
            .unwrap_or("")
            .contains("hierarchy"),
        "{d}"
    );
}

#[test]
fn categories_follow_their_sort_by_column() {
    let mut s = shop();
    if let Some(c) = s
        .model
        .as_mut()
        .and_then(|m| m.tables.iter_mut().find(|t| t.name == "Product"))
        .and_then(|t| t.columns.iter_mut().find(|c| c.name == "Category"))
    {
        c.sort_by_column = Some("Rank".into());
    }
    s.revision += 1;
    let t = add_visual(
        &mut s,
        "lineChart",
        json!([{"role": "Category", "field": "Product[Category]"}, {"role": "Y", "field": "Sum(Sales[Qty])"}]),
    );
    // Rank: Toys 1, Wheels 2 (blank first)
    assert_eq!(
        rows(&data(&s, &t)),
        vec![json!([null, 1]), json!(["Toys", 10]), json!(["Wheels", 4])]
    );
    // a descending sort on the category uses the sort-by column too
    let v = add_visual(
        &mut s,
        "tableEx",
        json!([{"role": "Values", "field": "Product[Category]"}]),
    );
    run(
        &mut s,
        "visual.set_sort",
        json!({"page": "Main", "visual": v, "field": "Product[Category]", "direction": "desc"}),
    );
    assert_eq!(rows(&data(&s, &v)), vec![json!(["Wheels"]), json!(["Toys"])]);
}

#[test]
fn conditional_formatting_is_evaluated() {
    let mut s = shop();
    let t = add_visual(
        &mut s,
        "tableEx",
        json!([{"role": "Values", "field": "Sales[Product]"}, {"role": "Values", "field": "Sum(Sales[Qty])"}]),
    );
    run(
        &mut s,
        "visual.conditional",
        json!({"page": "Main", "visual": t, "series": "Sum(Sales[Qty])",
               "rule": {"gradient": {"field": "Sum(Sales[Qty])", "min": "#000000", "max": "#FFFFFF"}}}),
    );
    run(
        &mut s,
        "visual.conditional",
        json!({"page": "Main", "visual": t, "target": "font_color", "series": "Sales[Product]",
               "rule": {"rules": {"field": "[Total]", "cases": [{"op": ">", "value": 100, "color": "#FF0000"}]}}}),
    );
    let d = data(&s, &t);
    // rows: Ball 10, Bike 3, Car 1, Kite 1 → gradient over [1, 10]
    let styles = d["cell_styles"].as_array().cloned().unwrap_or_default();
    let at = |r: u64, c: u64| {
        styles
            .iter()
            .find(|x| x["r"] == r && x["c"] == c)
            .cloned()
            .unwrap_or(Value::Null)
    };
    assert_eq!(at(0, 1)["bg"], "#FFFFFF", "{d}");
    assert_eq!(at(2, 1)["bg"], "#000000", "{d}");
    // [Total] is not in the visual: it is evaluated as a hidden field
    assert_eq!(at(1, 0)["fg"], "#FF0000", "{d}");
    assert_eq!(at(0, 0), Value::Null, "{d}");
    assert_eq!(d["rows"][0].as_array().map(Vec::len), Some(2));

    // data bars with explicit bounds: Ball (10) fills the cell, Bike (3) 30 % of it
    run(
        &mut s,
        "visual.conditional",
        json!({"page": "Main", "visual": t, "series": "Sum(Sales[Qty])",
               "rule": {"data_bars": {"positive": "#00FF00", "min": 0, "max": 10}}}),
    );
    let d = data(&s, &t);
    let styles = d["cell_styles"].as_array().cloned().unwrap_or_default();
    let bar = |r: u64| {
        styles
            .iter()
            .find(|x| x["r"] == r && x["c"] == 1)
            .map(|x| x["bar"].clone())
            .unwrap_or(Value::Null)
    };
    assert_eq!(bar(0), json!({"from": 0.0, "to": 1.0, "color": "#00FF00"}), "{d}");
    assert_eq!(bar(1)["to"], 0.3, "{d}");
}

/// Data bars' axis, direction (right to left) and Show bar only reach `cell_styles`.
#[test]
fn data_bars_axis_direction_and_bar_only_are_evaluated() {
    let mut s = shop();
    let t = add_visual(
        &mut s,
        "tableEx",
        json!([{"role": "Values", "field": "Sales[Product]"}, {"role": "Values", "field": "Sum(Sales[Qty])"}]),
    );
    run(
        &mut s,
        "visual.conditional",
        json!({"page": "Main", "visual": t, "series": "Sum(Sales[Qty])",
               "rule": {"data_bars": {"positive": "#00FF00", "negative": "#FF0000", "axis": "#333333",
                                      "min": -10, "max": 10}}}),
    );
    let bar = |s: &Session, r: u64| {
        let d = data(s, &t);
        d["cell_styles"]
            .as_array()
            .and_then(|a| a.iter().find(|x| x["r"] == r && x["c"] == 1))
            .map(|x| x["bar"].clone())
            .unwrap_or(Value::Null)
    };
    // Ball (10) from the axis in the middle to the right edge, the axis drawn
    assert_eq!(
        bar(&s, 0),
        json!({"from": 0.5, "to": 1.0, "color": "#00FF00", "axis": 0.5, "axis_color": "#333333"})
    );
    run(
        &mut s,
        "visual.conditional",
        json!({"page": "Main", "visual": t, "series": "Sum(Sales[Qty])",
               "rule": {"data_bars": {"positive": "#00FF00", "axis": "#333333", "min": -10, "max": 10,
                                      "reverse": true, "show_bar_only": true}}}),
    );
    // right to left: Ball's bar runs from the left edge to the axis; Bike (3) 15 % left of it
    assert_eq!(
        bar(&s, 0),
        json!({"from": 0.0, "to": 0.5, "color": "#00FF00", "axis": 0.5, "axis_color": "#333333", "only": true})
    );
    assert_eq!(bar(&s, 1)["from"], 0.35);
}

/// Icons, web URLs, a field bound twice and matrices get their `cell_styles` too.
#[test]
fn icons_web_urls_and_matrices_are_styled() {
    let mut s = shop();
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Link", "expression": "\"https://example.com/p/\" & SELECTEDVALUE(Sales[Product])"}),
    );
    let t = add_visual(
        &mut s,
        "tableEx",
        json!([{"role": "Values", "field": "Sales[Product]"}, {"role": "Values", "field": "Sum(Sales[Qty])"}]),
    );
    run(
        &mut s,
        "visual.conditional",
        json!({"page": "Main", "visual": t, "series": "Sum(Sales[Qty])",
               "rule": {"icons": {"field": "Sum(Sales[Qty])", "preset": "traffic", "thresholds": [2, 5],
                                  "position": "after", "align": "middle"}}}),
    );
    run(
        &mut s,
        "visual.conditional",
        json!({"page": "Main", "visual": t, "target": "web_url", "series": "Sales[Product]",
               "rule": {"web_url": {"field": "[Link]"}}}),
    );
    let d = data(&s, &t);
    let styles = d["cell_styles"].as_array().cloned().unwrap_or_default();
    let at = |r: u64, c: u64| {
        styles
            .iter()
            .find(|x| x["r"] == r && x["c"] == c)
            .cloned()
            .unwrap_or(Value::Null)
    };
    // rows: Ball 10, Bike 3, Car 1, Kite 1
    assert_eq!(
        at(0, 1)["icon"],
        json!({"name": "TrafficHigh", "position": "after", "align": "middle"}),
        "{d}"
    );
    assert_eq!(at(1, 1)["icon"]["name"], "TrafficMedium", "{d}");
    assert_eq!(at(2, 1)["icon"]["name"], "TrafficLow", "{d}");
    assert_eq!(at(0, 0)["url"], "https://example.com/p/Ball", "{d}");
    assert_eq!(at(3, 0)["url"], "https://example.com/p/Kite", "{d}");

    // A matrix: the producer styles its value cells by data row as for tables.
    let m = add_visual(
        &mut s,
        "pivotTable",
        json!([{"role": "Rows", "field": "Product[Category]"}, {"role": "Values", "field": "Sum(Sales[Qty])"}]),
    );
    run(
        &mut s,
        "visual.conditional",
        json!({"page": "Main", "visual": m, "series": "Sum(Sales[Qty])",
               "rule": {"rules": {"field": "Sum(Sales[Qty])", "cases": [{"op": ">", "value": 5, "color": "#00FF00"}]}}}),
    );
    let d = data(&s, &m);
    let styles = d["cell_styles"].as_array().cloned().unwrap_or_default();
    // categories: (blank: Kite) 1, Toys 10, Wheels 4
    let toys = d["rows"]
        .as_array()
        .unwrap()
        .iter()
        .position(|r| r[0] == "Toys")
        .unwrap() as u64;
    assert!(
        styles
            .iter()
            .any(|x| x["r"] == toys && x["c"] == 1 && x["bg"] == "#00FF00"),
        "{d}"
    );
    assert_eq!(styles.len(), 1, "{d}");
}

/// Data bars on the second binding of a field bound twice go on that column.
#[test]
fn data_bars_follow_the_binding_they_name() {
    let mut s = shop();
    let t = add_visual(
        &mut s,
        "tableEx",
        json!([{"role": "Values", "field": "Sales[Product]"}, {"role": "Values", "field": "Sum(Sales[Qty])"}]),
    );
    // Bound a second time, as Desktop does it: the copy has its own queryRef.
    {
        let vis = s
            .report
            .as_mut()
            .unwrap()
            .page_mut("Main")
            .unwrap()
            .visual_mut(&t)
            .unwrap();
        let mut query = bicraft_report::query::effective(vis).unwrap();
        let values = query["queryState"]["Values"]["projections"]
            .as_array_mut()
            .unwrap();
        let mut dup = values[1].clone();
        dup["queryRef"] = json!("Sum(Sales.Qty) (bar)");
        values.push(dup);
        let field = vis.fields[1].clone();
        vis.fields.push(field);
        vis.visual_extra.insert("query".into(), query);
    }
    run(
        &mut s,
        "visual.conditional",
        json!({"page": "Main", "visual": t, "series": "Sum(Sales[Qty])", "occurrence": 2,
               "rule": {"data_bars": {"min": 0, "max": 10}}}),
    );
    let d = data(&s, &t);
    let styles = d["cell_styles"].as_array().cloned().unwrap_or_default();
    assert!(!styles.is_empty(), "{d}");
    assert!(
        styles.iter().all(|x| x["c"] == 2 && x.get("bar").is_some()),
        "{d}"
    );
}

/// Sets a slicer's `objects` (selection, search, mode) as Desktop writes them.
fn slicer_objects(s: &mut Session, slicer: &str, objects: Value) {
    edit(s, |r| {
        for v in r.pages.iter_mut().flat_map(|p| p.visuals.iter_mut()) {
            if v.name == slicer {
                v.visual_extra.insert("objects".into(), objects.clone());
            }
        }
    });
}

#[test]
fn slicer_search_equality_and_range_selections() {
    let mut s = shop();
    let text = add_visual(
        &mut s,
        "textSlicer",
        json!([{"role": "Values", "field": "Sales[Product]"}]),
    );
    let card = add_visual(&mut s, "card", json!([{"role": "Values", "field": "[Total]"}]));
    // a text slicer's typed text filters the other visuals (Bike + Kite)
    let contains = json!({"Contains": {"Left": col("Sales", "Product"), "Right": lit("'i'")}});
    slicer_objects(
        &mut s,
        &text,
        json!({"general": [{"properties": {"filter": {"filter": where_(contains)}}}]}),
    );
    assert_eq!(rows(&data(&s, &card)), vec![json!([157.5])]);
    assert_eq!(
        visual_json(&s.snapshot(), &text)["slicer"],
        json!({"mode": "text", "search": "i", "match": "contains"})
    );

    // a single value written as an equality, in a dropdown
    let eq =
        json!({"Comparison": {"ComparisonKind": 0, "Left": col("Sales", "Product"), "Right": lit("'Car'")}});
    slicer_objects(
        &mut s,
        &text,
        json!({"general": [{"properties": {"filter": {"filter": where_(eq)}}}]}),
    );
    assert_eq!(rows(&data(&s, &card)), vec![json!([1000])]);
    assert_eq!(
        visual_json(&s.snapshot(), &text)["slicer"]["selected"],
        json!(["Car"])
    );

    // a "between" slicer: the range is shown and filters the card (Bike 100 + 50.5, Kite 7)
    let range = json!({"And": {
        "Left": {"Comparison": {"ComparisonKind": 2, "Left": col("Sales", "Amount"), "Right": lit("6D")}},
        "Right": {"Comparison": {"ComparisonKind": 4, "Left": col("Sales", "Amount"), "Right": lit("200D")}}}});
    let between = add_visual(
        &mut s,
        "slicer",
        json!([{"role": "Values", "field": "Sales[Amount]"}]),
    );
    slicer_objects(&mut s, &text, json!({}));
    slicer_objects(
        &mut s,
        &between,
        json!({"general": [{"properties": {"filter": {"filter": where_(range)}}}],
               "data": [{"properties": {"mode": {"expr": {"Literal": {"Value": "'Between'"}}}}}]}),
    );
    assert_eq!(rows(&data(&s, &card)), vec![json!([157.5])]);
    assert_eq!(
        visual_json(&s.snapshot(), &between)["slicer"],
        json!({"mode": "between", "range": {"from": 6, "to": 200, "filter_only": true},
               "domain": {"min": 5, "max": 1000}})
    );
}

/// A visual object property literal.
fn p(v: &str) -> Value {
    json!({"expr": lit(v)})
}

#[test]
fn slicer_displayed_range_relative_search_box_and_header() {
    let mut s = shop();
    let between = add_visual(
        &mut s,
        "slicer",
        json!([{"role": "Values", "field": "Sales[Amount]"}]),
    );
    // a strict filter: the number bound is flagged exclusive
    let gt = json!({"Comparison": {"ComparisonKind": 1, "Left": col("Sales", "Amount"), "Right": lit("6D")}});
    let mode = json!([{"properties": {"mode": p("'Between'")}}]);
    slicer_objects(
        &mut s,
        &between,
        json!({"general": [{"properties": {"filter": {"filter": where_(gt.clone())}}}], "data": mode}),
    );
    assert_eq!(
        visual_json(&s.snapshot(), &between)["slicer"]["range"],
        json!({"from": 6, "from_exclusive": true, "filter_only": true})
    );
    // Desktop displays the bounds stored in `data`, not the filter's
    slicer_objects(
        &mut s,
        &between,
        json!({"general": [{"properties": {"filter": {"filter": where_(gt)},
                                           "selfFilterEnabled": p("true")}}],
               "header": [{"properties": {"show": p("false")}}],
               "data": [{"properties": {"mode": p("'Between'"), "numericStart": p("10D"),
                                        "numericEnd": p("500D")}}]}),
    );
    let sl = &visual_json(&s.snapshot(), &between)["slicer"];
    assert_eq!(sl["range"], json!({"from": 10, "to": 500}));
    assert_eq!(sl["search_box"], json!(true));
    assert_eq!(sl["header_show"], json!(false));
    assert_eq!(sl["domain"], json!({"min": 5, "max": 1000}));
    // relative dates; a list slicer has no domain
    slicer_objects(
        &mut s,
        &between,
        json!({"data": [{"properties": {"mode": p("'Relative'"), "relativeRange": p("'Last'"),
                                        "relativeDuration": p("3D"),
                                        "relativePeriod": p("'Calendar Months'")}}]}),
    );
    let sl = &visual_json(&s.snapshot(), &between)["slicer"];
    let mut rel = sl["relative"].clone();
    // the covered days depend on today: three whole months before the current one
    let (from, to) = (
        rel["from"].as_str().unwrap_or(""),
        rel["to"].as_str().unwrap_or(""),
    );
    assert!(from.ends_with("-01") && to > from, "{rel}");
    if let Some(o) = rel.as_object_mut() {
        o.remove("from");
        o.remove("to");
    }
    assert_eq!(
        rel,
        json!({"range": "last", "duration": 3, "period": "calendar months"})
    );
    assert!(sl.get("domain").is_none());
}

#[test]
fn a_slicer_search_narrows_only_its_own_items() {
    let mut s = shop();
    let list = add_visual(
        &mut s,
        "slicer",
        json!([{"role": "Values", "field": "Sales[Product]"}]),
    );
    let card = add_visual(&mut s, "card", json!([{"role": "Values", "field": "[Total]"}]));
    let starts = json!({"StartsWith": {"Left": col("Sales", "Product"), "Right": lit("'b'")}});
    slicer_objects(
        &mut s,
        &list,
        json!({"general": [{"properties": {
            "filter": {"filter": where_(in_("Sales", "Product", &["'Ball'"]))},
            "selfFilter": {"filter": where_(starts)}}}]}),
    );
    let snap = s.snapshot();
    let v = visual_json(&snap, &list);
    assert_eq!(rows(&v["data"]), vec![json!(["Ball"]), json!(["Bike"])]);
    assert_eq!(
        v["slicer"],
        json!({"mode": "list", "selected": ["Ball"], "search": "b", "match": "starts_with"})
    );
    // the other visuals see the selection, not the search
    assert_eq!(rows(&visual_json(&snap, &card)["data"]), vec![json!([5])]);
}
