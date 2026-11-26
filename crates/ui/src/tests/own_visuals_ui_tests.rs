//! BIcraft's own visuals in the app (docs/gaps.md G29): a click on a mark of the BIcraft bullet
//! selects its row and filters the table beside it, Ctrl+click adds, its hover shows the mark's
//! tooltip; from the keyboard (`point_keys.rs`) Ctrl+Right enters its data points, the arrows
//! move without moving the visual, Enter selects, Ctrl+Space adds, Escape clears.

use std::path::Path;

use bicraft_engine::Session;
use egui::{Key, Modifiers, Pos2};
use egui_kittest::Harness;
use serde_json::{Value, json};

use super::backstage_tests::{engine_shell, steps};
use super::canvas_ui_tests::{click_with, key};
use super::harness;
use super::panes_ui_tests::{select, visual_named};
use crate::App;
use crate::shell::Caps;

fn shop() -> Session {
    let mut s = Session::new();
    let m = "#table(type table [K = text, V = Int64.Type], {{\"a\", 10}, {\"b\", 20}, {\"c\", 35}})";
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        ("table.import", json!({"name": "T", "m": m})),
        (
            "measure.add",
            json!({"table": "T", "name": "Total", "expression": "SUM(T[V])"}),
        ),
        (
            "measure.add",
            json!({"table": "T", "name": "Target", "expression": "30"}),
        ),
        ("report.new", json!({})),
    ] {
        s.execute(id, &p).unwrap();
    }
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    for (ty, x, fields) in [
        (
            "bicraftBullet",
            20,
            json!([{"role": "category", "field": "T[K]"}, {"role": "value", "field": "[Total]"}, {"role": "target", "field": "[Target]"}]),
        ),
        (
            "tableEx",
            560,
            json!([{"role": "Values", "field": "T[K]"}, {"role": "Values", "field": "[Total]"}]),
        ),
    ] {
        s.execute(
            "visual.add",
            &json!({"page": page, "visual_type": ty, "x": x, "y": 20, "width": 500, "height": 300, "fields": fields}),
        )
        .unwrap();
    }
    s.dirty = false;
    s.report_dirty = false;
    s
}

fn open() -> Harness<'static, App> {
    let mut app = App::new("en-US", engine_shell(Caps::DESKTOP, shop)).with_sync_render();
    app.open_path(Path::new("shop.pbip"));
    let mut h = harness(app, (1400.0, 900.0));
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

fn visual(h: &Harness<'_, App>, name: &str) -> Value {
    h.state().loaded.as_ref().unwrap().snap["report"]["pages"][0]["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == name)
        .cloned()
        .unwrap()
}

fn table_rows(h: &Harness<'_, App>) -> usize {
    let t = visual_named(h, "tableEx").visual;
    visual(h, &t)["data"]["rows"].as_array().map_or(0, Vec::len)
}

fn selected(h: &Harness<'_, App>, v: &str) -> usize {
    visual(h, v)["interaction"]["selected"]["points"]
        .as_array()
        .map_or(0, Vec::len)
}

/// The screen centre of the bullet's `i`-th data point.
fn point_at(h: &Harness<'_, App>, v: &str, i: usize) -> Pos2 {
    let t = crate::point_keys::targets(h.state(), v);
    let (x, y, w, hh) = t[i].bounds;
    let pl = h.state().placed.unwrap();
    pl.rect.min + egui::vec2(x + w / 2.0, y + hh / 2.0) * pl.zoom
}

#[test]
fn a_click_on_a_bicraft_visual_filters_the_page_and_ctrl_adds() {
    let mut h = open();
    let b = bullet(&h).visual;
    assert_eq!(table_rows(&h), 3);
    let targets = crate::point_keys::targets(h.state(), &b);
    assert_eq!(targets.len(), 3, "one data point per row: {targets:?}");
    assert!(
        targets[0].tip.iter().any(|(k, v)| k == "K" && v == "a"),
        "{:?}",
        targets[0].tip
    );
    let p = point_at(&h, &b, 0);
    click_with(&mut h, p, Modifiers::NONE);
    steps(&mut h, 4);
    assert_eq!(selected(&h, &b), 1);
    assert_eq!(table_rows(&h), 1, "the table keeps row a");
    let p = point_at(&h, &b, 1);
    click_with(&mut h, p, Modifiers::COMMAND);
    steps(&mut h, 4);
    assert_eq!(selected(&h, &b), 2, "Ctrl adds");
    assert_eq!(table_rows(&h), 2);
    // the same plain click on a, then again: cleared
    let p = point_at(&h, &b, 0);
    click_with(&mut h, p, Modifiers::NONE);
    steps(&mut h, 4);
    assert_eq!(selected(&h, &b), 1);
    click_with(&mut h, p, Modifiers::NONE);
    steps(&mut h, 4);
    assert_eq!(selected(&h, &b), 0);
    assert_eq!(table_rows(&h), 3);
}

#[test]
fn the_keyboard_walks_and_selects_the_data_points() {
    let mut h = open();
    let sel = bullet(&h);
    let b = sel.visual.clone();
    select(&mut h, &sel);
    let before = visual(&h, &b)["x"].clone();
    key(&mut h, Key::ArrowRight, Modifiers::COMMAND);
    steps(&mut h, 2);
    assert_eq!(crate::point_keys::focus(&h.ctx), Some((b.clone(), 0)));
    key(&mut h, Key::ArrowRight, Modifiers::NONE);
    steps(&mut h, 2);
    assert_eq!(crate::point_keys::focus(&h.ctx), Some((b.clone(), 1)));
    assert_eq!(visual(&h, &b)["x"], before, "the arrows do not move the visual");
    key(&mut h, Key::Enter, Modifiers::NONE);
    steps(&mut h, 4);
    assert_eq!(selected(&h, &b), 1);
    assert_eq!(table_rows(&h), 1, "row b only");
    key(&mut h, Key::ArrowLeft, Modifiers::NONE);
    key(&mut h, Key::Space, Modifiers::COMMAND);
    steps(&mut h, 4);
    assert_eq!(selected(&h, &b), 2, "Ctrl+Space adds a");
    key(&mut h, Key::Escape, Modifiers::NONE);
    steps(&mut h, 4);
    assert_eq!(selected(&h, &b), 0, "Escape clears");
    assert_eq!(crate::point_keys::focus(&h.ctx), None);
    assert_eq!(table_rows(&h), 3);
    // out of the points, the arrows move the visual again
    key(&mut h, Key::ArrowRight, Modifiers::NONE);
    steps(&mut h, 4);
    assert_ne!(visual(&h, &b)["x"], before);
}

/// The BIcraft bullet (its type is its guid): the page's visual that is not the table.
fn bullet(h: &Harness<'_, App>) -> crate::ribbon::VisualSel {
    let snap = h.state().loaded.as_ref().unwrap().snap.clone();
    let page = &snap["report"]["pages"][0];
    let v = page["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["type"] != "tableEx")
        .unwrap();
    crate::ribbon::VisualSel {
        page: page["name"].as_str().unwrap().into(),
        visual: v["name"].as_str().unwrap().into(),
    }
}
