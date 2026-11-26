//! Page and bookmark navigators on the canvas (G55): a reader's click on a bookmark navigator's
//! button runs `view.interact {action: "bookmark"}` and the next snapshot shows the bookmark's
//! page and hidden visuals, the report left as it was; a click on a page navigator's button
//! changes page; in the editor both act on Ctrl+click only, as Desktop's buttons do.

use std::path::Path;

use bicraft_engine::Session;
use egui::{Event, Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;
use serde_json::{Value, json};

use super::backstage_tests::{engine_shell, steps};
use super::harness;
use crate::App;
use crate::shell::Caps;

/// Two pages; the first holds a card, a bookmark navigator and a page navigator. Bookmark
/// "Hide card" hides the card (Current page off); bookmark "Go detail" shows page Detail.
fn session() -> Session {
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
    let card = s
        .execute(
            "visual.add",
            &json!({"page": first, "visual_type": "card", "x": 20, "y": 20, "width": 200,
                    "height": 120, "fields": [{"role": "Values", "field": "[Total]"}]}),
        )
        .unwrap()["visual"]
        .as_str()
        .unwrap()
        .to_string();
    for (kind, y) in [("bookmark_navigator", 200), ("page_navigator", 300)] {
        s.execute(
            "button.add",
            &json!({"page": first, "kind": kind, "x": 20, "y": y, "width": 400, "height": 60}),
        )
        .unwrap();
    }
    s.execute(
        "page.add",
        &json!({"display_name": "Detail", "width": 1280, "height": 720}),
    )
    .unwrap();
    let second = s.report.as_ref().unwrap().pages[1].name.clone();
    // "Hide card": the card hidden, the page left alone
    s.execute(
        "visual.set_hidden",
        &json!({"page": first, "visual": card, "hidden": true}),
    )
    .unwrap();
    s.execute("page.set_active", &json!({"page": first})).unwrap();
    s.execute("bookmark.capture", &json!({"display_name": "Hide card"}))
        .unwrap();
    s.execute(
        "visual.set_hidden",
        &json!({"page": first, "visual": card, "hidden": false}),
    )
    .unwrap();
    // "Go detail": page Detail
    s.execute("page.set_active", &json!({"page": second})).unwrap();
    s.execute(
        "bookmark.capture",
        &json!({"display_name": "Go detail", "current_page": true}),
    )
    .unwrap();
    s.execute("page.set_active", &json!({"page": first})).unwrap();
    s.dirty = false;
    s.report_dirty = false;
    s
}

fn open(reader: bool) -> Harness<'static, App> {
    let mut app = App::new("en-US", engine_shell(Caps::DESKTOP, session)).with_sync_render();
    app.open_path(Path::new("navigators.pbip"));
    let mut h = harness(app, (1400.0, 860.0));
    steps(&mut h, 6);
    if reader {
        h.state_mut().access = Some("reader".into());
    }
    until(&mut h, "the page drawn", |a| {
        a.shown.is_some() && a.placed.is_some()
    });
    steps(&mut h, 2);
    h
}

fn until(h: &mut Harness<'_, App>, what: &str, ok: impl Fn(&App) -> bool) {
    for _ in 0..400 {
        if ok(h.state()) {
            return;
        }
        h.step();
    }
    panic!("never: {what}");
}

fn snap(h: &Harness<'_, App>) -> Value {
    snap_of(h.state())
}

fn snap_of(a: &App) -> Value {
    a.loaded.as_ref().unwrap().snap.as_ref().clone()
}

/// The card's `hidden` in the snapshot.
fn card_hidden(a: &App) -> bool {
    snap_of(a)["report"]["pages"][0]["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["type"] == "card")
        .map(|v| v["hidden"] == json!(true))
        .unwrap()
}

/// The screen point of the navigator hit whose action has `key` = `value`.
fn button_at(h: &Harness<'_, App>, key: &str, value: &str) -> Pos2 {
    let app = h.state();
    let Some((_, Ok(img))) = app.shown.as_ref() else {
        panic!("no page drawn");
    };
    let hit = img
        .hits
        .iter()
        .find(|x| x.action_for(false).is_some_and(|a| a[key] == json!(value)))
        .unwrap_or_else(|| panic!("no hit with {key} = {value}"));
    let (x, y, w, hh) = hit.shape.bounds();
    let placed = app.placed.unwrap();
    placed.rect.min + egui::vec2(x + w / 2.0, y + hh / 2.0) * placed.zoom
}

fn click_with(h: &mut Harness<'_, App>, pos: Pos2, modifiers: Modifiers) {
    h.hover_at(pos);
    h.step();
    for pressed in [true, false] {
        h.event_modifiers(
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers,
            },
            modifiers,
        );
        h.step();
    }
    steps(h, 3);
}

fn bookmark_named(h: &Harness<'_, App>, display: &str) -> String {
    snap(h)["bookmarks"]["bookmarks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["display_name"] == display)
        .and_then(|b| b["name"].as_str())
        .unwrap()
        .to_string()
}

#[test]
fn a_readers_click_on_a_bookmark_navigator_applies_the_bookmark_to_the_view() {
    let mut h = open(true);
    assert!(!card_hidden(h.state()));
    let hide = bookmark_named(&h, "Hide card");
    let at = button_at(&h, "bookmark", &hide);
    click_with(&mut h, at, Modifiers::NONE);
    let sent = h.state().last_action.clone().unwrap();
    assert_eq!(sent["action"], "bookmark");
    assert_eq!(sent["bookmark"], json!(hide));
    until(&mut h, "the card hidden", card_hidden);
    let s = snap(&h);
    assert_eq!(s["interaction"]["bookmark"], json!(hide));
    assert_eq!(h.state().active_page(), 0, "Current page off: the page stays");
    assert_eq!(
        s["dirty"],
        json!({"model": false, "report": false, "queries": false}),
        "the report is unchanged"
    );
    // the other bookmark goes to its page
    let go = bookmark_named(&h, "Go detail");
    let at = button_at(&h, "bookmark", &go);
    click_with(&mut h, at, Modifiers::NONE);
    until(&mut h, "page Detail", |a| a.active_page() == 1);
}

#[test]
fn a_readers_click_on_a_page_navigator_changes_page() {
    let mut h = open(true);
    let detail = h.state().page_names()[1].0.clone();
    let at = button_at(&h, "page", &detail);
    click_with(&mut h, at, Modifiers::NONE);
    let sent = h.state().last_action.clone().unwrap();
    assert_eq!(sent["action"], "page");
    assert_eq!(sent["page"], json!(detail));
    // the hit names its navigator (the editor asks for Ctrl+click on it)
    let nav = snap(&h)["report"]["pages"][0]["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["type"] == "pageNavigator")
        .map(|v| v["name"].clone())
        .unwrap();
    assert_eq!(sent["visual"], nav);
    until(&mut h, "page Detail", |a| a.active_page() == 1);
}

#[test]
fn in_the_editor_navigators_act_on_ctrl_click() {
    let mut h = open(false);
    let hide = bookmark_named(&h, "Hide card");
    let at = button_at(&h, "bookmark", &hide);
    click_with(&mut h, at, Modifiers::NONE);
    steps(&mut h, 5);
    assert!(
        h.state()
            .last_action
            .as_ref()
            .is_none_or(|a| a["action"] != "bookmark"),
        "a plain click selects the navigator"
    );
    assert!(!card_hidden(h.state()));
    click_with(&mut h, at, Modifiers::COMMAND);
    until(&mut h, "the card hidden", card_hidden);
}
