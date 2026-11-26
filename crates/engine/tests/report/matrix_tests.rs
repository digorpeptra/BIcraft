//! A matrix's subtotals at every level of its row and column hierarchies, evaluated by the
//! measure in each subtotal's filter context (not added up), on a synthetic model whose
//! expected values are worked out by hand in the comments. Behaviour from Microsoft Learn's
//! "Create a matrix visual in Power BI" (subtotals per row and column level) and the public
//! PBIR `subTotals` object; the shapes Desktop saves are in docs/formats/pbir.md, "Matrix
//! expansion and per-level subtotals".

use serde_json::{Value, json};

use bicraft_engine::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Sales(Region, Category, Product, Channel, Amount, Cost) with `[Total]` = SUM(Amount) and
/// `[Margin]` = (SUM(Amount) - SUM(Cost)) / SUM(Amount):
///
/// | Region | Category | Product  | Channel | Amount | Cost |
/// |--------|----------|----------|---------|--------|------|
/// | North  | Bikes    | Road     | Web     | 100    | 60   |
/// | North  | Bikes    | Road     | Shop    | 50     | 20   |
/// | North  | Bikes    | Mountain | Web     | 200    | 150  |
/// | North  | Toys     | Ball     | Shop    | 10     | 5    |
/// | South  | Bikes    | Road     | Web     | 300    | 100  |
/// | South  | Toys     | Kite     | Web     | 40     | 30   |
/// | South  | Toys     | Kite     | Shop    | 60     | 30   |
pub(crate) fn shop() -> Session {
    let mut s = Session::new();
    let sales = r#"#table({"Region", "Category", "Product", "Channel", "Amount", "Cost"}, {
        {"North", "Bikes", "Road", "Web", 100, 60}, {"North", "Bikes", "Road", "Shop", 50, 20},
        {"North", "Bikes", "Mountain", "Web", 200, 150}, {"North", "Toys", "Ball", "Shop", 10, 5},
        {"South", "Bikes", "Road", "Web", 300, 100}, {"South", "Toys", "Kite", "Web", 40, 30},
        {"South", "Toys", "Kite", "Shop", 60, 30}})"#;
    run(&mut s, "model.new", json!({"name": "Shop"}));
    run(
        &mut s,
        "table.add",
        json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
    );
    for (c, ty) in [
        ("Region", "string"),
        ("Category", "string"),
        ("Product", "string"),
        ("Channel", "string"),
        ("Amount", "double"),
        ("Cost", "double"),
    ] {
        run(
            &mut s,
            "column.add",
            json!({"table": "Sales", "name": c, "data_type": ty}),
        );
    }
    for (name, expr) in [
        ("Total", "SUM(Sales[Amount])"),
        (
            "Margin",
            "DIVIDE(SUM(Sales[Amount]) - SUM(Sales[Cost]), SUM(Sales[Amount]))",
        ),
    ] {
        run(
            &mut s,
            "measure.add",
            json!({"table": "Sales", "name": name, "expression": expr}),
        );
    }
    run(&mut s, "report.new", json!({}));
    s
}

pub(crate) fn main_page(s: &Session) -> String {
    s.report
        .as_ref()
        .and_then(|r| r.pages.first())
        .map(|p| p.name.clone())
        .unwrap_or_else(|| panic!("no page"))
}

/// A matrix on `rows` and `columns` (column names of Sales) with Total and Margin.
pub(crate) fn matrix(s: &mut Session, rows: &[&str], columns: &[&str]) -> String {
    let page = main_page(s);
    let mut fields: Vec<Value> = rows
        .iter()
        .map(|c| json!({"role": "Rows", "field": format!("Sales[{c}]")}))
        .collect();
    fields.extend(
        columns
            .iter()
            .map(|c| json!({"role": "Columns", "field": format!("Sales[{c}]")})),
    );
    fields.push(json!({"role": "Values", "field": "[Total]"}));
    fields.push(json!({"role": "Values", "field": "[Margin]"}));
    let r = run(
        s,
        "visual.add",
        json!({"page": page, "visual_type": "pivotTable", "x": 0, "y": 0, "width": 600, "height": 400,
               "fields": fields}),
    );
    r["visual"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| panic!("no visual in {r}"))
}

pub(crate) fn data(s: &Session, visual: &str) -> Value {
    let snap = s.snapshot();
    for p in snap["report"]["pages"].as_array().cloned().unwrap_or_default() {
        for v in p["visuals"].as_array().cloned().unwrap_or_default() {
            if v["name"] == visual {
                assert_eq!(v["data"]["status"], "ok", "{}", v["data"]);
                return v["data"].clone();
            }
        }
    }
    panic!("visual {visual} not in the snapshot")
}

/// The subtotal level keeping `rows` row fields and `columns` column fields.
fn level(d: &Value, rows: u64, columns: u64) -> Vec<Value> {
    d["subtotals"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|l| l["rows"] == rows && l["columns"] == columns)
        .and_then(|l| l["data"].as_array().cloned())
        .unwrap_or_else(|| panic!("level ({rows}, {columns}) missing: {d}"))
}

/// The row of `rows` whose leading cells are `key` (null for the dropped grouping fields).
fn find<'a>(rows: &'a [Value], key: &[Value]) -> &'a Value {
    rows.iter()
        .find(|r| key.iter().enumerate().all(|(i, k)| r[i] == *k))
        .unwrap_or_else(|| panic!("no row {key:?} in {rows:?}"))
}

fn close(x: &Value, want: f64) {
    let got = x.as_f64().unwrap_or(f64::NAN);
    assert!((got - want).abs() < 1e-9, "{got} != {want}");
}

#[test]
fn subtotals_at_every_row_and_column_level_are_the_measure_in_their_context() {
    let mut s = shop();
    let page = main_page(&s);
    let m = matrix(&mut s, &["Region", "Category", "Product"], &["Channel"]);
    // every row level shown, expanded (Expand > All)
    run(
        &mut s,
        "view.interact",
        json!({"action": "matrix_expand", "page": page, "visual": m, "all": true}),
    );
    let d = data(&s, &m);
    assert_eq!(
        d["row_expansion"],
        json!({"open": [true, true, false], "toggled": []}),
        "{d}"
    );
    let n = Value::Null;
    let t = |x: &str| json!(x);
    // fields: Region, Category, Product, Channel, Total, Margin
    // grand total: 760; margin (760 - 395) / 760
    let g = level(&d, 0, 0);
    close(&g[0][4], 760.0);
    close(&g[0][5], 365.0 / 760.0);
    // North: 360, (360 - 235) / 360 = 0.3472…, not the sum (0.8429) nor the mean (0.4214) of
    // its children's margins (Bikes 120/350, Toys 5/10)
    let r1 = level(&d, 1, 0);
    let north = find(&r1, &[t("North"), n.clone(), n.clone(), n.clone()]);
    close(&north[4], 360.0);
    close(&north[5], 125.0 / 360.0);
    let south = find(&r1, &[t("South")]);
    close(&south[4], 400.0);
    close(&south[5], 240.0 / 400.0);
    // Region > Category: North Bikes 350, 120/350; South Toys 100, 40/100
    let r2 = level(&d, 2, 0);
    let nb = find(&r2, &[t("North"), t("Bikes"), n.clone()]);
    close(&nb[4], 350.0);
    close(&nb[5], 120.0 / 350.0);
    let st = find(&r2, &[t("South"), t("Toys")]);
    close(&st[5], 40.0 / 100.0);
    // Region > Category > Product over every channel: North Bikes Road 150, 70/150
    let r3 = level(&d, 3, 0);
    let road = find(&r3, &[t("North"), t("Bikes"), t("Road"), n.clone()]);
    close(&road[4], 150.0);
    close(&road[5], 70.0 / 150.0);
    // the column grand totals: Web 640, 300/640; Shop 120, 65/120
    let c0 = level(&d, 0, 1);
    let web = find(&c0, &[n.clone(), n.clone(), n.clone(), t("Web")]);
    close(&web[4], 640.0);
    close(&web[5], 300.0 / 640.0);
    let shop_ = find(&c0, &[n.clone(), n.clone(), n.clone(), t("Shop")]);
    close(&shop_[5], 65.0 / 120.0);
    // a row subtotal per column: North x Web 300, 90/300; North x Shop 60, 35/60
    let c1 = level(&d, 1, 1);
    close(
        &find(&c1, &[t("North"), n.clone(), n.clone(), t("Web")])[5],
        90.0 / 300.0,
    );
    close(
        &find(&c1, &[t("North"), n.clone(), n.clone(), t("Shop")])[5],
        35.0 / 60.0,
    );
    // North Bikes x Shop: 50, 30/50
    let c2 = level(&d, 2, 1);
    let nbs = find(&c2, &[t("North"), t("Bikes"), n.clone(), t("Shop")]);
    close(&nbs[4], 50.0);
    close(&nbs[5], 30.0 / 50.0);
    // the leaf level (every field) is the visual's own rows, not repeated
    assert!(
        !d["subtotals"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|l| l["rows"] == 3 && l["columns"] == 1),
        "{d}"
    );
    // the matrix's own rows agree: North Bikes Road x Web 100, 40/100
    let leaf = d["rows"].as_array().cloned().unwrap_or_default();
    let nbrw = find(&leaf, &[t("North"), t("Bikes"), t("Road"), t("Web")]);
    close(&nbrw[5], 40.0 / 100.0);
}

#[test]
fn five_level_hierarchies_get_a_subtotal_per_level() {
    // regression: the evaluator stopped at four levels per hierarchy, so the fifth level's
    // parents (Region > … > Channel) had no subtotal level of their own
    let mut s = shop();
    let page = main_page(&s);
    let m = matrix(&mut s, &["Region", "Category", "Product", "Channel", "Cost"], &[]);
    run(
        &mut s,
        "view.interact",
        json!({"action": "matrix_expand", "page": page, "visual": m, "all": true}),
    );
    let d = data(&s, &m);
    assert_eq!(
        d["row_expansion"]["open"],
        json!([true, true, true, true, false]),
        "{d}"
    );
    let t = |x: &str| json!(x);
    // North Bikes Road Web: 100 (one row; Cost 60)
    let r4 = level(&d, 4, 0);
    let w = find(&r4, &[t("North"), t("Bikes"), t("Road"), t("Web"), Value::Null]);
    close(&w[5], 100.0);
    close(&w[6], 40.0 / 100.0);
    // South Toys Kite: Web and Shop, 100, (100 - 60) / 100
    let r3 = level(&d, 3, 0);
    let k = find(&r3, &[t("South"), t("Toys"), t("Kite"), Value::Null]);
    close(&k[5], 100.0);
    close(&k[6], 0.4);
}

#[test]
fn per_level_subtotal_switches_labels_and_position_are_written_as_desktop_saves_them() {
    // the Format pane's Row subtotals card: Per row level on, the grand total (the entry of the
    // top field, Region) off, the Region subtotals (the entry of Category) labelled, at the bottom
    let mut s = shop();
    let page = main_page(&s);
    let m = matrix(&mut s, &["Region", "Category"], &[]);
    let d = data(&s, &m);
    assert_eq!(
        d["row_level_refs"],
        json!(["Sales.Region", "Sales.Category"]),
        "{d}"
    );
    assert!(d.get("row_level_totals").is_none(), "{d}");
    run(
        &mut s,
        "visual.format",
        json!({"page": page, "visual": m, "objects": [
            {"object": "subTotals", "property": "perRowLevel", "value": true},
            {"object": "subTotals", "property": "rowSubtotalsPosition", "value": "Bottom"},
            {"object": "subTotals", "property": "levelSubtotalEnabled", "value": false,
             "selector": {"metadata": "Sales.Region"}},
            {"object": "subTotals", "property": "levelSubtotalLabel", "value": "Sous-cat",
             "selector": {"metadata": "Sales.Category"}}]}),
    );
    let v = stored(&s, &m);
    let entries = v.visual_extra["objects"]["subTotals"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let lit = |x: &str| json!({"expr": {"Literal": {"Value": x}}});
    assert!(
        entries.contains(&json!({"properties": {"levelSubtotalEnabled": lit("false")},
                                 "selector": {"metadata": "Sales.Region"}})),
        "{entries:?}"
    );
    assert!(
        entries.contains(&json!({"properties": {"levelSubtotalLabel": lit("'Sous-cat'")},
                                 "selector": {"metadata": "Sales.Category"}})),
        "{entries:?}"
    );
    let snap = s.snapshot();
    let sv = snap["report"]["pages"][0]["visuals"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|x| x["name"] == m.as_str())
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        sv["data"]["row_level_totals"],
        json!([{"show": false}, {"label": "Sous-cat"}]),
        "{sv}"
    );
    assert_eq!(sv["format"]["table"]["row_subtotals_position"], "bottom", "{sv}");
    // the values stay the measure's: North 360 at the Region level (the top level shown; the
    // data keeps the Category rows under it)
    close(&find(&level(&sv["data"], 1, 0), &[json!("North")])[2], 360.0);
}

/// The matrix as the report stores it.
fn stored(s: &Session, visual: &str) -> bicraft_report::Visual {
    s.report
        .as_ref()
        .and_then(|r| r.pages.first())
        .and_then(|p| p.visuals.iter().find(|v| v.name == visual))
        .cloned()
        .unwrap_or_else(|| panic!("no visual {visual}"))
}

fn active(v: &bicraft_report::Visual, role: &str) -> Value {
    let ps = v.visual_extra["query"]["queryState"][role]["projections"].clone();
    Value::Array(
        ps.as_array()
            .into_iter()
            .flatten()
            .map(|p| p.get("active").cloned().unwrap_or(Value::Null))
            .collect(),
    )
}

fn expand(s: &mut Session, page: &str, visual: &str, extra: Value) -> Value {
    let mut p = json!({"page": page, "visual": visual});
    if let (Some(o), Value::Object(e)) = (p.as_object_mut(), extra) {
        o.extend(e);
    }
    run(s, "visual.expand", p)
}

#[test]
fn plus_on_a_member_writes_desktops_expansion_state_and_undoes() {
    let mut s = shop();
    let page = main_page(&s);
    let m = matrix(&mut s, &["Region", "Category", "Product"], &[]);
    s.report_dirty = false;
    // + on North: Category becomes shown, only North's open (Desktop's "Expand > Selection")
    let r = expand(&mut s, &page, &m, json!({"path": ["North"]}));
    assert_eq!(r["changed"], true, "{r}");
    assert!(s.report_dirty);
    let v = stored(&s, &m);
    assert_eq!(active(&v, "Rows"), json!([true, true, false]));
    assert_eq!(
        v.visual_extra["expansionStates"],
        json!([{"roles": ["Rows"],
                "levels": [{"queryRefs": ["Sales.Region"], "isCollapsed": true, "isPinned": true},
                           {"queryRefs": ["Sales.Category"], "isCollapsed": true, "isPinned": true}],
                "root": {"children": [{"identityValues": [{"Literal": {"Value": "'North'"}}], "isToggled": true}]}}])
    );
    let d = data(&s, &m);
    assert_eq!(
        d["row_expansion"],
        json!({"open": [false, false], "toggled": [["North"]]}),
        "{d}"
    );
    // the shown levels are Region and Category (the matrix's rows); North's two categories are
    // 350 and 10, South stays collapsed at 400
    assert_eq!(d["columns"].as_array().map(Vec::len), Some(4), "{d}");
    let r2 = d["rows"].as_array().cloned().unwrap_or_default();
    let t = |x: &str| json!(x);
    close(&find(&r2, &[t("North"), t("Bikes")])[2], 350.0);
    close(&find(&r2, &[t("North"), t("Toys")])[2], 10.0);
    close(&find(&level(&d, 1, 0), &[t("South")])[2], 400.0);
    // − on North: the toggle goes, the level stays shown
    expand(&mut s, &page, &m, json!({"path": ["North"], "collapse": true}));
    assert_eq!(
        data(&s, &m)["row_expansion"],
        json!({"open": [false, false], "toggled": []})
    );
    // undo twice: the matrix as it was, collapsed to Region without any state
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(data(&s, &m)["row_expansion"]["toggled"], json!([["North"]]));
    run(&mut s, "edit.undo", json!({}));
    let v = stored(&s, &m);
    assert!(
        v.visual_extra.get("expansionStates").is_none(),
        "{:?}",
        v.visual_extra
    );
    assert!(data(&s, &m).get("row_expansion").is_none());
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(data(&s, &m)["row_expansion"]["toggled"], json!([["North"]]));
}

#[test]
fn entire_level_all_and_the_header_steps_edit_the_report() {
    let mut s = shop();
    let page = main_page(&s);
    let m = matrix(&mut s, &["Region", "Category", "Product"], &[]);
    // Expand > Entire level on Region
    expand(&mut s, &page, &m, json!({"level": 0}));
    assert_eq!(
        data(&s, &m)["row_expansion"],
        json!({"open": [true, false], "toggled": []})
    );
    // Collapse > Selection on South: an exception of the open level
    expand(&mut s, &page, &m, json!({"path": ["South"], "collapse": true}));
    assert_eq!(
        data(&s, &m)["row_expansion"],
        json!({"open": [true, false], "toggled": [["South"]]})
    );
    // Expand > All: every level shown and open
    expand(&mut s, &page, &m, json!({"all": true}));
    assert_eq!(active(&stored(&s, &m), "Rows"), json!([true, true, true]));
    assert_eq!(
        data(&s, &m)["row_expansion"],
        json!({"open": [true, true, false], "toggled": []})
    );
    // Drill up: the deepest level goes
    expand(&mut s, &page, &m, json!({"step": "up"}));
    assert_eq!(active(&stored(&s, &m), "Rows"), json!([true, true, false]));
    assert_eq!(
        data(&s, &m)["row_expansion"],
        json!({"open": [true, false], "toggled": []})
    );
    // Collapse > All: the top level only
    expand(&mut s, &page, &m, json!({"all": true, "collapse": true}));
    assert_eq!(
        data(&s, &m)["row_expansion"],
        json!({"open": [false, false], "toggled": []})
    );
    expand(&mut s, &page, &m, json!({"step": "up"}));
    assert_eq!(active(&stored(&s, &m), "Rows"), json!([true, false, false]));
    // Go to the next level: Category alone (drilled), no expansion state left
    expand(&mut s, &page, &m, json!({"step": "next"}));
    let v = stored(&s, &m);
    assert_eq!(active(&v, "Rows"), json!([false, true, false]));
    assert_eq!(v.visual_extra["expansionStates"], json!([]));
    // Bikes 650 (North 350 + South 300), Toys 110: the leaf rows at the Category level
    let rows = data(&s, &m)["rows"].as_array().cloned().unwrap_or_default();
    assert_eq!(rows.len(), 2, "{rows:?}");
    // Expand all down one level: Category and Product, every Category open
    expand(&mut s, &page, &m, json!({"step": "expand_all"}));
    assert_eq!(active(&stored(&s, &m), "Rows"), json!([false, true, true]));
    assert_eq!(
        data(&s, &m)["row_expansion"],
        json!({"open": [true, false], "toggled": []})
    );
}

#[test]
fn column_hierarchies_expand_by_role() {
    let mut s = shop();
    let page = main_page(&s);
    let m = matrix(&mut s, &["Region"], &["Channel", "Category"]);
    expand(&mut s, &page, &m, json!({"role": "Columns", "path": ["Web"]}));
    let v = stored(&s, &m);
    assert_eq!(active(&v, "Columns"), json!([true, true]));
    assert_eq!(v.visual_extra["expansionStates"][0]["roles"], json!(["Columns"]));
    let d = data(&s, &m);
    assert_eq!(
        d["column_expansion"],
        json!({"open": [false, false], "toggled": [["Web"]]}),
        "{d}"
    );
    // North x Web x Bikes: 300, (300 - 210) / 300
    let t = |x: &str| json!(x);
    let leaf = d["rows"].as_array().cloned().unwrap_or_default();
    let nwb = find(&leaf, &[t("North"), t("Web"), t("Bikes")]);
    close(&nwb[3], 300.0);
    close(&nwb[4], 90.0 / 300.0);
    // the Web subtotal per region is the measure over the channel (North x Web 300, 90/300)
    close(&find(&level(&d, 1, 1), &[t("North"), t("Web")])[4], 90.0 / 300.0);
}

#[test]
fn the_expansion_round_trips_through_pbip_and_pbix() {
    let dir = crate::tempdir::TempDir::new("matrix-expand-files");
    for file in ["Shop.pbip", "Shop.pbix"] {
        let mut s = shop();
        let page = main_page(&s);
        let m = matrix(&mut s, &["Region", "Category", "Product"], &[]);
        expand(&mut s, &page, &m, json!({"level": 0}));
        expand(&mut s, &page, &m, json!({"path": ["North", "Bikes"]}));
        let want = stored(&s, &m);
        let shown = data(&s, &m)["row_expansion"].clone();
        assert_eq!(
            shown,
            json!({"open": [true, false, false], "toggled": [["North", "Bikes"]]})
        );
        run(&mut s, "file.save_as", json!({"path": dir.join(file)}));
        let mut back = Session::new();
        run(&mut back, "file.open", json!({"path": dir.join(file)}));
        let got = stored(&back, &m);
        assert_eq!(
            got.visual_extra["expansionStates"], want.visual_extra["expansionStates"],
            "{file}"
        );
        assert_eq!(active(&got, "Rows"), json!([true, true, true]), "{file}");
        assert_eq!(data(&back, &m)["row_expansion"], shown, "{file}");
    }
}

#[test]
fn bad_expand_params_are_errors() {
    let mut s = shop();
    let page = main_page(&s);
    let m = matrix(&mut s, &["Region", "Category"], &[]);
    let one = matrix(&mut s, &["Region"], &[]);
    for p in [
        json!({"page": page, "visual": m}),
        json!({"page": page, "visual": m, "path": ["North"], "level": 0}),
        json!({"page": page, "visual": m, "path": "North"}),
        json!({"page": page, "visual": m, "path": [[1]]}),
        json!({"page": page, "visual": m, "path": ["North", "Bikes", "Road"]}),
        json!({"page": page, "visual": m, "level": -1}),
        json!({"page": page, "visual": m, "level": 7}),
        json!({"page": page, "visual": m, "all": "yes"}),
        json!({"page": page, "visual": m, "step": "sideways"}),
        json!({"page": page, "visual": m, "step": "next", "all": true}),
        json!({"page": page, "visual": m, "step": "up"}),
        json!({"page": page, "visual": m, "role": "Values", "all": true}),
        json!({"page": page, "visual": m, "all": true, "colour": 1}),
        json!({"page": page, "visual": one, "all": true}),
        json!({"page": page, "visual": "nope", "all": true}),
    ] {
        assert!(s.execute("visual.expand", &p).is_err(), "{p}");
    }
    assert!(stored(&s, &m).visual_extra.get("expansionStates").is_none());
}
