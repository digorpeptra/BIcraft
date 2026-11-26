//! Screenshots of a matrix's subtotals and expansion drawn with wgpu (`--features gpu-tests`),
//! saved into `BICRAFT_DESKTOP_SHOTS` when set (`matrix-*`): a three-level matrix expanded with
//! `visual.expand` (one member collapsed), its +/- buttons, per-level subtotals at the bottom
//! with a level's label, and the Format pane's Row subtotals card with its per-level rows, in
//! English (light and dark) and French; then the right-click Expand submenu.

use std::path::{Path, PathBuf};

use bicraft_engine::Session;
use bicraft_ui::format_pane::{Group, Tab};
use bicraft_ui::ribbon::VisualSel;
use egui::{Event, Modifiers, PointerButton};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::{Temp, ready, visual, wait};
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

/// Sales(Region, Category, Product, Amount); a matrix `m` on Region > Category > Product with
/// +/- buttons, every level expanded but South, the Category subtotals at the bottom labelled
/// "Sous-total" and the grand total kept; saved as a PBIP project.
fn project(dir: &Path) -> PathBuf {
    let mut s = Session::new();
    let sales = r#"#table({"Region", "Category", "Product", "Amount"}, {
        {"North", "Bikes", "Road", 150}, {"North", "Bikes", "Mountain", 200}, {"North", "Toys", "Ball", 10},
        {"South", "Bikes", "Road", 300}, {"South", "Toys", "Kite", 100}})"#;
    let run = |s: &mut Session, id: &str, p: Value| s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    run(&mut s, "model.new", json!({"name": "Shop"}));
    run(
        &mut s,
        "table.add",
        json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
    );
    for (c, ty) in [
        ("Region", "string"),
        ("Category", "string"),
        ("Product", "string"),
        ("Amount", "double"),
    ] {
        run(
            &mut s,
            "column.add",
            json!({"table": "Sales", "name": c, "data_type": ty}),
        );
    }
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])", "format_string": "#,0"}),
    );
    run(&mut s, "report.new", json!({}));
    let page = s
        .report
        .as_ref()
        .map(|r| r.pages[0].name.clone())
        .unwrap_or_default();
    let r = run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "pivotTable", "x": 30, "y": 30, "width": 460, "height": 440,
               "fields": [{"role": "Rows", "field": "Sales[Region]"}, {"role": "Rows", "field": "Sales[Category]"},
                          {"role": "Rows", "field": "Sales[Product]"}, {"role": "Values", "field": "[Total]"}]}),
    );
    let made = r["visual"].as_str().unwrap_or_default().to_string();
    if let Some(v) = s
        .report
        .as_mut()
        .and_then(|r| r.pages.first_mut())
        .and_then(|p| p.visuals.iter_mut().find(|v| v.name == made))
    {
        v.name = "m".into();
    }
    run(
        &mut s,
        "visual.format",
        json!({"page": page, "visual": "m", "objects": [
            {"object": "rowHeaders", "property": "showExpandCollapseButtons", "value": true},
            {"object": "subTotals", "property": "perRowLevel", "value": true},
            {"object": "subTotals", "property": "rowSubtotalsPosition", "value": "Bottom"},
            {"object": "subTotals", "property": "levelSubtotalLabel", "value": "Sous-total",
             "selector": {"metadata": "Sales.Category"}}]}),
    );
    run(
        &mut s,
        "visual.expand",
        json!({"page": page, "visual": "m", "all": true}),
    );
    run(
        &mut s,
        "visual.expand",
        json!({"page": page, "visual": "m", "path": ["South"], "collapse": true}),
    );
    let path = dir.join("matrix.pbip");
    run(&mut s, "file.save_as", json!({"path": path}));
    path
}

fn page_name(a: &App) -> String {
    a.loaded
        .as_ref()
        .and_then(|l| l.snap["report"]["pages"][0]["name"].as_str().map(str::to_string))
        .unwrap_or_default()
}

#[test]
fn matrix_screenshots() {
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("matrix-shots");
    let path = project(dir.path());
    for (lang, suffix, dark) in [
        ("en-US", "en", false),
        ("fr-FR", "fr", false),
        ("en-US", "dark", true),
    ] {
        let mut app = crate::app(lang, None).with_sync_render();
        app.prefs.panes.data = false;
        app.prefs.panes.filters = false;
        app.open_path(&path);
        let mut h = Harness::builder()
            .with_size(egui::vec2(1400.0, 1080.0))
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
        let d = visual(h.state().loaded.as_ref().map(|l| l.snap.as_ref()), "m")["data"].clone();
        assert_eq!(
            d["row_expansion"],
            json!({"open": [true, true, false], "toggled": [["South"]]}),
            "{d}"
        );
        // the matrix selected, the Format visual tab on Row subtotals
        let page = page_name(h.state());
        h.state_mut().cmd.visual = Some(VisualSel {
            page,
            visual: "m".into(),
        });
        h.state_mut().panes.format.tab = Tab::Format;
        h.state_mut().panes.format.group = Group::Visual;
        h.state_mut().panes.format.open.insert("rowSubtotals".into());
        for _ in 0..20 {
            h.step();
        }
        wait(&mut h, "the snapshot", ready);
        h.step();
        let show = if suffix == "fr" {
            "Sous-totaux de lignes Category Afficher le sous-total"
        } else {
            "Row subtotals Category Show subtotal"
        };
        assert!(h.query_by_label(show).is_some(), "{show}");
        save(&mut h, &shots, &format!("matrix-subtotals-{suffix}"));
        // the right-click menu on South's row header, its Expand submenu open
        let at = {
            let (_, img) = h.state().shown.as_ref().unwrap();
            let img = img.as_ref().unwrap();
            let m = img
                .hits
                .iter()
                .find(|x| {
                    matches!(&x.action, bicraft_render::hits::HitAction::Menu(m) if m["path"] == json!(["South"]))
                })
                .unwrap();
            let (x, y, w, hh) = m.shape.bounds();
            let p = h.state().placed.unwrap();
            egui::pos2(
                p.rect.min.x + (x + w - 4.0) * p.zoom,
                p.rect.min.y + (y + hh / 2.0) * p.zoom,
            )
        };
        h.hover_at(at);
        h.step();
        for pressed in [true, false] {
            h.event_modifiers(
                Event::PointerButton {
                    pos: at,
                    button: PointerButton::Secondary,
                    pressed,
                    modifiers: Modifiers::NONE,
                },
                Modifiers::NONE,
            );
            h.step();
        }
        h.step();
        let expand = if suffix == "fr" { "Développer" } else { "Expand" };
        let r = h.get_by_label(expand).rect();
        h.hover_at(r.center());
        for _ in 0..4 {
            h.step();
        }
        save(&mut h, &shots, &format!("matrix-menu-{suffix}"));
    }
}
