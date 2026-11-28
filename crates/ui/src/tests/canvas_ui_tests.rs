//! Canvas editing (ROADMAP M11.16) driven through the app's own input: pointer drags that move
//! and resize visuals with the handles, snapping to edges with smart guides and to the grid, the
//! rubber band, Ctrl+click and Ctrl+A, arrow nudges, the ribbon's Align and Layer order menus,
//! Group, copy, paste and duplicate across pages and documents, one undo step each; hostile
//! clipboard text, a reader refused, and the same frames in the desktop and browser shells.

use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;

use bicraft_engine::Session;
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, Vec2, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::backstage_tests::{engine_shell, right_click, steps};
use super::{frame, harness, shell_app, shop_snapshot};
use crate::App;
use crate::app::Tab;
use crate::canvas::{self, Handle};
use crate::registry::Tab as RTab;
use crate::shell::Caps;

/// Sales(Category, Amount), [Total]; pages P1 (a card, a chart, a slicer) and P2. `fields`:
/// false leaves the model without the Sales table (another document's model).
pub(super) fn session(fields: bool) -> Session {
    let mut s = Session::new();
    let sales = r#"#table({"Category", "Amount"}, {{"Wheels", 100}, {"Toys", 5}})"#;
    s.execute("model.new", &json!({"name": "Shop"})).unwrap();
    if fields {
        for (id, p) in [
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
        ] {
            s.execute(id, &p).unwrap();
        }
    }
    s.execute("report.new", &json!({})).unwrap();
    s.execute("page.add", &json!({"display_name": "P2"})).unwrap();
    if fields {
        let page = s.report.as_ref().unwrap().pages[0].name.clone();
        for (ty, r, f) in [
            (
                "card",
                [100, 100, 300, 200],
                json!([{"role": "Values", "field": "[Total]"}]),
            ),
            (
                "clusteredColumnChart",
                [520, 100, 400, 300],
                json!([{"role": "Category", "field": "Sales[Category]"}, {"role": "Y", "field": "[Total]"}]),
            ),
            (
                "slicer",
                [100, 420, 250, 200],
                json!([{"role": "Values", "field": "Sales[Category]"}]),
            ),
        ] {
            s.execute(
                "visual.add",
                &json!({"page": page, "visual_type": ty, "x": r[0], "y": r[1], "width": r[2],
                        "height": r[3], "fields": f}),
            )
            .unwrap();
        }
    }
    s.dirty = false;
    s.report_dirty = false;
    s
}

pub(super) fn open(h: &mut Harness<'static, App>) {
    for _ in 0..200 {
        h.step();
        if h.state().shown.is_some() {
            break;
        }
    }
    steps(h, 2);
}

fn app() -> Harness<'static, App> {
    let mut app = App::new("en-US", engine_shell(Caps::DESKTOP, || session(true))).with_sync_render();
    app.open_path(Path::new("shop.pbip"));
    let mut h = harness(app, (1280.0, 800.0));
    open(&mut h);
    h
}

/// The visuals of page `page`: (name, type, x, y, width, height).
fn visuals(app: &App, page: usize) -> Vec<(String, String, f64, f64, f64, f64)> {
    let Some(l) = &app.loaded else { return Vec::new() };
    l.snap
        .pointer(&format!("/report/pages/{page}/visuals"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|v| {
            let n = |k: &str| v[k].as_f64().unwrap_or(f64::NAN);
            (
                v["name"].as_str().unwrap_or("").to_string(),
                v["type"].as_str().unwrap_or("").to_string(),
                n("x"),
                n("y"),
                n("width"),
                n("height"),
            )
        })
        .collect()
}

fn find(app: &App, ty: &str) -> (String, Rect) {
    let v = visuals(app, 0)
        .into_iter()
        .find(|v| v.1 == ty)
        .unwrap_or_else(|| panic!("no {ty}"));
    (
        v.0,
        Rect::from_min_size(pos2(v.2 as f32, v.3 as f32), vec2(v.4 as f32, v.5 as f32)),
    )
}

fn rect_of(app: &App, name: &str) -> Rect {
    for page in 0..2 {
        if let Some(v) = visuals(app, page).into_iter().find(|v| v.0 == name) {
            return Rect::from_min_size(pos2(v.2 as f32, v.3 as f32), vec2(v.4 as f32, v.5 as f32));
        }
    }
    panic!("no visual {name}");
}

/// The screen point of report point `p`.
fn screen(app: &App, p: Pos2) -> Pos2 {
    let pl = app.placed.unwrap();
    pl.rect.min + p.to_vec2() * pl.zoom
}

fn zoom(app: &App) -> f32 {
    app.placed.unwrap().zoom
}

pub(super) fn count(h: &Harness<'_, App>, id: &str) -> usize {
    h.state().sent.iter().filter(|c| c.id == id).count()
}

/// A press at `from`, moves to `to` in steps, a release; `mods` held (Alt: no snapping).
/// `before_release` runs on the last frame of the drag.
pub(super) fn drag_with(
    h: &mut Harness<'static, App>,
    from: Pos2,
    to: Pos2,
    mods: Modifiers,
    before_release: impl FnOnce(&Harness<'static, App>),
) {
    h.hover_at(from);
    h.step();
    h.event_modifiers(
        Event::PointerButton {
            pos: from,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: mods,
        },
        mods,
    );
    h.step();
    for k in 1..=6 {
        let p = from + (to - from) * (k as f32 / 6.0);
        h.event_modifiers(Event::PointerMoved(p), mods);
        h.step();
    }
    before_release(h);
    h.event_modifiers(
        Event::PointerButton {
            pos: to,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: mods,
        },
        mods,
    );
    steps(h, 4);
}

pub(super) fn drag(h: &mut Harness<'static, App>, from: Pos2, to: Pos2, mods: Modifiers) {
    drag_with(h, from, to, mods, |_| {});
}

pub(super) fn click_with(h: &mut Harness<'static, App>, pos: Pos2, mods: Modifiers) {
    h.hover_at(pos);
    h.step();
    for pressed in [true, false] {
        h.event_modifiers(
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: mods,
            },
            mods,
        );
        h.step();
    }
    steps(h, 2);
}

pub(super) fn key(h: &mut Harness<'static, App>, key: Key, modifiers: Modifiers) {
    for pressed in [true, false] {
        h.event_modifiers(
            Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers,
            },
            modifiers,
        );
    }
    steps(h, 4);
}

pub(super) fn undo(h: &mut Harness<'static, App>) {
    key(h, Key::Z, Modifiers::COMMAND);
}

#[test]
fn a_drag_moves_a_visual_in_one_undo_step() {
    let mut h = app();
    let (card, r) = find(h.state(), "card");
    let z = zoom(h.state());
    let from = screen(h.state(), r.center());
    // Alt: no snapping, the exact distance
    drag(&mut h, from, from + vec2(40.0, 30.0) * z, Modifiers::ALT);
    assert_eq!(count(&h, "visual.move"), 1, "one command for the gesture");
    assert_eq!(rect_of(h.state(), &card), r.translate(vec2(40.0, 30.0)));
    assert_eq!(
        h.state().cmd.visual.as_ref().map(|v| v.visual.clone()),
        Some(card.clone())
    );
    undo(&mut h);
    assert_eq!(rect_of(h.state(), &card), r);
    // a visual kept on the page: dragged far left, it stops at x = 0
    drag(&mut h, from, from - vec2(400.0, 0.0) * z, Modifiers::ALT);
    assert_eq!(rect_of(h.state(), &card).min.x, 0.0);
}

#[test]
fn the_eight_handles_resize() {
    let mut h = app();
    let (card, r0) = find(h.state(), "card");
    click_rep(&mut h, r0.center(), Modifiers::NONE);
    let z = zoom(h.state());
    let d = vec2(20.0, 10.0);
    for handle in Handle::ALL {
        let r = rect_of(h.state(), &card);
        let from = screen(h.state(), handle.pos(r));
        drag(&mut h, from, from + d * z, Modifiers::ALT);
        let got = rect_of(h.state(), &card);
        let mut want = r;
        let (fx, fy) = match handle {
            Handle::NW => (0.0, 0.0),
            Handle::N => (0.5, 0.0),
            Handle::NE => (1.0, 0.0),
            Handle::E => (1.0, 0.5),
            Handle::SE => (1.0, 1.0),
            Handle::S => (0.5, 1.0),
            Handle::SW => (0.0, 1.0),
            Handle::W => (0.0, 0.5),
        };
        if fx == 0.0 {
            want.min.x += d.x;
        }
        if fx == 1.0 {
            want.max.x += d.x;
        }
        if fy == 0.0 {
            want.min.y += d.y;
        }
        if fy == 1.0 {
            want.max.y += d.y;
        }
        assert_eq!(got, want, "{handle:?}");
        undo(&mut h);
        assert_eq!(rect_of(h.state(), &card), r, "{handle:?}: one undo step");
    }
    assert_eq!(count(&h, "visual.move"), 8);
}

#[test]
fn edges_snap_to_other_visuals_with_a_smart_guide_and_to_the_grid() {
    let mut h = app();
    let (card, r) = find(h.state(), "card");
    let (_, chart) = find(h.state(), "clusteredColumnChart");
    let z = zoom(h.state());
    let from = screen(h.state(), r.center());
    // the card's top edge lands 3 px under the chart's: it snaps onto it, a guide shows
    let mut guides = Vec::new();
    let mut status = None;
    drag_with(
        &mut h,
        from,
        from + vec2(37.0, chart.min.y - r.min.y + 3.0) * z,
        Modifiers::NONE,
        |h| {
            guides = h.state().canvas.gesture.as_ref().unwrap().guides.clone();
            status = canvas::status(h.state());
        },
    );
    assert!(
        guides.iter().any(|g| !g.vertical && g.at == chart.min.y),
        "a horizontal guide on the chart's top edge: {guides:?}"
    );
    assert!(status.is_some_and(|s| s.contains("300 × 200")));
    let got = rect_of(h.state(), &card);
    assert_eq!(got.min.y, chart.min.y, "snapped");
    // Snap to grid: far from any edge, the corner lands on the 8 px grid
    h.state_mut().prefs.snap_grid = true;
    let from = screen(h.state(), got.center());
    drag(&mut h, from, from + vec2(14.0, 80.0) * z, Modifiers::NONE);
    let g = rect_of(h.state(), &card);
    assert_eq!((g.min.x % 8.0, g.min.y % 8.0), (0.0, 0.0), "{g:?}");
}

#[test]
fn a_rubber_band_selects_then_the_group_moves_and_is_deleted_together() {
    let mut h = app();
    let (card, rc) = find(h.state(), "card");
    let (chart, rh) = find(h.state(), "clusteredColumnChart");
    let (slicer, _) = find(h.state(), "slicer");
    // from the empty corner over the card and the chart
    let a = screen(h.state(), pos2(20.0, 20.0));
    let b = screen(h.state(), pos2(600.0, 150.0));
    drag(&mut h, a, b, Modifiers::NONE);
    let sel = canvas::selection(h.state());
    assert_eq!(sel.len(), 2, "{sel:?}");
    assert!(sel.contains(&card) && sel.contains(&chart) && !sel.contains(&slicer));
    assert_eq!(count(&h, "visual.move"), 0, "a band moves nothing");
    // the group moves together, one command
    let z = zoom(h.state());
    let from = screen(h.state(), rc.center());
    drag(&mut h, from, from + vec2(0.0, 50.0) * z, Modifiers::ALT);
    assert_eq!(count(&h, "visual.move"), 1);
    assert_eq!(rect_of(h.state(), &card), rc.translate(vec2(0.0, 50.0)));
    assert_eq!(rect_of(h.state(), &chart), rh.translate(vec2(0.0, 50.0)));
    // resized together by the selection's handle
    let bounds = rect_of(h.state(), &card).union(rect_of(h.state(), &chart));
    let from = screen(h.state(), Handle::E.pos(bounds));
    drag(&mut h, from, from - vec2(82.0, 0.0) * z, Modifiers::ALT);
    let (c2, h2) = (rect_of(h.state(), &card), rect_of(h.state(), &chart));
    assert!(c2.width() < rc.width() && h2.width() < rh.width());
    assert!((h2.max.x - (bounds.max.x - 82.0)).abs() <= 1.0, "{h2:?}");
    // Delete removes both in one command, one undo step
    key(&mut h, Key::Delete, Modifiers::NONE);
    assert_eq!(visuals(h.state(), 0).len(), 1);
    assert_eq!(count(&h, "visual.remove"), 1);
    undo(&mut h);
    assert_eq!(visuals(h.state(), 0).len(), 3);
}

#[test]
fn ctrl_click_and_ctrl_a_select_several() {
    let mut h = app();
    let (card, rc) = find(h.state(), "card");
    let (chart, rh) = find(h.state(), "clusteredColumnChart");
    click_rep(&mut h, rc.center(), Modifiers::NONE);
    click_rep(&mut h, rh.center(), Modifiers::COMMAND);
    let sel = canvas::selection(h.state());
    assert_eq!(sel, vec![chart.clone(), card.clone()]);
    // the Visual tab and its Arrange group act on both
    click_rep(&mut h, rh.center(), Modifiers::COMMAND);
    assert_eq!(canvas::selection(h.state()), vec![card.clone()]);
    key(&mut h, Key::A, Modifiers::COMMAND);
    assert_eq!(canvas::selection(h.state()).len(), 3);
    // a plain click selects one again; Escape none
    click_rep(&mut h, rh.center(), Modifiers::NONE);
    assert_eq!(canvas::selection(h.state()), vec![chart.clone()]);
    key(&mut h, Key::Escape, Modifiers::NONE);
    assert!(canvas::selection(h.state()).is_empty());
}

#[test]
fn arrow_keys_nudge_the_selection() {
    let mut h = app();
    let (card, r) = find(h.state(), "card");
    click_rep(&mut h, r.center(), Modifiers::NONE);
    key(&mut h, Key::ArrowRight, Modifiers::NONE);
    assert_eq!(rect_of(h.state(), &card), r.translate(vec2(1.0, 0.0)));
    key(&mut h, Key::ArrowDown, Modifiers::SHIFT);
    assert_eq!(rect_of(h.state(), &card), r.translate(vec2(1.0, 10.0)));
    assert_eq!(count(&h, "visual.move"), 2, "one command per press");
    undo(&mut h);
    assert_eq!(rect_of(h.state(), &card), r.translate(vec2(1.0, 0.0)));
}

#[test]
fn the_arrange_menus_align_order_and_group() {
    let mut h = app();
    let (card, rc) = find(h.state(), "card");
    let (slicer, _) = find(h.state(), "slicer");
    click_rep(&mut h, rc.center(), Modifiers::NONE);
    let rs = rect_of(h.state(), &slicer);
    click_rep(&mut h, rs.center(), Modifiers::COMMAND);
    h.state_mut().ribbon_tab = RTab::Visual;
    steps(&mut h, 3);
    h.get_by_label("Align").click();
    steps(&mut h, 3);
    h.get_by_label("Align right").click();
    steps(&mut h, 4);
    assert_eq!(count(&h, "visual.align"), 1);
    let (c, s) = (rect_of(h.state(), &card), rect_of(h.state(), &slicer));
    assert_eq!(c.max.x, s.max.x, "right edges aligned");
    assert_eq!(c.max.x, 400.0);
    undo(&mut h);
    assert_eq!(rect_of(h.state(), &slicer), rs);
    // layer order: both to the back
    h.get_by_label("Layer order").click();
    steps(&mut h, 3);
    h.get_by_label("Send to back").click();
    steps(&mut h, 4);
    let last = h.state().sent.last().unwrap().clone();
    assert_eq!(last.id, "visual.order");
    assert_eq!(last.params["to"], "back");
    assert_eq!(last.params["visuals"].as_array().map(Vec::len), Some(2));
    // group, then ungroup
    h.get_by_label("Group").click();
    steps(&mut h, 4);
    let groups = || {
        h.state().loaded.as_ref().unwrap().snap["report"]["pages"][0]["visuals"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| v["type"].is_null())
            .count()
    };
    assert_eq!(groups(), 1);
    h.get_by_label("Ungroup").click();
    steps(&mut h, 4);
    assert_eq!(
        h.state().loaded.as_ref().unwrap().snap["report"]["pages"][0]["visuals"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| v["type"].is_null())
            .count(),
        0
    );
}

/// Ctrl+C, then the paste event on another page and in another document (whose model lacks
/// the fields: kept, with a warning), Ctrl+V's in-app fallback, Ctrl+D; each one undo step.
#[test]
fn copy_paste_and_duplicate_across_pages_and_documents() {
    let made = Rc::new(Cell::new(0));
    let m = made.clone();
    let shell = engine_shell(Caps::DESKTOP, move || {
        m.set(m.get() + 1);
        session(m.get() == 1)
    });
    let mut app = App::new("en-US", shell).with_sync_render();
    app.open_path(Path::new("one.pbip"));
    let mut h = harness(app, (1280.0, 800.0));
    open(&mut h);
    let (card, rc) = find(h.state(), "card");
    click_rep(&mut h, rc.center(), Modifiers::NONE);
    h.event(Event::Copy);
    steps(&mut h, 4);
    assert_eq!(count(&h, "visual.copy"), 1);
    let clip = h
        .state()
        .canvas
        .clip
        .clone()
        .expect("the in-app clipboard holds the clip");
    let text = serde_json::to_string(&clip).unwrap();
    // on P2: the paste event's text, the same place, the copy selected
    let p2 = h.state().page_names()[1].0.clone();
    h.state_mut().interact(json!({"action": "page", "page": p2}));
    steps(&mut h, 6);
    assert_eq!(h.state().active_page(), 1);
    h.event(Event::Paste(text.clone()));
    steps(&mut h, 6);
    let on2 = visuals(h.state(), 1);
    assert_eq!(on2.len(), 1);
    assert_eq!((on2[0].2, on2[0].3), (rc.min.x as f64, rc.min.y as f64));
    assert_ne!(on2[0].0, card);
    assert_eq!(canvas::selection(h.state()), vec![on2[0].0.clone()]);
    undo(&mut h);
    assert!(visuals(h.state(), 1).is_empty(), "one undo step");
    // another document: Ctrl+V with no paste event (an unreadable clipboard) uses the in-app one
    h.state_mut().open_path(Path::new("two.pbip"));
    open(&mut h);
    assert_eq!(h.state().active_doc(), 1);
    assert!(visuals(h.state(), 0).is_empty());
    key(&mut h, Key::V, Modifiers::COMMAND);
    steps(&mut h, 4);
    let there = visuals(h.state(), 0);
    assert_eq!(there.len(), 1, "pasted from the in-app clipboard");
    assert!(
        h.state().status.contains("broken references"),
        "{}",
        h.state().status
    );
    // Ctrl+D duplicates it, moved off its place
    key(&mut h, Key::D, Modifiers::COMMAND);
    let there = visuals(h.state(), 0);
    assert_eq!(there.len(), 2);
    assert_eq!((there[1].2 - there[0].2, there[1].3 - there[0].3), (20.0, 20.0));
    undo(&mut h);
    assert_eq!(visuals(h.state(), 0).len(), 1);
}

#[test]
fn the_visual_menu_copies_pastes_and_duplicates() {
    let mut h = app();
    let (_, rc) = find(h.state(), "card");
    right_rep(&mut h, rc.center());
    for label in ["Copy", "Paste", "Duplicate", "Delete visual"] {
        assert!(h.query_by_label(label).is_some(), "{label}");
    }
    h.get_by_label("Duplicate").click();
    steps(&mut h, 4);
    assert_eq!(visuals(h.state(), 0).len(), 4);
    right_rep(&mut h, rc.center());
    h.get_by_label("Copy").click();
    steps(&mut h, 4);
    assert!(h.state().canvas.clip.is_some());
    // the empty page's menu: Paste (the shell reads no clipboard here: the in-app one)
    right_rep(&mut h, pos2(1200.0, 700.0));
    assert!(h.query_by_label("Copy").is_none());
    h.get_by_label("Paste").click();
    steps(&mut h, 4);
    assert_eq!(visuals(h.state(), 0).len(), 5);
}

/// Clipboard text is untrusted: garbage, other JSON, a huge text and hostile clips never panic,
/// say why and paste nothing; a shell reader that fails falls back to the in-app clipboard.
#[test]
fn hostile_clipboards_never_break_the_canvas() {
    let mut h = app();
    let huge = format!(
        "{{\"bicraft\": \"visuals\", \"x\": \"{}\"}}",
        "x".repeat(canvas::MAX_CLIP_TEXT)
    );
    let deep = format!("{}{}", "[".repeat(5000), "]".repeat(5000));
    let hostile = json!({"bicraft": "visuals", "version": 1, "visuals": [
        {"name": "a", "position": {"x": 1e300, "y": 0, "width": 1, "height": 1}},
        {"name": "b", "visual_type": "card", "position": "nowhere"},
    ]})
    .to_string();
    for text in [
        "".to_string(),
        "hello".into(),
        "{\"bicraft\": \"pages\"}".into(),
        "[1, 2, 3]".into(),
        "\u{0}\u{ffff}".into(),
        huge,
        deep,
        hostile,
    ] {
        let before = visuals(h.state(), 0).len();
        h.event(Event::Paste(text));
        steps(&mut h, 5);
        assert_eq!(visuals(h.state(), 0).len(), before);
        assert!(
            !h.state().status.is_empty() || h.state().error.is_some(),
            "the window says why"
        );
        h.state_mut().status.clear();
        h.state_mut().error = None;
    }
    // a reader that fails: the in-app clipboard
    let (_, rc) = find(h.state(), "card");
    click_rep(&mut h, rc.center(), Modifiers::NONE);
    h.event(Event::Copy);
    steps(&mut h, 4);
    h.state_mut().shell.read_clipboard = Some(Box::new(|slot: canvas::ClipSlot| {
        *slot.borrow_mut() = Some(Err("denied".into()));
    }));
    crate::canvas::paste_command(h.state_mut());
    steps(&mut h, 4);
    assert_eq!(visuals(h.state(), 0).len(), 4);
    // a reader that gives garbage: the in-app clipboard too
    h.state_mut().shell.read_clipboard = Some(Box::new(|slot: canvas::ClipSlot| {
        *slot.borrow_mut() = Some(Ok("{not json".into()));
    }));
    crate::canvas::paste_command(h.state_mut());
    steps(&mut h, 4);
    assert_eq!(visuals(h.state(), 0).len(), 5);
}

#[test]
fn a_reader_cannot_move_paste_or_nudge() {
    let mut h = app();
    h.state_mut().access = Some("reader".into());
    steps(&mut h, 2);
    let (card, r) = find(h.state(), "card");
    let z = zoom(h.state());
    let from = screen(h.state(), r.center());
    drag(&mut h, from, from + vec2(40.0, 30.0) * z, Modifiers::ALT);
    click_with(&mut h, from, Modifiers::NONE);
    key(&mut h, Key::ArrowRight, Modifiers::NONE);
    assert_eq!(rect_of(h.state(), &card), r);
    assert_eq!(count(&h, "visual.move"), 0);
    // copying reads: allowed; pasting is refused with the reason
    h.event(Event::Copy);
    steps(&mut h, 4);
    assert_eq!(count(&h, "visual.copy"), 1);
    let text = serde_json::to_string(&h.state().canvas.clip.clone().unwrap()).unwrap();
    h.event(Event::Paste(text));
    steps(&mut h, 4);
    assert_eq!(count(&h, "visual.paste"), 0);
    assert!(h.state().status.contains("read-only"), "{}", h.state().status);
}

/// The selection, its handles, a drag's ghosts and guides and the band draw the same frames in
/// the desktop and browser shells.
#[test]
fn the_canvas_draws_the_same_frames_in_both_shells() {
    let snap = shop_snapshot();
    let mut desktop = shell_app(Caps::DESKTOP, &snap, "en-US");
    let mut browser = shell_app(Caps::BROWSER, &snap, "en-US");
    for h in [&mut desktop, &mut browser] {
        key(h, Key::A, Modifiers::COMMAND);
        assert_eq!(canvas::selection(h.state()).len(), 3);
        let page = h.state().page_names()[0].0.clone();
        let items: Vec<(String, Rect)> = canvas::page_items(h.state())
            .unwrap()
            .2
            .iter()
            .map(|i| (i.name.clone(), i.rect))
            .collect();
        h.state_mut().canvas.gesture = Some(canvas::Gesture {
            page,
            kind: canvas::Kind::Move,
            start: pos2(0.0, 0.0),
            now: items
                .iter()
                .map(|(n, r)| (n.clone(), r.translate(Vec2::splat(12.0))))
                .collect(),
            items,
            band: Some(Rect::from_min_size(pos2(5.0, 5.0), vec2(50.0, 50.0))),
            guides: vec![canvas::Guide {
                vertical: true,
                at: 260.0,
                from: 0.0,
                to: 500.0,
            }],
        });
        steps(h, 3);
    }
    let (a, b) = (frame(&desktop), frame(&browser));
    assert!(a.0 == b.0, "the meshes differ");
    assert!(a.1 == b.1, "the font atlases differ");
    let _ = Tab::Report;
}

/// A click at report point `p`.
fn click_rep(h: &mut Harness<'static, App>, p: Pos2, mods: Modifiers) {
    let s = screen(h.state(), p);
    click_with(h, s, mods);
}

/// A right click at report point `p`.
fn right_rep(h: &mut Harness<'static, App>, p: Pos2) {
    let s = screen(h.state(), p);
    right_click(h, s);
}

/// The group made of the card and the slicer (Visual › Group), then nothing selected.
fn grouped(h: &mut Harness<'static, App>) -> (String, String) {
    let (card, rc) = find(h.state(), "card");
    let (_, rs) = find(h.state(), "slicer");
    click_rep(h, rc.center(), Modifiers::NONE);
    click_rep(h, rs.center(), Modifiers::COMMAND);
    h.state_mut().ribbon_tab = RTab::Visual;
    steps(h, 3);
    h.get_by_label("Group").click();
    steps(h, 4);
    let group = visuals(h.state(), 0)
        .into_iter()
        .find(|v| v.1.is_empty())
        .expect("a group")
        .0;
    // an empty spot of the page clears the selection
    click_rep(h, pos2(1100.0, 650.0), Modifiers::NONE);
    assert!(canvas::selection(h.state()).is_empty());
    (group, card)
}

/// A click on a grouped visual selects its group first (as Desktop does), a second click the
/// visual itself; a drag on a member of an unselected group moves the whole group.
#[test]
fn a_click_selects_the_group_first_then_the_member() {
    let mut h = app();
    let (group, card) = grouped(&mut h);
    let rc = rect_of(h.state(), &card);
    click_rep(&mut h, rc.center(), Modifiers::NONE);
    assert_eq!(canvas::selection(h.state()), vec![group.clone()]);
    click_rep(&mut h, rc.center(), Modifiers::NONE);
    assert_eq!(canvas::selection(h.state()), vec![card.clone()]);
    click_rep(&mut h, pos2(1100.0, 650.0), Modifiers::NONE);
    let g0 = rect_of(h.state(), &group);
    let z = zoom(h.state());
    let from = screen(h.state(), rc.center());
    drag(&mut h, from, from + vec2(30.0, 0.0) * z, Modifiers::ALT);
    assert_eq!(rect_of(h.state(), &group), g0.translate(vec2(30.0, 0.0)));
    assert_eq!(canvas::selection(h.state()), vec![group]);
}

/// Shift while dragging a handle keeps the aspect ratio of the box taken.
#[test]
fn shift_resize_keeps_the_aspect_ratio() {
    let mut h = app();
    let (card, r0) = find(h.state(), "card");
    click_rep(&mut h, r0.center(), Modifiers::NONE);
    let z = zoom(h.state());
    let shift_alt = Modifiers {
        shift: true,
        alt: true,
        ..Modifiers::NONE
    };
    // SE: the larger of the two scales (300x200, +60 and +10: x1.2)
    let from = screen(h.state(), Handle::SE.pos(r0));
    drag(&mut h, from, from + vec2(60.0, 10.0) * z, shift_alt);
    assert_eq!(
        rect_of(h.state(), &card),
        Rect::from_min_size(r0.min, vec2(360.0, 240.0))
    );
    undo(&mut h);
    // E: the width's scale, centred vertically
    let from = screen(h.state(), Handle::E.pos(r0));
    drag(&mut h, from, from + vec2(-150.0, 0.0) * z, shift_alt);
    assert_eq!(
        rect_of(h.state(), &card),
        Rect::from_min_size(pos2(100.0, 150.0), vec2(150.0, 100.0))
    );
    undo(&mut h);
    // without Shift: free
    let from = screen(h.state(), Handle::SE.pos(r0));
    drag(&mut h, from, from + vec2(60.0, 10.0) * z, Modifiers::ALT);
    assert_eq!(
        rect_of(h.state(), &card),
        Rect::from_min_size(r0.min, vec2(360.0, 210.0))
    );
}

/// Ctrl+X cuts: the clip kept, the visual removed, one undo step; the shells' cut event and the
/// visual's menu Cut do the same.
#[test]
fn ctrl_x_cuts_the_selection() {
    let mut h = app();
    let (card, rc) = find(h.state(), "card");
    click_rep(&mut h, rc.center(), Modifiers::NONE);
    key(&mut h, Key::X, Modifiers::COMMAND);
    assert_eq!(count(&h, "visual.cut"), 1);
    assert_eq!(visuals(h.state(), 0).len(), 2);
    assert!(visuals(h.state(), 0).iter().all(|v| v.0 != card));
    assert!(h.state().canvas.clip.is_some(), "the clip is kept");
    assert!(canvas::selection(h.state()).is_empty());
    undo(&mut h);
    assert_eq!(visuals(h.state(), 0).len(), 3, "one undo step");
    click_rep(&mut h, rc.center(), Modifiers::NONE);
    h.event(Event::Cut);
    steps(&mut h, 4);
    assert_eq!(count(&h, "visual.cut"), 2);
    undo(&mut h);
    right_rep(&mut h, rc.center());
    h.get_by_label("Cut").click();
    steps(&mut h, 4);
    assert_eq!(count(&h, "visual.cut"), 3);
    assert_eq!(visuals(h.state(), 0).len(), 2);
}

/// The page menu's Paste puts the clip's top-left corner at the right-clicked point; Ctrl+V
/// keeps the copied place.
#[test]
fn the_menu_paste_goes_to_the_pointer() {
    let mut h = app();
    let (_, rc) = find(h.state(), "card");
    click_rep(&mut h, rc.center(), Modifiers::NONE);
    h.event(Event::Copy);
    steps(&mut h, 4);
    right_rep(&mut h, pos2(900.0, 500.0));
    h.get_by_label("Paste").click();
    steps(&mut h, 4);
    let last = h.state().sent.last().unwrap().clone();
    assert_eq!(last.id, "visual.paste");
    assert_eq!(
        (last.params["x"].as_f64(), last.params["y"].as_f64()),
        (Some(900.0), Some(500.0))
    );
    let made = canvas::selection(h.state());
    assert_eq!(made.len(), 1);
    assert_eq!(rect_of(h.state(), &made[0]).min, pos2(900.0, 500.0));
    key(&mut h, Key::V, Modifiers::COMMAND);
    steps(&mut h, 12);
    let last = h.state().sent.last().unwrap().clone();
    assert_eq!(last.id, "visual.paste");
    assert!(last.params.get("x").is_none(), "{}", last.params);
}

// ---------------------------------------------------------------------------------------------
// View › Page options (Gridlines, Snap to grid, Lock objects), Page view and the status bar's zoom
// ---------------------------------------------------------------------------------------------

fn view_button(h: &mut Harness<'static, App>, label: &str) {
    h.state_mut().ribbon_tab = RTab::View;
    steps(h, 3);
    h.get_by_label(label).click();
    steps(h, 3);
}

/// View › Snap to grid: a move and a resize land on the 8 px grid; off, they do not.
#[test]
fn snap_to_grid_puts_moves_and_resizes_on_the_grid() {
    let mut h = app();
    let (card, r) = find(h.state(), "card");
    let z = zoom(h.state());
    // off: a move by (37, 77), far from the other visuals' edges, keeps the odd offset
    let from = screen(h.state(), r.center());
    drag(&mut h, from, from + vec2(37.0, 77.0) * z, Modifiers::NONE);
    let off = rect_of(h.state(), &card);
    assert!(off.min.x % 8.0 != 0.0, "{off:?}");
    undo(&mut h);
    view_button(&mut h, "Snap to grid");
    assert!(h.state().prefs.snap_grid);
    let from = screen(h.state(), r.center());
    drag(&mut h, from, from + vec2(37.0, 77.0) * z, Modifiers::NONE);
    let moved = rect_of(h.state(), &card);
    assert_eq!((moved.min.x % 8.0, moved.min.y % 8.0), (0.0, 0.0), "{moved:?}");
    assert_ne!(moved, r);
    // the south-east handle: the moved corner lands on the grid
    let from = screen(h.state(), Handle::SE.pos(moved));
    drag(&mut h, from, from + vec2(23.0, -30.0) * z, Modifiers::NONE);
    let sized = rect_of(h.state(), &card);
    assert_eq!((sized.max.x % 8.0, sized.max.y % 8.0), (0.0, 0.0), "{sized:?}");
    assert_eq!(sized.min, moved.min);
    assert!(sized.width() > moved.width());
    // Alt still moves freely
    let from = screen(h.state(), sized.center());
    drag(&mut h, from, from + vec2(13.0, 0.0) * z, Modifiers::ALT);
    assert!(rect_of(h.state(), &card).min.x % 8.0 != 0.0);
}

/// View › Lock objects: no drag, resize or nudge moves a visual, a rubber band still selects;
/// unlocked, the drag moves it again.
#[test]
fn lock_objects_stops_moves_resizes_and_nudges() {
    let mut h = app();
    let (card, r) = find(h.state(), "card");
    let (chart, _) = find(h.state(), "clusteredColumnChart");
    let z = zoom(h.state());
    view_button(&mut h, "Lock objects");
    assert!(h.state().prefs.lock_objects);
    let from = screen(h.state(), r.center());
    drag(&mut h, from, from + vec2(40.0, 30.0) * z, Modifiers::ALT);
    assert_eq!(rect_of(h.state(), &card), r);
    assert!(h.state().status.contains("locked"), "{}", h.state().status);
    click_with(&mut h, from, Modifiers::NONE);
    assert_eq!(canvas::selection(h.state()), vec![card.clone()], "still selected");
    let corner = screen(h.state(), Handle::SE.pos(r));
    drag(&mut h, corner, corner + vec2(30.0, 30.0) * z, Modifiers::ALT);
    key(&mut h, Key::ArrowRight, Modifiers::NONE);
    assert_eq!(rect_of(h.state(), &card), r);
    assert_eq!(count(&h, "visual.move"), 0);
    // a band from the empty corner still selects
    let a = screen(h.state(), pos2(20.0, 20.0));
    let b = screen(h.state(), pos2(600.0, 150.0));
    drag(&mut h, a, b, Modifiers::NONE);
    let sel = canvas::selection(h.state());
    assert!(sel.contains(&card) && sel.contains(&chart), "{sel:?}");
    // unlocked: the drag moves
    view_button(&mut h, "Lock objects");
    assert!(!h.state().prefs.lock_objects);
    click_with(&mut h, from, Modifiers::NONE);
    drag(&mut h, from, from + vec2(40.0, 30.0) * z, Modifiers::ALT);
    assert_eq!(count(&h, "visual.move"), 1);
    assert_ne!(rect_of(h.state(), &card), r);
}

/// View › Gridlines is a toggle of its own, kept in the preferences (the file does not change).
#[test]
fn gridlines_toggle_alone() {
    let mut h = app();
    let sent = h.state().sent.len();
    view_button(&mut h, "Gridlines");
    assert!(h.state().prefs.show_gridlines);
    assert!(!h.state().prefs.snap_grid && !h.state().prefs.lock_objects);
    assert_eq!(h.state().sent.len(), sent, "no command: a view setting");
    view_button(&mut h, "Gridlines");
    assert!(!h.state().prefs.show_gridlines);
}

/// View › Gridlines draws the grid in the page image, over the page and under the visuals: a
/// grid point of the empty page changes, one under the card keeps the card's pixels.
#[test]
fn gridlines_are_drawn_under_the_visuals() {
    let mut h = app();
    let shown = |h: &Harness<'static, App>| match h.state().shown.as_ref() {
        Some((k, Ok(img))) => (*k, img.clone()),
        other => panic!("no page image: {:?}", other.map(|(k, _)| k)),
    };
    let px = |img: &bicraft_render::PageImage, x: f32, y: f32| {
        let (x, y) = ((x * img.scale) as usize, (y * img.scale) as usize);
        let i = (y * img.width as usize + x) * 4;
        img.rgba[i..i + 3].to_vec()
    };
    let (k0, plain) = shown(&h);
    assert!(k0.grid.is_none());
    view_button(&mut h, "Gridlines");
    steps(&mut h, 4);
    let (k1, grid) = shown(&h);
    let g = k1.grid().expect("the page is drawn with its grid");
    assert!(g.step.0 >= canvas::GRID && g.step.1 >= canvas::GRID, "{g:?}");
    let (_, card) = find(h.state(), "card");
    // multiples of every step the grid takes (8 doubled up to 1024 ... at most 128 here)
    let (empty, under) = (pos2(1024.0, 640.0), pos2(256.0, 256.0));
    assert!(card.contains(under) && !card.contains(empty));
    assert_ne!(
        px(&grid, empty.x, empty.y),
        px(&plain, empty.x, empty.y),
        "a dot on the page"
    );
    assert_eq!(
        px(&grid, under.x, under.y),
        px(&plain, under.x, under.y),
        "no dot over the card"
    );
    view_button(&mut h, "Gridlines");
    steps(&mut h, 4);
    assert!(shown(&h).0.grid.is_none());
}

/// View › Smart guides off: an edge 3 px from the chart's no longer snaps onto it and no guide
/// shows; on again, it snaps. A view setting: no command is sent.
#[test]
fn smart_guides_off_stops_snapping_to_other_visuals() {
    let mut h = app();
    let (card, r) = find(h.state(), "card");
    let (_, chart) = find(h.state(), "clusteredColumnChart");
    let z = zoom(h.state());
    let sent = h.state().sent.len();
    view_button(&mut h, "Smart guides");
    assert!(h.state().prefs.hide_guides);
    assert_eq!(h.state().sent.len(), sent, "no command: a view setting");
    let from = screen(h.state(), r.center());
    let to = from + vec2(37.0, chart.min.y - r.min.y + 3.0) * z;
    let mut guides = Vec::new();
    drag_with(&mut h, from, to, Modifiers::NONE, |h| {
        guides = h.state().canvas.gesture.as_ref().unwrap().guides.clone();
    });
    assert!(guides.is_empty(), "{guides:?}");
    let got = rect_of(h.state(), &card);
    assert!(
        (got.min.y - (chart.min.y + 3.0)).abs() < 1.0,
        "not snapped: {got:?}"
    );
    undo(&mut h);
    view_button(&mut h, "Smart guides");
    assert!(!h.state().prefs.hide_guides);
    drag(&mut h, from, to, Modifiers::NONE);
    assert_eq!(rect_of(h.state(), &card).min.y, chart.min.y, "snapped again");
}

/// The status bar's zoom: Zoom in and Zoom out step the scale by 10 %, Fit width fits the width;
/// View › Page view writes the page's display option (one command, one undo step) and the canvas
/// follows it again.
#[test]
fn the_status_bar_zoom_and_page_view_change_the_scale() {
    use crate::app::ViewZoom;
    let mut h = app();
    // a wide, low window: the page is held by its height, so fitting the width zooms in
    h.set_size(vec2(1800.0, 560.0));
    steps(&mut h, 6);
    let fitted = zoom(h.state());
    h.get_by_label("Zoom in on the page").click();
    steps(&mut h, 4);
    let Some(ViewZoom::Percent(p)) = h.state().view_zoom else {
        panic!("{:?}", h.state().view_zoom)
    };
    assert!(f32::from(p) > fitted * 100.0 && p % 10 == 0, "{p} vs {fitted}");
    assert!((zoom(h.state()) - f32::from(p) / 100.0).abs() < 1e-4);
    h.get_by_label("Zoom in on the page").click();
    steps(&mut h, 4);
    assert_eq!(h.state().view_zoom, Some(ViewZoom::Percent(p + 10)));
    h.get_by_label("Zoom out of the page").click();
    steps(&mut h, 4);
    assert_eq!(h.state().view_zoom, Some(ViewZoom::Percent(p)));
    assert!(h.query_by_label("Page zoom").is_some(), "the slider is labelled");
    h.get_by_label("Fit width").click();
    steps(&mut h, 4);
    let wide = zoom(h.state());
    assert!(wide > fitted, "{wide} vs {fitted}");
    // View › Page view › Fit to width: the page keeps it, the status bar follows the page
    h.get_by_label("Fit page").click();
    steps(&mut h, 4);
    view_button(&mut h, "Page view");
    h.get_by_label("Fit to width").click();
    steps(&mut h, 6);
    assert_eq!(count(&h, "page.set_display_option"), 1);
    let last = h.state().sent.last().unwrap().clone();
    assert_eq!(last.params["display_option"], "FitToWidth");
    assert_eq!(h.state().view_zoom, None);
    assert_eq!(h.state().page_zoom(), ViewZoom::FitWidth);
    assert!((zoom(h.state()) - wide).abs() < 1e-3);
    undo(&mut h);
    assert_eq!(h.state().page_zoom(), ViewZoom::FitPage);
    assert!((zoom(h.state()) - fitted).abs() < 1e-3);
}

/// A reader keeps the status bar's zoom; View › Page view (an edit of the file) is greyed.
#[test]
fn a_reader_zooms_but_cannot_set_the_page_view() {
    let mut h = app();
    h.state_mut().access = Some("reader".into());
    steps(&mut h, 2);
    h.get_by_label("Actual size").click();
    steps(&mut h, 4);
    assert_eq!(zoom(h.state()), 1.0);
    assert!(crate::ribbon::state(h.state(), "page.set_display_option").is_err());
}

/// The app over [`session`] with the chart given a background (the format painter's source).
fn painter_app() -> Harness<'static, App> {
    let make = || {
        let mut s = session(true);
        let page = s.report.as_ref().unwrap().pages[0].name.clone();
        let chart = s.report.as_ref().unwrap().pages[0]
            .visuals
            .iter()
            .find(|v| v.visual_type.as_deref() == Some("clusteredColumnChart"))
            .unwrap()
            .name
            .clone();
        s.execute(
            "visual.format",
            &json!({"page": page, "visual": chart, "background": {"show": true, "color": "#112233"}}),
        )
        .unwrap();
        s.dirty = false;
        s.report_dirty = false;
        s
    };
    let mut app = App::new("en-US", engine_shell(Caps::DESKTOP, make)).with_sync_render();
    app.open_path(Path::new("shop.pbip"));
    let mut h = harness(app, (1280.0, 800.0));
    open(&mut h);
    h.state_mut().ribbon_tab = RTab::Home;
    steps(&mut h, 2);
    h
}

fn background(app: &App, name: &str) -> Value {
    let Some(l) = &app.loaded else { return Value::Null };
    l.snap["report"]["pages"][0]["visuals"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|v| v["name"] == name)
        .map(|v| v["format"]["background"]["color"].clone())
        .unwrap_or(Value::Null)
}

/// Home › Format painter: on with the chart selected, the next click on the card copies the
/// chart's formatting onto it (one command, one undo step) and turns it off; Esc and a click on
/// the empty page turn it off; without a selection it stays off.
#[test]
fn the_format_painter_copies_onto_the_clicked_visual() {
    let mut h = painter_app();
    let (chart, rch) = find(h.state(), "clusteredColumnChart");
    let (card, rc) = find(h.state(), "card");
    assert_eq!(background(h.state(), &chart), json!("#112233"));
    assert_ne!(background(h.state(), &card), json!("#112233"));
    // nothing selected: the button does not turn on
    h.get_by_label("Format painter").click();
    steps(&mut h, 3);
    assert!(h.state().canvas.painter.is_none());
    // the chart selected: on, then a click on the card applies
    click_rep(&mut h, rch.center(), Modifiers::NONE);
    h.get_by_label("Format painter").click();
    steps(&mut h, 3);
    assert_eq!(
        h.state().canvas.painter.as_ref().map(|p| p.visual.clone()),
        Some(chart.clone())
    );
    click_rep(&mut h, rc.center(), Modifiers::NONE);
    steps(&mut h, 4);
    assert_eq!(count(&h, "visual.copy_format"), 1);
    let last = h.state().sent.last().unwrap().clone();
    assert_eq!(last.params["from"], json!(chart));
    assert_eq!(last.params["to"], json!([card]));
    assert!(h.state().canvas.painter.is_none(), "one click, then off");
    assert_eq!(background(h.state(), &card), json!("#112233"));
    assert_eq!(rect_of(h.state(), &card), rc, "the card keeps its place and size");
    assert_eq!(
        h.state().cmd.visual.as_ref().map(|v| v.visual.clone()),
        Some(card.clone())
    );
    undo(&mut h);
    assert_ne!(background(h.state(), &card), json!("#112233"));
    // Esc turns it off; so does a click on the empty page
    click_rep(&mut h, rch.center(), Modifiers::NONE);
    h.get_by_label("Format painter").click();
    steps(&mut h, 3);
    assert!(h.state().canvas.painter.is_some());
    key(&mut h, Key::Escape, Modifiers::NONE);
    assert!(h.state().canvas.painter.is_none());
    h.get_by_label("Format painter").click();
    steps(&mut h, 3);
    assert!(h.state().canvas.painter.is_some());
    click_rep(&mut h, pos2(1000.0, 650.0), Modifiers::NONE);
    assert!(h.state().canvas.painter.is_none());
    assert_eq!(count(&h, "visual.copy_format"), 1);
    // a second click on the button turns it off too
    click_rep(&mut h, rch.center(), Modifiers::NONE);
    h.get_by_label("Format painter").click();
    steps(&mut h, 3);
    h.get_by_label("Format painter").click();
    steps(&mut h, 3);
    assert!(h.state().canvas.painter.is_none());
}
