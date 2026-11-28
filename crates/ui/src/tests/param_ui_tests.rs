//! Model › Parameters › New parameter through the accessibility tree (egui_kittest, no GPU):
//! the ribbon button's menu, the Numeric range dialog running `parameter.what_if`, the Fields
//! dialog running `parameter.fields` with the fields ticked in order, the French labels, and
//! the palette's entry opening the same dialog.

use std::path::Path;

use bicraft_engine::Session;
use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use super::backstage_tests::{engine_shell, steps};
use super::harness;
use crate::App;
use crate::param_dialog::Kind;
use crate::registry::Tab as RTab;
use crate::shell::Caps;

/// Sales(Product, Amount) with [Total] and [Lines], and a report of one page.
fn session() -> Session {
    let mut s = Session::new();
    let sales = r#"#table({"Product", "Amount"}, {{"Bike", 100}, {"Car", 1000}})"#;
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
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
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Lines", "expression": "COUNTROWS(Sales)"}),
        ),
        ("report.new", json!({})),
    ] {
        s.execute(id, &p).unwrap();
    }
    s.dirty = false;
    s.report_dirty = false;
    s
}

fn app(lang: &str) -> Harness<'static, App> {
    let mut app = App::new(lang, engine_shell(Caps::DESKTOP, session)).with_sync_render();
    app.open_path(Path::new("shop.pbip"));
    let mut h = harness(app, (1280.0, 860.0));
    for _ in 0..200 {
        h.step();
        if h.state().shown.is_some() {
            break;
        }
    }
    steps(&mut h, 2);
    // the Model tab of the ribbon (the Model view)
    if let Some(v) = crate::ribbon::view_of(RTab::Model) {
        h.state_mut().show_tab(v);
    }
    h.state_mut().ribbon_tab = RTab::Model;
    steps(&mut h, 3);
    h
}

fn click(h: &mut Harness<'_, App>, label: &str) {
    h.get_by_label(label).click();
    steps(h, 3);
}

/// The text input labelled `label`, its text replaced by `text`.
fn type_into(h: &mut Harness<'_, App>, label: &str, text: &str) {
    h.get_by(|n| n.role() == Role::TextInput && n.label().as_deref() == Some(label))
        .click();
    steps(h, 2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.get_by(|n| n.role() == Role::TextInput && n.label().as_deref() == Some(label))
        .type_text(text);
    steps(h, 2);
}

fn last_sent(h: &Harness<'_, App>, id: &str) -> Option<Value> {
    h.state()
        .sent
        .iter()
        .rev()
        .find(|c| c.id == id)
        .map(|c| c.params.clone())
}

fn tables(h: &Harness<'_, App>) -> Vec<String> {
    h.state()
        .loaded
        .as_ref()
        .and_then(|l| l.snap.pointer("/model/tables").and_then(Value::as_array).cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|t| t.get("name").and_then(Value::as_str).map(str::to_string))
        .collect()
}

#[test]
fn numeric_range_from_the_new_parameter_menu_runs_what_if() {
    let mut h = app("en-US");
    click(&mut h, "New parameter");
    // the menu holds the group's two commands
    h.get_by_label("Fields");
    click(&mut h, "Numeric range");
    assert_eq!(
        h.state().cmd.parameter.as_ref().map(|d| d.kind),
        Some(Kind::WhatIf)
    );
    h.get_by_label("Add a slicer to this page");
    type_into(&mut h, "Name", "Discount");
    type_into(&mut h, "Maximum", "50");
    type_into(&mut h, "Increment", "5");
    type_into(&mut h, "Default", "10");
    click(&mut h, "Create");
    steps(&mut h, 6);
    let p = last_sent(&h, "parameter.what_if").expect("parameter.what_if was not sent");
    assert_eq!(p["name"], "Discount");
    assert_eq!(
        (
            p["min"].clone(),
            p["max"].clone(),
            p["increment"].clone(),
            p["default"].clone()
        ),
        (json!(0.0), json!(50.0), json!(5.0), json!(10.0))
    );
    assert_eq!(p["add_slicer"], true);
    assert!(h.state().cmd.parameter.is_none());
    assert!(tables(&h).contains(&"Discount".to_string()), "{:?}", tables(&h));
    assert!(h.state().error.is_none(), "{:?}", h.state().error);
}

#[test]
fn fields_dialog_sends_the_ticked_fields_in_order() {
    let mut h = app("en-US");
    click(&mut h, "New parameter");
    click(&mut h, "Fields");
    assert_eq!(
        h.state().cmd.parameter.as_ref().map(|d| d.kind),
        Some(Kind::Fields)
    );
    // Create without a field says why and sends nothing
    click(&mut h, "Create");
    h.get_by_label_contains("Add at least one field.");
    assert!(last_sent(&h, "parameter.fields").is_none());
    click(&mut h, "Lines (measure)");
    click(&mut h, "Product (column)");
    // ▲ moves Product before Lines
    assert_eq!(h.query_all_by_label("Move up").count(), 2);
    h.query_all_by_label("Move up").nth(1).unwrap().click();
    steps(&mut h, 3);
    click(&mut h, "Create");
    steps(&mut h, 6);
    let p = last_sent(&h, "parameter.fields").expect("parameter.fields was not sent");
    assert_eq!(p["name"], "Parameter");
    assert_eq!(p["fields"], json!(["Sales[Product]", "[Lines]"]));
    assert!(tables(&h).contains(&"Parameter".to_string()));
    assert!(h.state().error.is_none(), "{:?}", h.state().error);
}

#[test]
fn the_dialogs_speak_french_and_the_palette_opens_them() {
    let mut h = app("fr-FR");
    click(&mut h, "Nouveau paramètre");
    h.get_by_label("Champs");
    click(&mut h, "Plage numérique");
    h.get_by_label("Ajouter un segment à cette page");
    h.get_by_label("Incrément");
    // French decimals are read
    type_into(&mut h, "Incrément", "0,5");
    click(&mut h, "Créer");
    steps(&mut h, 6);
    let p = last_sent(&h, "parameter.what_if").unwrap();
    assert_eq!(p["increment"], json!(0.5));
    assert_eq!(p["data_type"], "whole");
    // the engine refuses a whole number type with a decimal increment: the error bar says so
    assert!(h.state().error.is_some());
    // the palette's entry opens the same dialog
    crate::ribbon::invoke(h.state_mut(), "parameter.fields", json!({}));
    steps(&mut h, 3);
    assert_eq!(
        h.state().cmd.parameter.as_ref().map(|d| d.kind),
        Some(Kind::Fields)
    );
    h.get_by_label("Champs, dans l'ordre");
}

/// A report with a what-if parameter Rate (0 to 10 by 1, default 5), its slicer and a card of
/// `[Rate Value]`, shown in the Report view.
fn what_if_app() -> Harness<'static, App> {
    let make = || {
        let mut s = session();
        s.execute(
            "parameter.what_if",
            &json!({"name": "Rate", "min": 0, "max": 10, "increment": 1, "default": 5}),
        )
        .unwrap();
        let page = s.report.as_ref().unwrap().pages[0].name.clone();
        s.execute(
            "visual.add",
            &json!({"page": page, "visual_type": "card", "x": 500, "y": 300, "width": 200, "height": 120,
                    "fields": [{"role": "Values", "field": "[Rate Value]"}]}),
        )
        .unwrap();
        s.dirty = false;
        s.report_dirty = false;
        s
    };
    let mut app = App::new("en-US", engine_shell(Caps::DESKTOP, make)).with_sync_render();
    app.open_path(Path::new("rate.pbip"));
    let mut h = harness(app, (1400.0, 860.0));
    for _ in 0..200 {
        h.step();
        if h.state().shown.is_some() && h.state().placed.is_some() {
            break;
        }
    }
    steps(&mut h, 2);
    h
}

/// The card's value in the latest snapshot.
fn card_value(h: &Harness<'_, App>) -> Value {
    let snap = &h.state().loaded.as_ref().unwrap().snap;
    snap["report"]["pages"][0]["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["type"] == json!("card"))
        .map(|v| v["data"]["rows"].clone())
        .unwrap_or(Value::Null)
}

fn settle(h: &mut Harness<'_, App>) {
    for _ in 0..50 {
        h.step();
    }
}

/// G64: the what-if slicer is a single-value slider, labelled for the accessibility tree with the
/// value shown; the keyboard and a drag pick a value through `view.interact`, and the
/// parameter's measure follows.
#[test]
fn the_what_if_slider_drags_to_a_value_and_the_measure_follows() {
    let mut h = what_if_app();
    let slider = |h: &Harness<'_, App>| {
        let n = h.get_by(|n| n.role() == Role::Slider && n.label().as_deref() == Some("Rate"));
        (n.rect(), n.accesskit_node().numeric_value())
    };
    // nothing selected: the default
    assert_eq!(card_value(&h), json!([[5]]));
    assert_eq!(slider(&h).1, Some(5.0));
    // → with the focus on the slider: the next value
    h.get_by(|n| n.role() == Role::Slider && n.label().as_deref() == Some("Rate"))
        .focus();
    h.step();
    h.key_press(Key::ArrowRight);
    settle(&mut h);
    assert_eq!(card_value(&h), json!([[6]]));
    assert_eq!(slider(&h).1, Some(6.0));
    // a drag from the handle to the track's start: the smallest value
    let (track, _) = slider(&h);
    let from = egui::pos2(track.min.x + track.width() * 0.6, track.center().y);
    let to = egui::pos2(track.min.x - 20.0, track.center().y);
    h.hover_at(from);
    h.step();
    h.event(egui::Event::PointerButton {
        pos: from,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    for k in 1..=10 {
        h.event(egui::Event::PointerMoved(from + (to - from) * (k as f32 / 10.0)));
        h.step();
    }
    h.event(egui::Event::PointerButton {
        pos: to,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    settle(&mut h);
    assert_eq!(card_value(&h), json!([[0]]));
    assert_eq!(slider(&h).1, Some(0.0));
}
