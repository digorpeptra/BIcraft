//! Screenshots of File › Export › Redact a copy for sharing (M11.37) drawn with wgpu
//! (`--features gpu-tests`), saved into `BICRAFT_DESKTOP_SHOTS` when set: light, dark and
//! French, after the copy was written.

use std::path::{Path, PathBuf};

use bicraft_engine::Session;
use bicraft_ui::backstage::{Backstage, Page, Task};
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

/// A project whose queries name a server, a database and a URL with a token.
fn hub(dir: &Path) -> PathBuf {
    let mut s = Session::new();
    let run = |s: &mut Session, id: &str, p: Value| s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    run(&mut s, "model.new", json!({"name": "Hub"}));
    run(
        &mut s,
        "table.add",
        json!({"name": "Sales", "partition": {"source_type": "m",
            "expression": "Sql.Database(\"sql-prod-07.example\", \"SalesDW\")"}}),
    );
    run(
        &mut s,
        "expression.add",
        json!({"name": "Api", "expression": "Web.Contents(\"https://api.example/v2\", [Headers = [Authorization = \"Bearer abc123\"]])"}),
    );
    run(&mut s, "report.new", json!({}));
    let path = dir.join("hub.pbip");
    run(&mut s, "file.save_as", json!({"path": path}));
    path
}

#[test]
fn redact_screenshots() {
    use bicraft_ui::theme::Brightness;
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("redact-shots");
    let path = hub(dir.path());
    for (lang, theme, b, data, button) in [
        (
            "en-US",
            "light",
            Brightness::Light,
            "Replace the stored text values too (PBIX)",
            "Choose where…",
        ),
        (
            "en-US",
            "dark",
            Brightness::Dark,
            "Replace the stored text values too (PBIX)",
            "Choose where…",
        ),
        (
            "fr-FR",
            "light",
            Brightness::Light,
            "Remplacer aussi les valeurs texte stockées (PBIX)",
            "Choisir où…",
        ),
    ] {
        let mut app = crate::app(lang, None).with_sync_render();
        app.set_theme(b);
        let out = dir
            .path()
            .join(format!("out-{theme}-{lang}"))
            .join("hub (redacted).pbip");
        std::fs::create_dir_all(out.parent().unwrap()).unwrap();
        let o2 = out.clone();
        // the Save dialog answers at once (nothing is shown on screen)
        app.shell.pick_save = Some(Box::new(move |_, _, _, _| Some(o2.clone())));
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
        wait(&mut h, "the first snapshot", ready);
        let mut bs = Backstage::new(Page::Export);
        bs.task = Task::Redact;
        h.state_mut().backstage = Some(bs);
        h.step();
        h.step();
        h.get_by_label(data);
        h.get_by_label(button).click();
        wait(&mut h, "the copy", |a: &App| {
            matches!(&a.output, Some(("redact", Some(Ok(_)))))
        });
        h.step();
        h.step();
        assert!(out.exists());
        save(&mut h, &shots, &format!("redact-{theme}-{lang}"));
    }
}
