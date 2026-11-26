//! Screenshots of Home › Transform data, the Power Query editor, drawn with wgpu (`--features
//! gpu-tests`), saved into `BICRAFT_DESKTOP_SHOTS` when set: the queries, applied steps,
//! formula bar and preview with a Filter rows form open, then a merge previewed with its table
//! column, the Advanced editor, a column filter menu, the queries list renaming a duplicate with
//! a deletion asked, and the question before changes are discarded (`pq-editor2-*`), in English
//! (light and dark) and French.

use std::path::{Path, PathBuf};

use bicraft_engine::Session;
use bicraft_ui::pq_editor::{self, Confirm, Form};
use bicraft_ui::registry::Tab as RTab;
use egui_kittest::Harness;
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

const SALES: &str = "let\n    // typed in\n    Source = #table(type table [Product = text, Qty = Int64.Type, City = text, Day = date], {{\"Bike\", 2, \"Paris\", #date(2026, 3, 1)}, {\"Car\", 1, \"Lyon\", #date(2026, 3, 2)}, {\"Kite\", 7, \"Paris\", #date(2026, 3, 4)}, {\"Pen\", 9, null, #date(2026, 3, 9)}}),\n    Kept = Table.SelectRows(Source, each [Qty] > 0),\n    #\"Sorted Rows\" = Table.Sort(Kept,{{\"Qty\", Order.Descending}})\nin\n    #\"Sorted Rows\"";

fn project(dir: &Path) -> PathBuf {
    let mut s = Session::new();
    let run = |s: &mut Session, id: &str, p: serde_json::Value| {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
    };
    run(&mut s, "model.new", json!({"name": "Shop"}));
    run(&mut s, "table.import", json!({"name": "Sales", "m": SALES}));
    run(
        &mut s,
        "expression.add",
        json!({"name": "Cities", "expression": "#table(type table [City = text, Country = text], {{\"Paris\", \"France\"}, {\"Lyon\", \"France\"}})"}),
    );
    run(&mut s, "report.new", json!({}));
    let path = dir.join("pq.pbip");
    run(&mut s, "file.save_as", json!({"path": path}));
    path
}

fn settled(a: &App) -> bool {
    a.cmd
        .pq
        .as_ref()
        .is_some_and(|e| !e.busy && e.step.is_some() && e.preview.is_some())
}

#[test]
fn power_query_editor_screenshots() {
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("pq-shots");
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
        // City picked, Filter rows asked
        if let Some(e) = h.state_mut().cmd.pq.as_mut() {
            e.columns = vec!["City".into()];
            e.form = Some(Form::Filter {
                column: "City".into(),
                op: 0,
                value: "Paris".into(),
            });
        }
        h.step();
        h.step();
        let e = h.state().cmd.pq.clone().unwrap();
        assert!(matches!(e.preview, Some(Ok(_))), "{:?}", e.preview);
        save(&mut h, &shots, &format!("pq-editor-{suffix}"));
        // merged with Cities: the table column
        if let Some(e) = h.state_mut().cmd.pq.as_mut() {
            e.form = None;
        }
        pq_editor::transform(
            h.state_mut(),
            json!({"kind": "merge", "other": "Cities", "keys": ["City"], "other_keys": ["City"]}),
        );
        wait(&mut h, "the merge", |a| {
            settled(a) && a.cmd.pq.as_ref().and_then(|e| e.step.clone()).as_deref() == Some("Merged Queries")
        });
        h.step();
        let e = h.state().cmd.pq.clone().unwrap();
        assert!(matches!(e.preview, Some(Ok(_))), "{:?}", e.preview);
        save(&mut h, &shots, &format!("pq-editor-merge-{suffix}"));
        // the Advanced editor over the whole query
        if let Some(e) = h.state_mut().cmd.pq.as_mut() {
            e.advanced = e.queries.first().map(|q| q.draft.clone());
        }
        h.step();
        h.step();
        save(&mut h, &shots, &format!("pq-editor2-advanced-{suffix}"));
        if let Some(e) = h.state_mut().cmd.pq.as_mut() {
            e.advanced = None;
        }
        // City's filter menu at step Kept, Lyon left out
        pq_editor::select_step(h.state_mut(), "Kept");
        wait(&mut h, "step Kept", |a| {
            settled(a) && a.cmd.pq.as_ref().and_then(|e| e.step.clone()).as_deref() == Some("Kept")
        });
        pq_editor::open_filter(h.state_mut(), "City");
        wait(&mut h, "the values", |a| {
            a.cmd
                .pq
                .as_ref()
                .and_then(|e| e.filter.as_ref())
                .is_some_and(|f| matches!(f.values, Some(Ok(_))))
        });
        if let Some(Some(Ok(v))) = h
            .state_mut()
            .cmd
            .pq
            .as_mut()
            .and_then(|e| e.filter.as_mut())
            .map(|f| f.values.as_mut())
            && let Some(x) = v.iter_mut().find(|x| x.0 == json!("Lyon"))
        {
            x.1 = false;
        }
        h.step();
        h.step();
        save(&mut h, &shots, &format!("pq-editor2-filter-{suffix}"));
        if let Some(e) = h.state_mut().cmd.pq.as_mut() {
            e.filter = None;
        }
        // a duplicate renamed, Cities about to be deleted
        pq_editor::new_query_of(h.state_mut(), "duplicate");
        wait(&mut h, "the duplicate", |a| {
            settled(a) && a.cmd.pq.as_ref().is_some_and(|e| e.queries.len() == 3)
        });
        if let Some(e) = h.state_mut().cmd.pq.as_mut() {
            e.renaming = Some("Sales by city".into());
            e.confirm = Some(Confirm::Delete(1));
        }
        h.step();
        h.step();
        save(&mut h, &shots, &format!("pq-editor2-queries-{suffix}"));
        // closing with changes asks first
        if let Some(e) = h.state_mut().cmd.pq.as_mut() {
            e.renaming = None;
            e.confirm = Some(Confirm::Close);
        }
        h.step();
        h.step();
        save(&mut h, &shots, &format!("pq-editor2-close-{suffix}"));
        h.state_mut().cmd.pq = None;
    }
}

/// The View tab's data profiling (`pqprof-*`): column quality and distribution under the
/// headers, the column profile of Qty in the bottom pane.
#[test]
fn power_query_profiling_screenshots() {
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("pqprof-shots");
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
        assert!(pq_editor::intercept(h.state_mut(), "query.profile", &json!({})));
        wait(&mut h, "the editor", settled);
        if let Some(e) = h.state_mut().cmd.pq.as_mut() {
            e.columns = vec!["Qty".into()];
        }
        pq_editor::refresh_profile(h.state_mut());
        wait(&mut h, "the profile", |a| {
            a.cmd
                .pq
                .as_ref()
                .and_then(|e| e.profiled.as_ref())
                .is_some_and(|p| p.as_ref().is_ok_and(|p| p.column.is_some()))
        });
        h.step();
        h.step();
        save(&mut h, &shots, &format!("pqprof-view-{suffix}"));
        h.state_mut().cmd.pq = None;
    }
}

#[test]
fn power_query_dependencies_screenshots() {
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("pqdeps-shots");
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
        assert!(pq_editor::intercept(
            h.state_mut(),
            "query.dependencies",
            &json!({})
        ));
        wait(&mut h, "the graph", |a| {
            a.cmd
                .pq
                .as_ref()
                .and_then(|e| e.deps.as_ref())
                .is_some_and(|d| matches!(d.graph, Some(Ok(_))))
        });
        h.step();
        h.step();
        save(&mut h, &shots, &format!("pqprof-deps-{suffix}"));
        h.state_mut().cmd.pq = None;
    }
}

/// The Advanced editor before and after Format (`m-format-*`, ROADMAP M11.35).
#[test]
fn m_format_screenshots() {
    const MESSY: &str = "let Source=#table(type table [Product=text,Qty=Int64.Type],{{\"Bike\",2},{\"Car\",1}}), // typed in\nKept=Table.SelectRows(Source,each [Qty]>0 and [Product]<>\"Pen\"),#\"Sorted Rows\"=Table.Sort(Kept,{{\"Qty\",Order.Descending}}) in #\"Sorted Rows\"";
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("m-format-shots");
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
        if let Some(e) = h.state_mut().cmd.pq.as_mut() {
            e.advanced = Some(MESSY.into());
        }
        h.step();
        h.step();
        save(&mut h, &shots, &format!("m-format-before-{suffix}"));
        pq_editor::format_text(h.state_mut(), true);
        wait(&mut h, "the formatted text", |a| {
            a.cmd
                .pq
                .as_ref()
                .is_some_and(|e| !e.busy && e.advanced.as_deref() != Some(MESSY))
        });
        h.step();
        h.step();
        let e = h.state().cmd.pq.clone().unwrap();
        assert!(e.problem.is_none(), "{:?}", e.problem);
        assert!(
            e.advanced
                .as_deref()
                .is_some_and(|t| t.starts_with("let\n    Source = ")),
            "{:?}",
            e.advanced
        );
        save(&mut h, &shots, &format!("m-format-after-{suffix}"));
        h.state_mut().cmd.pq = None;
    }
}
