//! Screenshots of Insert › Visuals › New visual project and Package a visual drawn with wgpu
//! (`--features gpu-tests`), saved into `BICRAFT_DESKTOP_SHOTS` when set: the Insert ribbon with
//! its buttons and the New visual project form filled (folder, name, data roles, format options),
//! then the form's answer after Run, and Package a visual's answer; in English light, French and
//! dark (M11.30).

use std::path::PathBuf;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::modelsec_shots::{open, shop_with, steps};
use super::{Temp, wait};
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

fn answered(a: &App) -> bool {
    a.form.as_ref().is_some_and(|f| f.result.is_some())
}

#[test]
fn visual_project_screenshots() {
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("vproj-shots");
    let path = shop_with(dir.path(), "vproj.pbip", Vec::new());
    for (lang, suffix, dark) in [
        ("en-US", "en", false),
        ("fr-FR", "fr", false),
        ("en-US", "dark", true),
    ] {
        let (insert, new, pack, run) = if lang == "fr-FR" {
            (
                "Insertion",
                "Nouveau projet de visuel",
                "Empaqueter un visuel",
                "Exécuter",
            )
        } else {
            ("Insert", "New visual project", "Package a visual", "Run")
        };
        let mut h = open(&path, lang, dark, insert);
        let project = dir.path().join(format!("bars-{suffix}"));
        let at = h.get_by_label(new).rect();
        super::click(&mut h, at.center(), false);
        steps(&mut h);
        {
            let f = h.state_mut().form.as_mut().unwrap();
            assert_eq!(f.id, "visual.scaffold");
            for (name, text) in [
                ("folder", project.display().to_string()),
                ("name", "shopBars".to_string()),
                ("display_name", "Shop bars".to_string()),
                (
                    "data_roles",
                    "region:Grouping:Region\nsales:Measure:Sales".to_string(),
                ),
                (
                    "format_options",
                    "bars.fill:color:Bar colour\nbars.fontSize:numeric".to_string(),
                ),
            ] {
                f.input_mut(name).unwrap().text = text;
            }
        }
        steps(&mut h);
        save(&mut h, &shots, &format!("vproj-new-{suffix}"));
        h.get_by_label(run).click();
        wait(&mut h, "the scaffold", answered);
        steps(&mut h);
        save(&mut h, &shots, &format!("vproj-created-{suffix}"));
        assert!(matches!(
            h.state().form.as_ref().and_then(|f| f.result.clone()),
            Some(Ok(Value::Object(_)))
        ));
        h.state_mut().form = None;
        steps(&mut h);
        let at = h.get_by_label(pack).rect();
        super::click(&mut h, at.center(), false);
        steps(&mut h);
        if let Some(f) = h.state_mut().form.as_mut() {
            f.input_mut("folder").unwrap().text = project.display().to_string();
        }
        h.get_by_label(run).click();
        wait(&mut h, "the package", answered);
        steps(&mut h);
        save(&mut h, &shots, &format!("vproj-packed-{suffix}"));
        let out = h.state().form.as_ref().and_then(|f| f.result.clone());
        assert!(
            matches!(&out, Some(Ok(v)) if v["has_icon"] == json!(true)),
            "{out:?}"
        );
    }
}
