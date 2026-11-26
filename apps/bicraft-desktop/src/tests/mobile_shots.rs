//! Screenshots of View › Mobile layout drawn with wgpu (`--features gpu-tests`), saved into
//! `BICRAFT_DESKTOP_SHOTS` when set (docs/desktop.md, "Mobile layout"): the phone canvas with
//! the Page visuals pane, a phone visual selected with its Format pane (`mobile-*`), in English
//! (light and dark) and French, and a narrow window reading the phone layout.

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

/// A page with a card, a column chart and a slicer; the card and the chart on the phone, the
/// card with a smaller title there.
fn project(dir: &Path) -> PathBuf {
    let mut s = Session::new();
    let sales =
        r#"#table({"Category", "Amount"}, {{"Wheels", 150}, {"Toys", 40}, {"Books", 90}, {"Games", 65}})"#;
    let run = |s: &mut Session, id: &str, p: Value| s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Category", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])", "format_string": "#,0"}),
        ),
        ("report.new", json!({})),
    ] {
        run(&mut s, id, p);
    }
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    let mut add = |ty: &str, (x, y, w, h): (i32, i32, i32, i32), binds: &[(&str, &str)]| {
        let r = run(
            &mut s,
            "visual.add",
            json!({"page": page, "visual_type": ty, "x": x, "y": y, "width": w, "height": h, "fields": []}),
        );
        let v = r["visual"].as_str().unwrap().to_string();
        for (role, field) in binds {
            run(
                &mut s,
                "visual.bind",
                json!({"page": page, "visual": v, "role": role, "field": field}),
            );
        }
        v
    };
    let card = add("card", (40, 40, 300, 180), &[("Values", "[Total]")]);
    let chart = add(
        "clusteredColumnChart",
        (380, 40, 700, 400),
        &[("Category", "Sales[Category]"), ("Y", "[Total]")],
    );
    let slicer = add("slicer", (40, 260, 300, 300), &[("Values", "Sales[Category]")]);
    let _ = slicer;
    run(
        &mut s,
        "visual.mobile",
        json!({"page": page, "visuals": [
            {"visual": card, "x": 0, "y": 0, "width": 320, "height": 160},
            {"visual": chart, "x": 0, "y": 168, "width": 320, "height": 280}
        ]}),
    );
    run(
        &mut s,
        "visual.mobile_format",
        json!({"page": page, "visual": card, "title": {"show": true, "text": "Sales", "font_size": 10}}),
    );
    let path = dir.join("mobile.pbip");
    run(&mut s, "file.save_as", json!({"path": path}));
    path
}

fn steps(h: &mut Harness<'_, App>, n: usize) {
    for _ in 0..n {
        h.step();
    }
}

fn harness(app: App, size: (f32, f32)) -> Harness<'static, App> {
    Harness::builder()
        .with_size(egui::vec2(size.0, size.1))
        .wgpu()
        .build_ui_state(
            |ui, app: &mut App| {
                app.logic(ui.ctx());
                app.ui(ui);
            },
            app,
        )
}

#[test]
fn mobile_layout_screenshots() {
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("mobile-shots");
    let path = project(dir.path());
    for (lang, suffix, dark, mobile, format) in [
        ("en-US", "en", false, "Mobile layout", "Format visual"),
        (
            "fr-FR",
            "fr",
            false,
            "Disposition mobile",
            "Mettre en forme le visuel",
        ),
        ("en-US", "dark", true, "Mobile layout", "Format visual"),
    ] {
        let mut app = crate::app(lang, None).with_sync_render();
        app.open_path(&path);
        let mut h = harness(app, (1500.0, 900.0));
        if dark {
            h.state_mut().set_theme(bicraft_ui::theme::Brightness::Dark);
        }
        wait(&mut h, "the first snapshot", ready);
        h.state_mut().ribbon_tab = RTab::View;
        steps(&mut h, 3);
        h.get_by_role_and_label(Role::Button, mobile).click();
        steps(&mut h, 8);
        assert!(bicraft_ui::mobile::editing(h.state()));
        save(&mut h, &shots, &format!("mobile-layout-{suffix}"));
        // the card selected, its Format pane on the phone
        let card = h
            .state()
            .loaded
            .as_ref()
            .and_then(|l| {
                l.snap["report"]["pages"][0]["visuals"]
                    .as_array()?
                    .iter()
                    .find(|v| v["type"] == "card")
                    .and_then(|v| v["name"].as_str().map(str::to_string))
            })
            .expect("the card");
        let page = h.state().page_names()[0].0.clone();
        bicraft_ui::canvas::select(h.state_mut(), &page, &[card]);
        steps(&mut h, 3);
        h.get_by_role_and_label(Role::Button, format).click();
        steps(&mut h, 8);
        save(&mut h, &shots, &format!("mobile-format-{suffix}"));
    }
    // a narrow window reads the phone layout when its preference is on
    let mut app = crate::app("en-US", None).with_sync_render();
    app.open_path(&path);
    let mut h = harness(app, (520.0, 900.0));
    wait(&mut h, "the first snapshot", ready);
    {
        let p = h.state_mut();
        p.prefs.mobile_narrow = true;
        p.prefs.panes.data = false;
        p.prefs.panes.build = false;
        p.prefs.panes.filters = false;
    }
    steps(&mut h, 8);
    assert!(bicraft_ui::mobile::drawn(h.state()));
    save(&mut h, &shots, "mobile-narrow-en");
}
