//! Screenshots of View › Themes › Customize current theme drawn with wgpu (`--features
//! gpu-tests`), saved into `BICRAFT_DESKTOP_SHOTS` when set: the Customize theme dialog's Name
//! and colors, Text, Visuals, Page and Filter pane sections, in English light, French and dark.

use std::path::PathBuf;

use bicraft_ui::theme_dialog::Section;
use egui_kittest::kittest::Queryable;

use super::themes_shots::{harness, save, shop};
use super::{Temp, ready, wait};

#[test]
fn customize_theme_screenshots() {
    use bicraft_ui::theme::Brightness;
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("theme-dialog-shots");
    let path = shop(dir.path());
    for (lang, suffix, b, gallery, item) in [
        (
            "en-US",
            "en",
            Brightness::Light,
            "Themes",
            "Customize current theme…",
        ),
        (
            "fr-FR",
            "fr",
            Brightness::Light,
            "Thèmes",
            "Personnaliser le thème actuel…",
        ),
        (
            "en-US",
            "dark",
            Brightness::Dark,
            "Themes",
            "Customize current theme…",
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
        h.get_by_label(item).click();
        for _ in 0..4 {
            h.step();
        }
        assert!(h.state().cmd.theme.is_some(), "the dialog opens");
        for (section, tab, name) in [
            (Section::NameColors, 0, "names"),
            (Section::NameColors, 1, "advanced"),
            (Section::Text, 1, "text"),
            (Section::Visuals, 1, "visuals"),
            (Section::Page, 0, "page"),
            (Section::FilterPane, 1, "filter-cards"),
        ] {
            if let Some(d) = h.state_mut().cmd.theme.as_mut() {
                d.section = section;
                d.tab = tab;
                // More colours open on the first capture
                d.more = name == "names";
            }
            for _ in 0..4 {
                h.step();
            }
            save(&mut h, &shots, &format!("theme-dialog-{name}-{suffix}"));
        }
    }
}
