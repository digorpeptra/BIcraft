//! Screenshots of the Power Query editor's folding indicators and View native query, drawn with
//! wgpu (`--features gpu-tests`), saved into `BICRAFT_DESKTOP_SHOTS` when set (`pqfold-*`), in
//! English (light and dark) and French. No source is reached: the indicators and the native
//! query are drawn from answers `query.folding` and `query.native` give (the probes themselves
//! are tested against the fake OData and TDS servers in `bicraft-ui` and `bicraft-engine`).

use std::path::{Path, PathBuf};

use bicraft_engine::Session;
use bicraft_ui::pq_editor::{self, DiagView, Diagnosis, Fold, NativeView};
use bicraft_ui::registry::Tab as RTab;
use egui::{Event, Modifiers, PointerButton};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

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

const ORDERS: &str = "let\n    Source = #table(type table [id = Int64.Type, qty = Int64.Type, city = text], {{1, 3, \"Paris\"}, {2, 5, \"Lyon\"}, {3, 8, \"Paris\"}}),\n    #\"dbo_Orders\" = Source,\n    #\"Removed Columns\" = Table.SelectColumns(#\"dbo_Orders\", {\"id\", \"qty\"}),\n    #\"Filtered Rows\" = Table.SelectRows(#\"Removed Columns\", each [qty] > 2),\n    #\"Changed Type\" = Table.TransformColumnTypes(#\"Filtered Rows\", {{\"qty\", type text}})\nin\n    #\"Changed Type\"";

fn project(dir: &Path) -> PathBuf {
    let mut s = Session::new();
    let run = |s: &mut Session, id: &str, p: serde_json::Value| {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
    };
    run(&mut s, "model.new", json!({"name": "Shop"}));
    run(&mut s, "table.import", json!({"name": "Orders", "m": ORDERS}));
    run(&mut s, "report.new", json!({}));
    let path = dir.join("fold.pbip");
    run(&mut s, "file.save_as", json!({"path": path}));
    path
}

/// The editor settled with its folding indicators read.
fn settled(a: &App) -> bool {
    a.cmd.pq.as_ref().is_some_and(|e| {
        !e.busy
            && e.step.is_some()
            && e.preview.is_some()
            && e.selected
                .and_then(|i| e.queries.get(i))
                .is_some_and(|q| q.folding.is_some())
    })
}

fn fold(name: &str, folding: &str, native: bool) -> Fold {
    Fold {
        name: name.into(),
        folding: folding.into(),
        native,
        reason: String::new(),
    }
}

fn right_click(h: &mut Harness<'_, App>, label: &str) {
    let pos = h.get_by_label(label).rect().center();
    h.hover_at(pos);
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos,
            button: PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.step();
    }
    h.step();
    h.step();
}

#[test]
fn folding_indicator_screenshots() {
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("pqfold-shots");
    let path = project(dir.path());
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
        h.state_mut().ribbon_tab = RTab::Home;
        h.step();
        pq_editor::open(h.state_mut(), Vec::new(), None);
        wait(&mut h, "the editor", settled);
        // a local query: nothing folds
        let q = h.state().cmd.pq.clone().unwrap().queries[0].clone();
        assert!(
            matches!(&q.folding, Some(Ok(f)) if f.iter().all(|x| x.folding == "not_folded")),
            "{:?}",
            q.folding
        );
        save(&mut h, &shots, &format!("pqfold-local-{suffix}"));
        // the indicators a SQL Server query gets (query.folding's answer)
        if let Some(e) = h.state_mut().cmd.pq.as_mut()
            && let Some(q) = e.queries.get_mut(0)
        {
            q.folding = Some(Ok(vec![
                fold("Source", "folded", false),
                fold("dbo_Orders", "folded", true),
                fold("Removed Columns", "folded", true),
                fold("Filtered Rows", "folded", true),
                fold("Changed Type", "partial", false),
            ]));
        }
        h.step();
        h.step();
        save(&mut h, &shots, &format!("pqfold-steps-{suffix}"));
        // the context menu of a step that does not fold
        right_click(&mut h, "Changed Type");
        save(&mut h, &shots, &format!("pqfold-menu-{suffix}"));
        h.key_press(egui::Key::Escape);
        h.step();
        // View native query of a folded step (query.native's answer)
        if let Some(e) = h.state_mut().cmd.pq.as_mut() {
            e.native = Some(NativeView {
                step: "Filtered Rows".into(),
                text: Some(Ok((
                    "SELECT [id], [qty] FROM [dbo].[Orders] WHERE [qty] > 2".into(),
                    "sql".into(),
                ))),
            });
        }
        h.step();
        h.step();
        save(&mut h, &shots, &format!("pqfold-native-{suffix}"));
        // Diagnose step (query.diagnose's answer)
        if let Some(e) = h.state_mut().cmd.pq.as_mut() {
            e.native = None;
            e.diagnosis = Some(DiagView {
                step: "Filtered Rows".into(),
                result: Some(Ok(Diagnosis {
                    elapsed_ms: 42,
                    rows: Some(2),
                    error: None,
                    requests: vec![
                        ("list".into(), "sqlserver".into(), 2, 9, "Sales".into()),
                        (
                            "query".into(),
                            "sqlserver".into(),
                            2,
                            21,
                            "SELECT [id], [qty] FROM [dbo].[Orders] WHERE [qty] > 2".into(),
                        ),
                    ],
                })),
            });
        }
        h.step();
        h.step();
        save(&mut h, &shots, &format!("pqfold-diagnose-{suffix}"));
        h.state_mut().cmd.pq = None;
    }
}
