//! Screenshots of Insert › Shapes, drawn with wgpu (`--features gpu-tests`), saved into
//! `BICRAFT_DESKTOP_SHOTS` when set: a page of shapes (filled, outlined, rotated, a line) with
//! the Shapes menu open, then a shape selected with the Format pane's Shape and Fill cards
//! (`shapes-*`), in English (light and dark) and French.

use std::path::{Path, PathBuf};

use bicraft_engine::Session;
use bicraft_ui::registry::Tab as RTab;
use egui::accesskit::Role;
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

/// A report whose page holds one shape of each kind shown, some formatted.
fn project(dir: &Path) -> PathBuf {
    let mut s = Session::new();
    let run = |s: &mut Session, id: &str, p: serde_json::Value| {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
    };
    run(&mut s, "model.new", json!({"name": "Shapes"}));
    run(&mut s, "report.new", json!({}));
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    let tiles = [
        "rectangle",
        "rectangleRounded",
        "oval",
        "triangleIsoc",
        "arrow",
        "heart",
        "pentagon",
        "hexagon",
        "speechbubbleRectangle",
        "arrowChevron",
        "tabCutCorner",
        "pill",
    ];
    for (i, tile) in tiles.iter().enumerate() {
        let (x, y) = (40 + (i % 4) as i64 * 300, 40 + (i / 4) as i64 * 210);
        let out = run(
            &mut s,
            "shape.add",
            json!({"page": page, "shape": tile, "x": x, "y": y, "width": 240, "height": 160}),
        );
        let v = out["visual"].as_str().unwrap().to_string();
        let color =
            |hex: &str| json!({"solid": {"color": {"expr": {"Literal": {"Value": format!("'{hex}'")}}}}});
        let objects = match i % 3 {
            1 => json!([{"object": "fill", "property": "fillColor", "value": color("#E66C37")},
                        {"object": "outline", "property": "lineColor", "value": color("#252423")},
                        {"object": "outline", "property": "weight", "value": 3}]),
            2 => json!([{"object": "fill", "property": "show", "value": false},
                        {"object": "outline", "property": "lineColor", "value": color("#118DFF")},
                        {"object": "outline", "property": "weight", "value": 4},
                        {"object": "rotation", "property": "shapeAngle", "value": 20}]),
            _ => json!([]),
        };
        if objects.as_array().is_some_and(|a| !a.is_empty()) {
            run(
                &mut s,
                "visual.format",
                json!({"page": page, "visual": v, "objects": objects}),
            );
        }
    }
    run(
        &mut s,
        "shape.add",
        json!({"page": page, "shape": "line", "x": 40, "y": 660, "width": 1100, "height": 40}),
    );
    let path = dir.join("shapes.pbip");
    run(&mut s, "file.save_as", json!({"path": path}));
    path
}

fn steps(h: &mut Harness<'_, App>, n: usize) {
    for _ in 0..n {
        h.step();
    }
}

#[test]
fn shapes_screenshots() {
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("shapes-shots");
    let path = project(dir.path());
    for (lang, suffix, dark, shapes, format, visual_cards, shape_card) in [
        (
            "en-US",
            "en",
            false,
            "Shapes",
            "Format visual",
            "Visual cards",
            "Shape",
        ),
        (
            "fr-FR",
            "fr",
            false,
            "Formes",
            "Mettre en forme le visuel",
            "Cartes du visuel",
            "Forme",
        ),
        (
            "en-US",
            "dark",
            true,
            "Shapes",
            "Format visual",
            "Visual cards",
            "Shape",
        ),
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
        h.state_mut().ribbon_tab = RTab::Insert;
        steps(&mut h, 3);
        h.get_by_label(shapes).click();
        steps(&mut h, 4);
        save(&mut h, &shots, &format!("shapes-menu-{suffix}"));
        h.key_press(egui::Key::Escape);
        steps(&mut h, 3);
        // the heart selected, its Format pane
        let heart = h
            .state()
            .loaded
            .as_ref()
            .and_then(|l| {
                l.snap["report"]["pages"][0]["visuals"]
                    .as_array()?
                    .iter()
                    .find(|v| v["shape"]["tile"] == "heart")
                    .and_then(|v| v["name"].as_str().map(str::to_string))
            })
            .expect("the heart");
        let page = h.state().page_names()[0].0.clone();
        bicraft_ui::canvas::select(h.state_mut(), &page, &[heart]);
        steps(&mut h, 3);
        h.get_by_role_and_label(Role::Button, format).click();
        steps(&mut h, 3);
        h.get_by_label(visual_cards).click();
        steps(&mut h, 4);
        h.get_by_role_and_label(Role::Button, shape_card).click();
        steps(&mut h, 6);
        save(&mut h, &shots, &format!("shapes-format-{suffix}"));
    }
}
