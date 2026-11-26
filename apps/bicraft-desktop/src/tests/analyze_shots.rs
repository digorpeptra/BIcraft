//! Screenshots of Analyze › Best practices with report findings (`m1139-*`), drawn with wgpu
//! (`--features gpu-tests`), saved into `BICRAFT_DESKTOP_SHOTS` when set: a measure removed from
//! the model leaves a card bound to it and a filter on a renamed column; the answer window lists
//! them with Go to visual and Fix, then Go to visual shows the card selected on its page. In
//! English (light and dark) and French.

use std::path::{Path, PathBuf};

use bicraft_engine::Session;
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

fn project(dir: &Path) -> PathBuf {
    let mut s = Session::new();
    let run = |s: &mut Session, id: &str, p: serde_json::Value| {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
    };
    let sales = r#"#table({"Region", "Amount"}, {{"North", 100}, {"South", 40}})"#;
    run(&mut s, "model.new", json!({"name": "Shop"}));
    run(
        &mut s,
        "table.add",
        json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
    );
    run(
        &mut s,
        "column.add",
        json!({"table": "Sales", "name": "Region", "data_type": "string"}),
    );
    run(
        &mut s,
        "column.add",
        json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
    );
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
    );
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Margin", "expression": "[Total] * 0.2"}),
    );
    run(&mut s, "report.new", json!({}));
    run(&mut s, "page.add", json!({"display_name": "Details"}));
    let pages: Vec<String> = s
        .report
        .as_ref()
        .unwrap()
        .pages
        .iter()
        .map(|p| p.name.clone())
        .collect();
    run(
        &mut s,
        "visual.add",
        json!({"page": pages[0], "visual_type": "clusteredColumnChart", "x": 20, "y": 20, "width": 480,
               "height": 300, "fields": [{"role": "Category", "field": "Sales[Region]"},
                                         {"role": "Y", "field": "[Total]"}]}),
    );
    run(
        &mut s,
        "visual.add",
        json!({"page": pages[1], "visual_type": "card", "x": 40, "y": 40, "width": 240, "height": 120,
               "fields": [{"role": "Values", "field": "[Margin]"}]}),
    );
    run(
        &mut s,
        "page.filter",
        json!({"page": pages[0], "field": "Sales[Region]", "in": ["North"]}),
    );
    run(&mut s, "measure.remove", json!({"name": "Margin", "force": true}));
    run(
        &mut s,
        "column.rename",
        json!({"table": "Sales", "name": "Region", "new_name": "Area"}),
    );
    let path = dir.join("analyze.pbip");
    run(&mut s, "file.save_as", json!({"path": path}));
    path
}

#[test]
fn best_practices_report_findings_screenshots() {
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("analyze-shots");
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
        let c = bicraft_ui::registry::find("model.analyze").unwrap();
        bicraft_ui::ribbon::send(h.state_mut(), c, json!({}));
        wait(&mut h, "the analysis", |a| {
            matches!(&a.cmd.result, Some((_, Some(Ok(_)))))
        });
        let n = match &h.state().cmd.result {
            Some((_, Some(Ok(v)))) => bicraft_ui::analyze_ui::report_findings(v).len(),
            _ => 0,
        };
        assert!(n >= 3, "{n} report findings");
        h.step();
        save(&mut h, &shots, &format!("m1139-analyze-{suffix}"));
        let card = h
            .state()
            .loaded
            .as_ref()
            .and_then(|l| l.snap.pointer("/report/pages/1/visuals/0/name"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap();
        let page = h.state().page_names()[1].0.clone();
        bicraft_ui::analyze_ui::apply(
            h.state_mut(),
            bicraft_ui::analyze_ui::Act::Go {
                page,
                visual: Some(card.clone()),
            },
        );
        wait(&mut h, "the card selected", |a| {
            ready(a) && a.cmd.visual.as_ref().is_some_and(|v| v.visual == card)
        });
        save(&mut h, &shots, &format!("m1139-goto-{suffix}"));
    }
}
