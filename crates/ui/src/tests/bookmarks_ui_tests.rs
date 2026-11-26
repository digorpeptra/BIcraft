//! The Bookmarks pane (`bookmarks_pane.rs`): the report's bookmarks in their order, groups
//! foldable, the Add, View, Rename, Delete, Group and Ungroup buttons and Desktop's Data /
//! Display / Current page options, every one an engine command, so the next snapshot shows it.

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

/// A report of two pages with a card on the first.
fn bookmarks_session() -> Session {
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
        "visual.add",
        &json!({"page": first, "visual_type": "card", "x": 20, "y": 20, "width": 200,
                "height": 120, "fields": [{"role": "Values", "field": "[Total]"}]}),
    )
    .unwrap();
    s.execute(
        "page.add",
        &json!({"display_name": "Detail", "width": 1280, "height": 720}),
    )
    .unwrap();
    s.execute("page.set_active", &json!({"page": first})).unwrap();
    s.dirty = false;
    s.report_dirty = false;
    s
}

/// The window on a fresh session with the Bookmarks pane open.
fn open() -> Harness<'static, App> {
    let mut app = App::new("en-US", engine_shell(Caps::DESKTOP, bookmarks_session)).with_sync_render();
    app.prefs.panes.bookmarks = true;
    app.open_path(Path::new("bookmarks.pbip"));
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

/// The snapshot's `bookmarks` view.
fn list(h: &Harness<'_, App>) -> Value {
    h.state().loaded.as_ref().unwrap().snap["bookmarks"].clone()
}

/// The display names of the listed bookmarks, in order.
fn listed(h: &Harness<'_, App>) -> Vec<String> {
    list(h)["bookmarks"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|b| b["display_name"].as_str().map(str::to_string))
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

/// Captures the active page's name, for the View check.
fn active_page(h: &Harness<'_, App>) -> String {
    h.state().loaded.as_ref().unwrap().snap["report"]["active_page"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

#[test]
fn the_bookmarks_pane_captures_views_renames_and_groups() {
    let mut h = open();
    h.get_by_label("Collapse the Bookmarks pane");
    assert!(listed(&h).is_empty(), "a fresh report has no bookmark");

    // Add: `bookmark.capture` with a default name.
    let first = active_page(&h);
    press(&mut h, "Add bookmark");
    assert_eq!(listed(&h), vec!["Bookmark 1".to_string()]);
    assert_eq!(list(&h)["count"], json!(1));

    // The bookmark's page comes with its Current page option, as Desktop's checkbox does.
    press(&mut h, "Bookmark 1");
    h.get_by_label("Current page").click();
    steps(&mut h, 4);
    assert_eq!(list(&h)["bookmarks"][0]["current_page"], json!(true));
    // The second page, then the bookmark's View (`bookmark.apply`) brings the first one back.
    let second = h.state().loaded.as_ref().unwrap().snap["report"]["pages"][1]["name"]
        .as_str()
        .unwrap()
        .to_string();
    crate::panes::exec(h.state_mut(), "page.set_active", json!({"page": second}));
    steps(&mut h, 4);
    assert_eq!(active_page(&h), second);
    press(&mut h, "View bookmark");
    steps(&mut h, 4);
    assert_eq!(active_page(&h), first, "the bookmark's page came back");

    // Rename in place: the editor takes the text, Enter calls `bookmark.set`.
    press(&mut h, "Rename");
    assert!(
        h.state().bookmarks.renaming.is_some(),
        "the rename editor opened: status {}",
        h.state().status
    );
    let editor = "Rename Bookmark 1";
    h.get_by_label(editor).click();
    steps(&mut h, 2);
    for _ in 0..12 {
        h.key_press(egui::Key::Backspace);
    }
    h.get_by_label(editor).type_text("Wheels");
    steps(&mut h, 2);
    h.key_press(egui::Key::Enter);
    steps(&mut h, 4);
    assert_eq!(listed(&h), vec!["Wheels".to_string()]);

    // A second bookmark, both grouped, then ungrouped.
    press(&mut h, "Add bookmark");
    assert_eq!(listed(&h), vec!["Wheels".to_string(), "Bookmark 1".to_string()]);
    press(&mut h, "Wheels");
    let row = h.get_by_label("Bookmark 1").rect().center();
    click_with(&mut h, row, Modifiers::COMMAND);
    press(&mut h, "Group");
    steps(&mut h, 4);
    assert_eq!(list(&h)["groups"].as_array().unwrap().len(), 1);
    assert_eq!(list(&h)["groups"][0]["display_name"], json!("Bookmark group 1"));
    assert_eq!(listed(&h).len(), 2, "the group's bookmarks stay listed");

    // A group folds: its bookmarks leave the list and come back.
    press(&mut h, "Bookmark group 1");
    h.get_by_label("Collapse the group").click();
    steps(&mut h, 3);
    assert!(h.query_by_label("Wheels").is_none(), "the member is folded away");
    press(&mut h, "Expand the group");
    h.get_by_label("Wheels");

    // Ungroup keeps the bookmarks.
    press(&mut h, "Bookmark group 1");
    press(&mut h, "Ungroup");
    steps(&mut h, 4);
    assert_eq!(list(&h)["groups"], json!([]));
    assert_eq!(listed(&h).len(), 2);

    // Delete the first bookmark.
    press(&mut h, "Wheels");
    press(&mut h, "Delete");
    steps(&mut h, 4);
    assert_eq!(listed(&h), vec!["Bookmark 1".to_string()]);

    // The pane's state and width live in the preferences, as every other pane's.
    h.get_by_label("Collapse the Bookmarks pane").click();
    steps(&mut h, 3);
    assert!(!h.state().prefs.panes.bookmarks);
    h.get_by_label("Open the Bookmarks pane").click();
    steps(&mut h, 3);
    assert!(h.state().prefs.panes.bookmarks);
}

/// The bookmarks session with a slicer on `Sales[Category]` beside the card.
fn slicer_session() -> Session {
    let mut s = bookmarks_session();
    let first = s.report.as_ref().unwrap().pages[0].name.clone();
    s.execute(
        "visual.add",
        &json!({"page": first, "visual_type": "slicer", "x": 260, "y": 20, "width": 220,
                "height": 120, "fields": [{"role": "Values", "field": "Sales[Category]"}]}),
    )
    .unwrap();
    s.dirty = false;
    s.report_dirty = false;
    s
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

/// The snapshot's slicer selection and card value on the first page.
fn slicer_and_card(h: &Harness<'_, App>) -> (Value, Value) {
    let snap = &h.state().loaded.as_ref().unwrap().snap;
    let visuals = snap["report"]["pages"][0]["visuals"].as_array().unwrap();
    let of = |ty: &str| visuals.iter().find(|v| v["type"] == json!(ty)).unwrap().clone();
    let sel = of("slicer")["slicer"]["selected"].clone();
    let sel = if sel.is_null() { json!([]) } else { sel };
    (sel, of("card")["data"]["rows"].clone())
}

fn settle(h: &mut Harness<'_, App>) {
    for _ in 0..50 {
        h.step();
    }
}

/// A slicer item clicked, Add bookmark, another item clicked, then View: the bookmark brings
/// the first selection back (`bookmark.capture` holds it, `bookmark.apply` restores it).
#[test]
fn a_bookmark_captures_and_restores_a_slicer_selection() {
    let mut app = App::new("en-US", engine_shell(Caps::DESKTOP, slicer_session)).with_sync_render();
    app.prefs.panes.bookmarks = true;
    app.open_path(Path::new("bookmarks.pbip"));
    let mut h = harness(app, (1400.0, 860.0));
    steps(&mut h, 6);
    for _ in 0..200 {
        h.step();
        if h.state().shown.is_some() && h.state().placed.is_some() {
            break;
        }
    }
    steps(&mut h, 2);
    let at = slicer_item(&h, "Toys");
    click_with(&mut h, at, Modifiers::NONE);
    settle(&mut h);
    assert_eq!(slicer_and_card(&h), (json!(["Toys"]), json!([[5]])));
    press(&mut h, "Add bookmark");
    settle(&mut h);
    assert_eq!(listed(&h), vec!["Bookmark 1".to_string()]);
    let at = slicer_item(&h, "Wheels");
    click_with(&mut h, at, Modifiers::NONE);
    settle(&mut h);
    assert_eq!(slicer_and_card(&h), (json!(["Wheels"]), json!([[100]])));
    press(&mut h, "Bookmark 1");
    press(&mut h, "View bookmark");
    settle(&mut h);
    assert_eq!(
        slicer_and_card(&h),
        (json!(["Toys"]), json!([[5]])),
        "the bookmark's slicer selection came back: status {}",
        h.state().status
    );
}
