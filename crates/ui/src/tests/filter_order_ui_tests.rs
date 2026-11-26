//! The Filters pane's card order (`filters_pane.rs`) on an engine session: a card dragged below
//! another in its section runs `filter.order {field, to}` (the custom order), each section's
//! sort list runs `filter.order {sort}`; the pane lists the cards in the snapshot's order, and a
//! reader can neither drag nor sort.

use std::path::Path;

use bicraft_engine::Session;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::backstage_tests::{engine_shell, steps};
use super::filters_ui_tests::{last_sent, page, shop_session};
use super::harness;
use super::panes_ui_tests::drag;
use crate::App;
use crate::shell::Caps;

/// The shop with two page filter cards, added Product then Category.
fn ordered_session() -> Session {
    let mut s = shop_session();
    let pg = s.report.as_ref().unwrap().pages[0].name.clone();
    for f in ["Sales[Product]", "Sales[Category]"] {
        s.execute("page.filter", &json!({"page": pg, "field": f}))
            .unwrap();
    }
    s.dirty = false;
    s.report_dirty = false;
    s
}

fn open_in(lang: &str) -> Harness<'static, App> {
    let mut app = App::new(lang, engine_shell(Caps::DESKTOP, ordered_session)).with_sync_render();
    app.open_path(Path::new("shop.pbip"));
    let mut h = harness(app, (1600.0, 900.0));
    for _ in 0..200 {
        h.step();
        if h.state().shown.is_some() && h.state().placed.is_some() {
            break;
        }
    }
    h.state_mut().prefs.panes.filters = true;
    steps(&mut h, 3);
    h
}

/// The page's cards in the snapshot's order (column names).
fn order(h: &Harness<'_, App>) -> Vec<String> {
    page(h)["filters"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|c| c["column"].as_str().unwrap_or("").to_string())
        .collect()
}

fn rect(h: &Harness<'_, App>, label: &str) -> egui::Rect {
    h.get_by_label(label).rect()
}

#[test]
fn a_card_dragged_below_another_takes_its_place() {
    let mut h = open_in("en-US");
    // A to Z by default
    assert_eq!(order(&h), ["Category", "Product"]);
    assert_eq!(page(&h)["filter_sort"], "ascending");
    let from = rect(&h, "Category is (All)").center();
    let below = rect(&h, "Product is (All)");
    drag(&mut h, from, egui::pos2(below.center().x, below.max.y + 6.0));
    let (id, p) = last_sent(&h);
    assert_eq!(id, "filter.order");
    assert_eq!(
        p,
        json!({"level": "page", "page": page(&h)["name"], "field": "Sales[Category]", "to": 1})
    );
    assert_eq!(order(&h), ["Product", "Category"]);
    assert_eq!(page(&h)["filter_sort"], "custom");
    // the pane follows: Product's card is now above Category's
    assert!(rect(&h, "Product is (All)").center().y < rect(&h, "Category is (All)").center().y);
    // the section's sort list: A to Z again
    h.get_by_label("Filters on this page sort order").click();
    steps(&mut h, 3);
    h.get_by_label("A to Z").click();
    steps(&mut h, 4);
    assert_eq!(last_sent(&h).1["sort"], "ascending");
    assert_eq!(order(&h), ["Category", "Product"]);
    // a reader neither sorts nor drags
    h.state_mut().access = Some("reader".into());
    steps(&mut h, 3);
    assert!(h.query_by_label("Filters on this page sort order").is_none());
    let sent = h.state().sent.len();
    let from = rect(&h, "Category is (All)").center();
    let below = rect(&h, "Product is (All)");
    drag(&mut h, from, egui::pos2(below.center().x, below.max.y + 6.0));
    assert!(
        !h.state().sent.iter().skip(sent).any(|c| c.id == "filter.order"),
        "a reader's drag orders nothing"
    );
}

#[test]
fn the_sort_list_speaks_french() {
    let mut h = open_in("fr-FR");
    h.get_by_label("Filtres sur cette page ordre de tri").click();
    steps(&mut h, 3);
    h.get_by_label("De Z à A").click();
    steps(&mut h, 4);
    assert_eq!(last_sent(&h).1["sort"], "descending");
    assert_eq!(order(&h), ["Product", "Category"]);
    let v: Value = page(&h)["filter_sort"].clone();
    assert_eq!(v, "descending");
}
