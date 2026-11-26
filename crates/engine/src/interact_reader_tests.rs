//! Tests of `view.interact` `spotlight` and `sort` (interact_reader.rs): view state only (the
//! report, the revision and the dirty flags never change), `reset` and a page change; the data
//! follow the reader's sort; bookmarks capture both (`singleVisual.display.mode`,
//! `singleVisual.orderBy`) and give them back, through PBIP and PBIX files too.

use serde_json::{Value, json};

use crate::Session;
use crate::tempdir::TempDir;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// A column chart and a card on the first page; a second page.
fn session() -> (Session, String, String, String, String) {
    let mut s = Session::new();
    let sales = r#"#table({"Category", "Amount"}, {{"Wheels", 100}, {"Toys", 5}, {"Bikes", 40}})"#;
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
            json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        ("report.new", json!({})),
        ("page.add", json!({"display_name": "Detail"})),
    ] {
        run(&mut s, id, p);
    }
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    let second = s.report.as_ref().unwrap().pages[1].name.clone();
    let r = run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "clusteredColumnChart", "x": 20, "y": 20,
               "width": 300, "height": 200,
               "fields": [{"role": "Category", "field": "Sales[Category]"}, {"role": "Y", "field": "[Total]"}]}),
    );
    let chart = r["visual"].as_str().unwrap().to_string();
    let r = run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "card", "x": 400, "y": 20, "width": 200, "height": 120,
               "fields": [{"role": "Values", "field": "[Total]"}]}),
    );
    let card = r["visual"].as_str().unwrap().to_string();
    s.dirty = false;
    s.report_dirty = false;
    (s, page, second, chart, card)
}

fn interaction(s: &Session) -> Value {
    s.snapshot()["interaction"].clone()
}

/// The snapshot entry of `visual` on the first page.
fn visual(s: &Session, visual: &str) -> Value {
    s.snapshot()["report"]["pages"][0]["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == json!(visual))
        .cloned()
        .unwrap()
}

/// The chart's categories in the order its data rows come.
fn categories(s: &Session, chart: &str) -> Vec<String> {
    let v = visual(s, chart);
    v["data"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r[0].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn spotlight_is_view_state() {
    let (mut s, page, second, chart, _) = session();
    let report = s.report.clone();
    let rev = s.revision;
    assert_eq!(interaction(&s)["spotlight"], Value::Null);
    let out = run(
        &mut s,
        "view.interact",
        json!({"action": "spotlight", "page": page, "visual": chart}),
    );
    assert_eq!(out["changed"], true);
    assert_eq!(
        interaction(&s)["spotlight"],
        json!({"page": page, "visual": chart})
    );
    assert_eq!(
        run(&mut s, "view.interactions", json!({}))["spotlight"],
        json!({"page": page, "visual": chart})
    );
    let again = run(
        &mut s,
        "view.interact",
        json!({"action": "spotlight", "page": page, "visual": chart}),
    );
    assert_eq!(again["changed"], false);
    // null ends it
    run(
        &mut s,
        "view.interact",
        json!({"action": "spotlight", "page": page, "visual": null}),
    );
    assert_eq!(interaction(&s)["spotlight"], Value::Null);
    // reset and a page change end it
    run(
        &mut s,
        "view.interact",
        json!({"action": "spotlight", "page": page, "visual": chart}),
    );
    assert_eq!(
        run(&mut s, "view.interact", json!({"action": "reset"}))["changed"],
        true
    );
    assert_eq!(interaction(&s)["spotlight"], Value::Null);
    run(
        &mut s,
        "view.interact",
        json!({"action": "spotlight", "page": page, "visual": chart}),
    );
    run(&mut s, "view.interact", json!({"action": "page", "page": second}));
    assert_eq!(interaction(&s)["spotlight"], Value::Null);
    assert_eq!(s.report, report);
    assert_eq!(s.revision, rev);
    assert!(!s.dirty && !s.report_dirty);
}

#[test]
fn reader_sort_sorts_the_view_not_the_report() {
    let (mut s, page, _, chart, _) = session();
    let report = s.report.clone();
    let rev = s.revision;
    run(
        &mut s,
        "view.interact",
        json!({"action": "sort", "page": page, "visual": chart, "field": "[Total]", "direction": "asc"}),
    );
    assert_eq!(
        visual(&s, &chart)["sort"],
        json!([{"field": "[Total]", "descending": false}])
    );
    assert_eq!(categories(&s, &chart), ["Toys", "Bikes", "Wheels"]);
    run(
        &mut s,
        "view.interact",
        json!({"action": "sort", "page": page, "visual": chart, "field": "Sales[Category]"}),
    );
    // a column sorts ascending by default
    assert_eq!(categories(&s, &chart), ["Bikes", "Toys", "Wheels"]);
    let st = run(&mut s, "view.interactions", json!({}));
    assert_eq!(
        st["sorts"],
        json!([{"page": page, "visual": chart,
                "sort": [{"field": "Sales[Category]", "descending": false}]}])
    );
    // the report never changed
    assert_eq!(s.report, report);
    assert_eq!(s.revision, rev);
    assert!(!s.dirty && !s.report_dirty);
    // null goes back to the report's sort; so does reset
    run(
        &mut s,
        "view.interact",
        json!({"action": "sort", "page": page, "visual": chart, "field": null}),
    );
    assert_eq!(run(&mut s, "view.interactions", json!({}))["sorts"], json!([]));
    run(
        &mut s,
        "view.interact",
        json!({"action": "sort", "page": page, "visual": chart, "field": "[Total]", "direction": "asc"}),
    );
    run(&mut s, "view.interact", json!({"action": "reset"}));
    assert_eq!(run(&mut s, "view.interactions", json!({}))["sorts"], json!([]));
    // the report's own sort is no override
    run(
        &mut s,
        "visual.set_sort",
        json!({"page": page, "visual": chart, "field": "[Total]", "direction": "desc"}),
    );
    let out = run(
        &mut s,
        "view.interact",
        json!({"action": "sort", "page": page, "visual": chart, "field": "[Total]", "direction": "desc"}),
    );
    assert_eq!(out["changed"], false);
    assert_eq!(categories(&s, &chart), ["Wheels", "Bikes", "Toys"]);
}

#[test]
fn hostile_spotlight_and_sort_params_are_errors() {
    let (mut s, page, _, chart, _) = session();
    let before = run(&mut s, "view.interactions", json!({}));
    for p in [
        json!({"action": "spotlight", "page": page, "visual": "nope"}),
        json!({"action": "spotlight", "page": "nope", "visual": chart}),
        json!({"action": "spotlight", "page": page, "visual": 3}),
        json!({"action": "spotlight", "page": page, "visual": chart, "extra": 1}),
        json!({"action": "sort", "page": page, "visual": chart}),
        json!({"action": "sort", "page": page, "visual": chart, "field": 7}),
        json!({"action": "sort", "page": page, "visual": chart, "field": "Sales[Nope]"}),
        json!({"action": "sort", "page": page, "visual": chart, "field": "[Total]", "direction": "up"}),
        json!({"action": "sort", "page": page, "visual": chart, "field": null, "direction": "asc"}),
        json!({"action": "sort", "page": page, "visual": "nope", "field": "[Total]"}),
    ] {
        assert!(s.execute("view.interact", &p).is_err(), "{p}");
    }
    assert_eq!(run(&mut s, "view.interactions", json!({})), before);
}

/// Captures a bookmark with the chart in spotlight and sorted by the reader.
fn capture(s: &mut Session, page: &str, chart: &str) {
    run(
        s,
        "view.interact",
        json!({"action": "spotlight", "page": page, "visual": chart}),
    );
    run(
        s,
        "view.interact",
        json!({"action": "sort", "page": page, "visual": chart, "field": "[Total]", "direction": "asc"}),
    );
    run(s, "bookmark.capture", json!({"display_name": "Lit"}));
    run(s, "view.interact", json!({"action": "reset"}));
}

/// Applying the bookmark gives the spotlight and the sort back.
fn check_applied(s: &mut Session, page: &str, chart: &str) {
    assert_eq!(interaction(s)["spotlight"], Value::Null);
    run(
        s,
        "view.interact",
        json!({"action": "bookmark", "bookmark": "Lit"}),
    );
    assert_eq!(
        interaction(s)["spotlight"],
        json!({"page": page, "visual": chart})
    );
    assert_eq!(categories(s, chart), ["Toys", "Bikes", "Wheels"]);
}

#[test]
fn bookmarks_carry_spotlight_and_reader_sort() {
    let (mut s, page, _, chart, card) = session();
    capture(&mut s, &page, &chart);
    let doc = s.report.as_ref().unwrap().bookmarks[0].document.clone();
    let containers = &doc["explorationState"]["sections"][&page]["visualContainers"];
    assert_eq!(
        containers[&chart]["singleVisual"]["display"]["mode"],
        json!("spotlight")
    );
    assert_eq!(
        containers[&chart]["singleVisual"]["orderBy"],
        json!([{"Direction": 1, "Expression": {"Measure": {
            "Expression": {"SourceRef": {"Entity": "Sales"}}, "Property": "Total"}}}])
    );
    assert!(containers.get(&card).is_none(), "{containers}");
    // the visual's own query is not changed by the reader's sort
    let v = &s.report.as_ref().unwrap().pages[0].visuals;
    assert!(v.iter().all(|v| {
        v.visual_extra
            .get("query")
            .and_then(|q| q.get("sortDefinition"))
            .is_none()
    }));
    check_applied(&mut s, &page, &chart);
    // a bookmark without spotlight ends it; one without orderBy drops the reader's sort
    run(&mut s, "view.interact", json!({"action": "reset"}));
    run(&mut s, "bookmark.capture", json!({"display_name": "Plain"}));
    check_applied(&mut s, &page, &chart);
    run(
        &mut s,
        "view.interact",
        json!({"action": "bookmark", "bookmark": "Plain"}),
    );
    assert_eq!(interaction(&s)["spotlight"], Value::Null);
    assert_eq!(run(&mut s, "view.interactions", json!({}))["sorts"], json!([]));
}

#[test]
fn bookmarks_display_mode_under_single_visual_hides() {
    let (mut s, page, _, chart, _) = session();
    run(
        &mut s,
        "visual.set_hidden",
        json!({"page": page, "visual": chart, "hidden": true}),
    );
    run(&mut s, "bookmark.capture", json!({"display_name": "Hidden"}));
    let doc = s.report.as_ref().unwrap().bookmarks[0].document.clone();
    let c = &doc["explorationState"]["sections"][&page]["visualContainers"][&chart];
    // the PBIR schema's place (VisualContainerState has no `display` of its own)
    assert_eq!(c["singleVisual"]["display"]["mode"], json!("hidden"));
    assert!(c.get("display").is_none(), "{c}");
    run(
        &mut s,
        "visual.set_hidden",
        json!({"page": page, "visual": chart, "hidden": false}),
    );
    run(
        &mut s,
        "view.interact",
        json!({"action": "bookmark", "bookmark": "Hidden"}),
    );
    assert_eq!(visual(&s, &chart)["hidden"], json!(true));
    run(&mut s, "view.interact", json!({"action": "reset"}));
    run(&mut s, "bookmark.apply", json!({"bookmark": "Hidden"}));
    assert!(
        s.report.as_ref().unwrap().pages[0]
            .visuals
            .iter()
            .any(|v| v.name == chart && v.is_hidden)
    );
}

#[test]
fn spotlight_and_sort_bookmarks_round_trip_through_pbip_and_pbix() {
    let dir = TempDir::new("reader-bookmarks");
    let (mut s, page, _, chart, _) = session();
    run(
        &mut s,
        "project.new",
        json!({"dir": dir.path(), "name": "Shop", "format": "tmdl"}),
    );
    let pbip = dir.join("Shop.pbip");
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": pbip}));
    capture(&mut s, &page, &chart);
    run(&mut s, "file.save", json!({"backup": false}));
    let pbix = dir.join("Shop.pbix");
    run(&mut s, "file.save_as", json!({"path": pbix}));
    for path in [pbip, pbix] {
        let mut again = Session::new();
        run(&mut again, "file.open", json!({"path": path}));
        check_applied(&mut again, &page, &chart);
    }
}
