//! Map visuals in the app (docs/formats/maps.md): a click on a bubble of a map and on an area of
//! a shape map's custom map selects that location and cross-filters the page's table, as other
//! visuals do; the shape map's Format › Map settings › Add a map type opens `shape_map.set_map`'s
//! form, whose run draws the new map.

use std::path::Path;

use bicraft_engine::Session;
use egui::accesskit::Role;
use egui::{Event, Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::backstage_tests::{engine_shell, steps};
use super::harness;
use super::panes_ui_tests::{select, visual_named};
use crate::App;
use crate::format_pane::Tab;
use crate::shell::Caps;

/// Squares around Paris and Lyon, keyed by name.
const TOPO: &str = r#"{"type":"FeatureCollection","features":[
 {"type":"Feature","properties":{"name":"Paris"},"geometry":{"type":"Polygon","coordinates":[[[1.5,48],[3.5,48],[3.5,49.5],[1.5,49.5],[1.5,48]]]}},
 {"type":"Feature","properties":{"name":"Lyon"},"geometry":{"type":"Polygon","coordinates":[[[4,45],[6,45],[6,46.5],[4,46.5],[4,45]]]}}]}"#;

/// Places(Region, Lat, Lon, Amount) with [Total]; one page with a bubble map, a shape map (with
/// the custom map unless `bare`) and a table.
fn session(bare: bool) -> Session {
    let mut s = Session::new();
    for (id, p) in [
        ("model.new", json!({"name": "Geo"})),
        (
            "table.import",
            json!({"name": "Places", "m": "let Source = #table(type table [Region = text, Lat = number, Lon = number, Amount = Int64.Type], {{\"Paris\", 48.85, 2.35, 50}, {\"Lyon\", 45.76, 4.83, 10}, {\"Lille\", 50.63, 3.06, 5}}) in Source"}),
        ),
        (
            "measure.add",
            json!({"table": "Places", "name": "Total", "expression": "SUM(Places[Amount])"}),
        ),
        ("report.new", json!({})),
    ] {
        s.execute(id, &p).unwrap();
    }
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    for (ty, rect, fields) in [
        (
            "map",
            [20, 20, 560, 330],
            json!([{"role": "Category", "field": "Places[Region]"},
                   {"role": "Y", "field": "Average(Places[Lat])"},
                   {"role": "X", "field": "Average(Places[Lon])"},
                   {"role": "Size", "field": "[Total]"}]),
        ),
        (
            "shapeMap",
            [600, 20, 560, 330],
            json!([{"role": "Category", "field": "Places[Region]"}, {"role": "Value", "field": "[Total]"}]),
        ),
        (
            "tableEx",
            [20, 380, 500, 200],
            json!([{"role": "Values", "field": "Places[Region]"}, {"role": "Values", "field": "[Total]"}]),
        ),
    ] {
        let out = s
            .execute(
                "visual.add",
                &json!({"page": page, "visual_type": ty, "x": rect[0], "y": rect[1], "width": rect[2],
                        "height": rect[3], "fields": fields}),
            )
            .unwrap();
        if ty == "shapeMap" && !bare {
            s.execute(
                "shape_map.set_map",
                &json!({"page": page, "visual": out["visual"], "data": TOPO, "name": "cities.json"}),
            )
            .unwrap();
        }
    }
    s.dirty = false;
    s.report_dirty = false;
    s
}

fn open(bare: bool) -> Harness<'static, App> {
    let mut app = App::new("en-US", engine_shell(Caps::DESKTOP, move || session(bare))).with_sync_render();
    app.open_path(Path::new("geo.pbip"));
    let mut h = harness(app, (1500.0, 900.0));
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

fn snap_visual(h: &Harness<'_, App>, name: &str) -> Value {
    h.state().loaded.as_ref().unwrap().snap["report"]["pages"][0]["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == name)
        .cloned()
        .unwrap()
}

/// The table's rows, from the latest snapshot.
fn table_rows(h: &Harness<'_, App>) -> Vec<Value> {
    let t = visual_named(h, "tableEx");
    snap_visual(h, &t.visual)["data"]["rows"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// The screen centre of the page hit of `visual` whose tooltip names `place`.
fn hit_of(h: &Harness<'_, App>, visual: &str, place: &str) -> Option<Pos2> {
    let app = h.state();
    let placed = app.placed?;
    let (_, img) = app.shown.as_ref()?;
    let hit = img.as_ref().ok()?.hits.iter().find(|x| {
        x.visual == visual && x.tip.iter().any(|(_, v)| v == place) && x.action_for(false).is_some()
    })?;
    let (x, y, w, hh) = hit.shape.bounds();
    Some(Pos2::new(
        placed.rect.min.x + (x + w / 2.0) * placed.zoom,
        placed.rect.min.y + (y + hh / 2.0) * placed.zoom,
    ))
}

fn click_at(h: &mut Harness<'_, App>, pos: Pos2) {
    h.hover_at(pos);
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.step();
    }
}

fn settle(h: &mut Harness<'_, App>) {
    for _ in 0..100 {
        h.step();
        if h.state().calls_pending() == 0 {
            break;
        }
    }
    steps(h, 3);
}

/// A click on a place of `ty` selects it: the engine's `select` with that location, and the
/// table filtered to it.
fn click_selects(ty: &str) {
    let mut h = open(false);
    let v = visual_named(&h, ty).visual;
    assert_eq!(table_rows(&h).len(), 3);
    let pos = hit_of(&h, &v, "Paris").unwrap_or_else(|| panic!("{ty}: no hit for Paris"));
    click_at(&mut h, pos);
    settle(&mut h);
    let a = h.state().last_action.clone().unwrap_or(Value::Null);
    assert_eq!(a["action"], "select", "{ty}: {a}");
    assert_eq!(a["visual"], v.as_str(), "{ty}");
    assert_eq!(a["point"], json!([{"c": 0, "v": "Paris"}]), "{ty}");
    assert_eq!(table_rows(&h), vec![json!(["Paris", 50])], "{ty}: cross-filtered");
    assert!(
        snap_visual(&h, &v)["interaction"]["selected"].is_object(),
        "{ty}: the source keeps its selection"
    );
}

#[test]
fn a_bubble_click_cross_filters_the_page() {
    click_selects("map");
}

#[test]
fn a_shape_click_cross_filters_the_page() {
    click_selects("shapeMap");
}

#[test]
fn map_settings_add_a_map_type_opens_its_form_and_draws_the_map() {
    let mut h = open(true);
    let sm = visual_named(&h, "shapeMap");
    assert_eq!(snap_visual(&h, &sm.visual)["shape_map"]["source"], "builtin");
    select(&mut h, &sm);
    h.get_by_role_and_label(Role::Button, "Format visual").click();
    steps(&mut h, 3);
    assert_eq!(h.state().panes.format.tab, Tab::Format);
    h.get_by_label("Visual cards").click();
    settle(&mut h);
    h.get_by_role_and_label(Role::Button, "Map settings").click();
    steps(&mut h, 3);
    h.get_by_role_and_label(Role::Button, "Map settings Custom map")
        .click();
    steps(&mut h, 3);
    let form = h.state().form.as_ref().map(|f| f.id);
    assert_eq!(
        form,
        Some("shape_map.set_map"),
        "the button opens the command's form"
    );
    // the form's data field takes the map text; Run draws it
    let f = h.state_mut().form.as_mut().unwrap();
    for i in &mut f.inputs {
        if i.field.name == "data" {
            i.text = TOPO.to_string();
        }
    }
    h.get_by_label("Run").click();
    settle(&mut h);
    let shot = snap_visual(&h, &sm.visual)["shape_map"].clone();
    assert_eq!(shot["source"], "custom", "{shot}");
    assert_eq!(shot["shapes"].as_array().map(Vec::len), Some(2));
    assert!(
        hit_of(&h, &sm.visual, "Lyon").is_some(),
        "the areas are drawn and clickable"
    );
}

#[test]
fn map_settings_world_countries_writes_the_built_in_map_and_draws_it() {
    let mut h = open(true);
    let sm = visual_named(&h, "shapeMap");
    select(&mut h, &sm);
    h.get_by_role_and_label(Role::Button, "Format visual").click();
    steps(&mut h, 3);
    h.get_by_label("Visual cards").click();
    settle(&mut h);
    h.get_by_role_and_label(Role::Button, "Map settings").click();
    steps(&mut h, 3);
    h.get_by_role_and_label(Role::Button, "Map settings Built-in map")
        .click();
    settle(&mut h);
    assert!(h.state().form.is_none(), "the built-in map runs at once");
    let shot = snap_visual(&h, &sm.visual)["shape_map"].clone();
    assert_eq!(shot["source"], "custom", "{}", shot["error"]);
    assert_eq!(shot["name"], "World countries (Natural Earth).topojson");
    assert!(shot["shapes"].as_array().map_or(0, Vec::len) > 170);
    // the countries are drawn; one without data names its ISO code in its tooltip
    let app = h.state();
    let (_, img) = app.shown.as_ref().unwrap();
    assert!(
        img.as_ref()
            .unwrap()
            .hits
            .iter()
            .any(|x| x.visual == sm.visual && x.tip.iter().any(|(_, v)| v == "FRA")),
        "the world's areas are drawn"
    );
}
