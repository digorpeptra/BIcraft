//! A matrix's Row subtotals and Column subtotals cards in the Format visual tab (egui_kittest,
//! over an engine session run in the test): the card's switch, Per row level, then each level's
//! Show subtotal switch and Subtotal label, each one `visual.format` command on the `subTotals`
//! entry whose `selector.metadata` names the level's field (docs/formats/pbir.md, "Matrix
//! expansion and per-level subtotals").

use std::path::Path;

use bicraft_engine::Session;
use egui::Key;
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

/// Sales(Region, Category, Amount) with [Total]; on the first page a matrix `m` on Region >
/// Category.
pub(super) fn matrix_session() -> Session {
    let mut s = Session::new();
    let sales = r#"#table({"Region", "Category", "Amount"}, {{"North", "Bikes", 350}, {"North", "Toys", 10}, {"South", "Bikes", 300}, {"South", "Toys", 100}})"#;
    let run = |s: &mut Session, id: &str, p: Value| s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    run(&mut s, "model.new", json!({"name": "Shop"}));
    run(
        &mut s,
        "table.add",
        json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
    );
    for (c, ty) in [("Region", "string"), ("Category", "string"), ("Amount", "double")] {
        run(
            &mut s,
            "column.add",
            json!({"table": "Sales", "name": c, "data_type": ty}),
        );
    }
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])", "format_string": "#,0"}),
    );
    run(&mut s, "report.new", json!({}));
    let page = s
        .report
        .as_ref()
        .map(|r| r.pages[0].name.clone())
        .unwrap_or_default();
    let r = run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "pivotTable", "x": 40, "y": 40, "width": 520, "height": 360,
               "fields": [{"role": "Rows", "field": "Sales[Region]"}, {"role": "Rows", "field": "Sales[Category]"},
                          {"role": "Values", "field": "[Total]"}]}),
    );
    let made = r["visual"].as_str().unwrap_or_default().to_string();
    if let Some(v) = s
        .report
        .as_mut()
        .and_then(|r| r.pages.first_mut())
        .and_then(|p| p.visuals.iter_mut().find(|v| v.name == made))
    {
        v.name = "m".into();
        // the +/- icons (rowHeaders.showExpandCollapseButtons)
        v.visual_extra.insert(
            "objects".into(),
            json!({"rowHeaders": [{"properties": {"showExpandCollapseButtons": {"expr": {"Literal": {"Value": "true"}}}}}]}),
        );
    }
    s.dirty = false;
    s.report_dirty = false;
    s
}

pub(super) fn open(lang: &str, size: (f32, f32)) -> Harness<'static, App> {
    let mut app = App::new(lang, engine_shell(Caps::DESKTOP, matrix_session)).with_sync_render();
    app.open_path(Path::new("matrix.pbip"));
    let mut h = harness(app, size);
    for _ in 0..200 {
        h.step();
        if h.state().shown.is_some() && h.state().placed.is_some() {
            break;
        }
    }
    steps(&mut h, 2);
    h
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

fn sel(h: &Harness<'_, App>) -> VisualSel {
    let snap = h
        .state()
        .loaded
        .as_ref()
        .map(|l| l.snap.clone())
        .unwrap_or_default();
    VisualSel {
        page: snap["report"]["pages"][0]["name"]
            .as_str()
            .unwrap_or_default()
            .into(),
        visual: "m".into(),
    }
}

fn data(h: &Harness<'_, App>) -> Value {
    let snap = h
        .state()
        .loaded
        .as_ref()
        .map(|l| l.snap.clone())
        .unwrap_or_default();
    snap["report"]["pages"][0]["visuals"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|v| v["name"] == "m")
        .map(|v| v["data"].clone())
        .unwrap_or(Value::Null)
}

/// The last edit sent (reads left out), without page and visual.
fn last_edit(h: &Harness<'_, App>) -> (&'static str, Value) {
    let c = h
        .state()
        .sent
        .iter()
        .rev()
        .find(|c| c.id != "visual.props" && c.id != "column.values")
        .unwrap_or_else(|| panic!("no edit sent"));
    let mut p = c.params.clone();
    if let Some(o) = p.as_object_mut() {
        o.shift_remove("page");
        o.shift_remove("visual");
    }
    (c.id, p)
}

/// The matrix selected, the Format visual tab on its Visual cards, `card` open.
pub(super) fn format_card(h: &mut Harness<'_, App>, visual_cards: &str, card: &str) {
    let s = sel(h);
    h.state_mut().cmd.visual = Some(s);
    steps(h, 3);
    h.state_mut().panes.format.tab = Tab::Format;
    steps(h, 3);
    h.get_by_label(visual_cards).click();
    settle(h);
    h.get_by_role_and_label(Role::Button, card).click();
    settle(h);
}

#[test]
fn the_row_subtotals_card_sets_each_level() {
    let mut h = open("en-US", (1500.0, 1000.0));
    format_card(&mut h, "Visual cards", "Row subtotals");
    // both subtotal cards, each its own
    assert!(
        h.query_by_role_and_label(Role::Button, "Column subtotals")
            .is_some()
    );
    assert!(h.query_by_label("Row subtotals Position").is_some());
    // per level off: no level rows
    assert!(h.query_by_label("Row subtotals Category Show subtotal").is_none());
    h.get_by_label("Row subtotals Per row level").click();
    settle(&mut h);
    assert_eq!(
        last_edit(&h),
        (
            "visual.format",
            json!({"objects": [{"object": "subTotals", "property": "perRowLevel", "value": true}]})
        )
    );
    // one Show subtotal per level shown; Category's subtotals (the Region members') off
    assert!(h.query_by_label("Row subtotals Region Show subtotal").is_some());
    h.get_by_label("Row subtotals Category Show subtotal").click();
    settle(&mut h);
    assert_eq!(
        last_edit(&h),
        (
            "visual.format",
            json!({"objects": [{"object": "subTotals", "property": "levelSubtotalEnabled", "value": false,
                                "selector": {"metadata": "Sales.Category"}}]})
        )
    );
    assert!(h.state().error.is_none(), "{:?}", h.state().error);
    assert_eq!(data(&h)["row_level_totals"], json!([{}, {"show": false}]));
    let sw = h.get_by_label("Row subtotals Category Show subtotal");
    assert_eq!(
        sw.accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::False)
    );
    // the grand total's label (the top field's entry), typed and committed with Enter
    let field = h.get_by_label("Row subtotals Region Subtotal label");
    assert_eq!(field.accesskit_node().role(), Role::TextInput);
    field.focus();
    steps(&mut h, 1);
    h.event(egui::Event::Text("Tout".into()));
    steps(&mut h, 1);
    h.key_press(Key::Enter);
    settle(&mut h);
    assert_eq!(
        last_edit(&h),
        (
            "visual.format",
            json!({"objects": [{"object": "subTotals", "property": "levelSubtotalLabel", "value": "Tout",
                                "selector": {"metadata": "Sales.Region"}}]})
        )
    );
    assert_eq!(
        data(&h)["row_level_totals"],
        json!([{"label": "Tout"}, {"show": false}])
    );
    // the card's own switch turns every row subtotal off
    h.get_by_role_and_label(Role::CheckBox, "Row subtotals").click();
    settle(&mut h);
    assert_eq!(
        last_edit(&h),
        (
            "visual.format",
            json!({"objects": [{"object": "subTotals", "property": "rowSubtotals", "value": false}]})
        )
    );
}

#[test]
fn the_column_subtotals_card_in_french() {
    let mut h = open("fr-FR", (1500.0, 1000.0));
    format_card(&mut h, "Cartes du visuel", "Sous-totaux de colonnes");
    assert!(
        h.query_by_label("Sous-totaux de colonnes Par niveau de colonne")
            .is_some()
    );
    assert!(
        h.query_by_label("Sous-totaux de colonnes Libellé du sous-total")
            .is_some()
    );
    // the matrix has no column fields: per level on lists no level
    h.get_by_label("Sous-totaux de colonnes Par niveau de colonne")
        .click();
    settle(&mut h);
    assert!(h.state().error.is_none(), "{:?}", h.state().error);
    assert!(
        h.query_by_label("Sous-totaux de colonnes Region Afficher le sous-total")
            .is_none()
    );
}
