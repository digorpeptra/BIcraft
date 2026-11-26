//! Screenshots of the Optimize group drawn with wgpu (`--features gpu-tests`), saved into
//! `BICRAFT_DESKTOP_SHOTS` when set: light, dark and French, with the report paused (Analyze ›
//! Optimize and the paused banner).

use std::path::{Path, PathBuf};

use bicraft_engine::Session;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
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

/// A page with a column chart of Total by Region and a card of Total, saved as a PBIP project.
fn shop(dir: &Path) -> PathBuf {
    let mut s = Session::new();
    let sales = r#"#table({"Region", "Amount"}, {{"North", 100}, {"South", 50}, {"East", 75}})"#;
    let run = |s: &mut Session, id: &str, p: Value| s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Region", "data_type": "string"}),
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
    let main = s
        .report
        .as_ref()
        .map(|r| r.pages[0].name.clone())
        .unwrap_or_default();
    run(
        &mut s,
        "visual.add",
        json!({"page": main, "visual_type": "clusteredColumnChart", "x": 20, "y": 20, "width": 520, "height": 300,
               "fields": [{"role": "Category", "field": "Sales[Region]"}, {"role": "Y", "field": "[Total]"}]}),
    );
    run(
        &mut s,
        "visual.add",
        json!({"page": main, "visual_type": "card", "x": 560, "y": 20, "width": 260, "height": 160,
               "fields": [{"role": "Values", "field": "[Total]"}]}),
    );
    let path = dir.join("optimize.pbip");
    run(&mut s, "file.save_as", json!({"path": path}));
    path
}

fn paused(a: &App) -> bool {
    a.loaded
        .as_ref()
        .and_then(|l| l.snap.pointer("/report/settings/paused"))
        .and_then(Value::as_bool)
        == Some(true)
}

#[test]
fn optimize_screenshots() {
    use bicraft_ui::theme::Brightness;
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("optimize-shots");
    let path = shop(dir.path());
    for (lang, theme, b, pause) in [
        ("en-US", "light", Brightness::Light, "Pause visuals"),
        ("en-US", "dark", Brightness::Dark, "Pause visuals"),
        ("fr-FR", "light", Brightness::Light, "Suspendre les visuels"),
    ] {
        let mut app = crate::app(lang, None).with_sync_render();
        app.prefs.panes.data = false;
        app.prefs.panes.filters = false;
        app.set_theme(b);
        app.open_path(&path);
        let mut h = Harness::builder()
            .with_size(egui::vec2(1500.0, 900.0))
            .wgpu()
            .build_ui_state(
                |ui, app: &mut App| {
                    app.logic(ui.ctx());
                    app.ui(ui);
                },
                app,
            );
        wait(&mut h, "the first snapshot", ready);
        h.state_mut().ribbon_tab = bicraft_ui::registry::Tab::Analyze;
        h.step();
        h.step();
        h.get_by_label(pause).click();
        wait(&mut h, "the pause", paused);
        h.step();
        h.step();
        save(&mut h, &shots, &format!("optimize-paused-{theme}-{lang}"));
    }
}
