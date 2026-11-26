//! Screenshots of File › Options and settings drawn with wgpu (`--features gpu-tests`), saved
//! into `BICRAFT_DESKTOP_SHOTS` when set: light, dark and French, after Personalize visuals and
//! the Filters pane's visibility to readers were changed.

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

/// A one-page report saved as a PBIP project.
fn shop(dir: &Path) -> PathBuf {
    let mut s = Session::new();
    let sales = r#"#table({"Region", "Amount"}, {{"North", 100}, {"South", 50}}) "#;
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
        ("report.new", json!({})),
    ] {
        run(&mut s, id, p);
    }
    let path = dir.join("settings.pbip");
    run(&mut s, "file.save_as", json!({"path": path}));
    path
}

fn setting(a: &App, key: &str) -> Value {
    a.loaded
        .as_ref()
        .and_then(|l| l.snap.pointer(&format!("/report/settings/{key}")))
        .cloned()
        .unwrap_or(Value::Null)
}

#[test]
fn report_settings_screenshots() {
    use bicraft_ui::theme::Brightness;
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("report-settings-shots");
    let path = shop(dir.path());
    for (lang, theme, b, (file, options, personalize, pane)) in [
        (
            "en-US",
            "light",
            Brightness::Light,
            (
                "File",
                "Options and settings",
                "Personalize visuals",
                "Show the Filters pane to report readers",
            ),
        ),
        (
            "en-US",
            "dark",
            Brightness::Dark,
            (
                "File",
                "Options and settings",
                "Personalize visuals",
                "Show the Filters pane to report readers",
            ),
        ),
        (
            "fr-FR",
            "light",
            Brightness::Light,
            (
                "Fichier",
                "Options et paramètres",
                "Personnaliser les visuels",
                "Afficher le volet Filtres aux lecteurs du rapport",
            ),
        ),
    ] {
        let mut app = crate::app(lang, None).with_sync_render();
        app.set_theme(b);
        app.open_path(&path);
        let mut h = Harness::builder()
            .with_size(egui::vec2(1400.0, 900.0))
            .wgpu()
            .build_ui_state(
                |ui, app: &mut App| {
                    app.logic(ui.ctx());
                    app.ui(ui);
                },
                app,
            );
        wait(&mut h, "the first snapshot", ready);
        h.get_by_label(file).click();
        h.step();
        h.step();
        h.get_by_label(options).click();
        h.step();
        h.step();
        let was = setting(h.state(), "personalize_visuals");
        h.get_by_label(personalize).click();
        wait(&mut h, "personalize visuals", |a| {
            setting(a, "personalize_visuals") != was
        });
        let was = setting(h.state(), "filter_pane_visible");
        h.get_by_label(pane).click();
        wait(&mut h, "the Filters pane's visibility", |a| {
            setting(a, "filter_pane_visible") != was
        });
        h.step();
        h.step();
        save(&mut h, &shots, &format!("report-settings-{theme}-{lang}"));
    }
}
