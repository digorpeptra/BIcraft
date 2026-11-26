//! The Selection pane (`selection_pane.rs`, ROADMAP M11.23) on an engine session: the page's
//! visuals listed front to back with its groups nested and foldable, the eye that hides and shows
//! a visual, a click that selects it on the canvas, a group renamed in place and the tab order
//! mode; every edit is an engine command, so the next snapshot shows it.

use std::path::Path;

use bicraft_engine::Session;
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::backstage_tests::{engine_shell, steps};
use super::harness;
use crate::App;
use crate::i18n::Lang;
use crate::shell::Caps;

/// A card, a column chart and a line chart; the card and the line chart grouped as "Pair".
fn selection_session() -> Session {
    let mut s = Session::new();
    let sales = r#"#table({"Category", "Amount"}, {{"Wheels", 100}, {"Toys", 5}, {"Games", 20}})"#;
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
        s.execute(id, &p).unwrap();
    }
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    let mut names = Vec::new();
    for (ty, rect, fields) in [
        (
            "card",
            [20, 20, 220, 120],
            json!([{"role": "Values", "field": "[Total]"}]),
        ),
        (
            "clusteredColumnChart",
            [260, 20, 400, 300],
            json!([{"role": "Category", "field": "Sales[Category]"}, {"role": "Y", "field": "[Total]"}]),
        ),
        (
            "lineChart",
            [260, 340, 400, 200],
            json!([{"role": "Category", "field": "Sales[Category]"}, {"role": "Y", "field": "[Total]"}]),
        ),
    ] {
        let out = s
            .execute(
                "visual.add",
                &json!({"page": page, "visual_type": ty, "x": rect[0], "y": rect[1],
                        "width": rect[2], "height": rect[3], "fields": fields}),
            )
            .unwrap();
        names.push(out["visual"].as_str().unwrap().to_string());
    }
    s.execute(
        "visual.group",
        &json!({"page": page, "visuals": [names[0], names[2]], "display_name": "Pair"}),
    )
    .unwrap();
    s.dirty = false;
    s.report_dirty = false;
    s
}

/// The window on a fresh session, the Report tab shown and the Selection pane open.
fn open() -> Harness<'static, App> {
    let mut app = App::new("en-US", engine_shell(Caps::DESKTOP, selection_session)).with_sync_render();
    app.prefs.panes.selection = true;
    app.open_path(Path::new("selection.pbip"));
    let mut h = harness(app, (1400.0, 860.0));
    steps(&mut h, 6);
    for _ in 0..200 {
        h.step();
        if h.state().shown.is_some() && h.state().placed.is_some() {
            break;
        }
    }
    steps(&mut h, 2);
    h
}

/// The snapshot's visual whose display label is `label` on the first page.
fn visual_labelled(h: &Harness<'_, App>, label: &str) -> Value {
    h.state().loaded.as_ref().unwrap().snap["report"]["pages"][0]["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| {
            v["title"].as_str() == Some(label)
                || v["type"].as_str() == Some(label)
                || v["type"]
                    .as_str()
                    .and_then(|t| crate::panes::type_label(t, Lang::En))
                    == Some(label)
        })
        .unwrap_or_else(|| panic!("no visual labelled {label}"))
        .clone()
}

fn press(h: &mut Harness<'_, App>, label: &str) {
    let pos = h.get_by_label(label).rect().center();
    click_with(h, pos, Modifiers::NONE);
}

/// A node of the Selection pane labelled `label`: the Visualizations pane's gallery cells carry
/// the same words as a visual's type ("Line chart", "Card"), and this test wants the pane's rows.
fn row_node<'a>(h: &'a Harness<'_, App>, label: &'a str) -> Option<egui_kittest::Node<'a>> {
    let left = h.get_by_label("Selection").rect().left() - 1.0;
    h.query_all_by_label(label).find(|n| n.rect().left() >= left)
}

/// The rect of the Selection pane's row labelled `label`.
fn row(h: &Harness<'_, App>, label: &str) -> egui::Rect {
    row_node(h, label)
        .unwrap_or_else(|| panic!("no `{label}` row in the Selection pane"))
        .rect()
}

/// A click on the Selection pane's row labelled `label`.
fn press_row(h: &mut Harness<'_, App>, label: &str) {
    let pos = row(h, label).center();
    click_with(h, pos, Modifiers::NONE);
}

/// A left click at `pos` with `mods` held (Ctrl+click adds to the selection).
fn click_with(h: &mut Harness<'_, App>, pos: egui::Pos2, mods: Modifiers) {
    h.hover_at(pos);
    h.step();
    for pressed in [true, false] {
        h.event_modifiers(
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: mods,
            },
            mods,
        );
        h.step();
    }
    steps(h, 2);
}

#[test]
fn the_selection_pane_hides_selects_renames_and_orders() {
    let mut h = open();
    h.get_by_label("Collapse the Selection pane");
    let group = visual_labelled(&h, "Pair");
    assert!(group["type"].is_null(), "the group row is a group: {group}");

    // The eye: `visual.set_hidden`, the snapshot shows the visual hidden (and its members when a
    // group is hidden).
    press(&mut h, "Hide Card");
    assert_eq!(visual_labelled(&h, "Card")["hidden"], json!(true));
    press(&mut h, "Show Card");
    assert_eq!(visual_labelled(&h, "Card")["hidden"], json!(false));
    press(&mut h, "Hide Pair");
    assert_eq!(visual_labelled(&h, "Pair")["hidden"], json!(true));
    assert_eq!(visual_labelled(&h, "Line chart")["hidden"], json!(false));
    press(&mut h, "Show Pair");

    // A group folds: its members leave the list, and come back.
    h.get_by_label("Collapse the group").click();
    steps(&mut h, 3);
    assert!(row_node(&h, "Line chart").is_none(), "the member is folded away");
    press(&mut h, "Expand the group");
    row(&h, "Line chart");

    // A click selects the visual on the canvas; Ctrl+click adds it.
    press_row(&mut h, "Clustered column chart");
    assert_eq!(
        crate::canvas::selection(h.state()),
        vec![
            visual_labelled(&h, "Clustered column chart")["name"]
                .as_str()
                .unwrap()
                .to_string()
        ]
    );
    let card_row = row(&h, "Card").center();
    click_with(&mut h, card_row, Modifiers::COMMAND);
    assert_eq!(crate::canvas::selection(h.state()).len(), 2);

    // Renaming a group: the editor takes the text and Enter calls `visual.rename_group`.
    press(&mut h, "Pair");
    assert_eq!(
        crate::canvas::selection(h.state()),
        vec![group["name"].as_str().unwrap().to_string()],
        "the group row selects the group"
    );
    press(&mut h, "Rename group");
    assert!(
        h.state().selection.renaming.is_some(),
        "the rename editor opened: status {}",
        h.state().status
    );
    let editor = "Rename Pair";
    h.get_by_label(editor).click();
    steps(&mut h, 2);
    for _ in 0..8 {
        h.key_press(Key::Backspace);
    }
    h.get_by_label(editor).type_text("Two");
    steps(&mut h, 2);
    h.key_press(Key::Enter);
    steps(&mut h, 4);
    let renamed = visual_labelled(&h, "Two");
    assert_eq!(renamed["name"], group["name"]);
    assert_eq!(renamed["title"], json!("Two"));
    assert!(h.query_by_label("Pair").is_none(), "the old name is gone");

    // The tab order mode lists every visual in keyboard order; a move rewrites tabOrder
    // 0, 1000, 2000… through `visual.tab_order`.
    press(&mut h, "Tab order");
    assert!(h.state().selection.tab_order);
    press_row(&mut h, "Card");
    press(&mut h, "Later in the tab order");
    let visuals = h.state().loaded.as_ref().unwrap().snap["report"]["pages"][0]["visuals"]
        .as_array()
        .unwrap()
        .clone();
    let mut tabs: Vec<i64> = visuals.iter().map(|v| v["tab_order"].as_i64().unwrap()).collect();
    tabs.sort_unstable();
    let expected: Vec<i64> = (0..visuals.len() as i64).map(|k| k * 1000).collect();
    assert_eq!(tabs, expected);

    // The pane's state and width live in the preferences, as every other pane's: the title row
    // collapses it and the strip opens it again.
    assert!(h.state().prefs.panes.selection);
    press(&mut h, "Collapse the Selection pane");
    assert!(!h.state().prefs.panes.selection);
    press(&mut h, "Open the Selection pane");
    assert!(h.state().prefs.panes.selection);
    h.get_by_label("Hide Two");
}
