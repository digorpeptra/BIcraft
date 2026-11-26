//! The Performance analyzer pane (ROADMAP review lot 14) through the accessibility tree
//! (egui_kittest, no GPU), over an engine session: the Analyze ribbon opens it, Start recording,
//! Refresh visuals (whose render times go back with `perf.render`), unrolling a row, Copy query,
//! Stop, Clear, Export, in English and French. Each gesture runs its `perf.*` command.

use std::path::Path;

use bicraft_engine::Session;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::backstage_tests::{engine_shell, steps};
use super::harness;
use crate::App;
use crate::registry::Tab as RTab;
use crate::shell::Caps;

/// A report page `Overview` with a column chart of Total by Region.
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
    let first = s.report.as_ref().unwrap().pages[0].name.clone();
    s.execute("page.rename", &json!({"page": first, "display_name": "Overview"}))
        .unwrap();
    s.execute(
        "visual.add",
        &json!({"page": "Overview", "visual_type": "clusteredColumnChart", "x": 20, "y": 20,
                "width": 400, "height": 250,
                "fields": [{"role": "Category", "field": "Sales[Region]"}, {"role": "Y", "field": "[Total]"}]}),
    )
    .unwrap();
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

fn events(h: &Harness<'_, App>) -> Vec<Value> {
    h.state()
        .cmd
        .perf
        .results
        .as_ref()
        .and_then(|r| r.get("events"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn open_pane(h: &mut Harness<'_, App>, button: &str) {
    h.state_mut().ribbon_tab = RTab::Analyze;
    steps(h, 3);
    h.get_by_role_and_label(egui::accesskit::Role::Button, button)
        .click();
    steps(h, 4);
}

#[test]
fn the_ribbon_opens_the_pane_and_refresh_visuals_records_each_visual() {
    let mut h = app("en-US");
    open_pane(&mut h, "Performance analyzer");
    assert!(h.state().cmd.perf.open);
    assert_eq!(sent(&h).last(), Some(&"perf.results"));
    h.get_by_label_contains("Start recording, then refresh");
    // Start recording
    h.get_by_label("Start recording").click();
    steps(&mut h, 4);
    assert!(sent(&h).contains(&"perf.start"));
    h.get_by_label("Stop");
    h.get_by_label_contains("Recording");
    // Refresh visuals: the page shown, then its render times go back to the engine
    h.get_by_label("Refresh visuals").click();
    steps(&mut h, 8);
    let page = h.state().page_names()[0].0.clone();
    assert_eq!(last(&h, "perf.refresh")["page"], json!(page));
    assert!(sent(&h).contains(&"perf.render"), "{:?}", sent(&h));
    let render = last(&h, "perf.render");
    let times = render["times"].as_object().unwrap();
    assert_eq!(times.len(), 1, "{render}");
    assert!(times.values().all(|t| t.as_f64().is_some_and(|x| x >= 0.0)));
    let ev = events(&h);
    let refreshed: Vec<&Value> = ev.iter().filter(|e| e["cause"] == "refresh").collect();
    assert_eq!(refreshed.len(), 1, "{ev:?}");
    assert!(refreshed[0]["render_ms"].as_f64().is_some(), "{ev:?}");
    assert!(refreshed[0]["evaluation_ms"].as_f64().is_some_and(|x| x >= 0.0));
    // one row per event, with its total
    assert!(h.get_all_by_label_contains("clusteredColumnChart").count() >= 1);
    h.get_by_label("Duration (ms)");
}

#[test]
fn a_row_unrolls_to_its_parts_and_copies_its_query() {
    let mut h = app("en-US");
    open_pane(&mut h, "Performance analyzer");
    h.get_by_label("Start recording").click();
    steps(&mut h, 4);
    h.get_by_label("Refresh visuals").click();
    steps(&mut h, 8);
    let seq = events(&h).iter().rev().find(|e| e["cause"] == "refresh").unwrap()["seq"]
        .as_u64()
        .unwrap();
    assert!(h.query_by_label("Copy query").is_none(), "rolled up");
    // the latest row first
    h.get_all_by_label_contains("clusteredColumnChart (")
        .next()
        .unwrap()
        .click();
    steps(&mut h, 3);
    assert!(h.state().cmd.perf.expanded.contains(&seq));
    h.get_all_by_label("Evaluation (BIcraft)").next().unwrap();
    h.get_all_by_label("Render").next().unwrap();
    h.get_all_by_label("Copy query").next().unwrap().click();
    steps(&mut h, 2);
    assert_eq!(h.state().status, "Query copied");
    let q = events(&h).iter().rev().find(|e| e["seq"] == seq).unwrap()["query"].clone();
    let q = q.as_str().unwrap();
    assert!(
        q.contains("SUMMARIZECOLUMNS") && q.contains("'Sales'[Region]"),
        "{q}"
    );
}

#[test]
fn stop_clear_and_export_run_their_commands() {
    let out = std::env::temp_dir().join(format!("bicraft-ui-perf-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&out);
    let mut shell = engine_shell(Caps::DESKTOP, session);
    let o2 = out.clone();
    shell.pick_save = Some(Box::new(move |_, _, _, _| Some(o2.clone())));
    let mut app = App::new("en-US", shell).with_sync_render();
    app.open_path(Path::new("r.pbip"));
    let mut h = harness(app, (1400.0, 900.0));
    steps(&mut h, 6);
    open_pane(&mut h, "Performance analyzer");
    h.get_by_label("Start recording").click();
    steps(&mut h, 4);
    h.get_by_label("Refresh visuals").click();
    steps(&mut h, 8);
    h.get_by_label("Export").click();
    steps(&mut h, 4);
    assert_eq!(last(&h, "perf.export")["out"], json!(out.display().to_string()));
    let doc: Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    assert_eq!(doc["measured_by"], "BIcraft");
    assert!(!doc["events"].as_array().unwrap().is_empty());
    h.get_by_label_contains("Exported to");
    h.get_by_label("Stop").click();
    steps(&mut h, 4);
    assert!(sent(&h).contains(&"perf.stop"));
    h.get_by_label("Start recording");
    h.get_by_label("Not recording");
    h.get_by_label("Clear").click();
    steps(&mut h, 4);
    assert!(sent(&h).contains(&"perf.clear"));
    assert!(events(&h).is_empty());
    // the ribbon button closes it again
    open_pane(&mut h, "Performance analyzer");
    assert!(!h.state().cmd.perf.open);
    let _ = std::fs::remove_file(&out);
}

#[test]
fn the_pane_speaks_french() {
    let mut h = app("fr-FR");
    open_pane(&mut h, "Analyseur de performances");
    h.get_by_label("Démarrer l'enregistrement").click();
    steps(&mut h, 4);
    h.get_by_label("Actualiser les visuels").click();
    steps(&mut h, 8);
    for label in ["Arrêter", "Effacer", "Exporter", "Nom", "Durée (ms)"] {
        h.get_by_label(label);
    }
    h.get_by_label_contains("Enregistrement en cours");
}
