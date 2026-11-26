//! File › Options and settings (`options_page.rs`) and the Filters pane's eye (`filters_pane.rs`)
//! through the accessibility tree (egui_kittest, no GPU), over an engine session: each checkbox
//! runs `report.settings` with the one value it changes and shows the file's value back; the eye
//! hides the Filters pane from readers, whose window then has neither the pane nor its rail icon.

use std::path::Path;

use bicraft_engine::Session;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::backstage_tests::{engine_shell, steps};
use super::harness;
use crate::App;
use crate::backstage::Page;
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
        ("report.new", json!({})),
    ] {
        s.execute(id, &p).unwrap();
    }
    s.dirty = false;
    s.report_dirty = false;
    s
}

fn app(lang: &str) -> Harness<'static, App> {
    let mut app = App::new(lang, engine_shell(Caps::DESKTOP, session)).with_sync_render();
    app.open_path(Path::new("r.pbip"));
    let mut h = harness(app, (1400.0, 900.0));
    steps(&mut h, 6);
    h
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

fn open_options(h: &mut Harness<'_, App>, file: &str, options: &str) {
    h.get_by_label(file).click();
    steps(h, 3);
    h.get_by_label(options).click();
    steps(h, 3);
    assert_eq!(h.state().backstage.as_ref().map(|b| b.page), Some(Page::Options));
}

#[test]
fn each_report_setting_runs_its_command_and_reads_back() {
    let mut h = app("en-US");
    open_options(&mut h, "File", "Options and settings");
    h.get_by_label("Report settings");
    for (label, param, value, read) in [
        (
            "Don't allow end users to save filters on this file in the Power BI service",
            "persistent_filters",
            json!(false),
            json!(false),
        ),
        (
            "Personalize visuals",
            "personalize_visuals",
            json!(true),
            json!(true),
        ),
        (
            "Cross-report drillthrough",
            "cross_report_drillthrough",
            json!(true),
            json!(true),
        ),
        (
            "Allow users to change filter types",
            "allow_change_filter_types",
            json!(false),
            json!(false),
        ),
        (
            "Show the Filters pane to report readers",
            "filter_pane_visible",
            json!(false),
            json!(false),
        ),
        (
            "Change default visual interaction from cross highlighting to cross filtering",
            "default_interaction",
            json!("filter"),
            json!("filter"),
        ),
    ] {
        h.get_by_label(label).click();
        steps(&mut h, 4);
        assert_eq!(last(&h, "report.settings"), json!({ param: value }), "{label}");
        assert_eq!(setting(&h, param), read, "{label}");
    }
    // a second click takes the value back
    h.get_by_label("Personalize visuals").click();
    steps(&mut h, 4);
    assert_eq!(setting(&h, "personalize_visuals"), json!(false));
}

#[test]
fn the_page_speaks_french() {
    let mut h = app("fr-FR");
    open_options(&mut h, "Fichier", "Options et paramètres");
    for label in [
        "Paramètres du rapport",
        "Filtres persistants",
        "Personnaliser les visuels",
        "Extraction entre rapports",
        "Autoriser les utilisateurs à changer les types de filtre",
    ] {
        h.get_by_label(label);
    }
    h.get_by_label("Personnaliser les visuels").click();
    steps(&mut h, 4);
    assert_eq!(setting(&h, "personalize_visuals"), json!(true));
}

#[test]
fn the_filters_pane_eye_hides_the_pane_from_readers() {
    let mut h = app("en-US");
    h.state_mut().prefs.panes.filters = true;
    steps(&mut h, 3);
    h.get_by_label("Hide the Filters pane from readers").click();
    steps(&mut h, 4);
    assert_eq!(last(&h, "report.settings"), json!({"filter_pane_visible": false}));
    assert_eq!(setting(&h, "filter_pane_visible"), json!(false));
    // the editor still has the pane, its eye now shows it again
    h.get_by_label("Collapse the Filters pane");
    h.get_by_label("Show the Filters pane to readers");
    // a reader has neither the pane nor its rail icon
    h.state_mut().access = Some("reader".into());
    steps(&mut h, 3);
    assert!(h.query_by_label("Collapse the Filters pane").is_none());
    assert!(h.query_by_label("Open the Filters pane").is_none());
    // shown again: the reader's pane is back
    h.state_mut().access = None;
    steps(&mut h, 3);
    h.get_by_label("Show the Filters pane to readers").click();
    steps(&mut h, 4);
    h.state_mut().access = Some("reader".into());
    steps(&mut h, 3);
    h.get_by_label("Collapse the Filters pane");
    assert!(h.query_by_label("Show the Filters pane to readers").is_none());
}
