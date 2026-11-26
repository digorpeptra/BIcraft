//! The mobile layout view (`mobile.rs`) on an engine session: View › Mobile layout shows the
//! phone canvas and the Page visuals pane; a visual dragged or double-clicked there lands on the
//! phone canvas; a drag moves it and a handle resizes it on Desktop's 18 x 20 grid, one undo step each,
//! the desktop layout untouched; Delete and the X take it off; the Format pane formats the phone
//! only; a narrow window reads the phone layout when its preference is on.

use std::path::Path;

use egui::accesskit::Role;
use egui::{Key, Modifiers, Pos2, Rect, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::backstage_tests::{engine_shell, steps};
use super::canvas_ui_tests::{count, drag, drag_with, key, open, session, undo};
use super::harness;
use crate::App;
use crate::canvas::Handle;
use crate::mobile::{self, PHONE_W};
use crate::shell::Caps;

/// The canvas tests' page (a card, a chart, a slicer), the card on the phone canvas at
/// (0, 0, 200, 200) when `placed`.
fn app_with(placed: bool, size: (f32, f32)) -> Harness<'static, App> {
    let make = move || {
        let mut s = session(true);
        if placed {
            let page = s.report.as_ref().unwrap().pages[0].name.clone();
            let card = s.report.as_ref().unwrap().pages[0]
                .visuals
                .iter()
                .find(|v| v.visual_type.as_deref() == Some("card"))
                .unwrap()
                .name
                .clone();
            s.execute(
                "visual.mobile",
                &json!({"page": page, "visual": card, "x": 0, "y": 0, "width": 200, "height": 200}),
            )
            .unwrap();
            s.dirty = false;
            s.report_dirty = false;
        }
        s
    };
    let mut app = App::new("en-US", engine_shell(Caps::DESKTOP, make)).with_sync_render();
    app.open_path(Path::new("shop.pbip"));
    let mut h = harness(app, size);
    open(&mut h);
    h
}

/// The window with View › Mobile layout on (by its ribbon button).
fn mobile_app(placed: bool) -> Harness<'static, App> {
    let mut h = app_with(placed, (1400.0, 900.0));
    h.get_by_role_and_label(Role::Button, "View").click();
    steps(&mut h, 3);
    h.get_by_role_and_label(Role::Button, "Mobile layout").click();
    steps(&mut h, 6);
    assert!(h.state().mobile.on);
    h
}

/// The real snapshot's entry of the visual of type `ty` on the first page.
fn entry(app: &App, ty: &str) -> Value {
    app.loaded.as_ref().unwrap().snap["report"]["pages"][0]["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["type"] == ty)
        .unwrap_or_else(|| panic!("no {ty}"))
        .clone()
}

fn name(app: &App, ty: &str) -> String {
    entry(app, ty)["name"].as_str().unwrap().to_string()
}

fn rect(v: &Value) -> Rect {
    let n = |k: &str| v[k].as_f64().unwrap() as f32;
    Rect::from_min_size(pos2(n("x"), n("y")), vec2(n("width"), n("height")))
}

/// The phone-canvas rectangle of the visual of type `ty` (`None`: not on it).
fn phone_rect(app: &App, ty: &str) -> Option<Rect> {
    let e = entry(app, ty);
    e.get("mobile").filter(|m| m.is_object()).map(rect)
}

/// The screen point of phone point `p`.
fn screen(app: &App, p: Pos2) -> Pos2 {
    let pl = app.placed.unwrap();
    pl.rect.min + p.to_vec2() * pl.zoom
}

#[test]
fn view_mobile_layout_shows_the_phone_canvas_and_the_page_visuals() {
    let mut h = mobile_app(true);
    assert!(mobile::drawn(h.state()));
    let pl = h.state().placed.unwrap();
    assert!(
        (pl.rect.width() / pl.zoom - 324.0).abs() < 0.5,
        "the canvas is Desktop's phone width, 324: {:?} at {}",
        pl.rect,
        pl.zoom
    );
    // Desktop's phone canvas scrolls to 3600 (Power BI Desktop 2.159.1231.0, docs/gaps.md G72)
    assert!(
        pl.rect.height() / pl.zoom >= 3600.0 - 0.5,
        "the phone canvas is 3600 high: {:?} at {}",
        pl.rect,
        pl.zoom
    );
    assert!(pl.rect.height() > pl.rect.width(), "portrait");
    // the Page visuals pane lists every visual with its appearance; the card is on the phone
    h.get_by_label("Page visuals");
    h.get_by_label("Card (shown, On phone)");
    h.get_by_label("Slicer (shown)");
    // off again: the page itself
    h.get_by_role_and_label(Role::Button, "Mobile layout").click();
    steps(&mut h, 4);
    assert!(!mobile::drawn(h.state()));
    let pl = h.state().placed.unwrap();
    assert!(pl.rect.width() > pl.rect.height(), "the desktop page again");
}

#[test]
fn a_visual_dragged_from_page_visuals_lands_on_the_phone_on_the_grid() {
    let mut h = mobile_app(false);
    let slicer = name(h.state(), "slicer");
    let from = h.get_by_label("Slicer (shown)").rect().center();
    let to = screen(h.state(), pos2(37.0, 61.0));
    drag(&mut h, from, to, Modifiers::NONE);
    steps(&mut h, 4);
    assert_eq!(count(&h, "visual.mobile"), 1, "one command");
    let r = phone_rect(h.state(), "slicer").expect("placed");
    assert_eq!(r.min.x % 18.0, 0.0, "{r:?} on Desktop's 18-wide grid columns");
    assert_eq!(r.min.y % 20.0, 0.0, "{r:?} on Desktop's 20-high grid rows");
    assert_eq!(r.width() % 18.0, 0.0, "{r:?} a whole number of columns");
    assert!(r.width() <= PHONE_W);
    // the desktop layout is the visual's own still
    assert_eq!(
        rect(&entry(h.state(), "slicer")),
        Rect::from_min_size(pos2(100.0, 420.0), vec2(250.0, 200.0))
    );
    assert_eq!(
        h.state().cmd.visual.as_ref().map(|v| v.visual.clone()),
        Some(slicer)
    );
    // a double click places the next one under it
    let row = h.get_by_label("Card (shown)").rect().center();
    // a harness step is 0.25 s: a person's quick double click
    h.ctx
        .options_mut(|o| o.input_options.max_double_click_delay = 2.0);
    h.hover_at(row);
    h.step();
    for _ in 0..2 {
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton {
                pos: row,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
        }
        h.step();
    }
    steps(&mut h, 4);
    let c = phone_rect(h.state(), "card").expect("the double-clicked card is placed");
    assert!(c.min.y >= r.max.y, "under the slicer: {c:?} {r:?}");
    undo(&mut h);
    assert!(phone_rect(h.state(), "card").is_none(), "one undo step");
}

#[test]
fn a_drag_moves_and_a_handle_resizes_on_the_phone_snapped_in_one_step() {
    let mut h = mobile_app(true);
    let card = name(h.state(), "card");
    let desk = rect(&entry(h.state(), "card"));
    let r0 = phone_rect(h.state(), "card").unwrap();
    let z = h.state().placed.unwrap().zoom;
    let from = screen(h.state(), r0.center());
    drag(&mut h, from, from + vec2(13.0, 21.0) * z, Modifiers::NONE);
    assert_eq!(count(&h, "visual.mobile"), 1);
    let r = phone_rect(h.state(), "card").unwrap();
    assert_eq!(r.size(), r0.size());
    assert_eq!(
        (r.min.x % 18.0, r.min.y % 20.0),
        (0.0, 0.0),
        "{r:?} on the 18 x 20 grid"
    );
    assert_ne!(r, r0, "moved");
    assert_eq!(rect(&entry(h.state(), "card")), desk, "the desktop layout stays");
    assert_eq!(
        h.state().cmd.visual.as_ref().map(|v| v.visual.clone()),
        Some(card)
    );
    undo(&mut h);
    assert_eq!(phone_rect(h.state(), "card").unwrap(), r0, "one undo step");
    // the bottom-right handle; Alt: no snapping, the exact distance
    let from = screen(h.state(), Handle::SE.pos(r0));
    let mut ghost = None;
    drag_with(&mut h, from, from - vec2(40.0, 30.0) * z, Modifiers::ALT, |h| {
        ghost = h.state().canvas.gesture.as_ref().map(|g| g.now.clone());
    });
    assert!(ghost.is_some(), "the phone canvas's own gesture");
    let r = phone_rect(h.state(), "card").unwrap();
    assert_eq!(r, Rect::from_min_size(r0.min, r0.size() - vec2(40.0, 30.0)));
    assert_eq!(count(&h, "visual.move"), 0, "the desktop layout is never moved");
    // an arrow nudges on the phone
    key(&mut h, Key::ArrowRight, Modifiers::NONE);
    assert_eq!(phone_rect(h.state(), "card").unwrap().min.x, r0.min.x + 1.0);
}

#[test]
fn delete_and_the_x_take_a_visual_off_the_phone_only() {
    let mut h = mobile_app(true);
    let r0 = phone_rect(h.state(), "card").unwrap();
    let at = screen(h.state(), r0.center());
    super::canvas_ui_tests::click_with(&mut h, at, Modifiers::NONE);
    assert!(h.state().cmd.visual.is_some());
    key(&mut h, Key::Delete, Modifiers::NONE);
    steps(&mut h, 3);
    assert!(phone_rect(h.state(), "card").is_none(), "off the phone canvas");
    assert_eq!(count(&h, "visual.remove"), 0);
    entry(h.state(), "card");
    undo(&mut h);
    assert_eq!(phone_rect(h.state(), "card"), Some(r0));
    // the X at its top right
    let at = screen(h.state(), r0.center());
    super::canvas_ui_tests::click_with(&mut h, at, Modifiers::NONE);
    h.get_by_label("Remove from mobile layout").click();
    steps(&mut h, 4);
    assert!(phone_rect(h.state(), "card").is_none());
    entry(h.state(), "card");
}

#[test]
fn the_format_pane_formats_the_phone_only() {
    let mut h = mobile_app(true);
    let r0 = phone_rect(h.state(), "card").unwrap();
    let at = screen(h.state(), r0.center());
    super::canvas_ui_tests::click_with(&mut h, at, Modifiers::NONE);
    h.get_by_role_and_label(Role::Button, "Format visual").click();
    steps(&mut h, 3);
    h.get_by_label("General cards").click();
    steps(&mut h, 6);
    h.get_by_label("Mobile layout: these settings apply to the phone only.");
    let read = h
        .state()
        .sent
        .iter()
        .rev()
        .find(|c| c.id == "visual.props")
        .expect("props read")
        .params
        .clone();
    assert_eq!(read["layout"], "mobile");
    h.get_by_role_and_label(Role::CheckBox, "Title").click();
    steps(&mut h, 6);
    let edits: Vec<_> = h
        .state()
        .sent
        .iter()
        .filter(|c| c.id != "visual.props")
        .map(|c| (c.id, c.params.clone()))
        .collect();
    assert_eq!(
        edits.last().map(|e| e.0),
        Some("visual.mobile_format"),
        "the phone's own formatting: {edits:?}"
    );
    assert_eq!(count(&h, "visual.set_title") + count(&h, "visual.format"), 0);
    let e = entry(h.state(), "card");
    assert_eq!(e["mobile"]["formatted"], true, "{e}");
    assert_ne!(
        e["mobile"]["format"]["title"]["show"], e["format"]["title"]["show"],
        "the phone's title differs from the desktop's: {e}"
    );
    // Clear mobile formatting puts it back
    h.get_by_label("Clear mobile formatting").click();
    steps(&mut h, 6);
    assert_eq!(entry(h.state(), "card")["mobile"]["formatted"], false);
}

#[test]
fn a_narrow_window_reads_the_phone_layout_when_asked() {
    let mut h = app_with(true, (560.0, 900.0));
    {
        let p = &mut h.state_mut().prefs.panes;
        p.data = false;
        p.build = false;
        p.filters = false;
    }
    steps(&mut h, 4);
    assert!(!mobile::drawn(h.state()), "off by default");
    h.state_mut().prefs.mobile_narrow = true;
    steps(&mut h, 4);
    assert!(mobile::drawn(h.state()), "a narrow window reads the phone layout");
    assert!(!mobile::editing(h.state()));
    let pl = h.state().placed.unwrap();
    assert!(pl.rect.width() > PHONE_W, "it fills the width: {:?}", pl.rect);
    // nothing is edited there: a drag moves nothing
    let from = pl.rect.min + vec2(100.0, 100.0);
    drag(&mut h, from, from + vec2(40.0, 40.0), Modifiers::ALT);
    assert_eq!(count(&h, "visual.mobile") + count(&h, "visual.move"), 0);
}
