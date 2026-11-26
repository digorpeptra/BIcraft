//! Rich content in the Report view (`rich.rs`, ROADMAP M11.20) on an engine session: Insert ›
//! Text box, typing, bold, "+ Value" and the commit; Insert › Image through the shell's dialog
//! (a stub) and through the workspace's files (the browser); Insert › Buttons, the action editor
//! and Ctrl+click navigation; the page's background and size; the same frames in both shells.

use std::path::Path;

use bicraft_engine::Session;
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::backstage_tests::{engine_shell, steps};
use super::frame;
use crate::App;
use crate::registry::Tab as RTab;
use crate::ribbon::VisualSel;
use crate::shell::Caps;
use crate::source::WsFile;

/// A 1×1 PNG.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00,
    0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00,
    0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0xDA, 0x63, 0x64, 0xF8, 0xCF, 0x50, 0x0F, 0x00, 0x03,
    0x86, 0x01, 0x80, 0x5A, 0x34, 0x7D, 0x6B, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42,
    0x60, 0x82,
];

/// Sales with [Total]; pages P1 and P2.
fn session() -> Session {
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
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])", "format_string": "#,0"}),
        ),
        ("report.new", json!({})),
        ("page.add", json!({"display_name": "P2"})),
    ] {
        s.execute(id, &p).unwrap();
    }
    let first = s.report.as_ref().unwrap().pages[0].name.clone();
    s.execute("page.rename", &json!({"page": first, "display_name": "P1"}))
        .unwrap();
    s.dirty = false;
    s.report_dirty = false;
    s
}

fn open_with(caps: Caps, setup: impl FnOnce(&mut App)) -> Harness<'static, App> {
    let mut app = App::new("en-US", engine_shell(caps, session)).with_sync_render();
    setup(&mut app);
    app.open_path(Path::new("shop.pbip"));
    let mut h = Harness::builder()
        .with_size(egui::vec2(1400.0, 860.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(
            |ui, app: &mut App| {
                app.logic(ui.ctx());
                app.ui(ui);
            },
            app,
        );
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

fn open() -> Harness<'static, App> {
    open_with(Caps::DESKTOP, |_| {})
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

fn press(h: &mut Harness<'_, App>, key: Key, modifiers: Modifiers) {
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
    steps(h, 3);
}

fn type_text(h: &mut Harness<'_, App>, s: &str) {
    h.event(Event::Text(s.into()));
    steps(h, 2);
}

/// The ribbon tab `caption` (the topmost node of that name).
fn tab(h: &mut Harness<'_, App>, caption: &str) {
    h.get_all_by_label(caption)
        .min_by(|a, b| a.rect().center().y.total_cmp(&b.rect().center().y))
        .unwrap()
        .click();
    steps(h, 3);
}

/// The snapshot's visuals of page `i`.
fn visuals(h: &Harness<'_, App>, i: usize) -> Vec<Value> {
    let snap = h.state().loaded.as_ref().unwrap().snap.clone();
    snap["report"]["pages"][i]["visuals"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn find(h: &Harness<'_, App>, ty: &str) -> Option<Value> {
    visuals(h, 0).into_iter().find(|v| v["type"] == ty)
}

fn last_sent(h: &Harness<'_, App>) -> (&'static str, Value) {
    let c = h.state().sent.last().unwrap();
    (c.id, c.params.clone())
}

/// A click at `pos` with `modifiers`.
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

fn on_canvas(h: &Harness<'_, App>, v: &Value) -> Pos2 {
    let placed = h.state().placed.unwrap();
    let n = |k: &str| v[k].as_f64().unwrap() as f32;
    placed.rect.min + egui::vec2(n("x") + n("width") / 2.0, n("y") + n("height") / 2.0) * placed.zoom
}

#[test]
fn insert_a_text_box_type_bold_and_a_value() {
    let mut h = open();
    tab(&mut h, "Insert");
    assert_eq!(h.state().ribbon_tab, RTab::Insert);
    h.get_by_label("Text box").click();
    steps(&mut h, 3);
    until(&mut h, "the new text box", |a| {
        a.cmd.rich.editor.is_some()
            && a.loaded
                .as_ref()
                .is_some_and(|l| l.snap.to_string().contains("\"textbox\""))
    });
    steps(&mut h, 4);
    assert_eq!(last_sent(&h).0, "textbox.add");
    type_text(&mut h, "Hello ");
    press(&mut h, Key::B, Modifiers::COMMAND);
    type_text(&mut h, "world");
    assert!(h.state().cmd.rich.editor.as_ref().unwrap().typing.bold);
    // + Value: the measure list, then [Total]
    h.get_by_label("+ Value").click();
    steps(&mut h, 3);
    h.get_by_label("[Total]").click();
    steps(&mut h, 3);
    assert_eq!(
        h.state().cmd.rich.editor.as_ref().unwrap().text,
        "Hello world«[Total]»"
    );
    h.get_by_label("✔ Done").click();
    steps(&mut h, 3);
    until(&mut h, "the commit", |a| {
        a.sent.last().is_some_and(|c| c.id == "textbox.set") && a.calls_pending() == 0
    });
    assert!(h.state().cmd.rich.editor.is_none());
    let (_, p) = last_sent(&h);
    assert_eq!(
        p["paragraphs"],
        json!([{"runs": [{"text": "Hello "}, {"text": "world", "bold": true}, {"value": "[Total]", "bold": true}]}])
    );
    until(&mut h, "the snapshot's text", |a| {
        a.loaded
            .as_ref()
            .is_some_and(|l| l.snap.to_string().contains("world"))
    });
    let tb = find(&h, "textbox").unwrap();
    let runs = &tb["text"]["paragraphs"][0]["runs"];
    assert_eq!(runs[1]["bold"], true, "{tb}");
    assert_eq!(runs[2]["field"], "[Total]");
    assert_eq!(runs[2]["text"], "125", "the value is evaluated: {tb}");
    // a double click opens it again with its runs; Esc closes without a command
    let at = on_canvas(&h, &tb);
    let n = h.state().sent.len();
    h.state_mut().cmd.visual = Some(VisualSel {
        page: h.state().page_names()[0].0.clone(),
        visual: tb["name"].as_str().unwrap().into(),
    });
    // a second away from the last click (a third click within 0.6 s would count as a triple click)
    steps(&mut h, 70);
    h.hover_at(at);
    h.step();
    for _ in 0..2 {
        for pressed in [true, false] {
            h.event(Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
            h.step();
        }
    }
    steps(&mut h, 3);
    let ed = h.state().cmd.rich.editor.clone().unwrap_or_else(|| {
        panic!(
            "the editor opens on a double click (selected {:?}, error {:?}, at {at:?})",
            h.state().cmd.visual,
            h.state().error
        )
    });
    assert_eq!(ed.text, "Hello world«[Total]»");
    assert!(ed.doc.styles[7].bold && !ed.doc.styles[0].bold);
    press(&mut h, Key::Escape, Modifiers::NONE);
    assert!(h.state().cmd.rich.editor.is_none());
    assert_eq!(h.state().sent.len(), n, "Esc sends nothing");
}

#[test]
fn insert_an_image_through_the_dialog_and_the_workspace() {
    // the desktop: the shell's dialog gives the bytes (a stub here)
    let mut h = open_with(Caps::DESKTOP, |app| {
        app.shell.pick_image = Some(Box::new(|_| Some(Ok(("logo.png".to_string(), PNG.to_vec())))));
    });
    tab(&mut h, "Insert");
    h.get_by_label("Image").click();
    steps(&mut h, 3);
    until(&mut h, "the image", |a| {
        a.loaded
            .as_ref()
            .is_some_and(|l| l.snap.to_string().contains("\"image\""))
    });
    let (id, p) = last_sent(&h);
    assert_eq!(id, "image.add");
    assert_eq!(p["name"], "logo.png");
    assert_eq!(p["data"], crate::rich::base64(PNG));
    let img = find(&h, "image").unwrap();
    assert_eq!(
        h.state().cmd.visual.as_ref().unwrap().visual,
        img["name"].as_str().unwrap()
    );
    // a file past the cap is refused before anything is sent
    let n = h.state().sent.len();
    h.state_mut().shell.pick_image = Some(Box::new(|_| {
        Some(Ok(("big.png".to_string(), vec![0u8; (16 << 20) + 1])))
    }));
    crate::rich::pick_image(h.state_mut(), crate::rich::ImageTarget::Insert);
    assert_eq!(h.state().sent.len(), n);
    assert!(h.state().error.as_deref().is_some_and(|e| e.contains("limit")));
    // the browser: the workspace's image files, then image.add {path}
    let dir = std::env::temp_dir().join(format!("bicraft-ui-rich-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("brand.png");
    std::fs::write(&file, PNG).unwrap();
    let path = file.display().to_string();
    let mut b = open_with(Caps::BROWSER, |app| app.shell.workspace = Some("ws".into()));
    b.state_mut().files = Some(Ok(vec![
        WsFile {
            path: path.clone(),
            size: Some(70),
            modified: String::new(),
        },
        WsFile {
            path: "notes.txt".into(),
            size: Some(5),
            modified: String::new(),
        },
    ]));
    tab(&mut b, "Insert");
    b.get_by_label("Image").click();
    steps(&mut b, 3);
    assert!(
        b.query_by_label("notes.txt").is_none(),
        "only image files are listed"
    );
    b.get_by_label(path.as_str()).click();
    steps(&mut b, 3);
    until(&mut b, "the workspace image", |a| {
        a.loaded
            .as_ref()
            .is_some_and(|l| l.snap.to_string().contains("\"image\""))
    });
    let (id, p) = last_sent(&b);
    assert_eq!((id, p["path"].as_str()), ("image.add", Some(path.as_str())));
    assert_eq!(p["name"], "brand.png");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn buttons_their_action_and_ctrl_click_navigation() {
    let mut h = open();
    tab(&mut h, "Insert");
    h.get_by_label("Buttons").click();
    steps(&mut h, 3);
    h.get_by_label("Blank").click();
    steps(&mut h, 3);
    until(&mut h, "the button", |a| {
        a.loaded
            .as_ref()
            .is_some_and(|l| l.snap.to_string().contains("\"actionButton\""))
    });
    assert_eq!(
        last_sent(&h),
        (
            "button.add",
            json!({"page": h.state().page_names()[0].0, "kind": "blank"})
        )
    );
    steps(&mut h, 3);
    // the Visual tab's Action button opens the editor
    tab(&mut h, "Visual");
    h.get_by_label("Action").click();
    steps(&mut h, 3);
    assert!(h.state().cmd.rich.action.is_some());
    let p2 = h.state().page_names()[1].0.clone();
    {
        let f = h.state_mut().cmd.rich.action.as_mut().unwrap();
        f.kind = "page_navigation".into();
        f.destination = p2.clone();
    }
    steps(&mut h, 3);
    h.get_by_label("Apply action").click();
    steps(&mut h, 3);
    until(&mut h, "the action", |a| {
        a.sent.last().is_some_and(|c| c.id == "visual.set_action") && a.calls_pending() == 0
    });
    assert_eq!(
        last_sent(&h).1,
        json!({"page": h.state().page_names()[0].0, "visual": h.state().cmd.visual.as_ref().unwrap().visual,
               "type": "page_navigation", "destination": p2})
    );
    // the style of the hover state
    {
        let f = h.state_mut().cmd.rich.action.as_mut().unwrap();
        f.state = "hover".into();
        f.text = "Next".into();
    }
    steps(&mut h, 2);
    h.get_by_label("Apply style").click();
    steps(&mut h, 3);
    let (id, p) = last_sent(&h);
    assert_eq!(id, "visual.format");
    assert_eq!(
        p["objects"][1],
        json!({"object": "text", "property": "text", "value": "Next", "state": "hover"})
    );
    until(&mut h, "the button's action in the snapshot", |a| {
        a.loaded
            .as_ref()
            .is_some_and(|l| l.snap.to_string().contains("PageNavigation"))
    });
    h.state_mut().cmd.rich.action = None;
    steps(&mut h, 3);
    // a plain click selects it; Ctrl+click navigates (Desktop's editing view)
    let bt = find(&h, "actionButton").unwrap();
    let at = on_canvas(&h, &bt);
    let before = h.state().active_page();
    click_with(&mut h, at, Modifiers::NONE);
    steps(&mut h, 3);
    assert_eq!(h.state().active_page(), before, "a plain click does not navigate");
    assert_eq!(
        h.state().cmd.visual.as_ref().unwrap().visual,
        bt["name"].as_str().unwrap()
    );
    click_with(&mut h, at, Modifiers::COMMAND);
    until(&mut h, "the navigation", |a| a.active_page() == 1);
    // a reader's plain click navigates
    let mut r = open();
    r.state_mut().access = Some("reader".into());
    let a = json!({"action": "button", "page": "x", "visual": "y"});
    assert!(!crate::rich::click_needs_ctrl(r.state(), &a, false));
    r.state_mut().access = None;
    assert!(crate::rich::click_needs_ctrl(r.state(), &a, false));
    assert!(!crate::rich::click_needs_ctrl(r.state(), &a, true));
}

#[test]
fn page_background_and_size() {
    let mut h = open();
    // the Report tab is the one shown (a click on it would collapse the ribbon)
    h.state_mut().select_ribbon_tab(RTab::Report);
    steps(&mut h, 3);
    h.get_by_label("Page format").click();
    steps(&mut h, 3);
    assert!(h.state().cmd.rich.page_format.is_some(), "the window opens");
    {
        let f = h.state_mut().cmd.rich.page_format.as_mut().unwrap();
        f.bg = Some([0x20, 0x40, 0x80]);
        f.size = "4:3".into();
        f.valign = "middle".into();
    }
    steps(&mut h, 2);
    h.get_by_label("Apply page format").click();
    steps(&mut h, 3);
    let (id, p) = last_sent(&h);
    assert_eq!(id, "page.format");
    assert_eq!(
        p,
        json!({"page": h.state().page_names()[0].0, "background_color": "#204080", "size": "4:3", "vertical_alignment": "middle"})
    );
    until(&mut h, "the new page size", |a| {
        a.loaded
            .as_ref()
            .is_some_and(|l| l.snap["report"]["pages"][0]["width"] == 960.0)
    });
    // the page is drawn in the new colour (a corner pixel, no visual there)
    until(&mut h, "the new page", |a| {
        a.shown.as_ref().is_some_and(|(_, r)| {
            r.as_ref()
                .is_ok_and(|img| img.rgba.get(0..3) == Some(&[0x20, 0x40, 0x80][..]))
        })
    });
    // its background image through the dialog
    h.state_mut().shell.pick_image = Some(Box::new(|_| Some(Ok(("bg.png".to_string(), PNG.to_vec())))));
    let page = h.state().page_names()[0].0.clone();
    crate::rich::pick_image(
        h.state_mut(),
        crate::rich::ImageTarget::Page {
            page: page.clone(),
            background: true,
        },
    );
    steps(&mut h, 3);
    let (id, p) = last_sent(&h);
    assert_eq!((id, p["page"].as_str()), ("page.format", Some(page.as_str())));
    assert_eq!(p["background_image"], crate::rich::base64(PNG));
}

/// The editor's toolbar and the windows draw the same frames in the desktop and the browser.
#[test]
fn rich_content_draws_the_same_frames_in_both_shells() {
    let mut desktop = open();
    let mut browser = open_with(Caps::BROWSER, |_| {});
    for h in [&mut desktop, &mut browser] {
        tab(h, "Insert");
        h.get_by_label("Text box").click();
        steps(h, 3);
        until(h, "the editor", |a| {
            a.cmd.rich.editor.is_some()
                && a.loaded
                    .as_ref()
                    .is_some_and(|l| l.snap.to_string().contains("\"textbox\""))
        });
        steps(h, 4);
        type_text(h, "Same frames");
        steps(h, 4);
    }
    let (a, b) = (frame(&desktop), frame(&browser));
    assert!(a.0 == b.0, "the editor's meshes differ");
    assert!(a.1 == b.1, "the font atlases differ");
    for h in [&mut desktop, &mut browser] {
        press(h, Key::Escape, Modifiers::NONE);
        tab(h, "Report");
        h.get_by_label("Page format").click();
        steps(h, 4);
    }
    let (a, b) = (frame(&desktop), frame(&browser));
    assert!(a.0 == b.0, "the page format window's meshes differ");
}

/// Damaged snapshots and hostile states never break the editor and the windows.
#[test]
fn damaged_snapshots_never_break_rich_content() {
    let mut h = open();
    let sel = VisualSel {
        page: h.state().page_names()[0].0.clone(),
        visual: "nope".into(),
    };
    h.state_mut().cmd.visual = Some(sel.clone());
    crate::rich::open_editor(h.state_mut(), &sel);
    crate::rich::intercept(h.state_mut(), "visual.set_action", &json!({}));
    crate::rich::intercept(h.state_mut(), "page.format", &json!({"page": "nope"}));
    crate::rich::intercept(h.state_mut(), "image.set", &json!({}));
    h.state_mut().cmd.rich.buttons = true;
    steps(&mut h, 3);
    for snap in [
        json!({"report": {"pages": [{"name": sel.page, "visuals": [{"name": "nope", "type": "textbox", "x": "a",
               "text": {"paragraphs": [{"runs": [{"text": 5, "font_size": -3, "color": "red"}]}]}}]}]}}),
        json!({"report": {"pages": 7}}),
        Value::Null,
    ] {
        h.state_mut().loaded.as_mut().unwrap().snap = std::sync::Arc::new(snap);
        steps(&mut h, 3);
        if let Some(sel) = h.state().cmd.visual.clone() {
            crate::rich::open_editor(h.state_mut(), &sel);
        }
        steps(&mut h, 3);
    }
    crate::rich::commit(h.state_mut());
    steps(&mut h, 3);
}

/// The text box editor keeps the colour the page shows behind the box (white text on a dark page
/// stays readable while it is edited), not a white sheet.
#[test]
fn the_text_editor_keeps_the_background_behind_the_box() {
    let mut h = open_with(Caps::DESKTOP, |_| {});
    let page = h.state().page_names()[0].0.clone();
    h.state_mut().run(
        crate::actions::Call {
            id: "page.format",
            params: json!({"page": page, "background_color": "#202020", "background_transparency": 0}),
        },
        crate::app::Then::Nothing,
    );
    until(&mut h, "the dark page", |a| {
        a.calls_pending() == 0
            && a.placed.is_some_and(|p| {
                a.backdrop_at(Rect::from_center_size(p.rect.center(), egui::vec2(60.0, 30.0)))
                    .is_some_and(|c| c.r() < 60 && c.g() < 60 && c.b() < 60)
            })
    });
    let p = h.state().placed.unwrap();
    let c = h
        .state()
        .backdrop_at(Rect::from_center_size(p.rect.center(), egui::vec2(60.0, 30.0)))
        .unwrap();
    assert!((0x18..=0x28).contains(&c.r()), "{c:?}");
    // outside the page: nothing to sample
    assert!(
        h.state()
            .backdrop_at(Rect::from_min_size(
                Pos2::new(-500.0, -500.0),
                egui::vec2(10.0, 10.0)
            ))
            .is_none()
    );
}

/// Insert › Shapes: the ribbon button's menu inserts the shape picked (`shape.add`) and selects
/// it; the Format visual tab's Shape card changes its tile shape and the Fill card its colour,
/// each one `visual.format`, and the page draws it.
#[test]
fn insert_shapes_menu_and_the_shape_cards() {
    use egui::accesskit::Role;
    let mut h = open();
    tab(&mut h, "Insert");
    h.get_by_label("Shapes").click();
    steps(&mut h, 3);
    h.get_by_label("Oval").click();
    steps(&mut h, 3);
    until(&mut h, "the shape", |a| {
        a.loaded
            .as_ref()
            .is_some_and(|l| l.snap.to_string().contains("\"tile\":\"oval\""))
    });
    let page = h.state().page_names()[0].0.clone();
    assert_eq!(
        last_sent(&h),
        ("shape.add", json!({"page": page, "shape": "oval"}))
    );
    let shape = find(&h, "shape").unwrap();
    assert_eq!(
        h.state().cmd.visual.as_ref().unwrap().visual,
        shape["name"].as_str().unwrap(),
        "the new shape is selected"
    );
    // the Format visual tab: the shape's own cards
    h.get_by_role_and_label(Role::Button, "Format visual").click();
    steps(&mut h, 3);
    h.get_by_label("Visual cards").click();
    steps(&mut h, 6);
    for card in ["Shape", "Rotation", "Fill", "Outline"] {
        h.get_by_role_and_label(Role::Button, card);
    }
    h.get_by_role_and_label(Role::Button, "Shape").click();
    steps(&mut h, 3);
    h.get_by_label("Shape Shape").click();
    steps(&mut h, 3);
    // an item of the list's first, visible rows
    h.get_by_label("Rounded rectangle").click();
    until(&mut h, "the rounded rectangle", |a| {
        a.calls_pending() == 0
            && a.loaded
                .as_ref()
                .is_some_and(|l| l.snap.to_string().contains("\"tile\":\"rectangleRounded\""))
    });
    let p = h
        .state()
        .sent
        .iter()
        .rev()
        .find(|c| c.id == "visual.format")
        .map(|c| c.params.clone())
        .expect("a visual.format");
    assert_eq!(
        p["objects"],
        json!([{"object": "shape", "property": "tileShape", "value": "rectangleRounded"}])
    );
    // Fill › Colour: a swatch of the popup; the page draws the shape in it
    h.get_by_role_and_label(Role::Button, "Fill").click();
    steps(&mut h, 3);
    h.get_by_label("Fill Colour").click();
    steps(&mut h, 3);
    h.get_by_label("Fill Colour #252423").click();
    until(&mut h, "the fill", |a| {
        a.calls_pending() == 0
            && a.loaded
                .as_ref()
                .is_some_and(|l| l.snap.to_string().contains("\"fill\":\"#252423\""))
    });
    let at = on_canvas(&h, &shape);
    until(&mut h, "the dark shape on the page", |a| {
        a.backdrop_at(Rect::from_center_size(at, egui::vec2(6.0, 6.0)))
            .is_some_and(|c| c.r() < 60 && c.g() < 60 && c.b() < 60)
    });
    assert!(h.state().error.is_none(), "{:?}", h.state().error);
}
