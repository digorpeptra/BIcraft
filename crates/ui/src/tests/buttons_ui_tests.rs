//! Buttons, shapes and navigators in the Report view (docs/formats/buttons.md): the pointer over
//! a button with an On hover / On press look draws the page again in that state; a reader's
//! click on the selected button of a bookmark navigator with Allow deselection deselects it; the
//! action editor writes a conditional (fx) destination that the reader's click follows; the
//! Format pane's "Apply settings to: State" writes the state picked.

use std::path::Path;

use bicraft_engine::Session;
use egui::accesskit::Role;
use egui::{Event, Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::backstage_tests::{engine_shell, steps};
use super::harness;
use crate::App;
use crate::shell::Caps;

/// Pages P1 and P2; on P1 a button "Go" with an On hover fill, a bookmark navigator with Allow
/// deselection over bookmarks "A" and "B", a plain shape; measure [Where] names page P2.
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
            json!({"table": "Sales", "name": "Where", "expression": "\"P2\""}),
        ),
        ("report.new", json!({})),
        ("page.add", json!({"display_name": "P2"})),
    ] {
        s.execute(id, &p).unwrap();
    }
    let first = s.report.as_ref().unwrap().pages[0].name.clone();
    s.execute("page.rename", &json!({"page": first, "display_name": "P1"}))
        .unwrap();
    s.execute("page.set_active", &json!({"page": first})).unwrap();
    let add = |s: &mut Session, kind: &str, y: u32| {
        s.execute(
            "button.add",
            &json!({"page": first, "kind": kind, "x": 20, "y": y, "width": 400, "height": 60}),
        )
        .unwrap()["visual"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let bt = add(&mut s, "blank", 20);
    s.execute(
        "visual.format",
        &json!({"page": first, "visual": bt, "objects": [
            {"object": "text", "property": "show", "value": true},
            {"object": "text", "property": "text", "value": "Go"},
            {"object": "fill", "property": "show", "value": true},
            {"object": "fill", "property": "fillColor", "value": {"color": "#102030"}},
            {"object": "fill", "property": "fillColor", "value": {"color": "#40C040"}, "state": "hover"},
            {"object": "fill", "property": "fillColor", "value": {"color": "#C04040"}, "state": "press"},
        ]}),
    )
    .unwrap();
    for name in ["A", "B"] {
        s.execute("bookmark.capture", &json!({"display_name": name}))
            .unwrap();
    }
    let nav = add(&mut s, "bookmark_navigator", 200);
    s.execute(
        "visual.format",
        &json!({"page": first, "visual": nav, "objects": [
            {"object": "bookmarks", "property": "allowDeselectionBookmark", "value": true},
        ]}),
    )
    .unwrap();
    s.execute(
        "shape.add",
        &json!({"page": first, "shape": "rectangle", "x": 600, "y": 20, "width": 200, "height": 100}),
    )
    .unwrap();
    s.dirty = false;
    s.report_dirty = false;
    s
}

fn open(reader: bool) -> Harness<'static, App> {
    let mut app = App::new("en-US", engine_shell(Caps::DESKTOP, session)).with_sync_render();
    app.open_path(Path::new("buttons.pbip"));
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

fn snap(a: &App) -> Value {
    a.loaded.as_ref().unwrap().snap.as_ref().clone()
}

fn visual(a: &App, ty: &str) -> Value {
    snap(a)["report"]["pages"][0]["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["type"] == ty)
        .cloned()
        .unwrap()
}

/// The screen centre of visual `v`.
fn on_canvas(a: &App, v: &Value) -> Pos2 {
    let placed = a.placed.unwrap();
    let n = |k: &str| v[k].as_f64().unwrap() as f32;
    placed.rect.min + egui::vec2(n("x") + n("width") / 2.0, n("y") + n("height") / 2.0) * placed.zoom
}

/// The screen point of the page's hit whose action has `key` = `value`.
fn hit_at(a: &App, key: &str, value: &str) -> Pos2 {
    let Some((_, Ok(img))) = a.shown.as_ref() else {
        panic!("no page drawn");
    };
    let hit = img
        .hits
        .iter()
        .find(|x| x.action_for(false).is_some_and(|a| a[key] == json!(value)))
        .unwrap_or_else(|| panic!("no hit with {key} = {value}"));
    let (x, y, w, hh) = hit.shape.bounds();
    let placed = a.placed.unwrap();
    placed.rect.min + egui::vec2(x + w / 2.0, y + hh / 2.0) * placed.zoom
}

/// The page's pixels on screen (the app renders raw RGBA).
fn png(a: &App) -> Vec<u8> {
    match a.shown.as_ref() {
        Some((_, Ok(img))) => img.rgba.clone(),
        _ => Vec::new(),
    }
}

fn pointer_button(h: &mut Harness<'_, App>, pos: Pos2, pressed: bool) {
    h.event_modifiers(
        Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        },
        Modifiers::NONE,
    );
    h.step();
}

fn click(h: &mut Harness<'_, App>, pos: Pos2) {
    h.hover_at(pos);
    h.step();
    pointer_button(h, pos, true);
    pointer_button(h, pos, false);
    steps(h, 3);
}

#[test]
fn the_pointer_over_a_button_draws_its_hover_and_press_states() {
    let mut h = open(true);
    let bt = visual(h.state(), "actionButton");
    let name = bt["name"].as_str().unwrap().to_string();
    let plain = png(h.state());
    let at = on_canvas(h.state(), &bt);
    h.hover_at(at);
    until(&mut h, "the hover state", |a| {
        a.pointer.as_ref().is_some_and(|p| p.visual == name && !p.press)
    });
    until(&mut h, "the page drawn in the hover state", |a| {
        a.shown.as_ref().is_some_and(|(k, _)| k.pointer != 0) && png(a) != plain
    });
    let hover = png(h.state());
    pointer_button(&mut h, at, true);
    until(&mut h, "the press state", |a| {
        a.pointer.as_ref().is_some_and(|p| p.press)
    });
    until(&mut h, "the page drawn in the press state", |a| {
        png(a) != hover && png(a) != plain
    });
    pointer_button(&mut h, at, false);
    // a shape without hover or press looks keeps the page as it is
    let shape = visual(h.state(), "shape");
    let off = on_canvas(h.state(), &shape);
    h.hover_at(off);
    until(&mut h, "no pointer state", |a| a.pointer.is_none());
    until(&mut h, "the plain page", |a| png(a) == plain);
}

#[test]
fn a_readers_click_on_the_selected_bookmark_deselects_it() {
    let mut h = open(true);
    let a = snap(h.state())["bookmarks"]["bookmarks"][0]["name"]
        .as_str()
        .unwrap()
        .to_string();
    let at = hit_at(h.state(), "bookmark", &a);
    click(&mut h, at);
    until(&mut h, "bookmark A applied", |s| {
        snap(s)["interaction"]["bookmark"] == json!(a)
    });
    // the selected button now deselects (Allow deselection, no deselection bookmark)
    let again = hit_at(h.state(), "action", "bookmark_deselect");
    click(&mut h, again);
    let sent = h.state().last_action.clone().unwrap();
    assert_eq!(sent["action"], "bookmark_deselect", "{sent}");
    until(&mut h, "no bookmark selected", |s| {
        snap(s)["interaction"]["bookmark"].is_null()
    });
}

#[test]
fn the_action_editor_writes_a_conditional_destination_the_reader_follows() {
    let mut h = open(false);
    let bt = visual(h.state(), "actionButton");
    let at = on_canvas(h.state(), &bt);
    click(&mut h, at);
    until(&mut h, "the button selected", |a| a.cmd.visual.is_some());
    h.get_all_by_label("Visual")
        .min_by(|a, b| a.rect().center().y.total_cmp(&b.rect().center().y))
        .unwrap()
        .click();
    steps(&mut h, 3);
    h.get_by_label("Action").click();
    steps(&mut h, 3);
    {
        let f = h.state_mut().cmd.rich.action.as_mut().unwrap();
        f.kind = "page_navigation".into();
    }
    steps(&mut h, 2);
    h.get_by_label("fx  Conditional destination (based on a field)")
        .click();
    steps(&mut h, 2);
    assert!(h.state().cmd.rich.action.as_ref().unwrap().fx);
    h.state_mut().cmd.rich.action.as_mut().unwrap().field = "[Where]".into();
    steps(&mut h, 2);
    h.get_by_label("Apply action").click();
    until(&mut h, "the action", |a| {
        a.sent.last().is_some_and(|c| c.id == "visual.set_action") && a.calls_pending() == 0
    });
    let p = h.state().sent.last().unwrap().params.clone();
    assert_eq!(p["type"], "page_navigation", "{p}");
    assert_eq!(p["destination_field"], "[Where]", "{p}");
    assert!(p.get("destination").is_none(), "{p}");
    until(&mut h, "the conditional destination evaluated", |a| {
        visual(a, "actionButton")["interaction"]["button"]["conditional"] == json!(true)
    });
    h.state_mut().cmd.rich.action = None;
    h.state_mut().access = Some("reader".into());
    steps(&mut h, 3);
    let p2 = h.state().page_names()[1].0.clone();
    let b = visual(h.state(), "actionButton")["interaction"]["button"].clone();
    assert_eq!(b["destination"], json!(p2), "{b}");
    click(&mut h, at);
    let sent = h.state().last_action.clone().unwrap();
    assert_eq!(sent["destination"], json!(p2), "{sent}");
    until(&mut h, "page P2", |a| a.active_page() == 1);
}

#[test]
fn the_format_pane_writes_the_state_picked() {
    let mut h = open(false);
    let shape = visual(h.state(), "shape");
    let at = on_canvas(h.state(), &shape);
    click(&mut h, at);
    until(&mut h, "the shape selected", |a| a.cmd.visual.is_some());
    h.get_by_role_and_label(Role::Button, "Format visual").click();
    steps(&mut h, 3);
    h.get_by_label("Visual cards").click();
    steps(&mut h, 6);
    h.get_by_label("Apply settings to: State");
    h.state_mut().panes.format.state = "press".into();
    steps(&mut h, 2);
    h.get_by_role_and_label(Role::Button, "Fill").click();
    steps(&mut h, 3);
    h.get_by_label("Fill Colour").click();
    steps(&mut h, 3);
    h.get_by_label("Fill Colour #252423").click();
    until(&mut h, "the press fill", |a| {
        a.calls_pending() == 0 && visual(a, "shape")["shape"]["states"]["press"]["fill"] == "#252423"
    });
    let p = h
        .state()
        .sent
        .iter()
        .rev()
        .find(|c| c.id == "visual.format")
        .map(|c| c.params.clone())
        .unwrap();
    assert_eq!(p["objects"][0]["state"], "press", "{p}");
    assert_ne!(
        visual(h.state(), "shape")["shape"]["fill"],
        "#252423",
        "the default state is left alone"
    );
}
