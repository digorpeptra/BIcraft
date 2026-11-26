//! The reader's drill on the canvas (egui_kittest, no GPU), over an engine session run in the
//! test: the header icons shown on hover and their clicks, drill mode, a matrix's +/- buttons,
//! right-click Drill through and the drillthrough page's Back button, a report-page tooltip
//! drawn under the pointer, and the same frames in the desktop's and the browser's shells.

use std::path::Path;

use bicraft_engine::Session;
use egui::{Event, Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use super::backstage_tests::engine_shell;
use super::{frame, harness};
use crate::App;
use crate::shell::Caps;

/// Sales(Category, Product, Amount) with a Products hierarchy; on Main a column chart on the
/// hierarchy (`chart`), a matrix on it with +/- icons (`matrix`) and a bar chart whose tooltip is
/// the Tip page (`bars`); Detail is a drillthrough page on Sales[Category] with a card and a
/// Back button.
pub(super) fn drill_session() -> Session {
    let mut s = Session::new();
    let sales = r#"#table({"Category", "Product", "Amount"}, {{"Wheels", "Bike", 150}, {"Wheels", "Car", 1000}, {"Toys", "Ball", 5}, {"Toys", "Kite", 7}})"#;
    let run = |s: &mut Session, id: &str, p: Value| s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
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
            json!({"table": "Sales", "name": "Product", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])", "format_string": "#,0"}),
        ),
        (
            "hierarchy.add",
            json!({"table": "Sales", "name": "Products", "levels": ["Category", "Product"]}),
        ),
        ("report.new", json!({})),
        ("page.add", json!({"display_name": "Detail"})),
        ("page.add", json!({"display_name": "Tip"})),
    ] {
        run(&mut s, id, p);
    }
    let main = s
        .report
        .as_ref()
        .map(|r| r.pages[0].name.clone())
        .unwrap_or_default();
    run(
        &mut s,
        "page.rename",
        json!({"page": main, "display_name": "Main"}),
    );
    let add = |s: &mut Session, page: &str, ty: &str, name: &str, (x, y, w, h): (i32, i32, i32, i32)| {
        let r = run(
            s,
            "visual.add",
            json!({"page": page, "visual_type": ty, "x": x, "y": y, "width": w, "height": h, "fields": []}),
        );
        // a fixed name, for the tests to find it
        let made = r["visual"].as_str().unwrap_or_default().to_string();
        if let Some(v) = s
            .report
            .as_mut()
            .and_then(|r| r.pages.iter_mut().find(|p| p.display_name == page))
            .and_then(|p| p.visuals.iter_mut().find(|v| v.name == made))
        {
            v.name = name.to_string();
        }
        name.to_string()
    };
    for (ty, name, at, role, value) in [
        (
            "clusteredColumnChart",
            "chart",
            (20, 20, 560, 300),
            "Category",
            "Y",
        ),
        ("pivotTable", "matrix", (620, 20, 400, 300), "Rows", "Values"),
    ] {
        let v = add(&mut s, "Main", ty, name, at);
        run(
            &mut s,
            "visual.bind_hierarchy",
            json!({"page": "Main", "visual": v, "role": role, "table": "Sales", "hierarchy": "Products"}),
        );
        run(
            &mut s,
            "visual.bind",
            json!({"page": "Main", "visual": v, "role": value, "field": "[Total]"}),
        );
    }
    let bars = add(&mut s, "Main", "clusteredBarChart", "bars", (20, 360, 560, 300));
    run(
        &mut s,
        "visual.bind",
        json!({"page": "Main", "visual": bars, "role": "Category", "field": "Sales[Category]"}),
    );
    run(
        &mut s,
        "visual.bind",
        json!({"page": "Main", "visual": bars, "role": "Y", "field": "[Total]"}),
    );
    // the matrix's +/- icons (rowHeaders.showExpandCollapseButtons)
    if let Some(v) = s
        .report
        .as_mut()
        .and_then(|r| r.pages.first_mut())
        .and_then(|p| p.visuals.iter_mut().find(|v| v.name == "matrix"))
    {
        v.visual_extra.insert(
            "objects".into(),
            json!({"rowHeaders": [{"properties": {"showExpandCollapseButtons": {"expr": {"Literal": {"Value": "true"}}}}}]}),
        );
    }
    run(
        &mut s,
        "page.set_type",
        json!({"page": "Detail", "type": "drillthrough", "fields": ["Sales[Category]"], "back_button": true}),
    );
    add(&mut s, "Detail", "card", "detail_card", (300, 200, 300, 150));
    run(
        &mut s,
        "visual.bind",
        json!({"page": "Detail", "visual": "detail_card", "role": "Values", "field": "[Total]"}),
    );
    run(&mut s, "page.set_type", json!({"page": "Tip", "type": "tooltip"}));
    add(&mut s, "Tip", "card", "tip_card", (10, 10, 300, 220));
    run(
        &mut s,
        "visual.bind",
        json!({"page": "Tip", "visual": "tip_card", "role": "Values", "field": "[Total]"}),
    );
    run(
        &mut s,
        "visual.tooltip_page",
        json!({"page": "Main", "visual": bars, "tooltip_page": "Tip"}),
    );
    s.dirty = false;
    s.report_dirty = false;
    s
}

fn app(caps: Caps) -> Harness<'static, App> {
    let mut app = App::new("en-US", engine_shell(caps, drill_session)).with_sync_render();
    app.prefs.panes.data = false;
    app.prefs.panes.build = false;
    app.prefs.panes.filters = false;
    app.open_path(Path::new("drill.pbip"));
    let mut h = harness(app, (1400.0, 900.0));
    for _ in 0..40 {
        h.step();
        if h.state().shown.is_some() {
            break;
        }
    }
    assert!(h.state().shown.is_some(), "the page was never drawn");
    h.step();
    h
}

fn snap(h: &Harness<'_, App>) -> Value {
    h.state()
        .loaded
        .as_ref()
        .map(|l| (*l.snap).clone())
        .unwrap_or(Value::Null)
}

fn visual(h: &Harness<'_, App>, name: &str) -> Value {
    snap(h)["report"]["pages"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|p| p["visuals"].as_array().cloned().unwrap_or_default())
        .find(|v| v["name"] == name)
        .unwrap_or(Value::Null)
}

/// The screen point of report position (`x`, `y`).
fn screen(h: &Harness<'_, App>, x: f32, y: f32) -> Pos2 {
    let p = h.state().placed.unwrap_or_else(|| panic!("no page placed"));
    Pos2::new(p.rect.min.x + x * p.zoom, p.rect.min.y + y * p.zoom)
}

/// The screen centre of the first hit of `visual` whose sent action matches `pick`.
fn hit(h: &Harness<'_, App>, visual: &str, pick: impl Fn(&Value) -> bool) -> Pos2 {
    let (_, img) = h.state().shown.as_ref().unwrap_or_else(|| panic!("no page"));
    let img = img.as_ref().unwrap_or_else(|e| panic!("{e}"));
    let found = img
        .hits
        .iter()
        .filter(|x| x.visual == visual)
        .find(|x| x.action_for(false).is_some_and(|a| pick(&a)))
        .unwrap_or_else(|| panic!("no such hit in {visual}"));
    let (x, y, w, hh) = found.shape.bounds();
    screen(h, x + w / 2.0, y + hh / 2.0)
}

fn press(h: &mut Harness<'_, App>, pos: Pos2, button: PointerButton) {
    h.hover_at(pos);
    h.step();
    for pressed in [true, false] {
        h.event_modifiers(
            Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers: Modifiers::NONE,
            },
            Modifiers::NONE,
        );
        h.step();
    }
    for _ in 0..4 {
        h.step();
    }
}

fn click(h: &mut Harness<'_, App>, pos: Pos2) {
    press(h, pos, PointerButton::Primary);
}

/// Hovers the middle of visual `name` (its header icons appear).
fn hover_visual(h: &mut Harness<'_, App>, name: &str) {
    let v = visual(h, name);
    let n = |k: &str| v[k].as_f64().unwrap_or(0.0) as f32;
    let p = screen(h, n("x") + n("width") / 2.0, n("y") + n("height") * 0.8);
    h.hover_at(p);
    h.step();
    h.step();
}

fn click_label(h: &mut Harness<'_, App>, label: &str) {
    let r = h.get_by_label(label).rect();
    click(h, r.center());
}

#[test]
fn header_icons_show_on_hover_and_drill() {
    let mut h = app(Caps::DESKTOP);
    assert!(
        h.query_by_label("Go to the next level").is_none(),
        "icons before any hover"
    );
    hover_visual(&mut h, "chart");
    assert!(h.query_by_label("Go to the next level").is_some());
    assert!(h.query_by_label("Expand all down one level").is_some());
    assert!(h.query_by_label("Turn drill mode on").is_some());
    // the matrix has its own icons; the bar chart, with one level, none
    hover_visual(&mut h, "bars");
    assert!(h.query_by_label("Go to the next level").is_none());

    hover_visual(&mut h, "chart");
    click_label(&mut h, "Go to the next level");
    assert_eq!(
        h.state().last_action.as_ref().map(|a| a["action"].clone()),
        Some(json!("drill_next"))
    );
    assert_eq!(visual(&h, "chart")["drill"]["top"], 1);
    assert_eq!(
        visual(&h, "chart")["data"]["rows"].as_array().map(Vec::len),
        Some(4)
    );
    hover_visual(&mut h, "chart");
    click_label(&mut h, "Drill up");
    assert_eq!(visual(&h, "chart")["drill"]["top"], 0);
    hover_visual(&mut h, "chart");
    click_label(&mut h, "Expand all down one level");
    assert_eq!(visual(&h, "chart")["drill"]["bottom"], 1);
    hover_visual(&mut h, "chart");
    click_label(&mut h, "Drill up");

    // drill mode: a click on a column drills into it
    hover_visual(&mut h, "chart");
    click_label(&mut h, "Turn drill mode on");
    assert_eq!(visual(&h, "chart")["drill"]["mode"], true);
    let p = hit(&h, "chart", |a| {
        a["action"] == "select" && a["point"][0]["v"] == "Toys"
    });
    click(&mut h, p);
    let d = visual(&h, "chart")["drill"].clone();
    assert_eq!(d["path"], json!([{"level": 0, "value": "Toys"}]), "{d}");
    assert_eq!(
        visual(&h, "chart")["data"]["rows"].as_array().map(Vec::len),
        Some(2)
    );
    // nothing was saved into the report
    assert_eq!(snap(&h)["dirty"]["report"], false);
}

#[test]
fn matrix_plus_minus_buttons_expand_and_collapse() {
    let mut h = app(Caps::DESKTOP);
    let p = hit(&h, "matrix", |a| {
        a["action"] == "matrix_expand" && a["path"] == json!(["Wheels"])
    });
    click(&mut h, p);
    assert_eq!(
        visual(&h, "matrix")["data"]["row_expansion"]["toggled"],
        json!([["Wheels"]])
    );
    let p = hit(&h, "matrix", |a| {
        a["action"] == "matrix_collapse" && a["path"] == json!(["Wheels"])
    });
    click(&mut h, p);
    assert_eq!(
        visual(&h, "matrix")["data"]["row_expansion"]["toggled"],
        json!([])
    );
}

/// The commands sent so far, by id.
fn sent_ids(h: &Harness<'_, App>) -> Vec<&'static str> {
    h.state().sent.iter().map(|c| c.id).collect()
}

fn settle(h: &mut Harness<'_, App>) {
    for _ in 0..60 {
        h.step();
        if h.state().calls_pending() == 0 {
            break;
        }
    }
    h.step();
}

#[test]
fn plus_minus_while_editing_writes_the_report_and_undoes() {
    // Desktop saves a matrix's expansion with the file: while editing, + is `visual.expand`
    let mut h = app(Caps::DESKTOP);
    assert_eq!(snap(&h)["dirty"]["report"], false);
    let p = hit(&h, "matrix", |a| {
        a["action"] == "matrix_expand" && a["path"] == json!(["Wheels"])
    });
    click(&mut h, p);
    settle(&mut h);
    let c = h
        .state()
        .sent
        .iter()
        .rev()
        .find(|c| c.id == "visual.expand")
        .cloned()
        .unwrap_or_else(|| panic!("no visual.expand in {:?}", sent_ids(&h)));
    assert_eq!(c.params["path"], json!(["Wheels"]), "{}", c.params);
    assert_eq!(snap(&h)["dirty"]["report"], true);
    assert_eq!(
        visual(&h, "matrix")["data"]["row_expansion"]["toggled"],
        json!([["Wheels"]])
    );
    // undo puts the + back, and the report is clean again
    h.state_mut().run(
        crate::actions::Call {
            id: "edit.undo",
            params: json!({}),
        },
        crate::app::Then::Nothing,
    );
    settle(&mut h);
    assert!(
        visual(&h, "matrix")["data"].get("row_expansion").is_none(),
        "{}",
        visual(&h, "matrix")["data"]
    );
    assert_eq!(snap(&h)["dirty"]["report"], false);
}

#[test]
fn right_click_expand_entire_level_and_the_header_icon_edit_the_matrix() {
    let mut h = app(Caps::DESKTOP);
    // the row header of Toys, right of its +/- button
    let at = {
        let (_, img) = h.state().shown.as_ref().unwrap_or_else(|| panic!("no page"));
        let img = img.as_ref().unwrap_or_else(|e| panic!("{e}"));
        let m = img
            .hits
            .iter()
            .find(|x| {
                x.visual == "matrix"
                    && matches!(&x.action, bicraft_render::hits::HitAction::Menu(m) if m["path"] == json!(["Toys"]))
            })
            .unwrap_or_else(|| panic!("no row header menu"));
        let (x, y, w, hh) = m.shape.bounds();
        screen(&h, x + w - 4.0, y + hh / 2.0)
    };
    press(&mut h, at, PointerButton::Secondary);
    // the submenu's button carries its arrow
    let r = h.get_by_label("Expand ⏵").rect();
    h.hover_at(r.center());
    for _ in 0..4 {
        h.step();
    }
    click_label(&mut h, "Entire level");
    settle(&mut h);
    let c = h
        .state()
        .sent
        .iter()
        .rev()
        .find(|c| c.id == "visual.expand")
        .cloned()
        .unwrap_or_else(|| panic!("no visual.expand in {:?}", sent_ids(&h)));
    assert_eq!(c.params["level"], json!(0), "{}", c.params);
    assert_eq!(
        visual(&h, "matrix")["data"]["row_expansion"],
        json!({"open": [true, false], "toggled": []})
    );
    // the header's Drill up on the matrix: back to the top level, in the report too
    hover_visual(&mut h, "matrix");
    click_label(&mut h, "Drill up");
    settle(&mut h);
    let c = h
        .state()
        .sent
        .last()
        .cloned()
        .unwrap_or_else(|| panic!("nothing sent"));
    assert_eq!((c.id, c.params["step"].clone()), ("visual.expand", json!("up")));
    // the header's Expand all down one level
    hover_visual(&mut h, "matrix");
    click_label(&mut h, "Expand all down one level");
    settle(&mut h);
    let c = h
        .state()
        .sent
        .last()
        .cloned()
        .unwrap_or_else(|| panic!("nothing sent"));
    assert_eq!(
        (c.id, c.params["step"].clone()),
        ("visual.expand", json!("expand_all"))
    );
    assert_eq!(
        visual(&h, "matrix")["data"]["row_expansion"],
        json!({"open": [true, false], "toggled": []})
    );
    assert_eq!(snap(&h)["dirty"]["report"], true);
}

#[test]
fn a_readers_plus_minus_stays_the_readers() {
    let mut h = app(Caps::DESKTOP);
    h.state_mut().access = Some("reader".into());
    h.step();
    let p = hit(&h, "matrix", |a| {
        a["action"] == "matrix_expand" && a["path"] == json!(["Wheels"])
    });
    click(&mut h, p);
    settle(&mut h);
    assert!(!sent_ids(&h).contains(&"visual.expand"), "{:?}", sent_ids(&h));
    assert_eq!(
        h.state().last_action.as_ref().map(|a| a["action"].clone()),
        Some(json!("matrix_expand"))
    );
    assert_eq!(
        visual(&h, "matrix")["data"]["row_expansion"]["toggled"],
        json!([["Wheels"]])
    );
    assert_eq!(snap(&h)["dirty"]["report"], false);
}

#[test]
fn plus_minus_buttons_have_accessible_labels_in_english_and_french() {
    let mut h = app(Caps::DESKTOP);
    let b = h.get_by_label("Expand Wheels");
    assert_eq!(b.accesskit_node().role(), egui::accesskit::Role::Button);
    assert!(h.query_by_label("Expand Toys").is_some());
    b.click();
    settle(&mut h);
    assert_eq!(
        visual(&h, "matrix")["data"]["row_expansion"]["toggled"],
        json!([["Wheels"]])
    );
    // expanded: its button now collapses
    assert!(h.query_by_label("Collapse Wheels").is_some());
    assert!(h.query_by_label("Expand Wheels").is_none());
    h.get_by_label("Collapse Wheels").click();
    settle(&mut h);
    assert_eq!(
        visual(&h, "matrix")["data"]["row_expansion"]["toggled"],
        json!([])
    );
    // French
    let mut fr = App::new("fr-FR", engine_shell(Caps::DESKTOP, drill_session)).with_sync_render();
    fr.prefs.panes.data = false;
    fr.prefs.panes.build = false;
    fr.prefs.panes.filters = false;
    fr.open_path(Path::new("drill.pbip"));
    let mut f = harness(fr, (1400.0, 900.0));
    for _ in 0..40 {
        f.step();
        if f.state().shown.is_some() {
            break;
        }
    }
    f.step();
    assert!(f.query_by_label("Développer Wheels").is_some());
}

#[test]
fn right_click_drill_through_opens_the_page_and_back_returns() {
    let mut h = app(Caps::DESKTOP);
    let p = hit(&h, "chart", |a| {
        a["action"] == "select" && a["point"][0]["v"] == "Wheels"
    });
    press(&mut h, p, PointerButton::Secondary);
    // the drill entries and the Drill through submenu
    assert!(h.query_by_label("Drill down").is_some());
    assert!(h.query_by_label("Show next level").is_some());
    let dt = h
        .query_by(|n| n.label().is_some_and(|l| l.starts_with("Drill through")))
        .unwrap_or_else(|| panic!("no Drill through entry"))
        .rect();
    h.hover_at(dt.center());
    for _ in 0..4 {
        h.step();
    }
    // the submenu's entry (a page tab has the same name, lower down)
    let r = h
        .get_all_by_label("Detail")
        .map(|n| n.rect())
        .min_by(|a, b| a.center().y.total_cmp(&b.center().y))
        .unwrap_or_else(|| panic!("no Detail entry"));
    click(&mut h, r.center());
    let s = snap(&h);
    let detail = s["report"]["pages"][1]["name"].clone();
    assert_eq!(s["report"]["active_page"], detail);
    assert_eq!(
        visual(&h, "detail_card")["data"]["rows"][0][0].as_f64(),
        Some(1150.0)
    );
    // the page's Back button
    let back = visual(&h, "detail_card");
    assert!(back.is_object());
    let names: Vec<String> = s["report"]["pages"][1]["visuals"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|v| v["interaction"]["button"]["kind"] == "Back")
        .filter_map(|v| v["name"].as_str().map(str::to_string))
        .collect();
    let p = hit(&h, &names[0], |a| a["action"] == "button");
    // in editing a button acts on Ctrl+click (M11.20, as Desktop's editing view)
    h.hover_at(p);
    h.step();
    for pressed in [true, false] {
        h.event_modifiers(
            Event::PointerButton {
                pos: p,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::COMMAND,
            },
            Modifiers::COMMAND,
        );
        h.step();
    }
    for _ in 0..4 {
        h.step();
    }
    let s = snap(&h);
    assert_eq!(s["report"]["active_page"], s["report"]["pages"][0]["name"]);
}

#[test]
fn a_report_page_tooltip_appears_on_hover() {
    let mut h = app(Caps::DESKTOP);
    let p = hit(&h, "bars", |a| {
        a["action"] == "select" && a["point"][0]["v"] == "Toys"
    });
    h.hover_at(p);
    for _ in 0..8 {
        h.step();
    }
    assert_eq!(snap(&h)["interaction"]["tooltip"]["visual"], "bars");
    assert!(crate::drill::tooltip_drawn(h.state()));
    assert!(h.query_by_label("Report page tooltip").is_some());
    // the tooltip page's card under the hovered point: Toys = 12
    assert_eq!(visual(&h, "tip_card")["data"]["rows"][0][0].as_f64(), Some(12.0));
    // away from the point: the tooltip goes
    h.hover_at(screen(&h, 1200.0, 700.0));
    for _ in 0..4 {
        h.step();
    }
    assert_eq!(snap(&h)["interaction"]["tooltip"], Value::Null);
}

#[test]
fn hover_icons_draw_the_same_frames_on_desktop_and_in_the_browser() {
    let mut a = app(Caps::DESKTOP);
    let mut b = app(Caps::BROWSER);
    for h in [&mut a, &mut b] {
        hover_visual(h, "chart");
        h.step();
    }
    assert!(a.query_by_label("Drill up").is_some());
    let (fa, fb) = (frame(&a), frame(&b));
    assert!(fa.0 == fb.0, "the meshes differ");
    assert!(fa.2 == fb.2, "the pages differ");
}
