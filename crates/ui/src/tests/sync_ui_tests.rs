//! The Sync slicers pane (`sync_pane.rs`, ROADMAP M11.25): the report's slicers listed page by
//! page, a group name and Desktop's Fields / Filters options, Synchronize and Unsync — every one
//! an engine command (`slicer.set_sync`), so the next snapshot shows the group.

use std::path::Path;

use bicraft_engine::Session;
use egui::Modifiers;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::backstage_tests::{engine_shell, steps};
use super::harness;
use crate::App;
use crate::shell::Caps;

/// Two pages, each with a titled slicer on `Sales[Category]` and a card.
fn sync_session() -> Session {
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
        s.execute(id, &p).unwrap();
    }
    let first = s.report.as_ref().unwrap().pages[0].name.clone();
    s.execute(
        "page.add",
        &json!({"display_name": "Detail", "width": 1280, "height": 720}),
    )
    .unwrap();
    let second = s.report.as_ref().unwrap().pages[1].name.clone();
    for (page, title) in [(&first, "By category"), (&second, "By category too")] {
        let out = s
            .execute(
                "visual.add",
                &json!({"page": page, "visual_type": "slicer", "x": 20, "y": 20, "width": 220,
                        "height": 120, "fields": [{"role": "Values", "field": "Sales[Category]"}]}),
            )
            .unwrap();
        let visual = out["visual"].as_str().unwrap().to_string();
        s.execute(
            "visual.set_title",
            &json!({"page": page, "visual": visual, "title": title}),
        )
        .unwrap();
        s.execute(
            "visual.add",
            &json!({"page": page, "visual_type": "card", "x": 260, "y": 20, "width": 200,
                    "height": 120, "fields": [{"role": "Values", "field": "[Total]"}]}),
        )
        .unwrap();
    }
    s.execute("page.set_active", &json!({"page": first})).unwrap();
    s.dirty = false;
    s.report_dirty = false;
    s
}

/// The window on a fresh session with the Sync slicers pane open.
fn open() -> Harness<'static, App> {
    let mut app = App::new("en-US", engine_shell(Caps::DESKTOP, sync_session)).with_sync_render();
    app.prefs.panes.sync = true;
    app.open_path(Path::new("sync.pbip"));
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

/// The slicers of the snapshot with their sync group, by title.
fn slicers(h: &Harness<'_, App>) -> Vec<(String, Value)> {
    let snap = &h.state().loaded.as_ref().unwrap().snap;
    snap.pointer("/report/pages")
        .and_then(Value::as_array)
        .map(|pages| {
            pages
                .iter()
                .flat_map(|p| {
                    p.get("visuals")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter(|v| v["type"] == json!("slicer"))
                        .map(|v| {
                            (
                                v["title"].as_str().unwrap_or_default().to_string(),
                                v["sync_group"].clone(),
                            )
                        })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn press(h: &mut Harness<'_, App>, label: &str) {
    let pos = h.get_by_label(label).rect().center();
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

/// The page's visual of type `ty` in the snapshot.
fn visual_of<'a>(h: &'a Harness<'_, App>, page: usize, ty: &str) -> &'a Value {
    let snap = &h.state().loaded.as_ref().unwrap().snap;
    snap["report"]["pages"][page]["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["type"] == json!(ty))
        .unwrap_or_else(|| panic!("no {ty} on page {page}"))
}

/// Puts the two slicers in group `Category` through the engine command.
fn sync_both(h: &mut Harness<'_, App>) {
    let snap = h.state().loaded.as_ref().unwrap().snap.clone();
    let segments: Vec<Value> = (0..2)
        .map(|i| {
            let p = &snap["report"]["pages"][i];
            let v = p["visuals"]
                .as_array()
                .unwrap()
                .iter()
                .find(|v| v["type"] == json!("slicer"))
                .unwrap();
            json!({"page": p["name"], "visual": v["name"]})
        })
        .collect();
    crate::panes::exec(
        h.state_mut(),
        "slicer.set_sync",
        json!({"segments": segments, "group": "Category"}),
    );
    steps(h, 4);
}

/// The screen point of the shown page's slicer item `value` (its hit region).
fn slicer_item(h: &Harness<'_, App>, value: &str) -> egui::Pos2 {
    let app = h.state();
    let (_, Ok(img)) = app.shown.as_ref().unwrap() else {
        panic!("the page is not drawn")
    };
    let hit = img
        .hits
        .iter()
        .find(|x| match &x.action {
            bicraft_render::hits::HitAction::Send { action, .. } => {
                action["action"] == json!("slicer_toggle") && action["value"] == json!(value)
            }
            _ => false,
        })
        .unwrap_or_else(|| panic!("no slicer item {value}"));
    let (x, y, w, hh) = hit.shape.bounds();
    let pl = app.placed.unwrap();
    pl.rect.min + egui::vec2(x + w / 2.0, y + hh / 2.0) * pl.zoom
}

/// The page shown now, by its display name.
fn shown_page(h: &Harness<'_, App>) -> String {
    let snap = &h.state().loaded.as_ref().unwrap().snap;
    let name = snap["interaction"]["page"]
        .as_str()
        .or_else(|| snap["report"]["active_page"].as_str())
        .unwrap_or_default()
        .to_string();
    snap["report"]["pages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == json!(name))
        .and_then(|p| p["display_name"].as_str())
        .unwrap_or_default()
        .to_string()
}

/// Shows page `index` through the reader's page action (a page tab click).
fn show_page(h: &mut Harness<'_, App>, index: usize) {
    let name = h.state().loaded.as_ref().unwrap().snap["report"]["pages"][index]["name"]
        .as_str()
        .unwrap()
        .to_string();
    h.state_mut().interact(json!({"action": "page", "page": name}));
    for _ in 0..50 {
        h.step();
    }
}

#[test]
fn a_click_on_a_synced_slicer_reaches_the_other_page() {
    let mut h = open();
    sync_both(&mut h);
    // a click on the item Toys of page 1's slicer, in the drawn page
    let at = slicer_item(&h, "Toys");
    click_with(&mut h, at, Modifiers::NONE);
    for _ in 0..50 {
        h.step();
    }
    assert_eq!(visual_of(&h, 0, "slicer")["slicer"]["selected"], json!(["Toys"]));
    assert_eq!(visual_of(&h, 0, "card")["data"]["rows"], json!([[5]]));
    // page 2's synced slicer has it, and its card is filtered
    assert_eq!(visual_of(&h, 1, "slicer")["slicer"]["selected"], json!(["Toys"]));
    assert_eq!(visual_of(&h, 1, "card")["data"]["rows"], json!([[5]]));
    // shown, page 2 draws the slicer with Toys selected: a click on Toys again clears the
    // group's selection (a plain click on the only selected item), on both pages
    show_page(&mut h, 1);
    assert_eq!(shown_page(&h), "Detail");
    let at = slicer_item(&h, "Toys");
    click_with(&mut h, at, Modifiers::NONE);
    for _ in 0..50 {
        h.step();
    }
    for page in 0..2 {
        let sel = &visual_of(&h, page, "slicer")["slicer"]["selected"];
        assert!(sel.is_null() || sel == &json!([]), "page {page}: {sel}");
        assert_eq!(visual_of(&h, page, "card")["data"]["rows"], json!([[105]]));
    }
}

/// Fields on: a field dropped on one slicer of the group goes to the others, one undo step.
#[test]
fn a_field_change_on_a_synced_slicer_reaches_the_group() {
    let mut h = open();
    sync_both(&mut h);
    let snap = h.state().loaded.as_ref().unwrap().snap.clone();
    let page = snap["report"]["pages"][0]["name"].as_str().unwrap().to_string();
    let slicer = visual_of(&h, 0, "slicer")["name"].as_str().unwrap().to_string();
    crate::panes::exec(
        h.state_mut(),
        "visual.bind",
        json!({"page": page, "visual": slicer, "role": "Values", "field": "Sales[Amount]"}),
    );
    steps(&mut h, 6);
    let fields = |h: &Harness<'_, App>, i: usize| visual_of(h, i, "slicer")["fields"].clone();
    assert_eq!(fields(&h, 0), fields(&h, 1));
    assert_eq!(
        fields(&h, 1).as_array().map(Vec::len),
        Some(2),
        "{}",
        fields(&h, 1)
    );
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::Z);
    steps(&mut h, 6);
    assert_eq!(fields(&h, 0).as_array().map(Vec::len), Some(1));
    assert_eq!(fields(&h, 1).as_array().map(Vec::len), Some(1), "one undo step");
}

#[test]
fn the_sync_pane_groups_slicers_and_unsyncs_them() {
    let mut h = open();
    h.get_by_label("Collapse the Sync slicers pane");
    // the two slicers, and only them, are listed
    let listed = slicers(&h);
    assert_eq!(listed.len(), 2, "{listed:?}");
    assert!(listed.iter().all(|(_, g)| g.is_null()), "{listed:?}");

    // Both selected, a group name typed, Synchronize: `slicer.set_sync`.
    press(&mut h, "By category");
    let other = h.get_by_label("By category too").rect().center();
    click_with(&mut h, other, Modifiers::COMMAND);
    assert_eq!(h.state().sync.selected.len(), 2, "Ctrl+click adds");
    h.get_by_label("Sync group name").click();
    steps(&mut h, 2);
    h.get_by_label("Sync group name").type_text("Category");
    steps(&mut h, 2);
    press(&mut h, "Synchronize");
    steps(&mut h, 4);
    let listed = slicers(&h);
    for (title, group) in &listed {
        assert_eq!(group["name"], json!("Category"), "{title}: {group}");
        assert_eq!(group["field_changes"], json!(true), "{title}: {group}");
        assert_eq!(group["filter_changes"], json!(true), "{title}: {group}");
    }

    // One of them out of the group: the other keeps it.
    press(&mut h, "By category too");
    press(&mut h, "Unsync");
    steps(&mut h, 4);
    let listed = slicers(&h);
    assert_eq!(listed[0].1["name"], json!("Category"), "{listed:?}");
    assert!(listed[1].1.is_null(), "{listed:?}");

    // The pane's state and width live in the preferences, as every other pane's.
    h.get_by_label("Collapse the Sync slicers pane").click();
    steps(&mut h, 3);
    assert!(!h.state().prefs.panes.sync);
    h.get_by_label("Open the Sync slicers pane").click();
    steps(&mut h, 3);
    assert!(h.state().prefs.panes.sync);
}
