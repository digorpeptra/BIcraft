//! Screenshots of the Filters pane formatted by the page (`page.format_filter_pane`) and of the
//! Format tab's page cards, drawn with wgpu (`--features gpu-tests`), saved into
//! `BICRAFT_DESKTOP_SHOTS` when set: light, dark and French.

use std::path::{Path, PathBuf};

use bicraft_engine::Session;
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

/// A one-page report with an applied and an available page filter card and a formatted Filters
/// pane, saved as a PBIP project.
fn shop(dir: &Path) -> PathBuf {
    let mut s = Session::new();
    let sales = r#"#table({"Region", "Product", "Amount"}, {{"North", "Bike", 100}, {"South", "Car", 50}}) "#;
    let run = |s: &mut Session, id: &str, p: Value| s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Region", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Product", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
        ),
        ("report.new", json!({})),
    ] {
        run(&mut s, id, p);
    }
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "clusteredColumnChart", "x": 20, "y": 20, "width": 560,
               "height": 320, "fields": [{"role": "Category", "field": "Sales[Region]"},
                                          {"role": "Y", "field": "Sum(Sales[Amount])"}]}),
    );
    run(
        &mut s,
        "page.filter",
        json!({"page": page, "field": "Sales[Region]", "in": ["North"]}),
    );
    run(
        &mut s,
        "page.filter",
        json!({"page": page, "field": "Sales[Product]"}),
    );
    run(&mut s, "report.filter", json!({"field": "Sales[Amount]"}));
    for p in [
        json!({"card": "pane", "background_color": "#1F2A44", "foreground_color": "#FFFFFF",
               "border": true, "border_color": "#E66C37", "title_size": 18, "header_size": 12,
               "input_box_color": "#2D3B5C", "checkbox_color": "#E66C37"}),
        json!({"card": "applied", "background_color": "#FFF4CE", "foreground_color": "#252423",
               "border": true, "border_color": "#E66C37", "text_size": 13}),
        json!({"card": "available", "background_color": "#2D3B5C", "foreground_color": "#FFFFFF",
               "border": false}),
    ] {
        let mut p = p;
        p["page"] = json!(page);
        run(&mut s, "page.format_filter_pane", p);
    }
    let path = dir.join("filterpane.pbip");
    run(&mut s, "file.save_as", json!({"path": path}));
    path
}

#[test]
fn filter_pane_screenshots() {
    use bicraft_ui::theme::Brightness;
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("filterpane-shots");
    let path = shop(dir.path());
    for (lang, theme, b, (pane_card, cards_card)) in [
        (
            "en-US",
            "light",
            Brightness::Light,
            ("Filter pane", "Filter cards"),
        ),
        ("en-US", "dark", Brightness::Dark, ("Filter pane", "Filter cards")),
        (
            "fr-FR",
            "light",
            Brightness::Light,
            ("Volet Filtre", "Cartes de filtre"),
        ),
    ] {
        let mut app = crate::app(lang, None).with_sync_render();
        app.set_theme(b);
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
        wait(&mut h, "the first snapshot", ready);
        h.state_mut().prefs.panes.build = true;
        h.state_mut().prefs.panes.filters = true;
        h.state_mut().panes.format.tab = bicraft_ui::format_pane::Tab::Format;
        h.step();
        h.step();
        h.get_by_label(pane_card).click();
        h.step();
        h.step();
        save(&mut h, &shots, &format!("filterpane-pane-{theme}-{lang}"));
        h.get_by_label(pane_card).click();
        h.step();
        h.step();
        h.step();
        h.get_by_label(cards_card).click();
        h.step();
        h.step();
        h.state_mut().panes.format.page.applied = true;
        h.step();
        h.step();
        save(&mut h, &shots, &format!("filterpane-cards-{theme}-{lang}"));
    }
}

/// The shop with three page cards in custom order (`filter.order`), unformatted.
fn ordered(dir: &Path) -> PathBuf {
    let mut s = Session::new();
    let sales = r#"#table({"Region", "Product", "Amount"}, {{"North", "Bike", 100}, {"South", "Car", 50}}) "#;
    let run = |s: &mut Session, id: &str, p: Value| s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    run(&mut s, "model.new", json!({"name": "Shop"}));
    run(
        &mut s,
        "table.add",
        json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
    );
    for (c, t) in [("Region", "string"), ("Product", "string"), ("Amount", "double")] {
        run(
            &mut s,
            "column.add",
            json!({"table": "Sales", "name": c, "data_type": t}),
        );
    }
    run(&mut s, "report.new", json!({}));
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    for f in ["Sales[Region]", "Sales[Product]", "Sales[Amount]"] {
        run(&mut s, "page.filter", json!({"page": page, "field": f}));
    }
    run(
        &mut s,
        "filter.order",
        json!({"level": "page", "page": page, "field": "Sales[Region]", "to": 0}),
    );
    let path = dir.join("filterorder.pbip");
    run(&mut s, "file.save_as", json!({"path": path}));
    path
}

/// The custom order with its sort list open, then a card dragged (the insertion line).
#[test]
fn filter_order_screenshots() {
    use bicraft_ui::theme::Brightness;
    use egui::{Event, Modifiers, PointerButton};
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("filterorder-shots");
    let path = ordered(dir.path());
    for (lang, theme, b, (sort, card)) in [
        (
            "en-US",
            "light",
            Brightness::Light,
            ("Filters on this page sort order", "Product is (All)"),
        ),
        (
            "en-US",
            "dark",
            Brightness::Dark,
            ("Filters on this page sort order", "Product is (All)"),
        ),
        (
            "fr-FR",
            "light",
            Brightness::Light,
            ("Filtres sur cette page ordre de tri", "Product est (Tout)"),
        ),
    ] {
        let mut app = crate::app(lang, None).with_sync_render();
        app.set_theme(b);
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
        wait(&mut h, "the first snapshot", ready);
        h.state_mut().prefs.panes.filters = true;
        h.step();
        h.step();
        h.get_by_label(sort).click();
        h.step();
        h.step();
        save(&mut h, &shots, &format!("filterpane-order-sort-{theme}-{lang}"));
        h.key_press(egui::Key::Escape);
        h.step();
        h.step();
        let r = h.get_by_label(card).rect();
        let from = r.center();
        let to = egui::pos2(from.x, r.min.y - 60.0);
        h.hover_at(from);
        h.step();
        h.event(Event::PointerButton {
            pos: from,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        });
        h.step();
        for i in 1..=6 {
            h.hover_at(from + (to - from) * (i as f32 / 6.0));
            h.step();
        }
        save(&mut h, &shots, &format!("filterpane-order-drag-{theme}-{lang}"));
    }
}
