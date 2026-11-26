//! Screenshots of View › Themes and high contrast drawn with wgpu (`--features gpu-tests`),
//! saved into `BICRAFT_DESKTOP_SHOTS` when set: the gallery open (light, dark, French), a
//! built-in theme applied, and the high-contrast app theme with the report in high contrast.

use std::path::{Path, PathBuf};

use bicraft_engine::Session;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::{Temp, ready, wait};
use crate::App;

pub(super) fn save(h: &mut Harness<'_, App>, dir: &Option<PathBuf>, name: &str) {
    let img = h.render().unwrap();
    assert!(img.width() > 0 && img.height() > 0);
    let Some(dir) = dir else { return };
    std::fs::create_dir_all(dir).unwrap();
    let size = tiny_skia::IntSize::from_wh(img.width(), img.height()).unwrap();
    let pm = tiny_skia::Pixmap::from_vec(img.as_raw().clone(), size).unwrap();
    pm.save_png(dir.join(format!("{name}.png"))).unwrap();
}

/// A page with a column chart of Total by Region, a card and a text box, saved as PBIP.
pub(super) fn shop(dir: &Path) -> PathBuf {
    let mut s = Session::new();
    let sales =
        r#"#table({"Region", "Amount"}, {{"North", 100}, {"South", 50}, {"East", 75}, {"West", 30}})"#;
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
        json!({"page": main, "visual_type": "pieChart", "x": 560, "y": 20, "width": 300, "height": 300,
               "fields": [{"role": "Category", "field": "Sales[Region]"}, {"role": "Y", "field": "[Total]"}]}),
    );
    run(
        &mut s,
        "visual.add",
        json!({"page": main, "visual_type": "card", "x": 20, "y": 340, "width": 260, "height": 140,
               "fields": [{"role": "Values", "field": "[Total]"}]}),
    );
    let path = dir.join("themes.pbip");
    run(&mut s, "file.save_as", json!({"path": path}));
    path
}

pub(super) fn harness(app: App) -> Harness<'static, App> {
    Harness::builder()
        .with_size(egui::vec2(1500.0, 900.0))
        .wgpu()
        .build_ui_state(
            |ui, app: &mut App| {
                app.logic(ui.ctx());
                app.ui(ui);
            },
            app,
        )
}

fn theme_is(name: &'static str) -> impl Fn(&App) -> bool {
    move |a: &App| {
        a.loaded
            .as_ref()
            .and_then(|l| l.snap.pointer("/report/theme/name"))
            .and_then(Value::as_str)
            == Some(name)
    }
}

#[test]
fn themes_screenshots() {
    use bicraft_ui::theme::Brightness;
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("themes-shots");
    let path = shop(dir.path());
    // the gallery open, then a theme applied
    for (lang, theme, b, gallery, tile, applied) in [
        (
            "en-US",
            "light",
            Brightness::Light,
            "Themes",
            "Harbor",
            "BIcraft Harbor",
        ),
        (
            "en-US",
            "dark",
            Brightness::Dark,
            "Themes",
            "Dusk",
            "BIcraft Dusk",
        ),
        (
            "fr-FR",
            "light",
            Brightness::Light,
            "Thèmes",
            "Verger",
            "BIcraft Orchard",
        ),
    ] {
        let mut app = crate::app(lang, None).with_sync_render();
        app.prefs.panes.data = false;
        app.prefs.panes.filters = false;
        app.set_theme(b);
        app.open_path(&path);
        let mut h = harness(app);
        wait(&mut h, "the first snapshot", ready);
        h.state_mut().ribbon_tab = bicraft_ui::registry::Tab::View;
        h.step();
        h.step();
        h.get_by_label(gallery).click();
        h.step();
        h.step();
        save(&mut h, &shots, &format!("themes-gallery-{theme}-{lang}"));
        h.get_by_label(tile).click();
        wait(&mut h, "the theme", theme_is(applied));
        for _ in 0..4 {
            h.step();
        }
        save(&mut h, &shots, &format!("themes-applied-{theme}-{lang}"));
    }
    // the high-contrast app theme, with the report in high contrast, in English and French
    for (lang, label) in [("en-US", "High contrast"), ("fr-FR", "Contraste élevé")] {
        let mut app = crate::app(lang, None).with_sync_render();
        app.prefs.panes.data = false;
        app.prefs.panes.filters = false;
        app.open_path(&path);
        let mut h = harness(app);
        wait(&mut h, "the first snapshot", ready);
        h.state_mut().ribbon_tab = bicraft_ui::registry::Tab::Home;
        h.step();
        h.step();
        h.get_by_label(label).click();
        wait(&mut h, "the high-contrast page", |a: &App| {
            a.shown.as_ref().is_some_and(|(k, _)| k.high_contrast)
        });
        for _ in 0..4 {
            h.step();
        }
        save(&mut h, &shots, &format!("themes-high-contrast-{lang}"));
        h.state_mut().ribbon_tab = bicraft_ui::registry::Tab::View;
        for _ in 0..3 {
            h.step();
        }
        save(&mut h, &shots, &format!("themes-high-contrast-view-{lang}"));
    }
}
