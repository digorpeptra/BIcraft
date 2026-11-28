//! The Optimize group (`optimize_ui.rs`, G60) through the accessibility tree (egui_kittest, no
//! GPU), over an engine session: Pause visuals toggles `report.pause_visuals` and shows the
//! paused banner, whose Refresh runs `report.refresh_visuals` and Resume visual queries resumes;
//! Optimization presets runs `report.query_reduction` or opens File › Options and settings, whose
//! Query reduction section sets each option; in English and French.

use std::path::Path;

use bicraft_engine::Session;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::backstage_tests::{engine_shell, steps};
use super::harness;
use crate::App;
use crate::backstage::Page;
use crate::registry::Tab as RTab;
use crate::shell::Caps;

fn session() -> Session {
    let mut s = Session::new();
    let sales = r#"#table({"Region", "Amount"}, {{"North", 100}, {"South", 50}})"#;
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Region", "data_type": "string"}),
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
        s.execute(id, &p).unwrap();
    }
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    s.execute(
        "visual.add",
        &json!({"page": page, "visual_type": "card", "x": 20, "y": 20, "width": 300, "height": 200,
                "fields": [{"role": "Values", "field": "[Total]"}]}),
    )
    .unwrap();
    s.dirty = false;
    s.report_dirty = false;
    s
}

fn app(lang: &str) -> Harness<'static, App> {
    let mut app = App::new(lang, engine_shell(Caps::DESKTOP, session)).with_sync_render();
    app.open_path(Path::new("r.pbip"));
    let mut h = harness(app, (1500.0, 900.0));
    steps(&mut h, 6);
    h.state_mut().ribbon_tab = RTab::Optimize;
    steps(&mut h, 3);
    h
}

fn sent(h: &Harness<'_, App>) -> Vec<&'static str> {
    h.state().sent.iter().map(|c| c.id).collect()
}

fn last(h: &Harness<'_, App>, id: &str) -> Value {
    h.state()
        .sent
        .iter()
        .rev()
        .find(|c| c.id == id)
        .map(|c| c.params.clone())
        .unwrap_or(Value::Null)
}

fn setting(h: &Harness<'_, App>, key: &str) -> Value {
    h.state()
        .loaded
        .as_ref()
        .and_then(|l| l.snap.pointer(&format!("/report/settings/{key}")))
        .cloned()
        .unwrap_or(Value::Null)
}

#[test]
fn pause_visuals_shows_the_banner_and_resumes() {
    let mut h = app("en-US");
    assert!(h.query_by_label("Resume visual queries").is_none());
    h.get_by_label("Pause visuals").click();
    steps(&mut h, 4);
    assert_eq!(last(&h, "report.pause_visuals"), json!({"paused": true}));
    assert_eq!(setting(&h, "paused"), json!(true));
    h.get_by_label_contains("Visual queries are paused");
    // the banner's Refresh: Refresh visuals
    h.get_by_label("Refresh").click();
    steps(&mut h, 4);
    assert_eq!(sent(&h).last(), Some(&"report.refresh_visuals"));
    assert_eq!(setting(&h, "paused"), json!(true), "still paused");
    // the ribbon's Refresh all visuals runs the same
    h.get_by_label("Refresh all visuals").click();
    steps(&mut h, 4);
    assert_eq!(
        sent(&h)
            .iter()
            .filter(|id| **id == "report.refresh_visuals")
            .count(),
        2
    );
    // Resume visual queries
    h.get_by_label("Resume visual queries").click();
    steps(&mut h, 4);
    assert_eq!(last(&h, "report.pause_visuals"), json!({"paused": false}));
    assert_eq!(setting(&h, "paused"), json!(false));
    assert!(h.query_by_label("Resume visual queries").is_none());
    // the toggle pauses and resumes
    h.get_by_label("Pause visuals").click();
    steps(&mut h, 4);
    h.get_by_label("Pause visuals").click();
    steps(&mut h, 4);
    assert_eq!(last(&h, "report.pause_visuals"), json!({"paused": false}));
}

#[test]
fn the_presets_menu_applies_a_preset_or_opens_the_options() {
    let mut h = app("en-US");
    h.get_by_label("Optimization presets").click();
    steps(&mut h, 3);
    h.get_by_label("Query reduction").click();
    steps(&mut h, 4);
    assert_eq!(
        last(&h, "report.query_reduction"),
        json!({"preset": "query_reduction"})
    );
    assert_eq!(setting(&h, "query_reduction")["preset"], "query_reduction");
    h.get_by_label("Optimization presets").click();
    steps(&mut h, 3);
    h.get_by_label("Interactivity").click();
    steps(&mut h, 4);
    assert_eq!(setting(&h, "query_reduction")["preset"], "interactivity");
    // Customize… opens File › Options and settings
    h.get_by_label("Optimization presets").click();
    steps(&mut h, 3);
    h.get_by_label("Customize…").click();
    steps(&mut h, 3);
    assert_eq!(h.state().backstage.as_ref().map(|b| b.page), Some(Page::Options));
}

#[test]
fn the_query_reduction_section_sets_each_option() {
    let mut h = app("en-US");
    let mut bs = crate::backstage::Backstage::new(Page::Options);
    bs.page = Page::Options;
    h.state_mut().backstage = Some(bs);
    steps(&mut h, 3);
    for (label, params) in [
        (
            "Disable cross highlighting/filtering by default",
            json!({"disable_cross_highlighting": true}),
        ),
        (
            "Add an Apply button to each slicer",
            json!({"slicers": "apply_button"}),
        ),
        (
            "Add a single Apply button to the filter pane to apply changes at once",
            json!({"filters": "single_button"}),
        ),
    ] {
        h.get_by_label(label).click();
        steps(&mut h, 4);
        assert_eq!(last(&h, "report.query_reduction"), params, "{label}");
    }
    assert_eq!(setting(&h, "query_reduction")["preset"], "query_reduction");
    h.get_by_label("Add an Apply button to each basic filter").click();
    steps(&mut h, 4);
    assert_eq!(setting(&h, "query_reduction")["preset"], "customized");
}

#[test]
fn the_optimize_group_speaks_french() {
    let mut h = app("fr-FR");
    h.get_by_label("Suspendre les visuels").click();
    steps(&mut h, 4);
    h.get_by_label_contains("Les requêtes des visuels sont en pause");
    h.get_by_label("Reprendre les requêtes des visuels");
    h.get_by_label("Actualiser tous les visuels");
    h.get_by_label("Préréglages d'optimisation").click();
    steps(&mut h, 3);
    h.get_by_label("Réduction des requêtes");
    h.get_by_label("Interactivité");
    h.get_by_label("Personnaliser…");
}

fn run_now(h: &mut Harness<'static, App>, id: &'static str, params: Value) {
    h.state_mut()
        .run(crate::actions::Call { id, params }, crate::app::Then::Nothing);
    steps(h, 4);
}

#[test]
fn the_optimize_tab_holds_pause_refresh_presets_and_the_performance_analyzer() {
    let mut h = app("en-US");
    h.get_by_label("Pause visuals");
    h.get_by_label("Refresh all visuals");
    h.get_by_label("Optimization presets");
    h.get_by_label("Performance analyzer").click();
    steps(&mut h, 4);
    assert!(h.state().cmd.perf.open, "the pane opens");
    // the tab row has it, after View
    let tabs = crate::ribbon::tabs(h.state());
    let at = |t: RTab| tabs.iter().position(|x| *x == t);
    assert!(at(RTab::Optimize) > at(RTab::View));
    // Analyze no longer carries the Optimize group
    h.state_mut().ribbon_tab = RTab::Analyze;
    steps(&mut h, 3);
    assert!(h.query_by_label("Pause visuals").is_none());
}

#[test]
fn a_waiting_visual_shows_pending_changes_and_refreshes_alone() {
    let mut h = app("en-US");
    h.get_by_label("Pause visuals").click();
    steps(&mut h, 4);
    assert!(h.query_by_label("Pending changes").is_none(), "nothing waits yet");
    run_now(
        &mut h,
        "measure.set",
        json!({"name": "Total", "expression": "SUM(Sales[Amount]) * 2"}),
    );
    // an icon and the words, with the visual's own Refresh
    h.get_by_label("Pending changes");
    h.get_by_label("Refresh visual").click();
    steps(&mut h, 4);
    let p = last(&h, "visual.refresh");
    let card = h.state().loaded.as_ref().unwrap().snap["report"]["pages"][0]["visuals"][0]["name"].clone();
    assert_eq!(p["visual"], card);
    assert!(p["page"].is_string());
    assert!(h.query_by_label("Pending changes").is_none(), "refreshed");
    assert_eq!(setting(&h, "paused"), json!(true), "still paused");
    // resumed: no strip even after a change
    h.get_by_label("Resume visual queries").click();
    steps(&mut h, 4);
    run_now(
        &mut h,
        "measure.set",
        json!({"name": "Total", "expression": "SUM(Sales[Amount]) * 3"}),
    );
    assert!(h.query_by_label("Pending changes").is_none());
}

#[test]
fn the_pending_strip_speaks_french() {
    let mut h = app("fr-FR");
    h.get_by_label("Suspendre les visuels").click();
    steps(&mut h, 4);
    run_now(
        &mut h,
        "measure.set",
        json!({"name": "Total", "expression": "SUM(Sales[Amount]) * 2"}),
    );
    h.get_by_label("Modifications en attente");
    h.get_by_label("Actualiser le visuel");
}

/// The shop with a Region slicer and query reduction's Apply button on each slicer.
fn slicer_session() -> Session {
    let mut s = session();
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    s.execute(
        "visual.add",
        &json!({"page": page, "visual_type": "slicer", "x": 400, "y": 20, "width": 200, "height": 200,
                "fields": [{"role": "Values", "field": "Sales[Region]"}]}),
    )
    .unwrap();
    s.execute("report.query_reduction", &json!({"slicers": "apply_button"}))
        .unwrap();
    s.dirty = false;
    s.report_dirty = false;
    s
}

#[test]
fn a_slicers_apply_button_applies_its_waiting_change() {
    let mut app = App::new("en-US", engine_shell(Caps::DESKTOP, slicer_session)).with_sync_render();
    app.open_path(Path::new("r.pbip"));
    let mut h = harness(app, (1500.0, 900.0));
    steps(&mut h, 8);
    let slicer = h.state().loaded.as_ref().unwrap().snap["report"]["pages"][0]["visuals"][1]["name"]
        .as_str()
        .unwrap()
        .to_string();
    let page = h.state().loaded.as_ref().unwrap().snap["report"]["pages"][0]["name"].clone();
    h.get_by_label(&format!("Apply ({slicer}): nothing to apply"));
    h.state_mut()
        .interact(json!({"action": "slicer_toggle", "page": page, "visual": slicer, "value": "North"}));
    steps(&mut h, 4);
    h.get_by_label(&format!("Apply ({slicer}): changes waiting"))
        .click();
    steps(&mut h, 4);
    assert_eq!(
        h.state().last_action,
        Some(json!({"action": "slicer_apply", "page": page, "visual": slicer}))
    );
    h.get_by_label(&format!("Apply ({slicer}): nothing to apply"));
}

/// The shop with a page filter card on Region and query reduction's `filters` option.
fn filtered(filters: &str) -> Session {
    let mut s = session();
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    s.execute("page.filter", &json!({"page": page, "field": "Sales[Region]"}))
        .unwrap();
    s.execute("report.query_reduction", &json!({"filters": filters}))
        .unwrap();
    s.dirty = false;
    s.report_dirty = false;
    s
}

fn filters_app(make: fn() -> Session) -> Harness<'static, App> {
    let mut app = App::new("en-US", engine_shell(Caps::DESKTOP, make)).with_sync_render();
    app.open_path(Path::new("r.pbip"));
    let mut h = harness(app, (1600.0, 900.0));
    steps(&mut h, 8);
    h.state_mut().prefs.panes.filters = true;
    let page = h.state().loaded.as_ref().unwrap().snap["report"]["pages"][0]["name"]
        .as_str()
        .unwrap()
        .to_string();
    h.state_mut()
        .filters
        .open
        .insert(format!("page:{page}::Sales[Region]"));
    steps(&mut h, 8);
    h
}

fn page_filter(h: &Harness<'_, App>) -> Value {
    h.state().loaded.as_ref().unwrap().snap["report"]["pages"][0]["filters"][0].clone()
}

#[test]
fn a_basic_filters_apply_button_holds_its_clicks() {
    let mut h = filters_app(|| filtered("apply_buttons"));
    let before = h.state().sent.len();
    h.get_by_label("South").click();
    steps(&mut h, 4);
    assert!(
        h.state().sent[before..].iter().all(|c| c.id != "page.filter"),
        "the click waits"
    );
    assert_eq!(page_filter(&h)["type"], "none");
    h.get_by_label("Not applied");
    h.get_by_label("Apply filter").click();
    steps(&mut h, 4);
    assert_eq!(last(&h, "page.filter")["not_in"], json!(["South"]));
    assert_eq!(page_filter(&h)["condition"], json!({"not_in": ["South"]}));
    assert!(h.query_by_label("Not applied").is_none());
}

#[test]
fn the_filter_panes_single_apply_button_applies_every_waiting_change() {
    let mut h = filters_app(|| filtered("single_button"));
    h.get_by_label("South").click();
    steps(&mut h, 4);
    assert_eq!(page_filter(&h)["type"], "none", "the click waits");
    assert!(h.query_by_label("Apply filter").is_none(), "no button per card");
    h.get_by_label("1 change not applied");
    h.get_by_label("Apply all").click();
    steps(&mut h, 4);
    assert_eq!(page_filter(&h)["condition"], json!({"not_in": ["South"]}));
    assert!(h.query_by_label("Apply all").is_none());
}
