//! The Visualizations pane's Analytics tab on an engine session (`analytics_pane.rs`, G56): the
//! tab and the cards a type allows, Add, an option, Remove, error bars with their bounds, undo;
//! readers see no tab. Every gesture sends one `visual.analytics` with the whole list.

use std::path::Path;

use bicraft_engine::Session;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use super::backstage_tests::{engine_shell, steps};
use super::harness;
use crate::App;
use crate::format_pane::Tab;
use crate::ribbon::VisualSel;
use crate::shell::Caps;

/// Sales(Category, Amount, Hi) with [Total] and [Upper]; one page with a column chart and a
/// scatter chart.
fn session() -> Session {
    let mut s = Session::new();
    let sales =
        r#"#table({"Category", "Amount", "Hi"}, {{"Wheels", 100, 10}, {"Toys", 50, 5}, {"Games", 20, 2}})"#;
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
            "column.add",
            json!({"table": "Sales", "name": "Hi", "data_type": "double"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Upper", "expression": "SUM(Sales[Amount]) + SUM(Sales[Hi])"}),
        ),
        ("report.new", json!({})),
    ] {
        s.execute(id, &p).unwrap();
    }
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    for (ty, x, fields) in [
        (
            "clusteredColumnChart",
            20,
            json!([{"role": "Category", "field": "Sales[Category]"}, {"role": "Y", "field": "[Total]"}]),
        ),
        (
            "scatterChart",
            440,
            json!([{"role": "Category", "field": "Sales[Category]"}, {"role": "X", "field": "[Total]"},
                   {"role": "Y", "field": "[Upper]"}]),
        ),
        (
            "lineChart",
            860,
            json!([{"role": "Category", "field": "Sales[Category]"}, {"role": "Y", "field": "[Total]"}]),
        ),
    ] {
        s.execute(
            "visual.add",
            &json!({"page": page, "visual_type": ty, "x": x, "y": 20, "width": 400, "height": 300,
                    "fields": fields}),
        )
        .unwrap();
    }
    s.dirty = false;
    s.report_dirty = false;
    s
}

fn open(reader: bool) -> Harness<'static, App> {
    let mut app = App::new("en-US", engine_shell(Caps::DESKTOP, session)).with_sync_render();
    app.open_path(Path::new("shop.pbip"));
    let mut h = harness(app, (1500.0, 1000.0));
    steps(&mut h, 6);
    if reader {
        h.state_mut().access = Some("reader".into());
    }
    for _ in 0..200 {
        h.step();
        if h.state().shown.is_some() && h.state().placed.is_some() {
            break;
        }
    }
    steps(&mut h, 2);
    h
}

fn visual_of(h: &Harness<'_, App>, ty: &str) -> VisualSel {
    let snap = h.state().loaded.as_ref().unwrap().snap.clone();
    let page = &snap["report"]["pages"][0];
    let v = page["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["type"] == ty)
        .unwrap();
    VisualSel {
        page: page["name"].as_str().unwrap().into(),
        visual: v["name"].as_str().unwrap().into(),
    }
}

fn snap_visual(h: &Harness<'_, App>, sel: &VisualSel) -> Value {
    let snap = h.state().loaded.as_ref().unwrap().snap.clone();
    snap["report"]["pages"][0]["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == sel.visual.as_str())
        .cloned()
        .unwrap()
}

fn settle(h: &mut Harness<'_, App>) {
    for _ in 0..100 {
        h.step();
        if h.state().calls_pending() == 0 {
            break;
        }
    }
    steps(h, 3);
}

/// The `visual.analytics` calls sent, their `lines`.
fn sent_lines(h: &Harness<'_, App>) -> Vec<Value> {
    h.state()
        .sent
        .iter()
        .filter(|c| c.id == "visual.analytics")
        .map(|c| c.params["lines"].clone())
        .collect()
}

/// The visual's analytics as the engine has them now.
fn analytics(h: &Harness<'_, App>, sel: &VisualSel) -> Value {
    h.state()
        .panes
        .format
        .own
        .as_ref()
        .filter(|o| o.visual == sel.visual)
        .map(|o| o.props["analytics"].clone())
        .unwrap_or(Value::Null)
}

/// Selects `sel` and shows the Analytics tab by a click.
fn analytics_tab(h: &mut Harness<'_, App>, sel: &VisualSel) {
    h.state_mut().cmd.visual = Some(sel.clone());
    steps(h, 3);
    h.get_by_role_and_label(Role::Button, "Analytics").click();
    settle(h);
    assert_eq!(h.state().panes.format.tab, Tab::Analytics);
}

#[test]
fn the_tab_lists_the_cards_the_type_allows() {
    let mut h = open(false);
    let col = visual_of(&h, "clusteredColumnChart");
    analytics_tab(&mut h, &col);
    for title in [
        "Y-axis constant line",
        "Average line",
        "Percentile line",
        "Trend line",
        "Error bars",
    ] {
        assert!(h.query_by_label(title).is_some(), "{title} card");
    }
    assert!(
        h.query_by_label("Ratio line").is_none(),
        "a column chart has no ratio line"
    );
    assert!(
        h.query_by_label("Forecast").is_none(),
        "a column chart has no forecast"
    );
    // a scatter chart: its ratio line and symmetry shading, no error bars
    let sc = visual_of(&h, "scatterChart");
    h.state_mut().cmd.visual = Some(sc);
    settle(&mut h);
    assert!(h.query_by_label("Ratio line").is_some());
    assert!(h.query_by_label("Symmetry shading").is_some());
    assert!(h.query_by_label("Error bars").is_none());
}

#[test]
fn add_set_an_option_and_remove_send_one_command_each() {
    let mut h = open(false);
    let col = visual_of(&h, "clusteredColumnChart");
    analytics_tab(&mut h, &col);
    h.get_by_label("Average line").click();
    steps(&mut h, 3);
    // Add
    h.get_by_label("Add Average line").click();
    settle(&mut h);
    assert_eq!(sent_lines(&h), vec![json!([{"kind": "average"}])]);
    assert_eq!(
        analytics(&h, &col),
        json!([{"kind": "average", "measure": "[Total]"}])
    );
    let rl = snap_visual(&h, &col)["format"]["reference_lines"].clone();
    assert_eq!(rl[0]["kind"], "average", "{rl}");
    // an option: the data label's switch
    h.get_by_role_and_label(Role::CheckBox, "Average line 1 Data label")
        .click();
    settle(&mut h);
    assert_eq!(
        sent_lines(&h).last(),
        Some(&json!([{"kind": "average", "measure": "[Total]", "data_label": true}]))
    );
    assert_eq!(analytics(&h, &col)[0]["data_label"], true);
    // Remove
    h.get_by_label("Remove Average line 1").click();
    settle(&mut h);
    assert_eq!(sent_lines(&h).last(), Some(&json!([])));
    assert_eq!(analytics(&h, &col), Value::Null);
    assert_eq!(sent_lines(&h).len(), 3, "one command per gesture");
    // undo brings the line back
    h.state_mut().run(
        crate::actions::Call {
            id: "edit.undo",
            params: json!({}),
        },
        crate::app::Then::Nothing,
    );
    settle(&mut h);
    assert_eq!(analytics(&h, &col)[0]["kind"], "average");
}

#[test]
fn error_bars_take_their_bounds_then_add() {
    let mut h = open(false);
    let col = visual_of(&h, "clusteredColumnChart");
    analytics_tab(&mut h, &col);
    h.get_by_label("Error bars").click();
    steps(&mut h, 3);
    let add = h.get_by_label("Add Error bars");
    assert!(add.accesskit_node().is_disabled(), "no bound picked yet");
    // the upper bound: [Upper] in the list
    h.get_by_label("New error bars' upper bound").click();
    steps(&mut h, 2);
    h.get_by_label("[Upper]").click();
    steps(&mut h, 3);
    h.get_by_label("Add Error bars").click();
    settle(&mut h);
    assert_eq!(
        sent_lines(&h),
        vec![json!([{"kind": "error_bars", "upper": "[Upper]"}])]
    );
    assert_eq!(
        analytics(&h, &col),
        json!([{"kind": "error_bars", "measure": "[Total]", "upper": "[Upper]"}])
    );
    // the data carries the bounds for the renderer
    let d = snap_visual(&h, &col)["data"].clone();
    assert_eq!(d["error_bars"][0]["column"], 1, "{d}");
    // an option of the error bars: the markers off
    h.get_by_role_and_label(Role::CheckBox, "Error bars 1 Markers")
        .click();
    settle(&mut h);
    assert_eq!(analytics(&h, &col)[0]["marker"], false);
}

#[test]
fn readers_have_no_analytics_tab() {
    let mut h = open(true);
    let col = visual_of(&h, "clusteredColumnChart");
    h.state_mut().cmd.visual = Some(col);
    steps(&mut h, 3);
    assert!(h.query_by_role_and_label(Role::Button, "Analytics").is_none());
    h.state_mut().panes.format.tab = Tab::Analytics;
    steps(&mut h, 3);
    assert_eq!(h.state().panes.format.tab, Tab::Build);
}

#[test]
fn find_anomalies_is_a_line_chart_card_with_its_options() {
    let mut h = open(false);
    // a column chart has no anomaly detection
    let col = visual_of(&h, "clusteredColumnChart");
    analytics_tab(&mut h, &col);
    assert!(h.query_by_label("Find anomalies").is_none());
    let line = visual_of(&h, "lineChart");
    h.state_mut().cmd.visual = Some(line.clone());
    settle(&mut h);
    h.get_by_label("Find anomalies").click();
    steps(&mut h, 3);
    assert!(
        h.query_by_label_contains("BIcraft estimate").is_some(),
        "the card says the result is BIcraft's estimate"
    );
    h.get_by_label("Add Find anomalies").click();
    settle(&mut h);
    assert_eq!(sent_lines(&h), vec![json!([{"kind": "anomaly_detection"}])]);
    assert_eq!(
        analytics(&h, &line),
        json!([{"kind": "anomaly_detection", "measure": "[Total]"}])
    );
    let rl = snap_visual(&h, &line)["format"]["reference_lines"].clone();
    assert_eq!(rl[0]["kind"], "anomaly_detection", "{rl}");
    // one at most: no Add once there
    assert!(h.query_by_label("Add Find anomalies").is_none());
    // an option: the markers off (the card is low in the pane: scrolled into view first)
    h.get_by_role_and_label(Role::CheckBox, "Find anomalies Anomaly markers")
        .scroll_to_me();
    steps(&mut h, 3);
    h.get_by_role_and_label(Role::CheckBox, "Find anomalies Anomaly markers")
        .click();
    settle(&mut h);
    assert_eq!(
        sent_lines(&h).last(),
        Some(&json!([{"kind": "anomaly_detection", "measure": "[Total]", "marker": false}]))
    );
    assert_eq!(analytics(&h, &line)[0]["marker"], false);
    // Remove
    h.get_by_label("Remove Find anomalies").scroll_to_me();
    steps(&mut h, 3);
    h.get_by_label("Remove Find anomalies").click();
    settle(&mut h);
    assert_eq!(analytics(&h, &line), Value::Null);
    assert_eq!(sent_lines(&h).len(), 3, "one command per gesture");
}
