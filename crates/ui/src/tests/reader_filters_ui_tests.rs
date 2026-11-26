//! A reader's Filters pane (`filters_pane.rs`, gap G41): an unlocked card's edits are the
//! reader's own view state (`view.interact` `filter` / `filter_clear`), never a report edit; a
//! locked card stays read-only; a reader removes no card.

use std::path::Path;

use bicraft_engine::Session;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::backstage_tests::{engine_shell, steps};
use super::filters_ui_tests::{page, shop_session, snap};
use super::harness;
use super::panes_ui_tests::visual_named;
use crate::App;
use crate::ribbon::VisualSel;
use crate::shell::Caps;

/// The shop with two page cards: Category (unlocked, no condition) and Product (locked).
fn reader_session() -> Session {
    let mut s = shop_session();
    let pg = s.report.as_ref().unwrap().pages[0].name.clone();
    s.execute("page.filter", &json!({"page": pg, "field": "Sales[Category]"}))
        .unwrap();
    s.execute(
        "page.filter",
        &json!({"page": pg, "field": "Sales[Product]", "locked": true}),
    )
    .unwrap();
    s.dirty = false;
    s.report_dirty = false;
    s
}

fn open_as_reader() -> Harness<'static, App> {
    let mut app = App::new("en-US", engine_shell(Caps::DESKTOP, reader_session)).with_sync_render();
    app.open_path(Path::new("shop.pbip"));
    let mut h = harness(app, (1600.0, 900.0));
    for _ in 0..200 {
        h.step();
        if h.state().shown.is_some() && h.state().placed.is_some() {
            break;
        }
    }
    h.state_mut().access = Some("reader".into());
    steps(&mut h, 3);
    h
}

fn categories(h: &Harness<'_, App>, sel: &VisualSel) -> Vec<String> {
    let v = page(h)["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == sel.visual.as_str())
        .cloned()
        .unwrap();
    let mut out: Vec<String> = v["data"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r[0].as_str().unwrap_or("").to_string())
        .collect();
    out.sort();
    out
}

fn card(h: &Harness<'_, App>, field: &str) -> Value {
    page(h)["filters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["field"] == field)
        .cloned()
        .unwrap()
}

fn click(h: &mut Harness<'_, App>, label: &str) {
    h.get_by_label(label).click();
    steps(h, 6);
}

/// Whether the UI sent any report edit (anything but reads).
fn edits_sent(h: &Harness<'_, App>) -> Vec<&'static str> {
    h.state()
        .sent
        .iter()
        .map(|c| c.id)
        .filter(|id| !matches!(*id, "column.values" | "measure.get" | "column.get" | "table.get"))
        .collect()
}

#[test]
fn a_reader_changes_an_unlocked_filter_for_their_view_only() {
    let mut h = open_as_reader();
    let chart = visual_named(&h, "clusteredColumnChart");
    let revision = snap(&h)["revision"].clone();
    // the unlocked card: its values, a click filters the page for this reader
    click(&mut h, "Category is (All)");
    steps(&mut h, 6);
    click(&mut h, "Toys");
    let sent = h.state().last_action.clone().unwrap();
    assert_eq!(sent["action"], "filter");
    assert_eq!(sent["field"], "Sales[Category]");
    assert_eq!(sent["not_in"], json!(["Toys"]));
    assert_eq!(categories(&h, &chart), vec!["Games", "Wheels"]);
    assert_eq!(
        card(&h, "Sales[Category]")["condition"],
        json!({"not_in": ["Toys"]})
    );
    h.get_by_label("Category is not Toys");
    // no report edit, no new revision
    assert_eq!(edits_sent(&h), Vec::<&str>::new());
    assert_eq!(snap(&h)["revision"], revision);
    // the eraser clears the reader's condition; a reader removes no card
    assert!(h.query_by_label("Remove filter on Category").is_none());
    click(&mut h, "Clear filter on Category");
    assert_eq!(h.state().last_action.clone().unwrap()["action"], "filter_clear");
    assert_eq!(categories(&h, &chart), vec!["Games", "Toys", "Wheels"]);
    assert_eq!(card(&h, "Sales[Category]")["type"], "none");
    // the locked card: its lock, values that do not change
    h.get_by_label("Locked filter: Product");
    click(&mut h, "Product is (All)");
    steps(&mut h, 6);
    let before = h.state().last_action.clone();
    click(&mut h, "Ball");
    assert_eq!(h.state().last_action, before, "a locked card sends nothing");
    assert_eq!(card(&h, "Sales[Product]")["type"], "none");
    assert_eq!(edits_sent(&h), Vec::<&str>::new());
}
