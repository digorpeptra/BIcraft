//! Screenshots of the Properties pane on several selected objects, drawn with wgpu
//! (`--features gpu-tests`), saved into `BICRAFT_DESKTOP_SHOTS` when set: two columns and a
//! measure picked with Ctrl+click in the diagram, in English light, French and dark.

use std::path::PathBuf;

use egui_kittest::Harness;

use super::model_multi_tests::row;
use super::model_tests::star;
use super::{Temp, click, wait};
use crate::App;
use bicraft_ui::app::Tab;

fn save(h: &mut Harness<'_, App>, dir: &Option<PathBuf>, name: &str) {
    let img = h.render().unwrap();
    assert!(img.width() > 0 && img.height() > 0);
    let Some(dir) = dir else { return };
    std::fs::create_dir_all(dir).unwrap();
    let size = tiny_skia::IntSize::from_wh(img.width(), img.height()).unwrap();
    let pm = tiny_skia::Pixmap::from_vec(img.as_raw().clone(), size).unwrap();
    pm.save_png(dir.join(format!("{name}.png"))).unwrap();
}

#[test]
fn multi_selection_screenshots() {
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("props-multi-shots");
    let path = star(dir.path());
    for (lang, suffix, dark) in [
        ("en-US", "en", false),
        ("fr-FR", "fr", false),
        ("en-US", "dark", true),
    ] {
        let mut app = crate::app(lang, None).with_sync_render();
        app.open_path(&path);
        app.tab = Tab::Model;
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
        if dark {
            h.state_mut().set_theme(bicraft_ui::theme::Brightness::Dark);
        }
        wait(&mut h, "the diagram", |a| {
            a.loaded.is_some() && a.model_view.diagram().is_some()
        });
        for (kind, name, ctrl) in [
            ("columns", "Amount", false),
            ("columns", "Cost", true),
            ("measures", "Total", true),
        ] {
            let p = row(h.state(), "Sales", kind, name);
            click(&mut h, p, ctrl);
        }
        for _ in 0..6 {
            h.step();
        }
        assert_eq!(h.state().model_view.selection().len(), 3);
        save(&mut h, &shots, &format!("props-multi-{suffix}"));
    }
}
