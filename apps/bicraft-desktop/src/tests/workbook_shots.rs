//! Screenshots of an imported Excel Power Pivot workbook (M11.33), drawn with wgpu (`--features
//! gpu-tests`), saved into `BICRAFT_DESKTOP_SHOTS` when set: the Model view and the Data view of
//! the synthetic workbook, and the error of a workbook without a data model (`workbook-*`), in
//! English (light and dark) and French.

use std::path::PathBuf;

use egui_kittest::Harness;

use super::workbook_tests::{click_open, workbook};
use super::{Temp, wait};
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

fn steps(h: &mut Harness<'_, App>, n: usize) {
    for _ in 0..n {
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn imported_workbook_screenshots() {
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("workbook-shots");
    let path = workbook(dir.path());
    let plain = dir.path().join("plain.xlsx");
    let pkg = bicraft_opc::Package {
        parts: vec![bicraft_opc::Part::new(
            "xl/workbook.xml",
            b"<workbook/>".to_vec(),
            false,
        )],
    };
    std::fs::write(&plain, pkg.to_bytes().unwrap()).unwrap();
    for (lang, suffix, dark) in [
        ("en-US", "en", false),
        ("fr-FR", "fr", false),
        ("en-US", "dark", true),
    ] {
        let mut app = crate::app(lang, None).with_sync_render();
        let picked = path.clone();
        app.shell.pick_file = Some(Box::new(move |_| Some(picked.clone())));
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
        h.step();
        h.step();
        click_open(&mut h, lang);
        wait(&mut h, "the snapshot", |a| a.loaded.is_some());
        assert_eq!(h.state().error, None);
        h.state_mut().tab = Tab::Model;
        wait(&mut h, "the diagram", |a| a.model_view.diagram().is_some());
        steps(&mut h, 4);
        save(&mut h, &shots, &format!("workbook-model-{suffix}"));
        bicraft_ui::data_view::open_table(h.state_mut(), "Sales");
        wait(&mut h, "the rows", |a| a.cmd.data.rows().is_some_and(|n| n > 0));
        steps(&mut h, 6);
        save(&mut h, &shots, &format!("workbook-data-{suffix}"));
        h.state_mut().open_path(&plain);
        wait(&mut h, "the error", |a| a.error.is_some());
        h.state_mut().tab = Tab::Report;
        steps(&mut h, 2);
        save(&mut h, &shots, &format!("workbook-error-{suffix}"));
    }
}
