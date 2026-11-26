//! Tests of `bookmarks.rs`: capturing the report's state (page, hidden visuals, filters), the
//! options that say what a bookmark applies, the Bookmarks pane's rename, order, groups and
//! deletion, hostile parameters, and the PBIP round trip of the documents and `bookmarks.json`.

use serde_json::{Value, json};

use crate::Session;
use crate::tempdir::TempDir;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Runs a command that must fail and checks the report did not change.
fn fails(s: &mut Session, id: &str, params: Value) -> String {
    let before = s.report.clone();
    let dirty = s.report_dirty;
    let err = s
        .execute(id, &params)
        .expect_err(&format!("{id} {params} should fail"));
    assert_eq!(s.report, before, "{id} changed the report although it failed");
    assert_eq!(s.report_dirty, dirty);
    err
}

/// A report with two pages: page 1 has a card and a column chart, page 2 a table. A model gives
/// the filters a field to name.
fn session() -> Session {
    let mut s = Session::new();
    let sales = r#"#table({"Category", "Amount"}, {{"Wheels", 100}, {"Toys", 5}})"#;
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
    let first = s.report.as_ref().unwrap().pages[0].name.clone();
    for (ty, x, fields) in [
        ("card", 20, json!([{"role": "Values", "field": "[Total]"}])),
        (
            "clusteredColumnChart",
            260,
            json!([{"role": "Category", "field": "Sales[Category]"}, {"role": "Y", "field": "[Total]"}]),
        ),
    ] {
        run(
            &mut s,
            "visual.add",
            json!({"page": first, "visual_type": ty, "x": x, "y": 20, "width": 200,
                   "height": 160, "fields": fields}),
        );
    }
    run(
        &mut s,
        "page.add",
        json!({"display_name": "Detail", "width": 1280, "height": 720}),
    );
    let second = s
        .report
        .as_ref()
        .unwrap()
        .pages
        .last()
        .map(|p| p.name.clone())
        .unwrap();
    run(
        &mut s,
        "visual.add",
        json!({"page": second, "visual_type": "tableEx", "x": 20, "y": 20, "width": 400,
               "height": 200,
               "fields": [{"role": "Values", "field": "Sales[Category]"}]}),
    );
    run(&mut s, "page.set_active", json!({"page": first}));
    s
}

fn page(s: &Session, i: usize) -> String {
    s.report
        .as_ref()
        .and_then(|r| r.pages.get(i))
        .map(|p| p.name.clone())
        .unwrap_or_default()
}

fn visuals(s: &Session, page: usize) -> Vec<(String, bool)> {
    s.report
        .as_ref()
        .and_then(|r| r.pages.get(page))
        .map(|p| p.visuals.iter().map(|v| (v.name.clone(), v.is_hidden)).collect())
        .unwrap_or_default()
}

fn bookmarks(s: &Session) -> Vec<String> {
    s.report
        .as_ref()
        .map(|r| r.bookmarks.iter().map(|b| b.name.clone()).collect())
        .unwrap_or_default()
}

fn list(s: &mut Session) -> Value {
    run(s, "bookmark.list", json!({}))
}

/// The capture holds the page, the hidden visuals and the options the parameters ask for, and
/// applying it restores them; renaming and re-capturing are idempotent.
#[test]
fn bookmarks_capture_apply_and_rename() {
    let mut s = session();
    let first = page(&s, 0);
    let second = page(&s, 1);
    let card = visuals(&s, 0)[0].0.clone();
    run(
        &mut s,
        "visual.set_hidden",
        json!({"page": first, "visual": card, "hidden": true}),
    );
    let out = run(
        &mut s,
        "bookmark.capture",
        json!({"display_name": "Card hidden", "current_page": true}),
    );
    let name = out["bookmark"].as_str().unwrap().to_string();
    assert_eq!(out["page"], json!(first));
    let doc = s.report.as_ref().unwrap().bookmarks[0].document.clone();
    assert_eq!(doc["displayName"], json!("Card hidden"));
    assert_eq!(doc["explorationState"]["activeSection"], json!(first));
    let containers = &doc["explorationState"]["sections"][&first]["visualContainers"];
    assert_eq!(
        containers[&card]["singleVisual"]["display"]["mode"],
        json!("hidden")
    );
    assert_eq!(doc["options"]["suppressActiveSection"], json!(false));

    let l = list(&mut s);
    assert_eq!(l["count"], json!(1));
    assert_eq!(l["bookmarks"][0]["name"], json!(name));
    assert_eq!(l["bookmarks"][0]["hidden"], json!(1));
    assert_eq!(l["bookmarks"][0]["page"], json!(first));

    // another state: page 2, the card shown; applying the bookmark goes back
    run(&mut s, "page.set_active", json!({"page": second}));
    run(
        &mut s,
        "visual.set_hidden",
        json!({"page": first, "visual": card, "hidden": false}),
    );
    let out = run(&mut s, "bookmark.apply", json!({"bookmark": name}));
    assert_eq!(out["changed"], json!(true));
    assert_eq!(out["page_changed"], json!(true));
    assert!(
        s.report.as_ref().unwrap().active_page.as_deref() == Some(first.as_str()),
        "the bookmark's page came back"
    );
    assert!(visuals(&s, 0)[0].1, "the card is hidden again");
    // a second application changes nothing
    let out = run(&mut s, "bookmark.apply", json!({"bookmark": name}));
    assert_eq!(out["changed"], json!(false), "{out}");

    // rename: the document's display name and the list
    let out = run(
        &mut s,
        "bookmark.set",
        json!({"bookmark": name, "display_name": "Wheels only"}),
    );
    assert_eq!(out["changed"], json!(true));
    let l = list(&mut s);
    assert_eq!(l["bookmarks"][0]["display_name"], json!("Wheels only"));
    // the same value again changes nothing
    let out = run(
        &mut s,
        "bookmark.set",
        json!({"bookmark": "Wheels only", "display_name": "Wheels only"}),
    );
    assert_eq!(out["changed"], json!(false), "{out}");

    // update: the capture follows the report, and re-running it changes nothing
    run(
        &mut s,
        "visual.set_hidden",
        json!({"page": first, "visual": card, "hidden": false}),
    );
    let out = run(&mut s, "bookmark.update", json!({"bookmark": name}));
    assert_eq!(out["changed"], json!(true));
    let out = run(&mut s, "bookmark.update", json!({"bookmark": name}));
    assert_eq!(out["changed"], json!(false), "{out}");
    let out = run(&mut s, "bookmark.apply", json!({"bookmark": name}));
    assert_eq!(out["hidden"], json!([]), "the bookmark now shows the card");
}

/// Data, Display and Current page: the options say what applying the bookmark leaves alone.
#[test]
fn bookmarks_apply_what_their_options_say() {
    let mut s = session();
    let first = page(&s, 0);
    let second = page(&s, 1);
    let card = visuals(&s, 0)[0].0.clone();
    // a filter the bookmark captures
    run(
        &mut s,
        "visual.filter",
        json!({"page": first, "visual": card, "field": "Sales[Category]", "in": ["Wheels"]}),
    );
    run(
        &mut s,
        "visual.set_hidden",
        json!({"page": first, "visual": card, "hidden": true}),
    );
    let name = run(
        &mut s,
        "bookmark.capture",
        json!({"display_name": "State", "current_page": true}),
    )["bookmark"]
        .as_str()
        .unwrap()
        .to_string();
    let doc = &s.report.as_ref().unwrap().bookmarks[0].document;
    assert!(
        doc["explorationState"]["sections"][&first]["visualContainers"][&card]["filters"]["byName"]
            .as_object()
            .is_some_and(|m| !m.is_empty()),
        "the visual filter was captured: {doc}"
    );

    // move away: page, hidden state and the filter
    run(&mut s, "page.set_active", json!({"page": second}));
    run(
        &mut s,
        "visual.set_hidden",
        json!({"page": first, "visual": card, "hidden": false}),
    );
    run(
        &mut s,
        "visual.clear_filters",
        json!({"page": first, "visual": card}),
    );
    // Data off: the filter stays as it is now
    run(&mut s, "bookmark.set", json!({"bookmark": name, "data": false}));
    let out = run(&mut s, "bookmark.apply", json!({"bookmark": name}));
    assert_eq!(out["page_changed"], json!(true));
    assert_eq!(out["filters_changed"], json!(false), "{out}");
    assert!(
        s.report.as_ref().unwrap().pages[0].visuals[0]
            .filter_config
            .is_none(),
        "Data off left the filters alone"
    );
    // Display off: the hidden state stays where it is now (shown), not the captured one
    run(
        &mut s,
        "visual.set_hidden",
        json!({"page": first, "visual": card, "hidden": false}),
    );
    run(
        &mut s,
        "bookmark.set",
        json!({"bookmark": name, "display": false}),
    );
    let out = run(&mut s, "bookmark.apply", json!({"bookmark": name}));
    assert_eq!(out["hidden"], json!([]), "{out}");
    assert!(!visuals(&s, 0)[0].1);
    // back to every option: the captured filter and hidden state come back
    run(
        &mut s,
        "bookmark.set",
        json!({"bookmark": name, "data": true, "display": true}),
    );
    let out = run(&mut s, "bookmark.apply", json!({"bookmark": name}));
    assert_eq!(out["filters_changed"], json!(true), "{out}");
    assert!(visuals(&s, 0)[0].1);
    assert!(
        s.report.as_ref().unwrap().pages[0].visuals[0]
            .filter_config
            .as_ref()
            .and_then(|c| c.get("filters"))
            .and_then(Value::as_array)
            .is_some_and(|f| !f.is_empty())
    );
    // Current page off: the page stays where the reader is
    run(&mut s, "page.set_active", json!({"page": second}));
    run(
        &mut s,
        "bookmark.set",
        json!({"bookmark": name, "current_page": false}),
    );
    let out = run(&mut s, "bookmark.apply", json!({"bookmark": name}));
    assert_eq!(out["page_changed"], json!(false), "{out}");
    assert_eq!(
        s.report.as_ref().unwrap().active_page,
        Some(second),
        "Current page off left the page alone"
    );
}

/// The order, a group and its bookmarks, and removal.
#[test]
fn bookmarks_order_groups_and_removal() {
    let mut s = session();
    for n in ["First", "Second", "Third"] {
        run(&mut s, "bookmark.capture", json!({"display_name": n}));
    }
    let names = bookmarks(&s);
    assert_eq!(names.len(), 3);
    // the order follows bookmarks.json
    let l = list(&mut s);
    let listed: Vec<&str> = l["bookmarks"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|b| b["display_name"].as_str())
        .collect();
    assert_eq!(listed, vec!["First", "Second", "Third"]);

    // a group takes the place of the first listed bookmark
    let out = run(
        &mut s,
        "bookmark.group",
        json!({"bookmarks": ["Second", "First"], "display_name": "Pair"}),
    );
    assert!(out["group"].as_str().is_some_and(|g| !g.is_empty()), "{out}");
    let l = list(&mut s);
    assert_eq!(l["groups"][0]["display_name"], json!("Pair"));
    let listed: Vec<&str> = l["bookmarks"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|b| b["display_name"].as_str())
        .collect();
    assert_eq!(listed, vec!["Second", "First", "Third"]);
    assert_eq!(l["bookmarks"][0]["group"], json!("Pair"));
    assert!(l["bookmarks"][2]["group"].is_null());

    // a capture goes into the group and the end of it
    run(
        &mut s,
        "bookmark.capture",
        json!({"display_name": "Fourth", "group": "Pair"}),
    );
    let l = list(&mut s);
    let listed: Vec<&str> = l["bookmarks"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|b| b["display_name"].as_str())
        .collect();
    assert_eq!(listed, vec!["Second", "First", "Fourth", "Third"]);

    // out of the group, to the end of the list
    let out = run(
        &mut s,
        "bookmark.set",
        json!({"bookmark": "First", "group": null}),
    );
    assert_eq!(out["changed"], json!(true));
    let l = list(&mut s);
    let listed: Vec<&str> = l["bookmarks"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|b| b["display_name"].as_str())
        .collect();
    assert_eq!(listed, vec!["Second", "Fourth", "Third", "First"]);
    // and to a position
    run(&mut s, "bookmark.set", json!({"bookmark": "First", "index": 0}));
    let l = list(&mut s);
    assert_eq!(l["bookmarks"][0]["display_name"], json!("First"));

    // a bookmark moves inside its group, and a group renames
    run(&mut s, "bookmark.set", json!({"bookmark": "Fourth", "index": 0}));
    let l = list(&mut s);
    let listed: Vec<&str> = l["bookmarks"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|b| b["display_name"].as_str())
        .collect();
    assert_eq!(listed, vec!["First", "Fourth", "Second", "Third"]);
    let out = run(
        &mut s,
        "bookmark.rename_group",
        json!({"group": "Pair", "display_name": "Pair renamed"}),
    );
    assert_eq!(out["changed"], json!(true), "{out}");
    let l = list(&mut s);
    assert_eq!(l["groups"][0]["display_name"], json!("Pair renamed"));
    assert_eq!(l["bookmarks"][1]["group"], json!("Pair renamed"));
    // the same name again changes nothing
    let out = run(
        &mut s,
        "bookmark.rename_group",
        json!({"group": "Pair renamed", "display_name": "Pair renamed"}),
    );
    assert_eq!(out["changed"], json!(false), "{out}");

    // ungrouping keeps the bookmarks in place
    let out = run(&mut s, "bookmark.ungroup", json!({"group": "Pair renamed"}));
    assert_eq!(out["bookmarks"].as_array().unwrap().len(), 2);
    let l = list(&mut s);
    assert_eq!(l["groups"], json!([]));
    assert_eq!(l["count"], json!(4));

    // removing a bookmark removes one; removing a group removes its bookmarks
    let names = bookmarks(&s);
    assert_eq!(names.len(), 4);
    let out = run(&mut s, "bookmark.remove", json!({"bookmark": "Third"}));
    assert_eq!(out["removed"], json!([names[2]]));
    let l = list(&mut s);
    assert_eq!(l["count"], json!(3));
    let out = run(
        &mut s,
        "bookmark.group",
        json!({"bookmarks": ["First", "Second"], "display_name": "Again"}),
    );
    let again = out["group"].as_str().unwrap().to_string();
    let out = run(&mut s, "bookmark.remove", json!({"bookmark": again}));
    assert_eq!(out["removed"].as_array().unwrap().len(), 2, "its bookmarks");
    assert_eq!(bookmarks(&s).len(), 1);
}

/// Hostile parameters fail without touching the report.
#[test]
fn bookmarks_reject_hostile_parameters() {
    let mut s = session();
    let name = run(&mut s, "bookmark.capture", json!({"display_name": "One"}))["bookmark"]
        .as_str()
        .unwrap()
        .to_string();
    let e = fails(&mut s, "bookmark.capture", json!({"display_name": ""}));
    assert!(e.contains("must not be empty"), "{e}");
    let e = fails(&mut s, "bookmark.capture", json!({"display_name": "  padded  "}));
    assert!(e.contains("whitespace"), "{e}");
    let e = fails(&mut s, "bookmark.capture", json!({"display_name": "One"}));
    assert!(e.contains("already exists"), "{e}");
    let e = fails(
        &mut s,
        "bookmark.capture",
        json!({"display_name": "Other", "group": "no such group"}),
    );
    assert!(e.contains("no bookmark group"), "{e}");
    let e = fails(
        &mut s,
        "bookmark.capture",
        json!({"display_name": "Other", "visuals": ["nope"]}),
    );
    assert!(e.contains("has no visual"), "{e}");
    let e = fails(
        &mut s,
        "bookmark.capture",
        json!({"display_name": "Other", "data": "yes"}),
    );
    assert!(e.contains("true or false"), "{e}");
    let e = fails(&mut s, "bookmark.apply", json!({"bookmark": "nope"}));
    assert!(e.contains("no bookmark named"), "{e}");
    let e = fails(&mut s, "bookmark.apply", json!({"bookmark": name, "x": 1}));
    assert!(e.contains("unknown parameter"), "{e}");
    let e = fails(&mut s, "bookmark.set", json!({"bookmark": name, "index": -1}));
    assert!(e.contains("whole number"), "{e}");
    let e = fails(
        &mut s,
        "bookmark.group",
        json!({"bookmarks": [], "display_name": "Empty"}),
    );
    assert!(e.contains("at least one"), "{e}");
    let e = fails(&mut s, "bookmark.ungroup", json!({"group": "no such group"}));
    assert!(e.contains("no bookmark group"), "{e}");
    // the bookmark is still there
    assert_eq!(bookmarks(&s).len(), 1);
}

/// A PBIP project round-trips the documents, `bookmarks.json` and their order.
#[test]
fn bookmarks_round_trip_through_a_pbip_project() {
    let dir = TempDir::new("bookmarks-round-trip");
    let mut s = session();
    run(
        &mut s,
        "project.new",
        json!({"dir": dir.path(), "name": "Shop", "format": "tmdl"}),
    );
    let pbip = dir.join("Shop.pbip");
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": pbip}));
    for n in ["First", "Second"] {
        run(&mut s, "bookmark.capture", json!({"display_name": n}));
    }
    run(
        &mut s,
        "bookmark.group",
        json!({"bookmarks": ["First", "Second"], "display_name": "Pair"}),
    );
    run(&mut s, "file.save", json!({"backup": false}));
    let mut again = Session::new();
    run(&mut again, "file.open", json!({"path": pbip}));
    let l = list(&mut again);
    assert_eq!(l["groups"][0]["display_name"], json!("Pair"));
    assert_eq!(l["count"], json!(2));
    let listed: Vec<&str> = l["bookmarks"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|b| b["display_name"].as_str())
        .collect();
    assert_eq!(listed, vec!["First", "Second"]);
    // the documents are the PBIR ones, with the schema the writer chose
    let doc = &again.report.as_ref().unwrap().bookmarks[0].document;
    assert!(
        doc["$schema"].as_str().is_some_and(|s| s.contains("/bookmark/")),
        "{doc}"
    );
}
