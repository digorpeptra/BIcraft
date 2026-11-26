//! Best practices' report findings (`analyze_ui.rs`): a measure removed from the model leaves a
//! card bound to it; the answer window lists it with Go to visual (the card is shown and
//! selected on its page) and Fix (visual.unbind runs, the analysis runs again and is clean).
//! Through the accessibility tree (egui_kittest, no GPU), on an engine session run in the test.

use std::path::Path;

use bicraft_engine::Session;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::backstage_tests::{engine_shell, steps};
use super::harness;
use crate::App;
use crate::app::Tab;
use crate::shell::Caps;

/// Sales with [Total] and [Margin]; page 1 has a card on [Total], page 2 ("Details") a card on
/// [Margin]; [Margin] is then removed from the model, the report left as it was.
pub(super) fn broken() -> Session {
    let mut s = Session::new();
    let sales = r#"#table({"Region", "Amount"}, {{"North", 100}, {"South", 5}})"#;
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
        (
            "measure.add",
            json!({"table": "Sales", "name": "Margin", "expression": "[Total] * 0.2"}),
        ),
        ("report.new", json!({})),
        ("page.add", json!({"display_name": "Details"})),
    ] {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    let pages: Vec<String> = s
        .report
        .as_ref()
        .unwrap()
        .pages
        .iter()
        .map(|p| p.name.clone())
        .collect();
    for (page, field) in [(&pages[0], "[Total]"), (&pages[1], "[Margin]")] {
        s.execute(
            "visual.add",
            &json!({"page": page, "visual_type": "card", "x": 10, "y": 10,
                    "width": 200, "height": 100, "fields": [{"role": "Values", "field": field}]}),
        )
        .unwrap_or_else(|e| panic!("visual.add: {e}"));
    }
    s.execute("measure.remove", &json!({"name": "Margin", "force": true}))
        .unwrap_or_else(|e| panic!("measure.remove: {e}"));
    s.execute("page.set_active", &json!({"page": pages[0]})).ok();
    s.dirty = false;
    s.report_dirty = false;
    s
}

pub(super) fn app() -> Harness<'static, App> {
    let mut app = App::new("en-US", engine_shell(Caps::DESKTOP, broken));
    app.open_path(Path::new("shop.pbip"));
    let mut h = harness(app, (1280.0, 800.0));
    steps(&mut h, 6);
    assert!(h.state().loaded.is_some(), "the snapshot never came");
    h
}

/// Runs Best practices as its ribbon button does.
pub(super) fn analyze(h: &mut Harness<'_, App>) {
    let c = crate::registry::find("model.analyze").unwrap();
    crate::ribbon::send(h.state_mut(), c, json!({}));
    steps(h, 6);
}

#[test]
fn a_broken_field_is_listed_with_go_to_visual() {
    let mut h = app();
    assert_eq!(h.state().active_page(), 0);
    analyze(&mut h);
    h.get_by_label("Report references");
    h.get_by_label_contains("'Sales'[Margin]");
    h.get_by_label("Go to visual").click();
    steps(&mut h, 8);
    let app = h.state();
    let card = app
        .loaded
        .as_ref()
        .and_then(|l| l.snap.pointer("/report/pages/1/visuals/0/name"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    assert!(app.cmd.result.is_none(), "Go to closes the window");
    assert_eq!(app.tab, Tab::Report);
    assert_eq!(app.active_page(), 1, "the card's page is shown");
    assert_eq!(
        app.cmd.visual.as_ref().map(|v| v.visual.clone()),
        card,
        "the card is selected"
    );
}

#[test]
fn fix_unbinds_the_stale_field_and_analyzes_again() {
    let mut h = app();
    analyze(&mut h);
    h.get_by_label("Fix (visual.unbind)").click();
    steps(&mut h, 10);
    h.get_by_label("✓ No broken reference in the report");
    let fields = h
        .state()
        .loaded
        .as_ref()
        .and_then(|l| l.snap.pointer("/report/pages/1/visuals/0/fields").cloned());
    assert_eq!(fields, Some(json!([])), "the card lost its stale field only");
}
