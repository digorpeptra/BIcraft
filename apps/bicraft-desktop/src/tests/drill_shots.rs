//! Screenshots of the reader's drill drawn with wgpu (`--features gpu-tests`), saved into
//! `BICRAFT_DESKTOP_SHOTS` when set (docs/desktop.md, "Drill, drillthrough and page tooltips").

use std::path::{Path, PathBuf};

use bicraft_engine::Session;
use egui::{Event, Modifiers, PointerButton};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::{Temp, click, click_hit, hit_pos, ready, visual, wait};
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

/// The drill fixture of `crates/ui/src/tests/drill_ui_tests.rs` (same visuals and positions),
/// saved as a PBIP project.
fn drill_shop(dir: &Path) -> PathBuf {
    let mut s = Session::new();
    let sales = r#"#table({"Category", "Product", "Amount"}, {{"Wheels", "Bike", 150}, {"Wheels", "Car", 1000}, {"Toys", "Ball", 5}, {"Toys", "Kite", 7}})"#;
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
            json!({"table": "Sales", "name": "Product", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])", "format_string": "#,0"}),
        ),
        (
            "hierarchy.add",
            json!({"table": "Sales", "name": "Products", "levels": ["Category", "Product"]}),
        ),
        ("report.new", json!({})),
        ("page.add", json!({"display_name": "Detail"})),
        ("page.add", json!({"display_name": "Tip"})),
    ] {
        run(&mut s, id, p);
    }
    let main = s
        .report
        .as_ref()
        .map(|r| r.pages[0].name.clone())
        .unwrap_or_default();
    run(
        &mut s,
        "page.rename",
        json!({"page": main, "display_name": "Main"}),
    );
    let add = |s: &mut Session, page: &str, ty: &str, name: &str, (x, y, w, h): (i32, i32, i32, i32)| {
        let r = run(
            s,
            "visual.add",
            json!({"page": page, "visual_type": ty, "x": x, "y": y, "width": w, "height": h, "fields": []}),
        );
        let made = r["visual"].as_str().unwrap_or_default().to_string();
        if let Some(v) = s
            .report
            .as_mut()
            .and_then(|r| r.pages.iter_mut().find(|p| p.display_name == page))
            .and_then(|p| p.visuals.iter_mut().find(|v| v.name == made))
        {
            v.name = name.to_string();
        }
        name.to_string()
    };
    for (ty, name, at, role, value) in [
        (
            "clusteredColumnChart",
            "chart",
            (20, 20, 560, 300),
            "Category",
            "Y",
        ),
        ("pivotTable", "matrix", (620, 20, 400, 300), "Rows", "Values"),
    ] {
        let v = add(&mut s, "Main", ty, name, at);
        run(
            &mut s,
            "visual.bind_hierarchy",
            json!({"page": "Main", "visual": v, "role": role, "table": "Sales", "hierarchy": "Products"}),
        );
        run(
            &mut s,
            "visual.bind",
            json!({"page": "Main", "visual": v, "role": value, "field": "[Total]"}),
        );
    }
    let bars = add(&mut s, "Main", "clusteredBarChart", "bars", (20, 360, 560, 300));
    run(
        &mut s,
        "visual.bind",
        json!({"page": "Main", "visual": bars, "role": "Category", "field": "Sales[Category]"}),
    );
    run(
        &mut s,
        "visual.bind",
        json!({"page": "Main", "visual": bars, "role": "Y", "field": "[Total]"}),
    );
    if let Some(v) = s
        .report
        .as_mut()
        .and_then(|r| r.pages.first_mut())
        .and_then(|p| p.visuals.iter_mut().find(|v| v.name == "matrix"))
    {
        v.visual_extra.insert(
            "objects".into(),
            json!({"rowHeaders": [{"properties": {"showExpandCollapseButtons": {"expr": {"Literal": {"Value": "true"}}}}}]}),
        );
    }
    run(
        &mut s,
        "page.set_type",
        json!({"page": "Detail", "type": "drillthrough", "fields": ["Sales[Category]"], "back_button": true}),
    );
    add(&mut s, "Detail", "card", "detail_card", (300, 200, 300, 150));
    run(
        &mut s,
        "visual.bind",
        json!({"page": "Detail", "visual": "detail_card", "role": "Values", "field": "[Total]"}),
    );
    run(&mut s, "page.set_type", json!({"page": "Tip", "type": "tooltip"}));
    add(&mut s, "Tip", "card", "tip_card", (10, 10, 300, 220));
    run(
        &mut s,
        "visual.bind",
        json!({"page": "Tip", "visual": "tip_card", "role": "Values", "field": "[Total]"}),
    );
    run(
        &mut s,
        "visual.tooltip_page",
        json!({"page": "Main", "visual": bars, "tooltip_page": "Tip"}),
    );
    let path = dir.join("drill.pbip");
    run(&mut s, "file.save_as", json!({"path": path}));
    path
}

fn chart_top(a: &App) -> Value {
    visual(a.loaded.as_ref().map(|l| l.snap.as_ref()), "chart")["drill"]["top"].clone()
}

/// The header icons on hover, the next level, a matrix member expanded with its +, the
/// right-click menu with Drill through, the drillthrough page, a report-page tooltip.
#[test]
fn drill_screenshots() {
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("drill-shots");
    let path = drill_shop(dir.path());
    let mut app = crate::app("en-US", None).with_sync_render();
    app.prefs.panes.data = false;
    app.prefs.panes.build = false;
    app.prefs.panes.filters = false;
    app.open_path(&path);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1400.0, 900.0))
        .wgpu()
        .build_ui_state(
            |ui, app: &mut App| {
                app.logic(ui.ctx());
                app.ui(ui);
            },
            app,
        );
    wait(&mut h, "the first snapshot", ready);
    let to_screen = |h: &Harness<'_, App>, x: f32, y: f32| {
        let p = h.state().placed.unwrap();
        egui::pos2(p.rect.min.x + x * p.zoom, p.rect.min.y + y * p.zoom)
    };
    let p = to_screen(&h, 300.0, 250.0);
    h.hover_at(p);
    h.step();
    h.step();
    save(&mut h, &shots, "drill-hover-icons");
    let r = h.get_by_label("Go to the next level").rect();
    click(&mut h, r.center(), false);
    wait(&mut h, "the next level", |a| ready(a) && chart_top(a) == 1);
    h.hover_at(to_screen(&h, 300.0, 250.0));
    h.step();
    h.step();
    save(&mut h, &shots, "drill-next-level");
    let r = h.get_by_label("Drill up").rect();
    click(&mut h, r.center(), false);
    wait(&mut h, "the top level", |a| ready(a) && chart_top(a) == 0);
    click_hit(&mut h, "matrix", false, |a| {
        a["action"] == "matrix_expand" && a["path"] == json!(["Wheels"])
    });
    save(&mut h, &shots, "drill-matrix-expanded");
    let p = hit_pos(h.state(), "chart", |a| {
        a["action"] == "select" && a["point"][0]["v"] == "Wheels"
    });
    h.hover_at(p);
    h.step();
    for pressed in [true, false] {
        h.event_modifiers(
            Event::PointerButton {
                pos: p,
                button: PointerButton::Secondary,
                pressed,
                modifiers: Modifiers::NONE,
            },
            Modifiers::NONE,
        );
        h.step();
    }
    let dt = h
        .query_by(|n| n.label().is_some_and(|l| l.starts_with("Drill through")))
        .unwrap_or_else(|| panic!("no Drill through entry"))
        .rect();
    h.hover_at(dt.center());
    for _ in 0..4 {
        h.step();
    }
    save(&mut h, &shots, "drill-context-menu");
    let r = h
        .get_all_by_label("Detail")
        .map(|n| n.rect())
        .min_by(|a, b| a.center().y.total_cmp(&b.center().y))
        .unwrap();
    click(&mut h, r.center(), false);
    wait(&mut h, "the drillthrough page", |a| {
        ready(a) && a.active_page() == 1
    });
    save(&mut h, &shots, "drill-drillthrough-page");
    let back = h
        .state()
        .loaded
        .as_ref()
        .and_then(|l| l.snap.pointer("/report/pages/1/visuals").cloned())
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .find(|v| v["interaction"]["button"]["kind"] == "Back")
        .and_then(|v| v["name"].as_str().map(str::to_string))
        .unwrap();
    click_hit(&mut h, &back, false, |a| a["action"] == "button");
    wait(&mut h, "the main page", |a| ready(a) && a.active_page() == 0);
    let p = hit_pos(h.state(), "bars", |a| {
        a["action"] == "select" && a["point"][0]["v"] == "Toys"
    });
    h.hover_at(p);
    for _ in 0..100 {
        h.step();
        if bicraft_ui::drill::tooltip_drawn(h.state()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    // egui shows a tooltip once the pointer has rested
    for _ in 0..20 {
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(bicraft_ui::drill::tooltip_drawn(h.state()));
    save(&mut h, &shots, "drill-tooltip-page");
}
