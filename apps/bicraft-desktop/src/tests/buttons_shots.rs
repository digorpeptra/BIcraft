//! Screenshots of buttons, shapes and navigators (docs/formats/buttons.md), drawn with wgpu
//! (`--features gpu-tests`), saved into `BICRAFT_DESKTOP_SHOTS` when set: a reader's page with
//! the pointer over a button in its On hover state, a disabled drillthrough button, a shape with
//! its text and a bookmark navigator (`btn-hover-*`); the action editor with a conditional (fx)
//! destination (`btn-action-*`); the Format pane's "Apply settings to: State" on a shape
//! (`btn-format-*`), in English (light and dark) and French.

use std::path::{Path, PathBuf};

use bicraft_engine::Session;
use bicraft_ui::registry::Tab as RTab;
use egui::accesskit::Role;
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

/// A page with a button (hover and press looks), a drillthrough button, a shape with text and a
/// bookmark navigator; a drillthrough page; measure [Where].
fn project(dir: &Path) -> PathBuf {
    let mut s = Session::new();
    let run = |s: &mut Session, id: &str, p: Value| s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    let sales = r#"#table({"Category", "Amount"}, {{"Wheels", 100}, {"Toys", 5}})"#;
    run(&mut s, "model.new", json!({"name": "Buttons"}));
    run(
        &mut s,
        "table.add",
        json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
    );
    run(
        &mut s,
        "column.add",
        json!({"table": "Sales", "name": "Category", "data_type": "string"}),
    );
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Where", "expression": "\"Details\""}),
    );
    run(&mut s, "report.new", json!({}));
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    run(&mut s, "page.add", json!({"display_name": "Details"}));
    let detail = s.report.as_ref().unwrap().pages[1].name.clone();
    run(&mut s, "page.set_active", json!({"page": page}));
    let add = |s: &mut Session, kind: &str, x: i64, y: i64, w: i64, h: i64| {
        run(
            s,
            "button.add",
            json!({"page": page, "kind": kind, "x": x, "y": y, "width": w, "height": h}),
        )["visual"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let go = add(&mut s, "blank", 60, 60, 300, 80);
    run(
        &mut s,
        "visual.format",
        json!({"page": page, "visual": go, "objects": [
            {"object": "text", "property": "show", "value": true},
            {"object": "text", "property": "text", "value": "Go to details"},
            {"object": "fill", "property": "show", "value": true},
            {"object": "fill", "property": "fillColor", "value": {"color": "#118DFF"}},
            {"object": "text", "property": "fontColor", "value": {"color": "#FFFFFF"}},
            {"object": "fill", "property": "fillColor", "value": {"color": "#0B5394"}, "state": "hover"},
            {"object": "text", "property": "text", "value": "Go!", "state": "hover"},
            {"object": "outline", "property": "show", "value": true, "state": "hover"},
            {"object": "outline", "property": "lineColor", "value": {"color": "#252423"}, "state": "hover"},
            {"object": "outline", "property": "weight", "value": 3, "state": "hover"},
        ]}),
    );
    run(
        &mut s,
        "visual.set_action",
        json!({"page": page, "visual": go, "type": "page_navigation", "destination_field": "[Where]"}),
    );
    run(
        &mut s,
        "page.set_type",
        json!({"page": detail, "type": "drillthrough", "fields": ["Sales[Category]"]}),
    );
    let drill = add(&mut s, "blank", 420, 60, 300, 80);
    run(
        &mut s,
        "visual.format",
        json!({"page": page, "visual": drill, "objects": [
            {"object": "text", "property": "show", "value": true},
            {"object": "text", "property": "text", "value": "Drill through"},
            {"object": "fill", "property": "show", "value": true},
            {"object": "fill", "property": "fillColor", "value": {"color": "#E66C37"}},
        ]}),
    );
    run(
        &mut s,
        "visual.set_action",
        json!({"page": page, "visual": drill, "type": "drillthrough", "destination": detail,
               "disabled_tooltip": "Select a category first"}),
    );
    let sh = run(
        &mut s,
        "shape.add",
        json!({"page": page, "shape": "rectangleRounded", "x": 780, "y": 40, "width": 360, "height": 120}),
    )["visual"]
        .as_str()
        .unwrap()
        .to_string();
    run(
        &mut s,
        "visual.format",
        json!({"page": page, "visual": sh, "objects": [
            {"object": "fill", "property": "fillColor", "value": {"color": "#F2C80F"}},
            {"object": "text", "property": "show", "value": true},
            {"object": "text", "property": "text", "value": "Shape text\nsecond line"},
            {"object": "text", "property": "fontSize", "value": 14},
            {"object": "fill", "property": "fillColor", "value": {"color": "#C8A000"}, "state": "press"},
        ]}),
    );
    for name in ["North", "South", "East"] {
        run(&mut s, "bookmark.capture", json!({"display_name": name}));
    }
    let nav = add(&mut s, "bookmark_navigator", 60, 260, 660, 60);
    run(
        &mut s,
        "visual.format",
        json!({"page": page, "visual": nav, "objects": [
            {"object": "bookmarks", "property": "allowDeselectionBookmark", "value": true},
            {"object": "fill", "property": "fillColor", "value": {"color": "#DDEEFF"}, "state": "hover"},
        ]}),
    );
    let path = dir.join("buttons.pbip");
    run(&mut s, "file.save_as", json!({"path": path}));
    path
}

fn steps(h: &mut Harness<'_, App>, n: usize) {
    for _ in 0..n {
        h.step();
    }
}

fn visual(h: &Harness<'_, App>, pick: impl Fn(&Value) -> bool) -> Value {
    h.state()
        .loaded
        .as_ref()
        .and_then(|l| {
            l.snap["report"]["pages"][0]["visuals"]
                .as_array()?
                .iter()
                .find(|v| pick(v))
                .cloned()
        })
        .expect("the visual")
}

fn centre(h: &Harness<'_, App>, v: &Value) -> egui::Pos2 {
    let placed = h.state().placed.expect("the page placed");
    let n = |k: &str| v[k].as_f64().unwrap_or(0.0) as f32;
    placed.rect.min + egui::vec2(n("x") + n("width") / 2.0, n("y") + n("height") / 2.0) * placed.zoom
}

#[test]
fn buttons_screenshots() {
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("buttons-shots");
    let path = project(dir.path());
    for (lang, suffix, dark, action, format, visual_cards) in [
        ("en-US", "en", false, "Action", "Format visual", "Visual cards"),
        (
            "fr-FR",
            "fr",
            false,
            "Action",
            "Mettre en forme le visuel",
            "Cartes du visuel",
        ),
        ("en-US", "dark", true, "Action", "Format visual", "Visual cards"),
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
        wait(&mut h, "the page placed", |a| {
            a.placed.is_some() && a.shown.is_some()
        });
        // a reader's pointer over the button: its On hover look
        h.state_mut().access = Some("reader".into());
        steps(&mut h, 3);
        let go = visual(&h, |v| v["button"]["text"] == "Go to details");
        let at = centre(&h, &go);
        h.hover_at(at);
        wait(&mut h, "the hover state", |a| a.pointer.is_some());
        steps(&mut h, 6);
        save(&mut h, &shots, &format!("btn-hover-{suffix}"));
        h.state_mut().access = None;
        // the button selected, the action editor with its fx destination
        let page = h.state().page_names()[0].0.clone();
        let name = go["name"].as_str().unwrap().to_string();
        bicraft_ui::canvas::select(h.state_mut(), &page, std::slice::from_ref(&name));
        h.state_mut().ribbon_tab = RTab::Visual;
        steps(&mut h, 3);
        h.get_by_label(action).click();
        steps(&mut h, 6);
        save(&mut h, &shots, &format!("btn-action-{suffix}"));
        h.state_mut().cmd.rich.action = None;
        steps(&mut h, 2);
        // the shape selected, the Format pane in its On press state
        let sh = visual(&h, |v| v["type"] == "shape");
        let sh = sh["name"].as_str().unwrap().to_string();
        bicraft_ui::canvas::select(h.state_mut(), &page, &[sh]);
        steps(&mut h, 3);
        h.get_by_role_and_label(Role::Button, format).click();
        steps(&mut h, 3);
        h.get_by_label(visual_cards).click();
        steps(&mut h, 4);
        h.state_mut().panes.format.state = "press".into();
        steps(&mut h, 6);
        save(&mut h, &shots, &format!("btn-format-{suffix}"));
    }
}
