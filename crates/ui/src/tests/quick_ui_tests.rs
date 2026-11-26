//! Quick measures through the accessibility tree (egui_kittest, no GPU): Home › Quick measure
//! opening the dialog and a year-to-date total run as `measure.quick`, the Data pane's New quick
//! measure on a field starting from it with a filtered value and its ticked values, and the
//! French labels.

use std::path::Path;

use bicraft_engine::Session;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::backstage_tests::{engine_shell, right_click, steps};
use super::harness;
use crate::App;
use crate::registry::Tab as RTab;
use crate::shell::Caps;

/// Sales(Date, Product, Amount) with [Total], and a report of one page.
fn session() -> Session {
    let mut s = Session::new();
    let sales = r#"#table(type table [Date = date, Product = text, Amount = number], {
        {#date(2024, 1, 5), "Bike", 100}, {#date(2024, 2, 5), "Car", 1000}, {#date(2024, 3, 5), "Ball", 5}})"#;
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Date", "data_type": "dateTime"}),
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
    s.dirty = false;
    s.report_dirty = false;
    s
}

fn app(lang: &str) -> Harness<'static, App> {
    let mut app = App::new(lang, engine_shell(Caps::DESKTOP, session)).with_sync_render();
    app.open_path(Path::new("shop.pbip"));
    let mut h = harness(app, (1400.0, 900.0));
    for _ in 0..200 {
        h.step();
        if h.state().shown.is_some() {
            break;
        }
    }
    steps(&mut h, 2);
    h
}

fn click(h: &mut Harness<'_, App>, label: &str) {
    h.get_by_label(label).click();
    steps(h, 3);
}

fn last_sent(h: &Harness<'_, App>, id: &str) -> Option<Value> {
    h.state()
        .sent
        .iter()
        .rev()
        .find(|c| c.id == id)
        .map(|c| c.params.clone())
}

fn measures(h: &Harness<'_, App>) -> Vec<String> {
    h.state()
        .loaded
        .as_ref()
        .and_then(|l| {
            l.snap
                .pointer("/model/tables/0/measures")
                .and_then(Value::as_array)
                .cloned()
        })
        .unwrap_or_default()
        .iter()
        .filter_map(|c| c.get("name").and_then(Value::as_str).map(str::to_string))
        .collect()
}

#[test]
fn home_quick_measure_makes_a_year_to_date_total() {
    let mut h = app("en-US");
    h.state_mut().ribbon_tab = RTab::Home;
    steps(&mut h, 2);
    click(&mut h, "Quick measure");
    assert!(h.state().cmd.quick.is_some(), "the Quick measures dialog is open");
    // the calculation picker, by category
    click(&mut h, "Calculation");
    h.get_by_label("Time intelligence");
    click(&mut h, "Year-to-date total");
    assert_eq!(h.state().cmd.quick.as_ref().unwrap().kind, "year_to_date");
    // OK without the fields says why and sends nothing
    click(&mut h, "OK");
    h.get_by_label_contains("Base value is needed.");
    assert!(last_sent(&h, "measure.quick").is_none());
    click(&mut h, "Base value");
    click(&mut h, "Sales · Total (measure)");
    click(&mut h, "Date");
    // only date columns are offered
    assert!(h.query_by_label("Sales · Product").is_none());
    click(&mut h, "Sales · Date");
    click(&mut h, "OK");
    steps(&mut h, 6);
    let p = last_sent(&h, "measure.quick").expect("measure.quick was not sent");
    assert_eq!(
        p,
        json!({"kind": "year_to_date", "params": {"base": "[Total]", "date": "Sales[Date]", "year_end_month": 12.0}})
    );
    assert!(h.state().cmd.quick.is_none());
    assert!(
        measures(&h).contains(&"Total YTD".to_string()),
        "{:?}",
        measures(&h)
    );
    assert!(h.state().error.is_none(), "{:?}", h.state().error);
}

#[test]
fn the_data_pane_menu_starts_from_the_field_and_ticks_filter_values() {
    let mut h = app("en-US");
    click(&mut h, "Sales");
    let at = h.get_all_by_label("Total").next().unwrap().rect().center();
    right_click(&mut h, at);
    steps(&mut h, 2);
    click(&mut h, "New quick measure");
    let d = h
        .state()
        .cmd
        .quick
        .clone()
        .expect("the Quick measures dialog is open");
    assert_eq!(d.fields.get("base"), Some(&json!("[Total]")));
    h.state_mut().cmd.quick.as_mut().unwrap().kind = "filtered_value";
    steps(&mut h, 2);
    click(&mut h, "Filter");
    click(&mut h, "Sales · Product");
    // the column's values, read for the list
    for _ in 0..50 {
        steps(&mut h, 1);
        if h.query_by_label("Bike").is_some() {
            break;
        }
    }
    click(&mut h, "Bike");
    click(&mut h, "Car");
    click(&mut h, "OK");
    steps(&mut h, 6);
    let p = last_sent(&h, "measure.quick").expect("measure.quick was not sent");
    assert_eq!(
        p,
        json!({"kind": "filtered_value", "params": {"base": "[Total]", "filter": "Sales[Product]", "values": ["Bike", "Car"]}})
    );
    assert!(
        measures(&h).contains(&"Total for Bike, Car".to_string()),
        "{:?}",
        measures(&h)
    );
    assert!(h.state().error.is_none(), "{:?}", h.state().error);
}

#[test]
fn the_dialog_speaks_french_and_the_table_menu_opens_it() {
    let mut h = app("fr-FR");
    click(&mut h, "Sales");
    let at = h.get_all_by_label("Sales").next().unwrap().rect().center();
    right_click(&mut h, at);
    steps(&mut h, 2);
    click(&mut h, "Nouvelle mesure rapide");
    assert!(h.state().cmd.quick.is_some());
    h.get_by_label("Mesures rapides");
    h.get_by_label("Valeur de base");
    h.get_by_label("Catégorie");
    click(&mut h, "Calcul");
    // the list scrolls: its last entries are listed, below the popup's fold
    h.get_by_label("Évaluation par étoiles");
    click(&mut h, "Calcul");
    steps(&mut h, 2);
    h.state_mut().cmd.quick.as_mut().unwrap().kind = "star_rating";
    steps(&mut h, 2);
    h.get_by_label("Valeur la plus haute");
    h.get_by_label("Nombre d'étoiles");
    click(&mut h, "OK");
    h.get_by_label_contains("Valeur de base : à renseigner.");
}

/// The calculations of 2026-11-24 ask their inputs and send them as the engine takes them.
#[test]
fn new_customers_correlation_and_fiscal_year_send_their_inputs() {
    use crate::i18n::Lang;
    use crate::quick_dialog::{CALCS, QuickDialog, call};
    for id in ["sales_from_new_customers", "correlation_coefficient"] {
        assert!(CALCS.iter().any(|c| c.id == id), "{id} is not in the dialog");
    }
    let dialog = |kind: &'static str, fields: Value| QuickDialog {
        kind,
        fields: fields.as_object().cloned().unwrap_or_default(),
        period: "month",
        values: None,
        picked: Vec::new(),
        name: String::new(),
        problem: None,
    };
    let c = call(
        &dialog(
            "sales_from_new_customers",
            json!({"base": "[Total]", "customer": "Sales[Product]", "date": "Sales[Date]"}),
        ),
        Lang::En,
    )
    .unwrap();
    assert_eq!(
        c.params["params"],
        json!({"base": "[Total]", "customer": "Sales[Product]", "date": "Sales[Date]"})
    );
    let e = call(
        &dialog("sales_from_new_customers", json!({"base": "[Total]"})),
        Lang::En,
    )
    .unwrap_err();
    assert!(e.contains("Customer"), "{e}");
    let c = call(
        &dialog(
            "correlation_coefficient",
            json!({"category": "Sales[Product]", "base": "[Total]", "second": "Sum(Sales[Amount])"}),
        ),
        Lang::En,
    )
    .unwrap();
    assert_eq!(
        c.params["params"],
        json!({"category": "Sales[Product]", "base": "[Total]", "second": "Sum(Sales[Amount])"})
    );
    let c = call(
        &dialog(
            "year_to_date",
            json!({"base": "[Total]", "date": "Sales[Date]", "year_end_month": "6"}),
        ),
        Lang::Fr,
    )
    .unwrap();
    assert_eq!(c.params["params"]["year_end_month"], json!(6.0));
}
