//! Screenshots of custom visuals drawn with wgpu (`--features gpu-tests`), saved into
//! `BICRAFT_DESKTOP_SHOTS` when set: the Visualizations pane's gallery with an imported visual's
//! cell and its "…" menu, the page's placeholder of that visual (its icon and name), and the
//! Remove a custom visual window; in English light, French and dark. The `.pbiviz` is our own.

use std::path::{Path, PathBuf};

use bicraft_engine::Session;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::{Temp, ready, wait};
use crate::App;

const GUID: &str = "bicraftTiny7C3E9A41B2D54F60";

fn save(h: &mut Harness<'_, App>, dir: &Option<PathBuf>, name: &str) {
    let img = h.render().unwrap();
    assert!(img.width() > 0 && img.height() > 0);
    let Some(dir) = dir else { return };
    std::fs::create_dir_all(dir).unwrap();
    let size = tiny_skia::IntSize::from_wh(img.width(), img.height()).unwrap();
    let pm = tiny_skia::Pixmap::from_vec(img.as_raw().clone(), size).unwrap();
    pm.save_png(dir.join(format!("{name}.png"))).unwrap();
}

/// A 20×20 icon of our own: a blue square with an orange bar.
fn icon_png() -> Vec<u8> {
    let mut pm = tiny_skia::Pixmap::new(20, 20).unwrap();
    pm.fill(tiny_skia::Color::from_rgba8(17, 141, 255, 255));
    let bar = tiny_skia::Rect::from_xywh(3.0, 12.0, 14.0, 4.0).unwrap();
    let mut paint = tiny_skia::Paint::default();
    paint.set_color_rgba8(255, 140, 0, 255);
    pm.fill_rect(bar, &paint, tiny_skia::Transform::identity(), None);
    pm.encode_png().unwrap()
}

fn b64(bytes: &[u8]) -> String {
    bicraft_ui::rich::base64(bytes)
}

/// Our own tiny `.pbiviz`.
fn pbiviz() -> Vec<u8> {
    let block =
        json!({"name": "bicraftTiny", "displayName": "Tiny gauge", "guid": GUID, "version": "1.0.0.0"});
    let manifest = json!({
        "visual": block, "author": {"name": "BIcraft tests"},
        "resources": [{"resourceId": "rId0", "sourceType": 5, "file": format!("resources/{GUID}.pbiviz.json")}]
    });
    let visual = json!({
        "visual": block, "apiVersion": "5.3.0",
        "capabilities": {"dataRoles": [{"name": "value", "kind": "Measure"}]},
        "content": {"js": "var tiny = 1;", "css": "", "iconBase64": format!("data:image/png;base64,{}", b64(&icon_png()))}
    });
    let mut p = bicraft_opc::Package::default();
    p.put("package.json", manifest.to_string().into_bytes(), false);
    p.put(
        &format!("resources/{GUID}.pbiviz.json"),
        visual.to_string().into_bytes(),
        false,
    );
    p.to_bytes().unwrap()
}

/// Sales with [Total], the custom visual imported and a visual of its type on the page.
fn shop(dir: &Path) -> PathBuf {
    let mut s = Session::new();
    let run = |s: &mut Session, id: &str, p: serde_json::Value| {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
    };
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.import",
            json!({"name": "Sales", "m": "#table(type table [Region = text, Amount = number], {{\"E\", 10}, {\"W\", 4}})"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        ("report.new", json!({})),
        ("visual.import_custom", json!({"data": b64(&pbiviz())})),
    ] {
        run(&mut s, id, p);
    }
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": GUID, "x": 40, "y": 40, "width": 420, "height": 260}),
    );
    let path = dir.join("custom.pbip");
    run(&mut s, "file.save_as", json!({"path": path}));
    path
}

fn steps(h: &mut Harness<'_, App>) {
    for _ in 0..6 {
        h.step();
    }
}

#[test]
fn custom_visual_screenshots() {
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("pbiviz-shots");
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
        wait(&mut h, "the first snapshot", ready);
        steps(&mut h);
        let more = if lang == "fr-FR" {
            "Plus de visuels"
        } else {
            "More visuals"
        };
        h.get_by_label(more).click();
        steps(&mut h);
        save(&mut h, &shots, &format!("pbiviz-gallery-{suffix}"));
        h.key_press(egui::Key::Escape);
        steps(&mut h);
        bicraft_ui::ribbon::invoke(h.state_mut(), "visual.remove_custom", json!({}));
        steps(&mut h);
        assert!(h.state().cmd.custom_visuals.remove.is_some());
        save(&mut h, &shots, &format!("pbiviz-remove-{suffix}"));
    }
}
