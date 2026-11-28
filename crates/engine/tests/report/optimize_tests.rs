//! The Optimize ribbon (`docs/gaps.md` G60): Pause visuals keeps each visual's last data (marked
//! `pending`) and evaluates nothing until Refresh visuals or a resume; the optimization presets
//! write `slowDataSourceSettings` and turn the default cross-highlighting and cross-filtering
//! off; both are kept by PBIP, PBIX and a legacy layout.

use std::path::Path;

use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::Session;

const SYNTHETIC_METADATA: &[u8] =
    include_bytes!("../../../metadata/tests/fixtures/synthetic_metadata.sqlitedb");

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Sales(Category, Amount) with [Total]; one page with a card of [Total] and a column chart of
/// [Total] by Category.
fn shop() -> (Session, String, String, String) {
    let mut s = Session::new();
    let sales = r#"#table({"Category", "Amount"}, {{"Wheels", 100}, {"Wheels", 50}, {"Toys", 5}})"#;
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
    let add = |s: &mut Session, ty: &str, fields: Value| {
        run(
            s,
            "visual.add",
            json!({"page": page, "visual_type": ty, "x": 0, "y": 0, "width": 200, "height": 200, "fields": fields}),
        )["visual"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let card = add(&mut s, "card", json!([{"role": "Values", "field": "[Total]"}]));
    let chart = add(
        &mut s,
        "clusteredColumnChart",
        json!([{"role": "Category", "field": "Sales[Category]"}, {"role": "Y", "field": "[Total]"}]),
    );
    (s, page, card, chart)
}

fn data(snap: &Value, visual: &str) -> Value {
    for p in snap["report"]["pages"].as_array().cloned().unwrap_or_default() {
        for v in p["visuals"].as_array().cloned().unwrap_or_default() {
            if v["name"] == visual {
                return v["data"].clone();
            }
        }
    }
    panic!("visual {visual} not in the snapshot")
}

fn visual(snap: &Value, name: &str) -> Value {
    for p in snap["report"]["pages"].as_array().cloned().unwrap_or_default() {
        for v in p["visuals"].as_array().cloned().unwrap_or_default() {
            if v["name"] == name {
                return v;
            }
        }
    }
    panic!("visual {name} not in the snapshot")
}

/// The card's value in a snapshot.
fn total(snap: &Value, card: &str) -> f64 {
    let d = data(snap, card);
    assert_eq!(d["status"], "ok", "{d}");
    d["rows"][0][0].as_f64().unwrap_or_else(|| panic!("{d}"))
}

#[test]
fn paused_visuals_keep_their_last_data_until_refreshed() {
    let (mut s, page, card, _) = shop();
    assert_eq!(total(&s.snapshot(), &card), 155.0);
    let out = run(&mut s, "report.pause_visuals", json!({}));
    assert_eq!(
        (out["paused"].as_bool(), out["changed"].as_bool()),
        (Some(true), Some(true))
    );
    assert_eq!(s.report.as_ref().unwrap().extra["settings"]["pauseQueries"], true);
    let snap = s.snapshot();
    assert_eq!(snap["report"]["settings"]["paused"], true);
    // nothing changed yet: the cached data, not pending
    assert_eq!(total(&snap, &card), 155.0);
    assert!(data(&snap, &card).get("pending").is_none());
    // a measure change would need an evaluation: the card keeps 155, marked pending
    run(
        &mut s,
        "measure.set",
        json!({"name": "Total", "expression": "SUM(Sales[Amount]) * 2"}),
    );
    let snap = s.snapshot();
    assert_eq!(total(&snap, &card), 155.0);
    assert_eq!(data(&snap, &card)["pending"], true);
    // a new visual has no earlier state: the pending status
    let added = run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "card", "x": 300, "y": 300, "width": 100, "height": 100,
               "fields": [{"role": "Values", "field": "[Total]"}]}),
    )["visual"]
        .as_str()
        .unwrap()
        .to_string();
    let snap = s.snapshot();
    assert_eq!(data(&snap, &added)["status"], "pending");
    // Refresh visuals: the next view evaluates once, the report stays paused
    let r = run(&mut s, "report.refresh_visuals", json!({}));
    assert_eq!(r["paused"], true);
    let snap = s.snapshot();
    assert_eq!(total(&snap, &card), 310.0);
    assert!(data(&snap, &card).get("pending").is_none());
    assert_eq!(total(&snap, &added), 310.0);
    assert_eq!(snap["report"]["settings"]["paused"], true);
    // the next change is pending again
    run(
        &mut s,
        "measure.set",
        json!({"name": "Total", "expression": "SUM(Sales[Amount]) * 3"}),
    );
    assert_eq!(total(&s.snapshot(), &card), 310.0);
    // resume: everything evaluates
    let out = run(&mut s, "report.pause_visuals", json!({"paused": false}));
    assert_eq!(out["paused"], false);
    let snap = s.snapshot();
    assert_eq!(total(&snap, &card), 465.0);
    assert!(
        s.report.as_ref().unwrap().extra["settings"]
            .get("pauseQueries")
            .is_none()
    );
    // pausing is one undo step
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.snapshot()["report"]["settings"]["paused"], true);
}

#[test]
fn the_presets_write_query_reduction_and_turn_default_interactions_off() {
    let (mut s, page, card, chart) = shop();
    let out = run(
        &mut s,
        "report.query_reduction",
        json!({"preset": "query_reduction"}),
    );
    assert_eq!(out["changed"], true);
    assert_eq!(out["preset"], "query_reduction");
    assert_eq!(
        s.report.as_ref().unwrap().extra["slowDataSourceSettings"],
        json!({"isCrossHighlightingDisabled": true, "isSlicerSelectionsButtonEnabled": true,
               "isApplyAllButtonEnabled": true})
    );
    // a selection in the chart no longer reaches the card by default
    run(
        &mut s,
        "view.interact",
        json!({"action": "select", "page": page, "visual": chart, "point": [{"c": 0, "v": "Toys"}]}),
    );
    let snap = s.snapshot();
    assert!(visual(&snap, &card)["interaction"].get("filtered_by").is_none());
    assert_eq!(total(&snap, &card), 155.0);
    // an Edit interactions rule still applies
    run(
        &mut s,
        "page.interactions",
        json!({"page": page, "rules": [{"source": chart, "target": card, "type": "filter"}]}),
    );
    let snap = s.snapshot();
    assert_eq!(visual(&snap, &card)["interaction"]["filtered_by"], json!(chart));
    assert_eq!(total(&snap, &card), 5.0);
    // Customize: one option; the preset reads customized
    let out = run(
        &mut s,
        "report.query_reduction",
        json!({"slicers": "instantly", "filters": "apply_buttons"}),
    );
    assert_eq!(out["preset"], "customized");
    assert_eq!(
        s.report.as_ref().unwrap().extra["slowDataSourceSettings"],
        json!({"isCrossHighlightingDisabled": true, "isFilterSelectionsButtonEnabled": true})
    );
    assert_eq!(
        s.snapshot()["report"]["settings"]["query_reduction"],
        json!({"preset": "customized", "disable_cross_highlighting": true, "slicers": "instantly",
               "filters": "apply_buttons"})
    );
    // Interactivity: Desktop's default, nothing left in the file
    let out = run(
        &mut s,
        "report.query_reduction",
        json!({"preset": "interactivity"}),
    );
    assert_eq!(out["preset"], "interactivity");
    assert!(
        s.report
            .as_ref()
            .unwrap()
            .extra
            .get("slowDataSourceSettings")
            .is_none()
    );
    assert_eq!(
        run(
            &mut s,
            "report.query_reduction",
            json!({"preset": "interactivity"})
        )["changed"],
        false
    );
}

fn reopen(path: &Path) -> Session {
    let mut back = Session::new();
    run(&mut back, "file.open", json!({"path": path}));
    back
}

fn check_kept(s: &Session, label: &str) {
    let r = s.report.as_ref().unwrap();
    assert_eq!(r.extra["settings"]["pauseQueries"], true, "{label}");
    assert_eq!(
        r.extra["slowDataSourceSettings"],
        json!({"isCrossHighlightingDisabled": true, "isSlicerSelectionsButtonEnabled": true,
               "isApplyAllButtonEnabled": true}),
        "{label}"
    );
    assert_eq!(
        s.snapshot()["report"]["settings"]["query_reduction"]["preset"],
        "query_reduction",
        "{label}"
    );
}

#[test]
fn files_keep_the_pause_and_the_preset() {
    let dir = TempDir::new("optimize-files");
    for file in ["Shop.pbip", "Shop.pbix"] {
        let (mut s, ..) = shop();
        run(&mut s, "report.pause_visuals", json!({}));
        run(
            &mut s,
            "report.query_reduction",
            json!({"preset": "query_reduction"}),
        );
        run(&mut s, "file.save_as", json!({"path": dir.join(file)}));
        check_kept(&reopen(&dir.join(file)), file);
    }
    for pbir in [false, true] {
        let path = dir.join(if pbir { "Pbir.pbix" } else { "Legacy.pbix" });
        let pkg = if pbir {
            bicraft_pbix::synthetic::pbir_pbix(SYNTHETIC_METADATA)
        } else {
            bicraft_pbix::synthetic::pbix(SYNTHETIC_METADATA)
        }
        .unwrap();
        std::fs::write(&path, pkg.to_bytes().unwrap()).unwrap();
        let mut s = reopen(&path);
        run(&mut s, "report.pause_visuals", json!({"paused": true}));
        run(
            &mut s,
            "report.query_reduction",
            json!({"preset": "query_reduction"}),
        );
        run(&mut s, "file.save", json!({}));
        check_kept(&reopen(&path), if pbir { "pbir parts" } else { "legacy layout" });
    }
}

#[test]
fn bad_optimize_params_are_refused() {
    let (mut s, ..) = shop();
    for (id, bad) in [
        ("report.pause_visuals", json!({"paused": "yes"})),
        ("report.pause_visuals", json!({"pause": true})),
        ("report.refresh_visuals", json!({"page": "x"})),
        ("report.query_reduction", json!({"preset": "fast"})),
        ("report.query_reduction", json!({"slicers": "later"})),
        ("report.query_reduction", json!({"filters": 1})),
        (
            "report.query_reduction",
            json!({"disable_cross_highlighting": "no"}),
        ),
    ] {
        assert!(s.execute(id, &bad).is_err(), "{id} {bad}");
    }
    s.report
        .as_mut()
        .unwrap()
        .extra
        .insert("slowDataSourceSettings".into(), json!([]));
    assert!(
        s.execute("report.query_reduction", &json!({"preset": "interactivity"}))
            .is_err()
    );
    let mut empty = Session::new();
    for id in [
        "report.pause_visuals",
        "report.refresh_visuals",
        "report.query_reduction",
    ] {
        assert!(empty.execute(id, &json!({})).is_err(), "{id}");
    }
}

#[test]
fn a_visuals_refresh_evaluates_it_alone_while_paused() {
    let (mut s, page, card, chart) = shop();
    let chart_before = data(&s.snapshot(), &chart);
    run(&mut s, "report.pause_visuals", json!({}));
    run(
        &mut s,
        "measure.set",
        json!({"name": "Total", "expression": "SUM(Sales[Amount]) * 2"}),
    );
    let snap = s.snapshot();
    assert_eq!(data(&snap, &card)["pending"], true);
    assert_eq!(data(&snap, &chart)["pending"], true);
    // the card's own Refresh: the card evaluates, the chart keeps its last state
    let out = run(&mut s, "visual.refresh", json!({"page": page, "visual": card}));
    assert_eq!(
        (out["paused"].as_bool(), out["visual"].as_str()),
        (Some(true), Some(card.as_str()))
    );
    let snap = s.snapshot();
    assert!(total(&snap, &card) > 155.0);
    assert!(data(&snap, &card).get("pending").is_none());
    let chart_now = data(&snap, &chart);
    assert_eq!(chart_now["pending"], true);
    assert_eq!(chart_now["rows"], chart_before["rows"]);
    assert_eq!(snap["report"]["settings"]["paused"], true);
    // once: the next change waits again
    run(
        &mut s,
        "measure.set",
        json!({"name": "Total", "expression": "SUM(Sales[Amount]) * 3"}),
    );
    assert_eq!(data(&s.snapshot(), &card)["pending"], true);
    // the report is not changed (no undo step, no unsaved edit)
    assert!(!bicraft_engine::is_mutating("visual.refresh"));
    for bad in [
        json!({"page": page}),
        json!({"page": page, "visual": "nope"}),
        json!({"page": "nope", "visual": card}),
        json!({"page": page, "visual": card, "all": true}),
    ] {
        assert!(s.execute("visual.refresh", &bad).is_err(), "{bad}");
    }
}

/// The card's title bound to [Total] (a measure-bound formatting value).
fn bind_title(s: &mut Session, card: &str) {
    let r = s.report.as_mut().unwrap();
    let v = r.pages[0].visuals.iter_mut().find(|v| v.name == card).unwrap();
    v.visual_extra.insert(
        "visualContainerObjects".into(),
        json!({"title": [{"properties": {
            "show": {"expr": {"Literal": {"Value": "true"}}},
            "text": {"expr": {"Measure": {"Expression": {"SourceRef": {"Entity": "Sales"}}, "Property": "Total"}}}
        }}]}),
    );
}

#[test]
fn measure_bound_formatting_waits_while_paused() {
    let (mut s, page, card, _) = shop();
    bind_title(&mut s, &card);
    let before = visual(&s.snapshot(), &card)["title"].clone();
    assert!(before.is_string(), "{before}");
    run(&mut s, "report.pause_visuals", json!({}));
    run(
        &mut s,
        "measure.set",
        json!({"name": "Total", "expression": "SUM(Sales[Amount]) * 2"}),
    );
    // the bound title is not evaluated: the one shown before stays
    assert_eq!(visual(&s.snapshot(), &card)["title"], before);
    run(&mut s, "visual.refresh", json!({"page": page, "visual": card}));
    assert_ne!(visual(&s.snapshot(), &card)["title"], before);
}

#[test]
fn report_page_tooltips_are_off_while_paused() {
    let (mut s, page, _, chart) = shop();
    run(&mut s, "page.add", json!({"display_name": "Tip"}));
    let tip = s.report.as_ref().unwrap().pages[1].name.clone();
    run(&mut s, "page.set_type", json!({"page": tip, "type": "tooltip"}));
    run(
        &mut s,
        "visual.tooltip_page",
        json!({"page": page, "visual": chart, "tooltip_page": tip}),
    );
    let hover = json!({"action": "tooltip", "page": page, "visual": chart, "point": [{"c": 0, "v": "Toys"}]});
    run(&mut s, "view.interact", hover.clone());
    run(&mut s, "report.pause_visuals", json!({}));
    let e = s.execute("view.interact", &hover).unwrap_err();
    assert!(e.contains("paused"), "{e}");
    // hiding a tooltip still works
    run(
        &mut s,
        "view.interact",
        json!({"action": "tooltip", "page": page, "visual": chart, "point": null}),
    );
    run(&mut s, "report.pause_visuals", json!({"paused": false}));
    run(&mut s, "view.interact", hover);
}

/// Sales with a Category slicer next to the card.
fn with_slicer(s: &mut Session, page: &str) -> String {
    run(
        s,
        "visual.add",
        json!({"page": page, "visual_type": "slicer", "x": 400, "y": 0, "width": 150, "height": 200,
               "fields": [{"role": "Values", "field": "Sales[Category]"}]}),
    )["visual"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn query_reductions_slicer_apply_button_holds_a_slicers_changes() {
    let (mut s, page, card, _) = shop();
    let slicer = with_slicer(&mut s, &page);
    let toys = json!({"action": "slicer_toggle", "page": page, "visual": slicer, "value": "Toys"});
    // instantly: a click filters the card
    run(&mut s, "view.interact", toys.clone());
    assert_eq!(total(&s.snapshot(), &card), 5.0);
    assert!(
        visual(&s.snapshot(), &slicer)["slicer"]
            .get("apply_button")
            .is_none()
    );
    run(&mut s, "view.interact", json!({"action": "reset"}));
    run(
        &mut s,
        "report.query_reduction",
        json!({"slicers": "apply_button"}),
    );
    let out = run(&mut s, "view.interact", toys);
    assert_eq!(out["pending"], true);
    let snap = s.snapshot();
    assert_eq!(total(&snap, &card), 155.0, "the change waits");
    assert_eq!(visual(&snap, &slicer)["slicer"]["apply_button"], true);
    assert_eq!(visual(&snap, &slicer)["slicer"]["pending"], true);
    // the slicer's Apply
    let out = run(
        &mut s,
        "view.interact",
        json!({"action": "slicer_apply", "page": page, "visual": slicer}),
    );
    assert_eq!(out["applied"], true);
    let snap = s.snapshot();
    assert_eq!(total(&snap, &card), 5.0);
    assert!(visual(&snap, &slicer)["slicer"].get("pending").is_none());
    // nothing pending: no change
    let out = run(
        &mut s,
        "view.interact",
        json!({"action": "slicer_apply", "page": page, "visual": slicer}),
    );
    assert_eq!(out["applied"], false);
    assert!(
        s.execute(
            "view.interact",
            &json!({"action": "slicer_apply", "page": page, "visual": card})
        )
        .is_err()
    );
}
