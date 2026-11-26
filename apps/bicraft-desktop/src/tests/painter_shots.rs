//! Screenshots of Home › Format painter and the ribbon chart, drawn with wgpu (`--features
//! gpu-tests`), saved into `BICRAFT_DESKTOP_SHOTS` when set: a page with a formatted column chart,
//! a plain one, a ribbon chart with spacing, borders and total labels, and a Gantt visual (its
//! placeholder); the painter on with the brush over the plain chart; the plain chart painted
//! (`painter-*`), in English (light and dark) and French.

use std::path::{Path, PathBuf};

use bicraft_engine::Session;
use bicraft_ui::registry::Tab as RTab;
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

fn project(dir: &Path) -> PathBuf {
    let mut s = Session::new();
    let run = |s: &mut Session, id: &str, p: serde_json::Value| {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
    };
    run(&mut s, "model.new", json!({"name": "Shop"}));
    run(
        &mut s,
        "table.import",
        json!({"name": "Sales", "m": "let Source = #table(type table [Quarter = text, Product = text, Amount = Int64.Type], {{\"Q1\", \"Bikes\", 40}, {\"Q1\", \"Kites\", 25}, {\"Q1\", \"Pens\", 10}, {\"Q2\", \"Bikes\", 30}, {\"Q2\", \"Kites\", 45}, {\"Q2\", \"Pens\", 12}, {\"Q3\", \"Bikes\", 20}, {\"Q3\", \"Kites\", 35}, {\"Q3\", \"Pens\", 38}, {\"Q4\", \"Bikes\", 50}, {\"Q4\", \"Kites\", 15}, {\"Q4\", \"Pens\", 30}}) in Source"}),
    );
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
    );
    run(&mut s, "report.new", json!({}));
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    let chart = json!([{"role": "Category", "field": "Sales[Quarter]"}, {"role": "Y", "field": "[Total]"}]);
    let mut names = Vec::new();
    for (ty, rect, fields, title) in [
        (
            "clusteredColumnChart",
            [20, 20, 560, 300],
            chart.clone(),
            "Formatted",
        ),
        ("clusteredColumnChart", [600, 20, 560, 300], chart, "Plain"),
        (
            "ribbonChart",
            [20, 340, 760, 360],
            json!([{"role": "Category", "field": "Sales[Quarter]"},
                   {"role": "Series", "field": "Sales[Product]"},
                   {"role": "Y", "field": "[Total]"}]),
            "Ribbons",
        ),
        ("ganttChart", [800, 340, 360, 360], json!([]), "Schedule"),
    ] {
        let out = run(
            &mut s,
            "visual.add",
            json!({"page": page, "visual_type": ty, "x": rect[0], "y": rect[1], "width": rect[2],
                   "height": rect[3], "fields": fields}),
        );
        let v = out["visual"].as_str().unwrap().to_string();
        run(
            &mut s,
            "visual.set_title",
            json!({"page": page, "visual": v, "title": title}),
        );
        names.push(v);
    }
    run(
        &mut s,
        "visual.set_title",
        json!({"page": page, "visual": names[0], "font_size": 16, "color": "#B03A2E", "bold": true}),
    );
    run(
        &mut s,
        "visual.format",
        json!({"page": page, "visual": names[0], "background": {"show": true, "color": "#FFF4E0"},
               "border": {"show": true, "color": "#B03A2E", "width": 2},
               "objects": [{"object": "labels", "property": "show", "value": true},
                           {"object": "dataPoint", "property": "fill", "value": {"color": "#B03A2E"}}]}),
    );
    run(
        &mut s,
        "visual.format",
        json!({"page": page, "visual": names[2], "objects": [
            {"object": "ribbonBands", "property": "borderShow", "value": true},
            {"object": "layout", "property": "ribbonGapSize", "value": 6},
            {"object": "totals", "property": "show", "value": true},
            {"object": "labels", "property": "show", "value": true}]}),
    );
    let path = dir.join("painter.pbip");
    run(&mut s, "file.save_as", json!({"path": path}));
    path
}

fn steps(h: &mut Harness<'_, App>, n: usize) {
    for _ in 0..n {
        h.step();
    }
}

/// The screen centre of the visual titled `title` on the first page.
fn centre(h: &Harness<'_, App>, title: &str) -> (String, egui::Pos2) {
    let app = h.state();
    let v = app.loaded.as_ref().and_then(|l| {
        l.snap["report"]["pages"][0]["visuals"]
            .as_array()?
            .iter()
            .find(|v| v["title"] == title)
            .cloned()
    });
    let v = v.expect("the visual");
    let n = |k: &str| v[k].as_f64().unwrap_or(0.0) as f32;
    let pl = app.placed.expect("the page is placed");
    let p = egui::pos2(n("x") + n("width") / 2.0, n("y") + n("height") / 2.0);
    (
        v["name"].as_str().unwrap_or("").to_string(),
        pl.rect.min + p.to_vec2() * pl.zoom,
    )
}

fn click(h: &mut Harness<'_, App>, pos: egui::Pos2) {
    h.hover_at(pos);
    h.step();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        h.step();
    }
    steps(h, 2);
}

#[test]
fn format_painter_screenshots() {
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("painter-shots");
    let path = project(dir.path());
    for (lang, suffix, dark, button) in [
        ("en-US", "en", false, "Format painter"),
        ("fr-FR", "fr", false, "Copier la mise en forme"),
        ("en-US", "dark", true, "Format painter"),
    ] {
        let mut app = crate::app(lang, None).with_sync_render();
        app.open_path(&path);
        let mut h = Harness::builder()
            .with_size(egui::vec2(1500.0, 900.0))
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
        steps(&mut h, 6);
        save(&mut h, &shots, &format!("painter-page-{suffix}"));
        let (_, from) = centre(&h, "Formatted");
        let (plain, to) = centre(&h, "Plain");
        click(&mut h, from);
        h.get_by_label(button).click();
        steps(&mut h, 3);
        assert!(h.state().canvas.painter.is_some(), "{suffix}: the painter is on");
        h.hover_at(to);
        steps(&mut h, 3);
        save(&mut h, &shots, &format!("painter-on-{suffix}"));
        click(&mut h, to);
        wait(&mut h, "the painted snapshot", |a| {
            a.canvas.painter.is_none() && ready(a) && a.cmd.visual.as_ref().is_some_and(|v| v.visual == plain)
        });
        steps(&mut h, 6);
        save(&mut h, &shots, &format!("painter-done-{suffix}"));
    }
}
