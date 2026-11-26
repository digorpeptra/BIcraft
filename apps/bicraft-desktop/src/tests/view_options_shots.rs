//! Screenshots of View › Page options drawn with wgpu (`--features gpu-tests`), saved into
//! `BICRAFT_DESKTOP_SHOTS` when set: Gridlines and Snap to grid on, a visual selected, the status
//! bar's zoom at 80 % (English light, French, dark), and the Page view menu open.

use std::path::PathBuf;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::themes_shots::{save, shop};
use super::{Temp, ready, wait};
use crate::App;
use bicraft_ui::app::ViewZoom;
use bicraft_ui::registry::Tab as RTab;

#[test]
fn view_options_screenshots() {
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("view-options-shots");
    let path = shop(dir.path());
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
        wait(&mut h, "the page", ready);
        {
            let a = h.state_mut();
            a.prefs.show_gridlines = true;
            a.prefs.snap_grid = true;
            a.ribbon_tab = RTab::View;
            a.view_zoom = Some(ViewZoom::Percent(80));
        }
        for _ in 0..4 {
            h.step();
        }
        // select the chart by its title: its handles over the grid
        let page = h.state().placed.unwrap();
        let p = page.rect.min + egui::vec2(300.0, 30.0) * page.zoom;
        super::click(&mut h, p, false);
        for _ in 0..4 {
            h.step();
        }
        assert!(h.state().prefs.show_gridlines);
        assert_eq!(h.state().placed.map(|p| p.zoom), Some(0.8));
        save(&mut h, &shots, &format!("view-options-{suffix}"));
        if suffix == "en" {
            h.get_by_label("Page view").click();
            for _ in 0..4 {
                h.step();
            }
            save(&mut h, &shots, "view-options-page-view-menu");
        }
    }
}
