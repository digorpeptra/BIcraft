//! Screenshots of Model › New parameter drawn with wgpu (`--features gpu-tests`), saved into
//! `BICRAFT_DESKTOP_SHOTS` when set: the ribbon's menu, the Numeric range and Fields dialogs, and
//! the report with a field parameter's slicer choosing what a table shows, in English (light and
//! dark) and French.

use std::path::{Path, PathBuf};

use bicraft_engine::Session;
use bicraft_ui::app::Tab;
use bicraft_ui::param_dialog::{self, Kind};
use bicraft_ui::registry::Tab as RTab;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::{Temp, build_shop, click_hit, ready, wait};
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

/// The shop fixture with a second measure, a field parameter of [Total] and [Lines] (its slicer
/// on Main), a table bound to it, and a what-if parameter.
fn params_shop(dir: &Path) -> (PathBuf, String, String) {
    let mut s = Session::new();
    let shop = build_shop(&mut s);
    let run = |s: &mut Session, id: &str, p: serde_json::Value| {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
    };
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Lines", "expression": "COUNTROWS(Sales)"}),
    );
    let r = run(
        &mut s,
        "parameter.fields",
        json!({"name": "Metric", "fields": ["[Total]", "[Lines]"], "page": shop.page}),
    );
    let slicer = r["slicer"]["visual"].as_str().unwrap().to_string();
    run(
        &mut s,
        "visual.add",
        json!({"page": shop.page, "visual_type": "tableEx", "x": 880, "y": 420, "width": 360, "height": 260,
               "fields": [{"role": "Values", "field": "Sales[Category]"}, {"role": "Values", "field": "Metric[Metric]"}]}),
    );
    run(
        &mut s,
        "parameter.what_if",
        json!({"name": "Discount", "min": 0, "max": 50, "increment": 5, "default": 10, "page": shop.page}),
    );
    let path = dir.join("params.pbip");
    run(&mut s, "file.save_as", json!({"path": path}));
    (path, shop.page, slicer)
}

#[test]
fn new_parameter_screenshots() {
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("params-shots");
    let (path, page, slicer) = params_shop(dir.path());
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
        // the Model tab's New parameter menu
        h.state_mut().show_tab(Tab::Model);
        h.state_mut().ribbon_tab = RTab::Model;
        h.step();
        h.step();
        let new = if suffix == "fr" {
            "Nouveau paramètre"
        } else {
            "New parameter"
        };
        h.get_by_label(new).click();
        h.step();
        h.step();
        save(&mut h, &shots, &format!("params-menu-{suffix}"));
        h.key_press(egui::Key::Escape);
        h.step();
        // the dialogs over the report
        h.state_mut().show_tab(Tab::Report);
        h.step();
        param_dialog::open(h.state_mut(), Kind::WhatIf);
        h.step();
        h.step();
        save(&mut h, &shots, &format!("params-numeric-{suffix}"));
        param_dialog::open(h.state_mut(), Kind::Fields);
        if let Some(d) = h.state_mut().cmd.parameter.as_mut() {
            d.chosen = vec!["[Total]".into(), "Sales[Category]".into()];
        }
        h.step();
        h.step();
        save(&mut h, &shots, &format!("params-fields-{suffix}"));
        h.state_mut().cmd.parameter = None;
        h.step();
        // the report: nothing chosen (both measures), then Lines chosen in the slicer
        wait(&mut h, "the table's data", ready);
        h.step();
        save(&mut h, &shots, &format!("params-report-{suffix}"));
        // a click on Lines in the parameter's slicer
        let sent = click_hit(&mut h, &slicer, false, |a| {
            a["action"] == "slicer_toggle" && a["value"] == "Lines"
        });
        assert_eq!(sent["page"], page.as_str());
        assert!(h.state().error.is_none(), "{:?}", h.state().error);
        save(&mut h, &shots, &format!("params-report-lines-{suffix}"));
    }
}
