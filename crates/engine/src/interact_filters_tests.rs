//! Tests of `view.interact` `filter` and `filter_clear` (interact_reader.rs, filter_edit.rs
//! `reader_change`): a reader changes an unlocked filter card for the session only; the visuals
//! follow, the report, the revision, the dirty flags and the saved files do not; locked and
//! hidden cards are refused; `reset` drops the change.

use serde_json::{Value, json};

use crate::Session;
use crate::tempdir::TempDir;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// A column chart (Category, Total) and a card (Total) on one page; a page filter card on
/// Sales[Category] (unlocked, no condition), a locked report card on Sales[Amount] and a
/// hidden visual card on the chart's [Total].
fn session() -> (Session, String, String, String) {
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
    ] {
        run(&mut s, id, p);
    }
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
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
    run(
        &mut s,
        "page.filter",
        json!({"page": page, "field": "Sales[Category]"}),
    );
    run(
        &mut s,
        "report.filter",
        json!({"field": "Sales[Amount]", "op": ">", "value": 1, "locked": true}),
    );
    run(
        &mut s,
        "visual.filter",
        json!({"page": page, "visual": chart, "field": "[Total]", "hidden": true}),
    );
    s.dirty = false;
    s.report_dirty = false;
    (s, page, chart, card)
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

/// The card's value.
fn total(s: &Session, card: &str) -> f64 {
    visual(s, card)["data"]["rows"][0][0].as_f64().unwrap()
}

/// The chart's categories.
fn categories(s: &Session, chart: &str) -> Vec<String> {
    let mut c: Vec<String> = visual(s, chart)["data"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r[0].as_str().unwrap().to_string())
        .collect();
    c.sort();
    c
}

/// The page's card on Sales[Category] as the snapshot lists it.
fn page_card(s: &Session) -> Value {
    s.snapshot()["report"]["pages"][0]["filters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["field"] == json!("Sales[Category]"))
        .cloned()
        .unwrap()
}

#[test]
fn a_reader_filter_filters_the_view_not_the_report() {
    let (mut s, page, chart, card) = session();
    let report = s.report.clone();
    let rev = s.revision;
    let all = total(&s, &card);
    run(
        &mut s,
        "view.interact",
        json!({"action": "filter", "page": page, "field": "Sales[Category]", "in": ["Bikes"]}),
    );
    // the page's visuals follow the reader's filter, and the Filters pane shows it
    assert!(total(&s, &card) < all);
    assert_eq!(categories(&s, &chart), ["Bikes"]);
    assert_eq!(page_card(&s)["type"], json!("basic"));
    let st = run(&mut s, "view.interactions", json!({}));
    assert_eq!(st["bookmark_filters"][0]["scope"], json!("page"));
    // the report never changed
    assert_eq!(s.report, report);
    assert_eq!(s.revision, rev);
    assert!(!s.dirty && !s.report_dirty);
    // the eraser clears it, the card kept
    run(
        &mut s,
        "view.interact",
        json!({"action": "filter_clear", "page": page, "field": "Sales[Category]"}),
    );
    assert_eq!(total(&s, &card), all);
    assert_eq!(page_card(&s)["type"], json!("none"));
    assert_eq!(
        run(&mut s, "view.interactions", json!({}))["bookmark_filters"],
        json!([])
    );
    // a visual's bound field without a card filters that visual only
    run(
        &mut s,
        "view.interact",
        json!({"action": "filter", "page": page, "visual": chart, "field": "Sales[Category]",
               "not_in": ["Wheels"]}),
    );
    assert_eq!(categories(&s, &chart), ["Bikes", "Toys"]);
    assert_eq!(total(&s, &card), all);
    // reset drops every reader filter
    run(
        &mut s,
        "view.interact",
        json!({"action": "filter", "page": page, "field": "Sales[Category]", "in": ["Toys"]}),
    );
    assert!(total(&s, &card) < all);
    run(&mut s, "view.interact", json!({"action": "reset"}));
    assert_eq!(total(&s, &card), all);
    assert_eq!(categories(&s, &chart), ["Bikes", "Toys", "Wheels"]);
    assert_eq!(
        run(&mut s, "view.interactions", json!({}))["bookmark_filters"],
        json!([])
    );
    assert_eq!(s.report, report);
}

#[test]
fn locked_and_hidden_cards_stay_the_authors() {
    let (mut s, page, chart, _) = session();
    let before = run(&mut s, "view.interactions", json!({}));
    for (p, why) in [
        (
            json!({"action": "filter", "field": "Sales[Amount]", "op": ">", "value": 50}),
            "locked",
        ),
        (
            json!({"action": "filter_clear", "field": "Sales[Amount]"}),
            "locked",
        ),
        (
            json!({"action": "filter", "page": page, "visual": chart, "field": "[Total]",
                   "op": ">", "value": 50}),
            "hidden",
        ),
        (
            json!({"action": "filter", "page": page, "field": "Sales[Amount]", "op": ">", "value": 1}),
            "no card",
        ),
        (
            json!({"action": "filter", "page": page, "field": "Sales[Category]", "in": ["Toys"], "locked": true}),
            "author",
        ),
    ] {
        let e = s.execute("view.interact", &p).unwrap_err();
        assert!(e.contains(why), "{p}: {e}");
    }
    assert_eq!(run(&mut s, "view.interactions", json!({})), before);
    // the locked card keeps its condition and its lock in the view
    let report_cards = s.snapshot()["report"]["filters"].clone();
    assert_eq!(report_cards[0]["locked"], json!(true));
    assert_eq!(report_cards[0]["type"], json!("advanced"));
}

#[test]
fn changing_a_filter_type_follows_the_report_setting() {
    let (mut s, page, _, _) = session();
    run(
        &mut s,
        "report.settings",
        json!({"allow_change_filter_types": false}),
    );
    let advanced = json!({"action": "filter", "page": page, "field": "Sales[Category]",
                          "op": "starts_with", "value": "B"});
    let e = s.execute("view.interact", &advanced).unwrap_err();
    assert!(e.contains("filter types"), "{e}");
    // the card's own type is taken
    run(
        &mut s,
        "view.interact",
        json!({"action": "filter", "page": page, "field": "Sales[Category]", "in": ["Toys"]}),
    );
    run(
        &mut s,
        "report.settings",
        json!({"allow_change_filter_types": true}),
    );
    run(&mut s, "view.interact", advanced);
    assert_eq!(page_card(&s)["type"], json!("advanced"));
}

#[test]
fn a_reader_filter_is_not_saved() {
    let dir = TempDir::new("reader-filters");
    let (mut s, page, _, card) = session();
    run(
        &mut s,
        "project.new",
        json!({"dir": dir.path(), "name": "Shop", "format": "tmdl"}),
    );
    let pbip = dir.join("Shop.pbip");
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": pbip}));
    let all = total(&s, &card);
    run(
        &mut s,
        "view.interact",
        json!({"action": "filter", "page": page, "field": "Sales[Category]", "in": ["Bikes"]}),
    );
    assert!(total(&s, &card) < all);
    run(&mut s, "file.save", json!({"backup": false}));
    let mut again = Session::new();
    run(&mut again, "file.open", json!({"path": pbip}));
    assert_eq!(total(&again, &card), all);
    assert_eq!(page_card(&again)["type"], json!("none"));
}
