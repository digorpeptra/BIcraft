//! Tests of `slicer_sync.rs`: the slicers' sync groups (Desktop's "Sync slicers" pane), the
//! Fields / Filters options, taking a slicer out of its group, hostile parameters, and the PBIP
//! round trip of `syncGroup`.

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

/// A report of two pages, each with a slicer on `Sales[Category]` and a card.
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
    for (i, page) in [first, second].into_iter().enumerate() {
        run(
            &mut s,
            "visual.add",
            json!({"page": page, "visual_type": "slicer", "x": 20, "y": 20 + i * 130,
                   "width": 220, "height": 120,
                   "fields": [{"role": "Values", "field": "Sales[Category]"}]}),
        );
        run(
            &mut s,
            "visual.add",
            json!({"page": page, "visual_type": "card", "x": 260, "y": 20, "width": 200,
                   "height": 120, "fields": [{"role": "Values", "field": "[Total]"}]}),
        );
    }
    s
}

fn slicers(s: &Session) -> Vec<(String, String)> {
    s.report
        .as_ref()
        .map(|r| {
            r.pages
                .iter()
                .flat_map(|p| {
                    p.visuals
                        .iter()
                        .filter(|v| v.visual_type.as_deref() == Some("slicer"))
                        .map(move |v| (p.name.clone(), v.name.clone()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn group_of(s: &Session, page: &str, visual: &str) -> Value {
    s.report
        .as_ref()
        .and_then(|r| r.pages.iter().find(|p| p.name == page))
        .and_then(|p| p.visuals.iter().find(|v| v.name == visual))
        .and_then(|v| v.visual_extra.get("syncGroup").cloned())
        .unwrap_or(Value::Null)
}

fn list(s: &mut Session) -> Value {
    run(s, "slicer.sync_list", json!({}))
}

/// Two slicers of two pages share a group; the list shows both and their group; taking one out
/// leaves the other, and the same call again changes nothing.
#[test]
fn slicers_share_a_group_and_leave_it() {
    let mut s = session();
    let list0 = list(&mut s);
    assert_eq!(list0["count"], json!(2), "{list0}");
    assert!(
        list0["groups"].as_array().is_some_and(Vec::is_empty),
        "no group yet: {list0}"
    );
    let all = slicers(&s);
    let out = run(
        &mut s,
        "slicer.set_sync",
        json!({"segments": [{"page": all[0].0, "visual": all[0].1},
                            {"page": all[1].0, "visual": all[1].1}],
               "group": "Category"}),
    );
    assert_eq!(out["changed"], json!(true), "{out}");
    for (page, visual) in &all {
        let g = group_of(&s, page, visual);
        assert_eq!(g["groupName"], json!("Category"), "{g}");
        assert_eq!(g["fieldChanges"], json!(true), "{g}");
        assert_eq!(g["filterChanges"], json!(true), "{g}");
    }
    let l = list(&mut s);
    assert_eq!(l["groups"][0]["name"], json!("Category"));
    assert_eq!(l["groups"][0]["segments"].as_array().unwrap().len(), 2);
    assert_eq!(l["segments"][0]["group"], json!("Category"));
    // the same call again changes nothing
    let out = run(
        &mut s,
        "slicer.set_sync",
        json!({"page": all[0].0, "visual": all[0].1, "group": "Category"}),
    );
    assert_eq!(out["changed"], json!(false), "{out}");
    // the options are written, and only the filters are synchronized
    let out = run(
        &mut s,
        "slicer.set_sync",
        json!({"page": all[0].0, "visual": all[0].1, "group": "Category",
               "field_changes": false, "filter_changes": true}),
    );
    assert_eq!(out["changed"], json!(true), "{out}");
    let g = group_of(&s, &all[0].0, &all[0].1);
    assert_eq!(g["fieldChanges"], json!(false));
    assert_eq!(g["filterChanges"], json!(true));
    // out of the group, the other keeps it
    let out = run(
        &mut s,
        "slicer.set_sync",
        json!({"page": all[1].0, "visual": all[1].1, "group": null}),
    );
    assert_eq!(out["changed"], json!(true), "{out}");
    assert_eq!(group_of(&s, &all[1].0, &all[1].1), Value::Null);
    assert_eq!(group_of(&s, &all[0].0, &all[0].1)["groupName"], json!("Category"));
    let l = list(&mut s);
    assert_eq!(l["groups"][0]["segments"].as_array().unwrap().len(), 1);
    assert!(l["segments"][1]["group"].is_null());
}

/// Hostile parameters fail without touching the report.
#[test]
fn slicers_reject_hostile_parameters() {
    let mut s = session();
    let all = slicers(&s);
    let e = fails(&mut s, "slicer.set_sync", json!({"page": all[0].0, "group": "x"}));
    assert!(e.contains("missing parameter `visual`"), "{e}");
    let e = fails(&mut s, "slicer.set_sync", json!({"segments": [], "group": "x"}));
    assert!(e.contains("at least one"), "{e}");
    let e = fails(
        &mut s,
        "slicer.set_sync",
        json!({"page": all[0].0, "visual": "nope", "group": "x"}),
    );
    assert!(e.contains("has no visual"), "{e}");
    let e = fails(
        &mut s,
        "slicer.set_sync",
        json!({"page": all[0].0, "visual": all[0].1}),
    );
    assert!(e.contains("missing parameter `group`"), "{e}");
    let e = fails(
        &mut s,
        "slicer.set_sync",
        json!({"page": all[0].0, "visual": all[0].1, "group": "  "}),
    );
    assert!(e.contains("must not be empty"), "{e}");
    let e = fails(
        &mut s,
        "slicer.set_sync",
        json!({"page": all[0].0, "visual": all[0].1, "group": "x", "field_changes": "yes"}),
    );
    assert!(e.contains("true or false"), "{e}");
    // a card is not a slicer
    let card = s.report.as_ref().unwrap().pages[0]
        .visuals
        .iter()
        .find(|v| v.visual_type.as_deref() == Some("card"))
        .map(|v| v.name.clone())
        .unwrap();
    let e = fails(
        &mut s,
        "slicer.set_sync",
        json!({"page": all[0].0, "visual": card, "group": "x"}),
    );
    assert!(e.contains("is not a slicer"), "{e}");
    assert_eq!(group_of(&s, &all[0].0, &all[0].1), Value::Null);
}

/// A PBIP project round-trips `syncGroup` and the list keeps it.
#[test]
fn sync_groups_round_trip_through_a_pbip_project() {
    let dir = TempDir::new("slicer-sync-round-trip");
    let mut s = session();
    run(
        &mut s,
        "project.new",
        json!({"dir": dir.path(), "name": "Shop", "format": "tmdl"}),
    );
    let pbip = dir.join("Shop.pbip");
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": pbip}));
    let all = slicers(&s);
    assert_eq!(all.len(), 2, "the project keeps its slicers");
    run(
        &mut s,
        "slicer.set_sync",
        json!({"page": all[0].0, "visual": all[0].1, "group": "Category"}),
    );
    run(&mut s, "file.save", json!({"backup": false}));
    let mut again = Session::new();
    run(&mut again, "file.open", json!({"path": pbip}));
    let g = group_of(&again, &all[0].0, &all[0].1);
    assert_eq!(g["groupName"], json!("Category"), "{g}");
    let l = list(&mut again);
    assert_eq!(l["groups"][0]["name"], json!("Category"));
    assert_eq!(l["groups"][0]["segments"].as_array().unwrap().len(), 1);
}
