//! Screenshots of map visuals, drawn with wgpu (`--features gpu-tests`), saved into
//! `BICRAFT_DESKTOP_SHOTS` when set: a page with a bubble map of French cities on the Natural
//! Earth outlines, a shape map on the built-in World countries (Natural Earth) map zoomed on
//! European countries' sales, a filled map of the same countries, a table and a map of city
//! names only (placed offline at Natural Earth's populated places); the page with Paris selected (cross-filtered); the
//! shape map's Format › Map settings card (`maps-*`), in English (light and dark) and French.

use std::path::{Path, PathBuf};

use bicraft_engine::Session;
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

fn project(dir: &Path) -> PathBuf {
    let mut s = Session::new();
    let run = |s: &mut Session, id: &str, p: serde_json::Value| {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
    };
    run(&mut s, "model.new", json!({"name": "Geo"}));
    run(
        &mut s,
        "table.import",
        json!({"name": "Shops", "m": "let Source = #table(type table [City = text, Lat = number, Lon = number, Amount = Int64.Type], {{\"Paris\", 48.85, 2.35, 120}, {\"Lille\", 50.63, 3.06, 40}, {\"Lyon\", 45.76, 4.83, 70}, {\"Marseille\", 43.3, 5.37, 55}, {\"Rennes\", 48.11, -1.68, 30}, {\"Strasbourg\", 48.57, 7.75, 25}, {\"Toulouse\", 43.6, 1.44, 45}}) in Source"}),
    );
    run(
        &mut s,
        "measure.add",
        json!({"table": "Shops", "name": "Sales", "expression": "SUM(Shops[Amount])"}),
    );
    // countries by name, and two by ISO code (CHE, PRT)
    run(
        &mut s,
        "table.import",
        json!({"name": "Countries", "m": "let Source = #table(type table [Country = text, Amount = Int64.Type], {{\"France\", 120}, {\"Germany\", 95}, {\"Spain\", 60}, {\"Italy\", 70}, {\"Belgium\", 25}, {\"CHE\", 20}, {\"Netherlands\", 30}, {\"PRT\", 15}, {\"United Kingdom\", 50}, {\"Poland\", 35}}) in Source"}),
    );
    run(
        &mut s,
        "measure.add",
        json!({"table": "Countries", "name": "Country sales", "expression": "SUM(Countries[Amount])"}),
    );
    run(&mut s, "report.new", json!({}));
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    for (ty, rect, fields, title) in [
        (
            "map",
            [20, 20, 600, 330],
            json!([{"role": "Category", "field": "Shops[City]"},
                   {"role": "Y", "field": "Average(Shops[Lat])"},
                   {"role": "X", "field": "Average(Shops[Lon])"},
                   {"role": "Size", "field": "[Sales]"}]),
            "Sales by city",
        ),
        (
            "shapeMap",
            [640, 20, 620, 330],
            json!([{"role": "Category", "field": "Countries[Country]"}, {"role": "Value", "field": "[Country sales]"}]),
            "Sales by country",
        ),
        (
            "filledMap",
            [20, 370, 400, 330],
            json!([{"role": "Category", "field": "Countries[Country]"}]),
            "Countries",
        ),
        (
            "tableEx",
            [440, 370, 360, 330],
            json!([{"role": "Values", "field": "Shops[City]"}, {"role": "Values", "field": "[Sales]"}]),
            "Shops",
        ),
        (
            "map",
            [820, 370, 440, 330],
            json!([{"role": "Category", "field": "Shops[City]"}]),
            "Cities by name",
        ),
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
        if ty == "shapeMap" {
            run(
                &mut s,
                "shape_map.set_map",
                json!({"page": page, "visual": v, "builtin": "world_countries"}),
            );
            run(
                &mut s,
                "visual.format",
                json!({"page": page, "visual": v, "objects": [
                    {"object": "zoom", "property": "autoZoom", "value": true}]}),
            );
        }
    }
    let path = dir.join("maps.pbip");
    run(&mut s, "file.save_as", json!({"path": path}));
    path
}

fn steps(h: &mut Harness<'_, App>, n: usize) {
    for _ in 0..n {
        h.step();
    }
}

fn visual_of(h: &Harness<'_, App>, ty: &str) -> String {
    h.state()
        .loaded
        .as_ref()
        .and_then(|l| {
            l.snap["report"]["pages"][0]["visuals"]
                .as_array()?
                .iter()
                .find(|v| v["type"] == ty)
                .and_then(|v| v["name"].as_str().map(str::to_string))
        })
        .expect("the visual")
}

#[test]
fn maps_screenshots() {
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("maps-shots");
    let path = project(dir.path());
    for (lang, suffix, dark, format, visual_cards, settings) in [
        (
            "en-US",
            "en",
            false,
            "Format visual",
            "Visual cards",
            "Map settings",
        ),
        (
            "fr-FR",
            "fr",
            false,
            "Mettre en forme le visuel",
            "Cartes du visuel",
            "Paramètres de la carte",
        ),
        (
            "en-US",
            "dark",
            true,
            "Format visual",
            "Visual cards",
            "Map settings",
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
        steps(&mut h, 6);
        save(&mut h, &shots, &format!("maps-page-{suffix}"));
        // Paris clicked on the bubble map: the page cross-filtered
        let page = h.state().page_names()[0].0.clone();
        let map = visual_of(&h, "map");
        h.state_mut().interact(
            json!({"action": "select", "page": page, "visual": map, "point": [{"c": 0, "v": "Paris"}], "ctrl": false}),
        );
        steps(&mut h, 12);
        save(&mut h, &shots, &format!("maps-selected-{suffix}"));
        h.state_mut()
            .interact(json!({"action": "clear_selection", "page": page}));
        steps(&mut h, 6);
        // the shape map's Map settings
        let sm = visual_of(&h, "shapeMap");
        bicraft_ui::canvas::select(h.state_mut(), &page, &[sm]);
        steps(&mut h, 3);
        h.get_by_role_and_label(Role::Button, format).click();
        steps(&mut h, 3);
        h.get_by_label(visual_cards).click();
        steps(&mut h, 4);
        h.get_by_role_and_label(Role::Button, settings).click();
        steps(&mut h, 6);
        save(&mut h, &shots, &format!("maps-format-{suffix}"));
    }
}
