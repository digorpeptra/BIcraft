//! The Filters pane's commands and the M11.13 field edits on an engine session: page and report
//! filters (basic, advanced, top N refused), empty cards, clear versus remove, the rendered data
//! with the filters applied, `column.values`, `visual.rename_field` (titles, headers, auto
//! title) and `visual.bind_hierarchy` (the projections Desktop writes, the render), each through
//! a PBIP and a PBIX round trip.

use serde_json::{Value, json};

use crate::Session;
use crate::tempdir::TempDir;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn fails(s: &mut Session, id: &str, params: Value) -> String {
    match s.execute(id, &params) {
        Ok(v) => panic!("{id} {params} should fail, got {v}"),
        Err(e) => e,
    }
}

/// Sales(Category, Product, Amount) with [Total] and a hierarchy Geo (Category > Product); a
/// page "Main" with a table (Category, [Total]) and a column chart (Category, [Total]).
fn shop() -> (Session, String, String) {
    let mut s = Session::new();
    let sales = r#"#table({"Category", "Product", "Amount"}, {{"Wheels", "Bike", 100}, {"Wheels", "Car", 1000}, {"Toys", "Ball", 5}, {"Games", "Chess", 20}, {"Toys", "Kite", 7}})"#;
    for (id, p) in [
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
            json!({"table": "Sales", "name": "Geo", "levels": ["Category", "Product"]}),
        ),
        ("report.new", json!({})),
        ("page.add", json!({"display_name": "Main"})),
    ] {
        run(&mut s, id, p);
    }
    let mut add = |ty: &str| {
        run(
            &mut s,
            "visual.add",
            json!({"page": "Main", "visual_type": ty, "x": 0, "y": 0, "width": 300, "height": 300,
                   "fields": [{"role": if ty == "tableEx" { "Values" } else { "Category" }, "field": "Sales[Category]"},
                              {"role": if ty == "tableEx" { "Values" } else { "Y" }, "field": "[Total]"}]}),
        )["visual"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let table = add("tableEx");
    let chart = add("clusteredColumnChart");
    (s, table, chart)
}

fn visual_json(s: &Session, visual: &str) -> Value {
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

fn page_json(s: &Session) -> Value {
    let snap = s.snapshot();
    snap["report"]["pages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["display_name"] == "Main")
        .cloned()
        .unwrap()
}

/// The first column of the visual's data rows.
fn categories(s: &Session, visual: &str) -> Vec<Value> {
    let d = visual_json(s, visual)["data"].clone();
    assert_eq!(d["status"], "ok", "{d}");
    d["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r[0].clone())
        .collect()
}

#[test]
fn page_and_report_filters_filter_the_page_and_round_trip() {
    let (mut s, table, chart) = shop();
    assert_eq!(categories(&s, &table).len(), 3);
    // a basic page filter: every visual of the page
    let out = run(
        &mut s,
        "page.filter",
        json!({"page": "Main", "field": "Sales[Category]", "in": ["Wheels", "Toys"]}),
    );
    assert_eq!(out["changed"], true);
    assert_eq!(out["filters"][0]["type"], "basic");
    assert_eq!(out["filters"][0]["condition"], json!({"in": ["Wheels", "Toys"]}));
    let mut cats = categories(&s, &table);
    cats.sort_by_key(|v| v.to_string());
    assert_eq!(cats, vec![json!("Toys"), json!("Wheels")]);
    assert_eq!(categories(&s, &chart).len(), 2);
    // an advanced report filter on another column narrows it further
    run(
        &mut s,
        "report.filter",
        json!({"field": "Sales[Product]", "conditions": [{"op": "starts_with", "value": "B"}, {"op": "=", "value": "Kite"}], "logic": "or"}),
    );
    let snap = s.snapshot();
    assert_eq!(snap["report"]["filters"][0]["type"], "advanced");
    assert_eq!(snap["report"]["filters"][0]["condition"]["logic"], "or");
    let mut cats = categories(&s, &table);
    cats.sort_by_key(|v| v.to_string());
    assert_eq!(cats, vec![json!("Toys"), json!("Wheels")], "Bike, Ball, Kite");
    run(
        &mut s,
        "report.filter",
        json!({"field": "Sales[Product]", "op": "=", "value": "Car"}),
    );
    assert_eq!(categories(&s, &table), vec![json!("Wheels")]);
    // the snapshot lists the page's cards
    assert_eq!(page_json(&s)["filters"].as_array().unwrap().len(), 1);
    // measures and top N stay on visuals
    let e = fails(
        &mut s,
        "page.filter",
        json!({"page": "Main", "field": "[Total]", "op": ">", "value": 1}),
    );
    assert!(e.contains("columns"), "{e}");
    let e = fails(&mut s, "report.filter", json!({"field": "Sum(Sales[Amount])"}));
    assert!(e.contains("columns"), "{e}");

    // PBIP and PBIX keep them
    let dir = TempDir::new("filter-edit-levels");
    for path in [dir.join("pbip").join("Shop.pbip"), dir.join("Shop.pbix")] {
        run(&mut s, "file.save_as", json!({"path": path}));
        let mut back = Session::new();
        run(&mut back, "file.open", json!({"path": path}));
        let r = back.report.as_ref().unwrap();
        let page = r.pages.iter().find(|p| p.display_name == "Main").unwrap();
        let pf = crate::filter_edit::cards(page.filter_config.as_ref());
        assert_eq!(pf[0]["condition"], json!({"in": ["Wheels", "Toys"]}), "{path:?}");
        let rf = crate::filter_edit::cards(r.filter_config.as_ref());
        assert_eq!(rf[0]["condition"], json!({"op": "=", "value": "Car"}), "{path:?}");
    }
}

#[test]
fn empty_cards_clear_and_remove() {
    let (mut s, table, _) = shop();
    // a field dropped on a section: an empty card, no condition
    for (id, p) in [
        ("page.filter", json!({"page": "Main", "field": "Sales[Product]"})),
        ("report.filter", json!({"field": "Sales[Category]"})),
        (
            "visual.filter",
            json!({"page": "Main", "visual": table, "field": "Sales[Product]"}),
        ),
    ] {
        let out = run(&mut s, id, p);
        assert_eq!(out["changed"], true, "{id}");
        assert_eq!(out["filters"][0]["type"], "none", "{id}");
        assert_eq!(out["filters"][0]["how_created"], "User", "{id}");
    }
    let page = page_json(&s);
    assert_eq!(page["filters"][0]["field"], "Sales[Product]");
    assert_eq!(page["filters"][0]["kind"], "column");
    assert_eq!(visual_json(&s, &table)["filters"][0]["field"], "Sales[Product]");
    // again: nothing new
    let out = run(
        &mut s,
        "page.filter",
        json!({"page": "Main", "field": "Sales[Product]"}),
    );
    assert_eq!(out["changed"], false);
    // a condition on it, then Clear filter keeps the card
    run(
        &mut s,
        "page.filter",
        json!({"page": "Main", "field": "Sales[Product]", "not_in": ["Car"]}),
    );
    let out = run(
        &mut s,
        "page.filter",
        json!({"page": "Main", "field": "Sales[Product]", "in": ["Bike"]}),
    );
    assert_eq!(out["filters"].as_array().unwrap().len(), 1, "replaced in place");
    assert_eq!(categories(&s, &table), vec![json!("Wheels")]);
    let out = run(
        &mut s,
        "page.clear_filters",
        json!({"page": "Main", "field": "Sales[Product]", "keep_cards": true}),
    );
    assert_eq!(out["filters"][0]["type"], "none");
    assert_eq!(categories(&s, &table).len(), 3);
    // Remove filter drops it
    let out = run(
        &mut s,
        "page.clear_filters",
        json!({"page": "Main", "field": "Sales[Product]"}),
    );
    assert_eq!(out["filters"], json!([]));
    assert!(s.report.as_ref().unwrap().pages[1].filter_config.is_none());
    let e = fails(
        &mut s,
        "page.clear_filters",
        json!({"page": "Main", "field": "Sales[Product]"}),
    );
    assert!(e.contains("no card"), "{e}");
    // the visual's empty card goes with visual.clear_filters {field}
    run(
        &mut s,
        "visual.clear_filters",
        json!({"page": "Main", "visual": table, "field": "Sales[Product]"}),
    );
    assert_eq!(visual_json(&s, &table)["filters"], json!([]));
    // a visual's basic filter, cleared with keep_cards
    run(
        &mut s,
        "visual.filter",
        json!({"page": "Main", "visual": table, "field": "Sales[Category]", "in": ["Toys"]}),
    );
    assert_eq!(categories(&s, &table), vec![json!("Toys")]);
    run(
        &mut s,
        "visual.clear_filters",
        json!({"page": "Main", "visual": table, "field": "Sales[Category]", "keep_cards": true}),
    );
    let v = visual_json(&s, &table);
    assert_eq!(v["filters"][0]["type"], "none");
    assert_eq!(categories(&s, &table).len(), 3);
    // undo steps back one command at a time
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(categories(&s, &table), vec![json!("Toys")]);
    // bad parameters are errors
    for (id, p) in [
        (
            "page.filter",
            json!({"page": "Main", "field": "Sales[Product]", "value": 1}),
        ),
        ("page.filter", json!({"page": "Main", "field": 3})),
        (
            "report.filter",
            json!({"page": "Main", "field": "Sales[Product]"}),
        ),
        ("report.clear_filters", json!({"keep_cards": "yes"})),
        ("page.filter", json!({"page": "Nope", "field": "Sales[Product]"})),
    ] {
        fails(&mut s, id, p);
    }
}

#[test]
fn column_values_lists_distinct_sorted_values() {
    let (mut s, _, _) = shop();
    let v = run(
        &mut s,
        "column.values",
        json!({"table": "sales", "column": "category"}),
    );
    assert_eq!(v["values"], json!(["Games", "Toys", "Wheels"]));
    assert_eq!(v["available"], true);
    assert_eq!(v["column"], "Category");
    let v = run(
        &mut s,
        "column.values",
        json!({"table": "Sales", "column": "Amount", "limit": 2}),
    );
    assert_eq!(v["values"], json!([5, 7]));
    assert_eq!(
        (v["count"].as_u64(), v["truncated"].as_bool()),
        (Some(5), Some(true))
    );
    let v = run(
        &mut s,
        "column.values",
        json!({"table": "Sales", "column": "Product", "search": "ITE"}),
    );
    assert_eq!(v["values"], json!(["Kite"]));
    let rev = s.revision;
    run(
        &mut s,
        "column.values",
        json!({"table": "Sales", "column": "Product"}),
    );
    assert_eq!(s.revision, rev, "read-only");
    for p in [
        json!({"table": "Nope", "column": "x"}),
        json!({"table": "Sales", "column": "Nope"}),
        json!({"table": "Sales", "column": "Product", "limit": 0}),
        json!({"table": "Sales", "column": "Product", "limit": 99999}),
    ] {
        fails(&mut s, "column.values", p);
    }
}

#[test]
fn a_field_renamed_for_a_visual_names_its_header_title_and_round_trips() {
    let (mut s, table, chart) = shop();
    let out = run(
        &mut s,
        "visual.rename_field",
        json!({"page": "Main", "visual": chart, "role": "Y", "field": "[Total]", "name": "Revenue"}),
    );
    assert_eq!(out["renamed"], "Revenue");
    let v = visual_json(&s, &chart);
    assert_eq!(v["fields"][1]["display_name"], "Revenue");
    assert_eq!(v["fields"][1]["label"], "Revenue");
    assert_eq!(v["auto_title"], "Revenue by Category");
    let names: Vec<Value> = v["data"]["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].clone())
        .collect();
    assert!(names.contains(&json!("Revenue")), "{names:?}");
    // the table by position, the other visual untouched
    run(
        &mut s,
        "visual.rename_field",
        json!({"page": "Main", "visual": table, "role": "Values", "index": 0, "name": "Kind"}),
    );
    assert_eq!(visual_json(&s, &table)["data"]["columns"][0]["name"], "Kind");
    assert!(visual_json(&s, &table)["fields"][1].get("display_name").is_none());
    // kept through a move and an aggregation-free type change
    run(
        &mut s,
        "visual.move_field",
        json!({"page": "Main", "visual": chart, "role": "Y", "field": "[Total]", "to_role": "Tooltips"}),
    );
    assert_eq!(visual_json(&s, &chart)["fields"][1]["display_name"], "Revenue");
    run(
        &mut s,
        "visual.move_field",
        json!({"page": "Main", "visual": chart, "role": "Tooltips", "field": "[Total]", "to_role": "Y"}),
    );
    // PBIP and PBIX: the projection's displayName
    let dir = TempDir::new("filter-edit-rename");
    for path in [dir.join("pbip").join("Shop.pbip"), dir.join("Shop.pbix")] {
        run(&mut s, "file.save_as", json!({"path": path}));
        let mut back = Session::new();
        run(&mut back, "file.open", json!({"path": path}));
        let r = back.report.as_ref().unwrap();
        let v = r
            .pages
            .iter()
            .flat_map(|p| p.visuals.iter())
            .find(|v| v.name == chart)
            .unwrap();
        let total = v.fields.iter().find(|b| b.role == "Y").unwrap();
        assert_eq!(total.display_name.as_deref(), Some("Revenue"), "{path:?}");
        let q = bicraft_report::query::effective(v).unwrap();
        assert_eq!(
            q["queryState"]["Y"]["projections"][0]["displayName"], "Revenue",
            "{path:?}"
        );
        assert_eq!(
            visual_json(&back, &chart)["auto_title"],
            "Revenue by Category",
            "{path:?}"
        );
    }
    // back to the field's name
    run(
        &mut s,
        "visual.rename_field",
        json!({"page": "Main", "visual": chart, "role": "Y", "field": "[Total]", "name": null}),
    );
    assert_eq!(visual_json(&s, &chart)["auto_title"], "Total by Category");
    for p in [
        json!({"page": "Main", "visual": chart, "role": "Y", "field": "[Total]", "name": null}),
        json!({"page": "Main", "visual": chart, "role": "Y", "field": "[Nope]", "name": "x"}),
        json!({"page": "Main", "visual": chart, "role": "Y", "index": 9, "name": "x"}),
        json!({"page": "Main", "visual": chart, "role": "Y", "field": "[Total]", "name": "a\nb"}),
        json!({"page": "Main", "visual": chart, "role": "Y", "field": "[Total]"}),
        json!({"page": "Main", "visual": chart, "role": "Y", "field": "[Total]", "name": 4}),
    ] {
        fails(&mut s, "visual.rename_field", p);
    }
}

#[test]
fn a_whole_hierarchy_binds_its_levels_as_desktop_writes_them() {
    let (mut s, table, chart) = shop();
    // a matrix with the hierarchy on its rows
    let m = run(
        &mut s,
        "visual.add",
        json!({"page": "Main", "visual_type": "pivotTable", "x": 0, "y": 320, "width": 300, "height": 300,
               "fields": [{"role": "Values", "field": "[Total]"}]}),
    )["visual"]
        .as_str()
        .unwrap()
        .to_string();
    let out = run(
        &mut s,
        "visual.bind_hierarchy",
        json!({"page": "Main", "visual": m, "role": "Rows", "hierarchy": "geo"}),
    );
    assert_eq!(out["levels"], json!(["Category", "Product"]));
    assert_eq!(out["hierarchy"], "Geo");
    let v = visual_json(&s, &m);
    let rows: Vec<&Value> = v["fields"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["role"] == "Rows")
        .collect();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["kind"], "hierarchy_level");
    assert_eq!(rows[0]["hierarchy"], "Geo");
    assert_eq!(rows[1]["name"], "Product");
    // the query: HierarchyLevel projections, Desktop's queryRefs, the top level active
    let r = s.report.as_ref().unwrap();
    let vis = r
        .pages
        .iter()
        .flat_map(|p| p.visuals.iter())
        .find(|v| v.name == m)
        .unwrap();
    let q = bicraft_report::query::effective(vis).unwrap();
    let p = &q["queryState"]["Rows"]["projections"];
    assert_eq!(p[0]["queryRef"], "Sales.Geo.Category");
    assert_eq!(p[0]["nativeQueryRef"], "Geo Category");
    assert_eq!(p[0]["field"]["HierarchyLevel"]["Level"], "Category");
    assert_eq!(
        p[0]["field"]["HierarchyLevel"]["Expression"]["Hierarchy"]["Hierarchy"],
        "Geo"
    );
    assert_eq!(
        (p[0]["active"].as_bool(), p[1]["active"].as_bool()),
        (Some(true), Some(false))
    );
    // it renders: the matrix at its top level
    let d = &v["data"];
    assert_eq!(d["status"], "ok", "{d}");
    assert_eq!(d["rows"].as_array().unwrap().len(), 3, "{d}");
    // a chart's axis, two levels named; a table shows every level
    run(
        &mut s,
        "visual.unbind",
        json!({"page": "Main", "visual": chart, "role": "Category", "field": "Sales[Category]"}),
    );
    run(
        &mut s,
        "visual.bind_hierarchy",
        json!({"page": "Main", "visual": chart, "role": "Category", "hierarchy": "Geo", "table": "Sales", "levels": ["category"]}),
    );
    assert_eq!(
        visual_json(&s, &chart)["data"]["rows"].as_array().unwrap().len(),
        3
    );
    run(
        &mut s,
        "visual.bind_hierarchy",
        json!({"page": "Main", "visual": table, "role": "Values", "hierarchy": "Geo", "levels": ["Product"], "index": 0}),
    );
    let td = visual_json(&s, &table)["data"].clone();
    assert_eq!(td["rows"].as_array().unwrap().len(), 5, "{td}");
    // refused: twice, a measure role, unknown names
    for p in [
        json!({"page": "Main", "visual": m, "role": "Rows", "hierarchy": "Geo"}),
        json!({"page": "Main", "visual": m, "role": "Values", "hierarchy": "Geo"}),
        json!({"page": "Main", "visual": m, "role": "Rows", "hierarchy": "Nope"}),
        json!({"page": "Main", "visual": m, "role": "Rows", "hierarchy": "Geo", "levels": ["Nope"]}),
        json!({"page": "Main", "visual": m, "role": "Rows", "hierarchy": "Geo", "levels": []}),
        json!({"page": "Main", "visual": m, "role": "Rows", "hierarchy": "Geo", "table": "Nope"}),
    ] {
        fails(&mut s, "visual.bind_hierarchy", p);
    }
    // PBIP and PBIX keep the levels
    let dir = TempDir::new("filter-edit-hier");
    for path in [dir.join("pbip").join("Shop.pbip"), dir.join("Shop.pbix")] {
        run(&mut s, "file.save_as", json!({"path": path}));
        let mut back = Session::new();
        run(&mut back, "file.open", json!({"path": path}));
        let r = back.report.as_ref().unwrap();
        let vis = r
            .pages
            .iter()
            .flat_map(|p| p.visuals.iter())
            .find(|v| v.name == m)
            .unwrap();
        let q = bicraft_report::query::effective(vis).unwrap();
        let p = &q["queryState"]["Rows"]["projections"];
        assert_eq!(p[1]["queryRef"], "Sales.Geo.Product", "{path:?}");
        assert_eq!(p[0]["active"], true, "{path:?}");
        assert_eq!(
            visual_json(&back, &m)["data"]["rows"].as_array().unwrap().len(),
            3,
            "{path:?}"
        );
    }
}
