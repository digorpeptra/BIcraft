//! The reader's drill, matrix expansion, drillthrough and report-page tooltips (`view.interact`
//! drill_*, matrix_*, drillthrough, tooltip; `view.keep_drill`), on a synthetic model with a
//! literal table. Behaviour from Microsoft Learn's drill, drillthrough and tooltip pages
//! (documented, not measured in Desktop); the drilled values are checked against `dax.evaluate`.

use serde_json::{Value, json};

use bicraft_engine::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn act(s: &mut Session, p: Value) -> Value {
    run(s, "view.interact", p)
}

/// Sales(Category, Product, Amount): Wheels = 1150.5 (Bike 150.5, Car 1000), Toys = 12 (Ball 5,
/// Kite 7); Total = 1162.5.
fn shop() -> Session {
    let mut s = Session::new();
    let sales = r#"#table({"Category", "Product", "Amount"}, {{"Wheels", "Bike", 100}, {"Wheels", "Bike", 50.5}, {"Wheels", "Car", 1000}, {"Toys", "Ball", 5}, {"Toys", "Kite", 7}})"#;
    let steps = [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Category", "data_type": "string"}),
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
            "hierarchy.add",
            json!({"table": "Sales", "name": "Products", "levels": ["Category", "Product"]}),
        ),
        ("report.new", json!({})),
        ("page.add", json!({"display_name": "Main"})),
        ("page.add", json!({"display_name": "Detail"})),
        ("page.add", json!({"display_name": "Tip"})),
    ];
    for (id, p) in steps {
        run(&mut s, id, p);
    }
    s
}

fn add_visual(s: &mut Session, page: &str, visual_type: &str, fields: Value) -> String {
    let r = run(
        s,
        "visual.add",
        json!({"page": page, "visual_type": visual_type, "x": 0, "y": 0, "width": 300, "height": 200,
               "fields": fields}),
    );
    r.get("visual")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| panic!("no visual name in {r}"))
}

fn page_name(s: &Session, display: &str) -> String {
    s.report
        .as_ref()
        .and_then(|r| r.pages.iter().find(|p| p.display_name == display))
        .map(|p| p.name.clone())
        .unwrap_or_else(|| panic!("no page {display}"))
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

/// The visual's rows, sorted (the order is not what these tests check).
fn norm(v: Value) -> Value {
    match v {
        Value::Number(n) => json!(n.as_f64()),
        Value::Array(a) => Value::Array(a.into_iter().map(norm).collect()),
        other => other,
    }
}

fn rows(s: &Session, visual: &str) -> Vec<Value> {
    let d = visual_json(&s.snapshot(), visual)["data"].clone();
    assert_eq!(d["status"], "ok", "{d}");
    let mut r = d["rows"].as_array().cloned().unwrap_or_default();
    r = r.into_iter().map(norm).collect();
    r.sort_by_key(|x| x.to_string());
    r
}

/// `EVALUATE` rows as `[group…, value]` arrays, sorted.
fn dax(s: &mut Session, query: &str) -> Vec<Value> {
    let r = run(s, "dax.evaluate", json!({"query": query}));
    let t = &r["results"][0];
    let names: Vec<String> = t["columns"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c["name"].as_str().map(str::to_string))
        .collect();
    let mut out: Vec<Value> = t["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|row| norm(Value::Array(names.iter().map(|n| row[n].clone()).collect())))
        .collect();
    out.sort_by_key(|x| x.to_string());
    out
}

fn chart(s: &mut Session) -> (String, String) {
    let main = page_name(s, "Main");
    let v = add_visual(s, "Main", "clusteredColumnChart", json!([]));
    // a hierarchy on the axis as Desktop binds it: the first level active
    run(
        s,
        "visual.bind_hierarchy",
        json!({"page": main, "visual": v, "role": "Category", "table": "Sales", "hierarchy": "Products"}),
    );
    run(
        s,
        "visual.bind",
        json!({"page": main, "visual": v, "role": "Y", "field": "[Total]"}),
    );
    (main, v)
}

#[test]
fn drill_down_up_next_level_and_expand_follow_the_hierarchy() {
    let mut s = shop();
    let (main, v) = chart(&mut s);
    let stored = s.report.clone();
    let revision = s.revision;

    // the top level, as Desktop shows a new hierarchy (the first level active)
    let d = visual_json(&s.snapshot(), &v)["drill"].clone();
    assert_eq!(d["levels"], json!(["Category", "Product"]), "{d}");
    assert_eq!((d["top"].clone(), d["bottom"].clone()), (json!(0), json!(0)));
    assert_eq!(
        (d["can_up"].clone(), d["can_down"].clone()),
        (json!(false), json!(true))
    );
    let top = dax(
        &mut s,
        "EVALUATE SUMMARIZECOLUMNS(Sales[Category], \"T\", [Total])",
    );
    assert_eq!(rows(&s, &v), top);
    assert_eq!(top, vec![json!(["Toys", 12.0]), json!(["Wheels", 1150.5])]);

    // drill down on Wheels: its products only
    let r = act(
        &mut s,
        json!({"action": "drill_down", "page": main, "visual": v, "point": [{"c": 0, "v": "Wheels"}]}),
    );
    assert_eq!(r["changed"], true);
    let wheels = dax(
        &mut s,
        "EVALUATE CALCULATETABLE(SUMMARIZECOLUMNS(Sales[Product], \"T\", [Total]), Sales[Category] = \"Wheels\")",
    );
    assert_eq!(rows(&s, &v), wheels);
    assert_eq!(wheels, vec![json!(["Bike", 150.5]), json!(["Car", 1000.0])]);
    let d = visual_json(&s.snapshot(), &v)["drill"].clone();
    assert_eq!(d["path"], json!([{"level": 0, "value": "Wheels"}]), "{d}");
    assert_eq!(
        (d["can_up"].clone(), d["can_down"].clone()),
        (json!(true), json!(false))
    );
    // at the lowest level, no further drill down
    assert!(
        s.execute(
            "view.interact",
            &json!({"action": "drill_down", "page": main, "visual": v, "point": [{"c": 0, "v": "Car"}]})
        )
        .is_err()
    );

    // drill up: back to the categories
    act(&mut s, json!({"action": "drill_up", "page": main, "visual": v}));
    assert_eq!(rows(&s, &v), top);

    // go to the next level: every product, no filter
    act(&mut s, json!({"action": "drill_next", "page": main, "visual": v}));
    let products = dax(
        &mut s,
        "EVALUATE SUMMARIZECOLUMNS(Sales[Product], \"T\", [Total])",
    );
    assert_eq!(rows(&s, &v), products);
    assert_eq!(products.len(), 4);
    act(&mut s, json!({"action": "drill_up", "page": main, "visual": v}));

    // expand all down one level: both levels, grouped by both
    act(
        &mut s,
        json!({"action": "drill_expand", "page": main, "visual": v}),
    );
    let both = dax(
        &mut s,
        "EVALUATE SUMMARIZECOLUMNS(Sales[Category], Sales[Product], \"T\", [Total])",
    );
    assert_eq!(rows(&s, &v), both);
    let d = visual_json(&s.snapshot(), &v)["drill"].clone();
    assert_eq!((d["top"].clone(), d["bottom"].clone()), (json!(0), json!(1)));
    act(&mut s, json!({"action": "drill_up", "page": main, "visual": v}));
    assert_eq!(rows(&s, &v), top);
    // the top level has nothing above it
    assert!(
        s.execute(
            "view.interact",
            &json!({"action": "drill_up", "page": main, "visual": v})
        )
        .is_err()
    );

    // drill mode: a click drills instead of selecting; at the lowest level it selects
    act(&mut s, json!({"action": "drill_mode", "page": main, "visual": v}));
    let r = act(
        &mut s,
        json!({"action": "select", "page": main, "visual": v, "point": [{"c": 0, "v": "Toys"}]}),
    );
    assert_eq!(r["drilled"], true, "{r}");
    assert_eq!(rows(&s, &v), vec![json!(["Ball", 5.0]), json!(["Kite", 7.0])]);
    let r = act(
        &mut s,
        json!({"action": "select", "page": main, "visual": v, "point": [{"c": 0, "v": "Ball"}]}),
    );
    assert!(r.get("drilled").is_none(), "{r}");
    assert_eq!(r["state"]["selections"][0]["visual"], json!(v));

    // nothing reached the report
    assert_eq!(s.report, stored);
    assert_eq!(s.revision, revision);
    // a visual without a hierarchy has no drill
    let card = add_visual(
        &mut s,
        "Main",
        "card",
        json!([{"role": "Values", "field": "[Total]"}]),
    );
    assert!(visual_json(&s.snapshot(), &card).get("drill").is_none());
    assert!(
        s.execute(
            "view.interact",
            &json!({"action": "drill_next", "page": main, "visual": card})
        )
        .is_err()
    );
}

#[test]
fn drilling_filters_other_visuals_only_when_turned_on() {
    let mut s = shop();
    // a new report says "Drilling filters other visuals" (Desktop writes it on); turned off here
    run(
        &mut s,
        "report.settings",
        json!({"drill_filters_other_visuals": false}),
    );
    let (main, v) = chart(&mut s);
    let card = add_visual(
        &mut s,
        "Main",
        "card",
        json!([{"role": "Values", "field": "[Total]"}]),
    );
    act(
        &mut s,
        json!({"action": "drill_down", "page": main, "visual": v, "point": [{"c": 0, "v": "Wheels"}]}),
    );
    // the setting off: drilling does not filter the page
    assert_eq!(rows(&s, &card), vec![json!([1162.5])]);
    if let Some(x) = s
        .report
        .as_mut()
        .and_then(|r| r.pages.iter_mut().find(|p| p.name == main))
        .and_then(|p| p.visuals.iter_mut().find(|x| x.name == v))
    {
        x.visual_extra
            .insert("drillFilterOtherVisuals".into(), json!(true));
    }
    s.revision += 1;
    assert_eq!(rows(&s, &card), vec![json!([1150.5])]);
    // "Apply drill down filters to": the visual only, then the report's default
    run(
        &mut s,
        "visual.drill_filter",
        json!({"page": main, "visual": v, "apply_to": "visual"}),
    );
    assert_eq!(rows(&s, &card), vec![json!([1162.5])]);
    let r = run(
        &mut s,
        "visual.drill_filter",
        json!({"page": main, "visual": v, "apply_to": "default"}),
    );
    assert_eq!(r["filters_other_visuals"], false);
    assert_eq!(rows(&s, &card), vec![json!([1162.5])]);
    run(
        &mut s,
        "report.settings",
        json!({"drill_filters_other_visuals": true}),
    );
    assert_eq!(rows(&s, &card), vec![json!([1150.5])]);
    // a NoFilter interaction stops it
    let target = card.clone();
    if let Some(p) = s
        .report
        .as_mut()
        .and_then(|r| r.pages.iter_mut().find(|p| p.name == main))
    {
        p.extra.insert(
            "visualInteractions".into(),
            json!([{"source": v, "target": target, "type": "NoFilter"}]),
        );
    }
    s.revision += 1;
    assert_eq!(rows(&s, &card), vec![json!([1162.5])]);
}

#[test]
fn keep_drill_writes_desktops_fields_and_reads_them_back() {
    let mut s = shop();
    let (main, v) = chart(&mut s);
    act(
        &mut s,
        json!({"action": "drill_down", "page": main, "visual": v, "point": [{"c": 0, "v": "Wheels"}]}),
    );
    let drilled = rows(&s, &v);
    let r = run(&mut s, "view.keep_drill", json!({}));
    assert_eq!(r["kept"], json!([{"page": main, "visual": v}]), "{r}");
    assert!(s.report_dirty);
    let vis = s
        .report
        .as_ref()
        .and_then(|r| r.pages.iter().find(|p| p.name == main))
        .and_then(|p| p.visuals.iter().find(|x| x.name == v))
        .cloned()
        .unwrap_or_else(|| panic!("visual"));
    let ps = &vis.visual_extra["query"]["queryState"]["Category"]["projections"];
    assert_eq!(
        (ps[0]["active"].clone(), ps[1]["active"].clone()),
        (json!(false), json!(true))
    );
    let f = &vis.filter_config.as_ref().unwrap_or_else(|| panic!("filters"))["filters"][0];
    assert_eq!(f["howCreated"], "Drill");
    assert!(
        !f["name"].as_str().unwrap_or_default().starts_with("bicraft"),
        "{f}"
    );
    // the reader state is gone, the saved state shows the same thing
    assert_eq!(run(&mut s, "view.interactions", json!({}))["drills"], json!([]));
    assert_eq!(rows(&s, &v), drilled);
    let d = visual_json(&s.snapshot(), &v)["drill"].clone();
    assert_eq!(d["path"], json!([{"level": 0, "value": "Wheels"}]), "{d}");
    // and drilling up from the saved state works
    act(&mut s, json!({"action": "drill_up", "page": main, "visual": v}));
    assert_eq!(rows(&s, &v).len(), 2);
    assert_eq!(rows(&s, &v)[0], json!(["Toys", 12.0]));
}

#[test]
fn matrix_rows_expand_and_collapse() {
    let mut s = shop();
    let main = page_name(&s, "Main");
    let m = add_visual(&mut s, "Main", "pivotTable", json!([]));
    run(
        &mut s,
        "visual.bind_hierarchy",
        json!({"page": main, "visual": m, "role": "Rows", "table": "Sales", "hierarchy": "Products"}),
    );
    run(
        &mut s,
        "visual.bind",
        json!({"page": main, "visual": m, "role": "Values", "field": "[Total]"}),
    );
    let data = |s: &Session| visual_json(&s.snapshot(), &m)["data"].clone();
    assert!(data(&s).get("row_expansion").is_none());
    // +: one member, the next level appears with only that member open
    act(
        &mut s,
        json!({"action": "matrix_expand", "page": main, "visual": m, "path": ["Wheels"]}),
    );
    let d = data(&s);
    assert_eq!(
        d["row_expansion"],
        json!({"open": [false, false], "toggled": [["Wheels"]]}),
        "{d}"
    );
    let leaf: Vec<Value> = d["rows"].as_array().cloned().unwrap_or_default();
    assert_eq!(leaf.len(), 4, "{d}");
    // −: back to collapsed
    act(
        &mut s,
        json!({"action": "matrix_collapse", "page": main, "visual": m, "path": ["Wheels"]}),
    );
    assert_eq!(
        data(&s)["row_expansion"],
        json!({"open": [false, false], "toggled": []})
    );
    // entire level
    act(
        &mut s,
        json!({"action": "matrix_expand", "page": main, "visual": m, "level": 0}),
    );
    assert_eq!(
        data(&s)["row_expansion"],
        json!({"open": [true, false], "toggled": []})
    );
    act(
        &mut s,
        json!({"action": "matrix_collapse", "page": main, "visual": m, "path": ["Toys"]}),
    );
    assert_eq!(
        data(&s)["row_expansion"],
        json!({"open": [true, false], "toggled": [["Toys"]]})
    );
    act(
        &mut s,
        json!({"action": "matrix_collapse", "page": main, "visual": m, "all": true}),
    );
    assert_eq!(
        data(&s)["row_expansion"],
        json!({"open": [false, false], "toggled": []})
    );
    // a member deeper than the levels shown is refused; charts have no +/-
    assert!(
        s.execute(
            "view.interact",
            &json!({"action": "matrix_expand", "page": main, "visual": m, "path": ["Wheels", "Bike", "x"]})
        )
        .is_err()
    );
    let (_, c) = chart(&mut s);
    assert!(
        s.execute(
            "view.interact",
            &json!({"action": "matrix_expand", "page": main, "visual": c, "level": 0})
        )
        .is_err()
    );
    // drill on a matrix: up removes the level the expansion added, next level shows the products
    act(&mut s, json!({"action": "drill_up", "page": main, "visual": m}));
    assert_eq!(rows(&s, &m).len(), 2);
    act(&mut s, json!({"action": "drill_next", "page": main, "visual": m}));
    assert_eq!(rows(&s, &m).len(), 4);
}

/// A drillthrough page on Sales[Category] with a card and a Back button; a slicer on Main.
fn with_drillthrough(s: &mut Session, keep_all: bool) -> (String, String, String, String) {
    let (main, v) = chart(s);
    let detail = page_name(s, "Detail");
    run(
        s,
        "page.set_type",
        json!({"page": detail, "type": "drillthrough", "fields": ["Sales[Product]"],
               "keep_all_filters": keep_all, "back_button": true}),
    );
    let card = add_visual(
        s,
        "Detail",
        "card",
        json!([{"role": "Values", "field": "[Total]"}]),
    );
    (main, v, detail, card)
}

#[test]
fn drillthrough_opens_the_page_filtered_and_back_returns() {
    let mut s = shop();
    let (main, v, detail, card) = with_drillthrough(&mut s, true);
    // the chart at its top level shows no product: no drillthrough from it
    let snap = s.snapshot();
    assert!(
        visual_json(&snap, &v)["interaction"]
            .get("drillthrough")
            .is_none()
    );
    act(&mut s, json!({"action": "drill_next", "page": main, "visual": v}));
    let snap = s.snapshot();
    let dt = visual_json(&snap, &v)["interaction"]["drillthrough"].clone();
    assert_eq!(dt[0]["page"], json!(detail), "{dt}");
    assert_eq!(dt[0]["columns"], json!([0]));
    assert_eq!(dt[0]["cross_report"], false);

    let r = act(
        &mut s,
        json!({"action": "drillthrough", "page": main, "visual": v, "point": [{"c": 0, "v": "Car"}],
               "target": detail}),
    );
    assert_eq!(r["page"], json!(detail));
    let snap = s.snapshot();
    assert_eq!(snap["report"]["active_page"], json!(detail));
    assert_eq!(snap["interaction"]["can_go_back"], true);
    assert_eq!(rows(&s, &card), vec![json!([1000.0])]);
    // the Back button returns to the source page
    let back = s
        .report
        .as_ref()
        .and_then(|r| r.pages.iter().find(|p| p.name == detail))
        .and_then(|p| {
            p.visuals
                .iter()
                .find(|x| x.visual_type.as_deref() == Some("actionButton"))
        })
        .map(|x| x.name.clone())
        .unwrap_or_else(|| panic!("back button"));
    act(
        &mut s,
        json!({"action": "button", "page": detail, "visual": back}),
    );
    assert_eq!(s.snapshot()["report"]["active_page"], json!(main));
    // a point without the drillthrough field, and a page that is not one, are refused
    act(&mut s, json!({"action": "drill_up", "page": main, "visual": v}));
    assert!(
        s.execute(
            "view.interact",
            &json!({"action": "drillthrough", "page": main, "visual": v, "point": [{"c": 0, "v": "Toys"}],
                    "target": detail})
        )
        .is_err()
    );
    assert!(
        s.execute(
            "view.interact",
            &json!({"action": "drillthrough", "page": main, "visual": v, "point": [{"c": 0, "v": "Toys"}],
                    "target": main})
        )
        .is_err()
    );
}

#[test]
fn keep_all_filters_carries_the_source_visuals_filters() {
    for keep in [true, false] {
        let mut s = shop();
        let (main, v) = chart(&mut s);
        let detail = page_name(&s, "Detail");
        run(
            &mut s,
            "page.set_type",
            json!({"page": detail, "type": "drillthrough", "fields": ["Sales[Category]"], "keep_all_filters": keep}),
        );
        let card = add_visual(
            &mut s,
            "Detail",
            "card",
            json!([{"role": "Values", "field": "[Total]"}]),
        );
        // a slicer on Main keeps Bike: the chart shows Wheels = 150.5
        let slicer = add_visual(
            &mut s,
            "Main",
            "slicer",
            json!([{"role": "Values", "field": "Sales[Product]"}]),
        );
        act(
            &mut s,
            json!({"action": "slicer_toggle", "page": main, "visual": slicer, "value": "Bike"}),
        );
        assert_eq!(rows(&s, &v), vec![json!(["Wheels", 150.5])]);
        act(
            &mut s,
            json!({"action": "drillthrough", "page": main, "visual": v, "point": [{"c": 0, "v": "Wheels"}],
                   "target": detail}),
        );
        // kept: Wheels and Bike; not kept: Wheels only
        let want = if keep { 150.5 } else { 1150.5 };
        assert_eq!(rows(&s, &card), vec![json!([want])], "keep_all {keep}");
        let st = run(&mut s, "view.interactions", json!({}));
        let kept = st["drillthrough"][0]["kept_filters"].as_u64().unwrap_or(0);
        assert_eq!(kept > 0, keep, "keep_all {keep}: {st}");
    }
}

#[test]
fn cross_report_drillthrough_is_refused() {
    let mut s = shop();
    let (main, v) = chart(&mut s);
    let detail = page_name(&s, "Detail");
    run(
        &mut s,
        "page.set_type",
        json!({"page": detail, "type": "drillthrough", "fields": ["Sales[Category]"], "cross_report": true}),
    );
    let snap = s.snapshot();
    assert_eq!(
        visual_json(&snap, &v)["interaction"]["drillthrough"][0]["cross_report"],
        true
    );
    let e = s
        .execute(
            "view.interact",
            &json!({"action": "drillthrough", "page": main, "visual": v, "point": [{"c": 0, "v": "Toys"}],
                    "target": detail}),
        )
        .err()
        .unwrap_or_default();
    assert!(e.contains("cross-report"), "{e}");
}

#[test]
fn report_page_tooltips_get_the_hovered_points_context() {
    let mut s = shop();
    let (main, v) = chart(&mut s);
    let tip = page_name(&s, "Tip");
    run(&mut s, "page.set_type", json!({"page": tip, "type": "tooltip"}));
    let card = add_visual(
        &mut s,
        "Tip",
        "card",
        json!([{"role": "Values", "field": "[Total]"}]),
    );
    // no tooltip page set and no tooltip fields: the default tooltip
    assert!(
        visual_json(&s.snapshot(), &v)["interaction"]
            .get("tooltip_page")
            .is_none()
    );
    run(
        &mut s,
        "visual.tooltip_page",
        json!({"page": main, "visual": v, "tooltip_page": tip}),
    );
    assert_eq!(
        visual_json(&s.snapshot(), &v)["interaction"]["tooltip_page"],
        json!(tip)
    );
    assert_eq!(rows(&s, &card), vec![json!([1162.5])]);
    act(
        &mut s,
        json!({"action": "tooltip", "page": main, "visual": v, "point": [{"c": 0, "v": "Toys"}]}),
    );
    let snap = s.snapshot();
    assert_eq!(snap["interaction"]["tooltip"]["target"], json!(tip));
    assert_eq!(rows(&s, &card), vec![json!([12.0])]);
    // the same point again changes nothing; null hides it
    let r = act(
        &mut s,
        json!({"action": "tooltip", "page": main, "visual": v, "point": [{"c": 0, "v": "Toys"}]}),
    );
    assert_eq!(r["changed"], false);
    act(
        &mut s,
        json!({"action": "tooltip", "page": main, "visual": v, "point": null}),
    );
    assert_eq!(rows(&s, &card), vec![json!([1162.5])]);
    assert_eq!(s.snapshot()["interaction"]["tooltip"], Value::Null);

    // "Auto": a visual that uses a tooltip field of the page shows it without being told
    run(
        &mut s,
        "page.set_type",
        json!({"page": tip, "type": "tooltip", "fields": ["Sales[Category]"]}),
    );
    let other = add_visual(
        &mut s,
        "Main",
        "clusteredBarChart",
        json!([{"role": "Category", "field": "Sales[Category]"}, {"role": "Y", "field": "[Total]"}]),
    );
    assert_eq!(
        visual_json(&s.snapshot(), &other)["interaction"]["tooltip_page"],
        json!(tip)
    );
    let none = add_visual(
        &mut s,
        "Main",
        "clusteredBarChart",
        json!([{"role": "Category", "field": "Sales[Product]"}, {"role": "Y", "field": "[Total]"}]),
    );
    assert!(
        visual_json(&s.snapshot(), &none)["interaction"]
            .get("tooltip_page")
            .is_none()
    );
    // a visual without a tooltip page refuses the action
    assert!(
        s.execute(
            "view.interact",
            &json!({"action": "tooltip", "page": main, "visual": none, "point": [{"c": 0, "v": "Bike"}]})
        )
        .is_err()
    );
}

#[test]
fn a_drillthrough_button_drills_through_the_selected_point() {
    let mut s = shop();
    let (main, v) = chart(&mut s);
    let detail = page_name(&s, "Detail");
    run(
        &mut s,
        "page.set_type",
        json!({"page": detail, "type": "drillthrough", "fields": ["Sales[Category]"]}),
    );
    let card = add_visual(
        &mut s,
        "Detail",
        "card",
        json!([{"role": "Values", "field": "[Total]"}]),
    );
    let button = add_visual(&mut s, "Main", "actionButton", json!([]));
    let lit = |x: &str| json!({"expr": {"Literal": {"Value": x}}});
    if let Some(b) = s
        .report
        .as_mut()
        .and_then(|r| r.pages.iter_mut().find(|p| p.name == main))
        .and_then(|p| p.visuals.iter_mut().find(|x| x.name == button))
    {
        b.visual_extra.insert(
            "visualContainerObjects".into(),
            json!({"visualLink": [{"properties": {"show": lit("true"), "type": lit("'Drillthrough'"),
                                                   "drillthroughSection": lit(&format!("'{detail}'"))}}]}),
        );
    }
    s.revision += 1;
    // without a selected point the button does nothing (Desktop greys it)
    assert!(
        s.execute(
            "view.interact",
            &json!({"action": "button", "page": main, "visual": button})
        )
        .is_err()
    );
    act(
        &mut s,
        json!({"action": "select", "page": main, "visual": v, "point": [{"c": 0, "v": "Toys"}]}),
    );
    let r = act(
        &mut s,
        json!({"action": "button", "page": main, "visual": button}),
    );
    assert_eq!(r["button"], "Drillthrough");
    assert_eq!(s.snapshot()["report"]["active_page"], json!(detail));
    assert_eq!(rows(&s, &card), vec![json!([12.0])]);
}

/// A bookmark captures the reader's drill (G55): the visual's active levels in
/// `singleVisual.activeProjections` and the drilled value in a `filters.byExpr` entry made by a
/// drill (`howCreated` 2); applying it restores the drill in the reader and with
/// `bookmark.apply`, and a bookmark taken without a drill resets it.
#[test]
fn a_bookmark_captures_and_restores_the_drill() {
    let mut s = shop();
    let (main, v) = chart(&mut s);
    let top = rows(&s, &v);
    let plain = run(&mut s, "bookmark.capture", json!({"display_name": "Top"}))["bookmark"]
        .as_str()
        .unwrap()
        .to_string();
    act(
        &mut s,
        json!({"action": "drill_down", "page": main, "visual": v, "point": [{"c": 0, "v": "Wheels"}]}),
    );
    let wheels = rows(&s, &v);
    assert_ne!(wheels, top);
    let bm = run(&mut s, "bookmark.capture", json!({"display_name": "Wheels"}))["bookmark"]
        .as_str()
        .unwrap()
        .to_string();
    let doc = s
        .report
        .as_ref()
        .unwrap()
        .bookmarks
        .last()
        .unwrap()
        .document
        .clone();
    let c = doc["explorationState"]["sections"][&main]["visualContainers"][&v].clone();
    let active = c["singleVisual"]["activeProjections"].clone();
    assert!(active.as_object().is_some_and(|o| !o.is_empty()), "{c}");
    assert!(active.to_string().contains("Product"), "{active}");
    let f = &c["filters"]["byExpr"][0];
    assert_eq!(f["howCreated"], 2, "{c}");
    assert!(f["filter"].to_string().contains("Wheels"), "{f}");

    // back to the top, then the bookmark drills again
    act(&mut s, json!({"action": "drill_up", "page": main, "visual": v}));
    assert_eq!(rows(&s, &v), top);
    act(&mut s, json!({"action": "bookmark", "bookmark": bm}));
    assert_eq!(rows(&s, &v), wheels, "view.interact bookmark");
    let d = visual_json(&s.snapshot(), &v)["drill"].clone();
    assert_eq!(d["path"], json!([{"level": 0, "value": "Wheels"}]), "{d}");

    // a bookmark taken at the top level drills up
    act(&mut s, json!({"action": "bookmark", "bookmark": plain}));
    assert_eq!(rows(&s, &v), top, "the plain bookmark resets the drill");

    // bookmark.apply restores it too
    run(&mut s, "bookmark.apply", json!({"bookmark": bm}));
    assert_eq!(rows(&s, &v), wheels, "bookmark.apply");
    run(&mut s, "bookmark.apply", json!({"bookmark": plain}));
    assert_eq!(rows(&s, &v), top);
}

/// A bookmark captures a visual's drill mode (G55): turned on, it is written as BIcraft's own
/// formatting-object update (`singleVisual.objects.merge.bicraftDrill`, valid against the
/// public bookmark schema), and applying the bookmark turns it back on, in the reader and with
/// `bookmark.apply`; a bookmark taken with it off turns it off.
#[test]
fn a_bookmark_round_trips_the_drill_mode() {
    let mut s = shop();
    let (main, v) = chart(&mut s);
    let mode = |s: &Session| visual_json(&s.snapshot(), &v)["drill"]["mode"].clone();
    let off = run(&mut s, "bookmark.capture", json!({"display_name": "Off"}))["bookmark"]
        .as_str()
        .unwrap()
        .to_string();
    act(
        &mut s,
        json!({"action": "drill_mode", "page": main, "visual": v, "on": true}),
    );
    assert_eq!(mode(&s), json!(true));
    let on = run(&mut s, "bookmark.capture", json!({"display_name": "On"}))["bookmark"]
        .as_str()
        .unwrap()
        .to_string();
    let report = s.report.as_ref().unwrap();
    let doc = &report.bookmarks.last().unwrap().document;
    let c = &doc["explorationState"]["sections"][&main]["visualContainers"][&v];
    assert_eq!(
        c.pointer("/singleVisual/objects/merge/bicraftDrill/0/properties/drillMode/expr/Literal/Value"),
        Some(&json!("true")),
        "{c}"
    );
    let issues: Vec<_> = bicraft_pbir::validate_documents(&bicraft_pbir::write_documents(report))
        .into_iter()
        .filter(|i| i.path.contains("bookmarks"))
        .collect();
    assert!(issues.is_empty(), "{issues:?}");

    // off again; the bookmark turns it on, the other one off
    act(
        &mut s,
        json!({"action": "drill_mode", "page": main, "visual": v, "on": false}),
    );
    assert_eq!(mode(&s), json!(false));
    act(&mut s, json!({"action": "bookmark", "bookmark": on}));
    assert_eq!(mode(&s), json!(true), "view.interact bookmark");
    act(&mut s, json!({"action": "bookmark", "bookmark": off}));
    assert_eq!(mode(&s), json!(false), "the bookmark without it");
    run(&mut s, "bookmark.apply", json!({"bookmark": on}));
    assert_eq!(mode(&s), json!(true), "bookmark.apply");
}
