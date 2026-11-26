//! Screenshots of New group drawn with wgpu (`--features gpu-tests`), saved into
//! `BICRAFT_DESKTOP_SHOTS` when set: the Data pane's column menu, the Groups dialog for a list
//! group and for bins, and a table visual on the group column, in English (light and dark) and
//! French; and the Quick measures dialog with the field menu that opens it.

use std::path::{Path, PathBuf};

use bicraft_engine::Session;
use bicraft_ui::app::Tab;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::{Temp, build_shop, ready, wait};
use crate::App;

fn save(h: &mut Harness<'_, App>, dir: &Option<PathBuf>, name: &str) {
    let img = h.render().unwrap();
    assert!(img.width() > 0 && img.height() > 0);
    let Some(dir) = dir else { return };
    std::fs::create_dir_all(dir).unwrap();
    let size = tiny_skia::IntSize::from_wh(img.width(), img.height()).unwrap();
    let pm = tiny_skia::Pixmap::from_vec(img.as_raw().clone(), size).unwrap();
    pm.save_png(dir.join(format!("{name}.png"))).unwrap();
}

/// The shop fixture with a list group of its categories and a table bound to it.
fn groups_shop(dir: &Path) -> PathBuf {
    let mut s = Session::new();
    let shop = build_shop(&mut s);
    let run = |s: &mut Session, id: &str, p: serde_json::Value| {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
    };
    run(
        &mut s,
        "column.group",
        json!({"table": "Sales", "column": "Category", "groups": [{"name": "Play", "values": ["Toys", "Games"]}]}),
    );
    run(
        &mut s,
        "visual.add",
        json!({"page": shop.page, "visual_type": "tableEx", "x": 880, "y": 420, "width": 360, "height": 220,
               "fields": [{"role": "Values", "field": "Sales[Category (groups)]"}, {"role": "Values", "field": "[Total]"}]}),
    );
    let path = dir.join("groups.pbip");
    run(&mut s, "file.save_as", json!({"path": path}));
    path
}

#[test]
fn groups_screenshots() {
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("groups-shots");
    let path = groups_shop(dir.path());
    for (lang, suffix, dark) in [
        ("en-US", "en", false),
        ("fr-FR", "fr", false),
        ("en-US", "dark", true),
    ] {
        let mut app = crate::app(lang, None).with_sync_render();
        app.open_path(&path);
        let mut h = Harness::builder()
            .with_size(egui::vec2(1400.0, 860.0))
            .wgpu()
            .build_ui_state(
                |ui, app: &mut App| {
                    app.logic(ui.ctx());
                    app.ui(ui);
                },
                app,
            );
        if dark {
            h.state_mut().set_theme(bicraft_ui::theme::Brightness::Dark);
        }
        wait(&mut h, "the first snapshot", ready);
        h.state_mut().show_tab(Tab::Report);
        h.step();
        wait(&mut h, "the table's data", ready);
        h.step();
        save(&mut h, &shots, &format!("groups-report-{suffix}"));
        // the list dialog on Category (its group made, one value left)
        bicraft_ui::group_dialog::open(h.state_mut(), "Sales", "Category");
        for _ in 0..50 {
            h.step();
            if h.state().cmd.group.as_ref().is_some_and(|d| d.values.is_some()) {
                break;
            }
        }
        if let Some(d) = h.state_mut().cmd.group.as_mut() {
            d.groups = vec![("Play".into(), vec![json!("Toys"), json!("Games")])];
            d.group = Some(0);
        }
        h.step();
        h.step();
        save(&mut h, &shots, &format!("groups-list-{suffix}"));
        // bins on Amount
        bicraft_ui::group_dialog::open(h.state_mut(), "Sales", "Amount");
        for _ in 0..50 {
            h.step();
            if h.state().cmd.group.as_ref().is_some_and(|d| d.values.is_some()) {
                break;
            }
        }
        if let Some(d) = h.state_mut().cmd.group.as_mut() {
            d.bin_size = "25".into();
        }
        h.step();
        h.step();
        save(&mut h, &shots, &format!("groups-bins-{suffix}"));
        h.state_mut().cmd.group = None;
        h.step();
        // the Data pane's column menu
        h.get_all_by_label("Sales").next().unwrap().click();
        h.step();
        h.step();
        let amount = h.get_all_by_label("Amount").next().map(|n| n.rect().center());
        if let Some(at) = amount {
            h.hover_at(at);
            h.step();
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Secondary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                });
                h.step();
            }
            h.step();
            save(&mut h, &shots, &format!("groups-menu-{suffix}"));
        }
        assert!(h.state().error.is_none(), "{:?}", h.state().error);
    }
}

/// The Quick measures dialog (an average per category, opened on [Total]) and the Data pane's
/// field menu that opens it, in English (light and dark) and French: `groups-quick-*`.
#[test]
fn quick_measure_screenshots() {
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("quick-shots");
    let path = groups_shop(dir.path());
    for (lang, suffix, dark) in [
        ("en-US", "en", false),
        ("fr-FR", "fr", false),
        ("en-US", "dark", true),
    ] {
        let mut app = crate::app(lang, None).with_sync_render();
        app.open_path(&path);
        let mut h = Harness::builder()
            .with_size(egui::vec2(1400.0, 860.0))
            .wgpu()
            .build_ui_state(
                |ui, app: &mut App| {
                    app.logic(ui.ctx());
                    app.ui(ui);
                },
                app,
            );
        if dark {
            h.state_mut().set_theme(bicraft_ui::theme::Brightness::Dark);
        }
        wait(&mut h, "the first snapshot", ready);
        h.state_mut().show_tab(Tab::Report);
        h.step();
        // the Data pane's menu on [Total]
        h.get_all_by_label("Sales").next().unwrap().click();
        h.step();
        h.step();
        let total = h.get_all_by_label("Total").next().map(|n| n.rect().center());
        if let Some(at) = total {
            h.hover_at(at);
            h.step();
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Secondary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                });
                h.step();
            }
            h.step();
            save(&mut h, &shots, &format!("groups-quick-menu-{suffix}"));
            h.key_press(egui::Key::Escape);
            h.step();
        }
        // the dialog, an average per category filled in
        bicraft_ui::quick_dialog::open(h.state_mut(), Some("[Total]"));
        if let Some(d) = h.state_mut().cmd.quick.as_mut() {
            d.kind = "average_per_category";
            d.fields.insert("category".into(), json!("Sales[Category]"));
        }
        h.step();
        h.step();
        h.step();
        save(&mut h, &shots, &format!("groups-quick-{suffix}"));
        assert!(h.state().error.is_none(), "{:?}", h.state().error);
    }
}
