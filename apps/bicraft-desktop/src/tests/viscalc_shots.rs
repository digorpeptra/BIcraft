//! Screenshots of the visual calculations edit window drawn with wgpu (`--features gpu-tests`),
//! saved into `BICRAFT_DESKTOP_SHOTS` when set: a table with a running sum and a percent of
//! grand total, its [Total] hidden, the window open on the running sum (`viscalc-edit-*`), then
//! the report with the window closed (`viscalc-report-*`), in English (light and dark) and French.

use std::path::{Path, PathBuf};

use bicraft_engine::Session;
use egui_kittest::Harness;
use serde_json::{Value, json};

use super::{Temp, ready, wait};
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

/// Sales(Product, Amount), [Total]; a table (Product, [Total], two calculations, [Total]
/// hidden) saved as a PBIP project.
fn shop(dir: &Path) -> PathBuf {
    let mut s = Session::new();
    let sales = r#"#table({"Product", "Amount"}, {{"Ball", 50}, {"Bike", 150}, {"Car", 300}, {"Kite", 20}})"#;
    let run = |s: &mut Session, id: &str, p: Value| s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
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
        ("report.new", json!({})),
    ] {
        run(&mut s, id, p);
    }
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "tableEx", "x": 40, "y": 40, "width": 520, "height": 260,
               "fields": [{"role": "Values", "field": "Sales[Product]"}, {"role": "Values", "field": "[Total]"}]}),
    );
    let visual = s.report.as_ref().unwrap().pages[0].visuals[0].name.clone();
    for (name, e) in [
        ("Running sum", "RUNNINGSUM([Total])"),
        ("Share", "DIVIDE([Total], COLLAPSEALL([Total], ROWS))"),
    ] {
        run(
            &mut s,
            "visual.calculation.add",
            json!({"page": page, "visual": visual, "name": name, "expression": e}),
        );
    }
    run(
        &mut s,
        "visual.calculation.set",
        json!({"page": page, "visual": visual, "name": "Total", "hidden": true}),
    );
    let path = dir.join("viscalc.pbip");
    run(&mut s, "file.save_as", json!({"path": path}));
    path
}

fn table(a: &App) -> Option<(String, String)> {
    let p = a.loaded.as_ref()?.snap.pointer("/report/pages/0")?.clone();
    let v = p
        .get("visuals")?
        .as_array()?
        .iter()
        .find(|v| v["type"] == "tableEx")?
        .clone();
    (v["data"]["status"] == "ok").then(|| {
        (
            p["name"].as_str().unwrap_or_default().into(),
            v["name"].as_str().unwrap_or_default().into(),
        )
    })
}

#[test]
fn visual_calculations_screenshots() {
    use bicraft_ui::theme::Brightness;
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("viscalc-shots");
    let path = shop(dir.path());
    for (lang, suffix, b) in [
        ("en-US", "en", Brightness::Light),
        ("fr-FR", "fr", Brightness::Light),
        ("en-US", "dark", Brightness::Dark),
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
        h.state_mut().set_theme(b);
        wait(&mut h, "the first snapshot", ready);
        wait(&mut h, "the table's data", |a| table(a).is_some());
        let (page, visual) = table(h.state()).unwrap();
        for _ in 0..4 {
            h.step();
        }
        save(&mut h, &shots, &format!("viscalc-report-{suffix}"));
        bicraft_ui::viscalc_dialog::open(h.state_mut(), page, visual, Some("Running sum".into()));
        for _ in 0..6 {
            h.step();
        }
        let e = h.state().cmd.viscalc.clone().unwrap();
        assert_eq!(e.expression, "RUNNINGSUM([Total])");
        save(&mut h, &shots, &format!("viscalc-edit-{suffix}"));
    }
}
