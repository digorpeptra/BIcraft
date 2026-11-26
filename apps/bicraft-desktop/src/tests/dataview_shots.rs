//! Screenshots of the Data view drawn with wgpu (`--features gpu-tests`), saved into
//! `BICRAFT_DESKTOP_SHOTS` when set: the grid on the report surface with its row and column
//! lines (M11.41), then a long open error wrapped in the banner above it, the view laid out
//! below the banner (M11.42), in English light, French and dark.

use std::path::PathBuf;

use egui_kittest::Harness;

use super::modelsec_shots::{open, shop_with, steps};
use super::{Temp, wait};
use crate::App;
use bicraft_ui::app::Tab as View;

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
fn data_view_grid_and_banner_screenshots() {
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("dataview-shots");
    let path = shop_with(dir.path(), "dataview.pbip", Vec::new());
    for (lang, suffix, dark) in [
        ("en-US", "en", false),
        ("fr-FR", "fr", false),
        ("en-US", "dark", true),
    ] {
        let model = if lang == "fr-FR" { "Modèle" } else { "Model" };
        let mut h = open(&path, lang, dark, model);
        h.state_mut().show_tab(View::Data);
        wait(&mut h, "the first rows", |a| a.cmd.data.row_count.is_some());
        steps(&mut h);
        save(&mut h, &shots, &format!("dataview-grid-{suffix}"));
        let shown = path.display().to_string();
        let msg = format!(
            "{shown}: {}",
            "the report could not be read because a definition file names a page that does not \
             exist; the model was opened and can be edited, saved and converted, but the report \
             view stays empty until the page is restored or the reference is removed. "
                .repeat(3)
        );
        h.state_mut().error = Some(bicraft_ui::app::with_path(&shown, &msg));
        steps(&mut h);
        save(&mut h, &shots, &format!("dataview-banner-{suffix}"));
    }
}
