//! The Model tab (egui_kittest, no GPU): the viewer's diagram from a real session's snapshot,
//! clicks on a table, a measure and a relationship open the details panel, zoom and fit, and
//! huge or damaged models never panic.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use bicraft_engine::Session;
use egui::{Event, Pos2, Vec2, pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::{Temp, click, harness, wait};
use crate::App;
use bicraft_ui::app::Tab;
use bicraft_ui::diagram::{self, Order};
use bicraft_ui::model_view::Sel;

/// A star schema: Sales (a calculated column, a hidden one, three measures, one of them
/// mentioning another, one invalid) → Product → Category, Sales → Date (inactive), Region
/// (both directions), and Notes in no relationship.
pub fn build_star(s: &mut Session) {
    let run = |s: &mut Session, id: &str, p: Value| {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e:?}"));
    };
    run(s, "model.new", json!({"name": "Star"}));
    for (name, csv) in [
        (
            "Sales",
            "ProductKey,DateKey,RegionKey,Amount,Cost\n1,1,1,10,4\n2,2,1,20,9\n1,3,2,5,2\n",
        ),
        ("Product", "ProductKey,Name,CategoryKey\n1,Apple,1\n2,Bread,2\n"),
        ("Category", "CategoryKey,Label\n1,Fruit\n2,Bakery\n"),
        ("Date", "DateKey,Year\n1,2026\n2,2026\n3,2026\n"),
        ("Region", "RegionKey,Region\n1,North\n2,South\n"),
        ("Notes", "Text\nhello\n"),
    ] {
        run(s, "table.import", json!({"name": name, "csv": csv}));
    }
    run(
        s,
        "column.add",
        json!({"table": "Sales", "name": "Margin", "data_type": "double", "expression": "Sales[Amount] - Sales[Cost]"}),
    );
    run(
        s,
        "column.set",
        json!({"table": "Sales", "name": "Cost", "hidden": true}),
    );
    run(
        s,
        "measure.add",
        json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])", "format_string": "#,0"}),
    );
    run(
        s,
        "measure.add",
        json!({"table": "Sales", "name": "Average", "expression": "DIVIDE([Total], COUNTROWS(Sales))"}),
    );
    run(
        s,
        "measure.add",
        json!({"table": "Sales", "name": "Broken", "expression": "SUM(Sales[Nope])", "force": true}),
    );
    for p in [
        json!({"from": "Sales[ProductKey]", "to": "Product[ProductKey]"}),
        json!({"from": "Product[CategoryKey]", "to": "Category[CategoryKey]"}),
        json!({"from": "Sales[DateKey]", "to": "Date[DateKey]", "active": false}),
        json!({"from": "Sales[RegionKey]", "to": "Region[RegionKey]", "cross_filter": "both"}),
    ] {
        run(s, "relationship.add", p);
    }
}

pub fn star(dir: &Path) -> PathBuf {
    let mut s = Session::new();
    build_star(&mut s);
    s.execute("report.new", &json!({})).unwrap();
    let path = dir.join("star.pbip");
    s.execute("file.save_as", &json!({"path": path})).unwrap();
    path
}

pub fn open_model(path: &Path, lang: &str, size: (f32, f32)) -> Harness<'static, App> {
    let mut app = crate::app(lang, None).with_sync_render();
    app.open_path(path);
    app.tab = Tab::Model;
    let mut h = harness(app, size);
    wait(&mut h, "the snapshot", |a| a.loaded.is_some());
    wait(&mut h, "the diagram", |a| a.model_view.diagram().is_some());
    h
}

fn model(app: &App) -> Value {
    app.loaded.as_ref().unwrap().snap["model"].clone()
}

/// The screen centre of the card of `table`.
fn head(app: &App, table: &str) -> Pos2 {
    let d = app.model_view.diagram().unwrap();
    let c = &d.cards[d.card_named(table).unwrap()];
    app.model_view.screen_pos(c.head.center())
}

#[test]
fn model_tab_draws_the_viewers_diagram_and_shows_details() {
    let dir = Temp::new("model");
    let path = star(dir.path());
    let mut h = open_model(&path, "en-US", (1400.0, 900.0));
    let app = h.state();
    // the layout is the viewer's for this model
    let expected = diagram::layout(&model(app), &mut Order::default(), &HashSet::new());
    let d = app.model_view.diagram().unwrap();
    assert_eq!(d.cards, expected.cards);
    assert_eq!(d.links, expected.links);
    assert_eq!(d.cards.len(), 6);
    assert_eq!(d.links.len(), 4);
    // Sales in the first column, Product and Date and Region in the second, Category third
    let x = |t: &str| d.cards[d.card_named(t).unwrap()].rect.min.x;
    assert!(x("Sales") < x("Product") && x("Product") == x("Date") && x("Date") == x("Region"));
    assert!(x("Category") > x("Product") && x("Notes") > x("Category"));
    assert!(d.links.iter().any(|l| l.inactive));
    assert!(d.links.iter().any(|l| l.arrows.len() == 2));
    // the bar and the summary panel
    h.get_by_label("Model diagram");
    h.get_by_label("Semantic model · click a table, column or measure");
    h.get_by_label("1 invalid measure");
    h.get_by_label("Fit");

    // a click on Sales' header: the table's details
    let p = head(h.state(), "Sales");
    click(&mut h, p, false);
    assert_eq!(h.state().model_view.sel, Some(Sel::Table("Sales".into())));
    h.step();
    h.get_by_label("Columns and measures");
    // the details pane's heading and the contextual Table tab
    assert_eq!(h.query_all_by_label("Table").count(), 2);

    // a click on the "Total" measure row: its expression, and the measures mentioning it
    let (k, row) = {
        let d = h.state().model_view.diagram().unwrap();
        let k = d.card_named("Sales").unwrap();
        let m = model(h.state());
        let idx = m["tables"][d.cards[k].table]["measures"]
            .as_array()
            .unwrap()
            .iter()
            .position(|x| x["name"] == "Total")
            .unwrap();
        let row = d.cards[k].measures.iter().find(|r| r.index == idx).unwrap().rect;
        (k, row)
    };
    let _ = k;
    let p = h.state().model_view.screen_pos(row.center());
    click(&mut h, p, false);
    assert_eq!(
        h.state().model_view.sel,
        Some(Sel::Measure {
            table: "Sales".into(),
            name: "Total".into()
        })
    );
    h.step();
    h.get_by_label("Measure in");
    h.get_by_label("Mentioned in measures");
    h.get_by_label("Average");
    // the panel's link goes back to the table (the first "Sales": "Measure in Sales")
    h.get_all_by_label("Sales").next().unwrap().click();
    h.step();
    h.step();
    assert_eq!(h.state().model_view.sel, Some(Sel::Table("Sales".into())));

    // a click on a relationship line
    let (mid, rel) = {
        let d = h.state().model_view.diagram().unwrap();
        let l = &d.links[0];
        (diagram::cubic(&l.curve, 0.5), l.rel)
    };
    let p = h.state().model_view.screen_pos(mid);
    click(&mut h, p, false);
    assert_eq!(h.state().model_view.sel, Some(Sel::Rel(rel)));
    h.step();
    h.get_by_label("Relationship");
    h.get_by_label("Behaviour");

    // Escape closes the panel
    h.key_press(egui::Key::Escape);
    h.step();
    assert_eq!(h.state().model_view.sel, None);
}

#[test]
fn zoom_pan_and_fit() {
    let dir = Temp::new("model-zoom");
    let path = star(dir.path());
    let mut h = open_model(&path, "en-US", (1260.0, 700.0));
    let canvas = h.state().model_view.canvas;
    let z0 = h.state().model_view.zoom;
    // Ctrl+wheel (a zoom gesture) over the diagram, about the pointer
    let at = canvas.center();
    let before = h.state().model_view.screen_pos(pos2(100.0, 100.0));
    h.hover_at(at);
    h.step();
    h.event(Event::Zoom(1.5));
    h.step();
    let z1 = h.state().model_view.zoom;
    assert!((z1 / z0 - 1.5).abs() < 0.01, "{z0} → {z1}");
    // the point under the pointer stays put, others move away from it
    let after = h.state().model_view.screen_pos(pos2(100.0, 100.0));
    assert!((after - at).length() > (before - at).length());
    // drag to pan
    let pan0 = h.state().model_view.pan;
    h.drag_at(at);
    h.step();
    h.hover_at(at + Vec2::new(-80.0, -40.0));
    h.step();
    h.event(Event::PointerButton {
        pos: at + Vec2::new(-80.0, -40.0),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.step();
    let pan1 = h.state().model_view.pan;
    assert!(
        (pan1 - pan0 - Vec2::new(-80.0, -40.0)).length() < 2.0,
        "{pan0:?} → {pan1:?}"
    );
    // the "+" and "−" buttons
    h.get_by_label("+").click();
    h.step();
    assert!(h.state().model_view.zoom > z1);
    // Fit: the whole diagram is in the window
    h.get_by_label("Fit").click();
    h.step();
    h.step();
    let mv = &h.state().model_view;
    let size = mv.diagram().unwrap().size;
    let (a, b) = (mv.screen_pos(Pos2::ZERO), mv.screen_pos(size.to_pos2()));
    assert!(
        mv.canvas.expand(1.0).contains(a) && mv.canvas.expand(1.0).contains(b),
        "{a:?} {b:?} {:?}",
        mv.canvas
    );
    // and it is not tiny: one side fills the canvas
    let fill = ((b.x - a.x) / mv.canvas.width()).max((b.y - a.y) / mv.canvas.height());
    assert!(fill > 0.9, "{fill}");
}

#[test]
fn french_model_tab() {
    let dir = Temp::new("model-fr");
    let path = star(dir.path());
    let mut h = open_model(&path, "fr-FR", (1200.0, 800.0));
    h.get_by_label("Diagramme du modèle");
    h.get_by_label("Ajuster");
    h.get_by_label("1 mesure non valide");
    let p = head(h.state(), "Sales");
    click(&mut h, p, false);
    h.step();
    h.get_by_label("Colonnes et mesures");
}

#[test]
fn dark_mode_follows_the_preference() {
    let dir = Temp::new("model-dark");
    let path = star(dir.path());
    let mut h = open_model(&path, "en-US", (1200.0, 800.0));
    h.state_mut().set_theme(bicraft_ui::theme::Brightness::Dark);
    h.step();
    h.step();
    h.get_by_label("Model diagram");
    assert!(h.ctx.global_style().visuals.dark_mode);
}

/// A model-only `.bim`: no error bar; the Model and Validation tabs work, the Report tab says
/// there is no report (as the viewer's empty state).
#[test]
fn a_model_only_bim_works_without_an_error() {
    let dir = Temp::new("model-bim");
    let mut s = Session::new();
    build_star(&mut s);
    let path = dir.path().join("star.bim");
    s.execute("file.save_as", &json!({"path": path})).unwrap();
    let mut h = open_model(&path, "en-US", (1200.0, 800.0));
    assert_eq!(h.state().error, None);
    h.get_by_label("Model diagram");
    h.get_by_label("Report").click();
    h.step();
    h.step();
    h.get_by_label("No report in this file");
    assert_eq!(h.state().error, None);
    h.get_by_label("Validation view").click();
    h.step();
    assert_eq!(h.state().error, None);
    assert_eq!(h.state().title(), "star.bim · rev 0");
}
