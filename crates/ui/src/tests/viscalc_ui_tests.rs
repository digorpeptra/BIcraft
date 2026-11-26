//! The visual calculations edit window (`viscalc_dialog.rs`) on an engine session: More options ›
//! New calculation opens it without sending anything, a template fills the formula bar, Apply
//! adds the calculation (its values in the window's visual matrix), a field's switch hides it,
//! a calculation clicked is edited and Delete removes it; the palette's Edit visual
//! calculation opens it on the calculation named.

use std::path::Path;

use bicraft_engine::Session;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::backstage_tests::{engine_shell, steps};
use super::harness;
use super::panes_ui_tests::{select, visual_named};
use crate::App;
use crate::ribbon::VisualSel;
use crate::shell::Caps;

/// Sales(Product, Amount) with [Total]; one page with a table (Product, [Total]).
fn shop_session() -> Session {
    let mut s = Session::new();
    let sales = r#"#table({"Product", "Amount"}, {{"Ball", 50}, {"Bike", 150}, {"Car", 300}})"#;
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
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
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        ("report.new", json!({})),
    ] {
        s.execute(id, &p).unwrap();
    }
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    s.execute(
        "visual.add",
        &json!({"page": page, "visual_type": "tableEx", "x": 20, "y": 20, "width": 500,
                "height": 300, "fields": [{"role": "Values", "field": "Sales[Product]"},
                                          {"role": "Values", "field": "[Total]"}]}),
    )
    .unwrap();
    s.dirty = false;
    s.report_dirty = false;
    s
}

fn open(lang: &str) -> Harness<'static, App> {
    let mut app = App::new(lang, engine_shell(Caps::DESKTOP, shop_session)).with_sync_render();
    app.open_path(Path::new("shop.pbip"));
    let mut h = harness(app, (1600.0, 900.0));
    for _ in 0..200 {
        h.step();
        if h.state().shown.is_some() && h.state().placed.is_some() {
            break;
        }
    }
    steps(&mut h, 2);
    h
}

fn table(h: &Harness<'_, App>, sel: &VisualSel) -> Value {
    let snap = h.state().loaded.as_ref().unwrap().snap.clone();
    snap["report"]["pages"][0]["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == sel.visual.as_str())
        .cloned()
        .unwrap()
}

/// The ids of the visual calculation commands sent.
fn sent(h: &Harness<'_, App>) -> Vec<(&'static str, Value)> {
    h.state()
        .sent
        .iter()
        .filter(|c| c.id.starts_with("visual.calculation."))
        .map(|c| (c.id, c.params.clone()))
        .collect()
}

fn click(h: &mut Harness<'_, App>, label: &str) {
    h.get_by_label(label).click();
    steps(h, 4);
}

/// A column of the visual's evaluated data, by name.
fn column(v: &Value, name: &str) -> (Value, Vec<Value>) {
    let cols = v["data"]["columns"].as_array().unwrap();
    let i = cols
        .iter()
        .position(|c| c["name"] == name)
        .unwrap_or_else(|| panic!("no column {name} in {}", v["data"]["columns"]));
    let rows = v["data"]["rows"].as_array().unwrap();
    (cols[i].clone(), rows.iter().map(|r| r[i].clone()).collect())
}

/// Opens the window from the visual's More options.
pub(super) fn new_calculation(h: &mut Harness<'_, App>) -> VisualSel {
    let sel = visual_named(h, "tableEx");
    select(h, &sel);
    h.get_by_label("More options").click();
    steps(h, 2);
    click(h, "New calculation");
    sel
}

#[test]
fn new_calculation_opens_the_window_and_a_template_applies() {
    let mut h = open("en-US");
    let sel = new_calculation(&mut h);
    // the window, nothing sent yet
    assert!(h.state().cmd.viscalc.is_some(), "the window is open");
    assert_eq!(sent(&h), vec![]);
    h.get_by_label("Visual matrix");
    h.get_by_label("Show on the visual: Total");
    // Templates › Running sum fills the bar and the name
    click(&mut h, "Templates");
    click(&mut h, "Running sum");
    let e = h.state().cmd.viscalc.clone().unwrap();
    assert_eq!(e.expression, "RUNNINGSUM([Total])");
    assert_eq!(e.name, "Running sum");
    click(&mut h, "Apply");
    let s = sent(&h);
    assert_eq!(s.len(), 1, "{s:?}");
    assert_eq!(s[0].0, "visual.calculation.add");
    assert_eq!(
        s[0].1,
        json!({"page": sel.page, "visual": sel.visual, "name": "Running sum",
               "expression": "RUNNINGSUM([Total])"})
    );
    // the window stays, now editing it; the visual's data has its values
    let e = h.state().cmd.viscalc.clone().unwrap();
    assert_eq!(e.editing.as_deref(), Some("Running sum"));
    let v = table(&h, &sel);
    let (c, values) = column(&v, "Running sum");
    assert_eq!(c["kind"], "visual_calculation");
    assert_eq!(values, vec![json!(50.0), json!(200.0), json!(500.0)]);
    // the field list shows it, with its delete button
    h.get_by_label("Show on the visual: Running sum");
    h.get_by_label("Delete Running sum");
    // an edit of the expression sends set
    h.state_mut().cmd.viscalc.as_mut().unwrap().expression = "[Total] * 2".into();
    steps(&mut h, 2);
    click(&mut h, "Apply");
    let s = sent(&h);
    assert_eq!(s.last().unwrap().0, "visual.calculation.set");
    assert_eq!(s.last().unwrap().1["expression"], "[Total] * 2");
    let (_, values) = column(&table(&h, &sel), "Running sum");
    assert_eq!(values, vec![json!(100.0), json!(300.0), json!(600.0)]);
    // Back to report closes the window
    click(&mut h, "Back to report");
    assert!(h.state().cmd.viscalc.is_none());
}

#[test]
fn the_switch_hides_a_field_and_delete_removes_a_calculation() {
    let mut h = open("en-US");
    let sel = new_calculation(&mut h);
    h.state_mut().cmd.viscalc.as_mut().unwrap().expression = "RUNNINGSUM([Total])".into();
    steps(&mut h, 2);
    click(&mut h, "Apply");
    // the default name
    assert_eq!(sent(&h)[0].1["name"], "Calculation");
    // Total hidden: still in the data, flagged
    click(&mut h, "Show on the visual: Total");
    let (id, p) = sent(&h).last().cloned().unwrap();
    assert_eq!(id, "visual.calculation.set");
    assert_eq!(p["name"], "Total");
    assert_eq!(p["hidden"], true);
    let v = table(&h, &sel);
    let (c, _) = column(&v, "Total");
    assert_eq!(c["hidden"], true);
    let (_, values) = column(&v, "Calculation");
    assert_eq!(values, vec![json!(50.0), json!(200.0), json!(500.0)]);
    h.get_by_label("Total (hidden)");
    // shown again
    click(&mut h, "Show on the visual: Total");
    assert_eq!(sent(&h).last().unwrap().1["hidden"], false);
    // Delete removes the calculation
    click(&mut h, "Delete Calculation");
    let (id, p) = sent(&h).last().cloned().unwrap();
    assert_eq!(id, "visual.calculation.remove");
    assert_eq!(p["name"], "Calculation");
    let v = table(&h, &sel);
    assert!(
        v["fields"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["kind"] != "visual_calculation"),
        "{}",
        v["fields"]
    );
    assert_eq!(h.state().cmd.viscalc.as_ref().unwrap().editing, None);
}

#[test]
fn an_invalid_expression_keeps_the_window_with_the_error() {
    let mut h = open("en-US");
    new_calculation(&mut h);
    h.state_mut().cmd.viscalc.as_mut().unwrap().expression = "[Nothing] + 1".into();
    steps(&mut h, 2);
    click(&mut h, "Apply");
    assert_eq!(sent(&h).len(), 1);
    let e = h.state().cmd.viscalc.clone().unwrap();
    assert_eq!(e.editing, None, "nothing was added");
    assert!(
        h.state().error.as_deref().is_some_and(|x| x.contains("Nothing")),
        "{:?}",
        h.state().error
    );
}

#[test]
fn the_palette_edits_a_calculation_and_french_labels() {
    let mut h = open("fr-FR");
    let sel = visual_named(&h, "tableEx");
    select(&mut h, &sel);
    h.state_mut().run(
        crate::actions::Call {
            id: "visual.calculation.add",
            params: json!({"page": sel.page, "visual": sel.visual, "name": "Cumul",
                           "expression": "RUNNINGSUM([Total])"}),
        },
        crate::app::Then::Nothing,
    );
    steps(&mut h, 4);
    assert!(h.state().cmd.viscalc.is_none(), "an add with its expression runs");
    // Edit visual calculation with a name and no change opens the window on it
    let params = json!({"name": "Cumul"});
    assert!(crate::viscalc_dialog::intercept(
        h.state_mut(),
        "visual.calculation.set",
        &params
    ));
    steps(&mut h, 3);
    let e = h.state().cmd.viscalc.clone().unwrap();
    assert_eq!(e.editing.as_deref(), Some("Cumul"));
    assert_eq!(e.expression, "RUNNINGSUM([Total])");
    h.get_by_label("Matrice visuelle");
    h.get_by_label("Retour au rapport");
    h.get_by_label("Afficher sur le visuel : Total");
}
