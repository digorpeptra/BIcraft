//! The Power Query editor's Transform and Add column tabs and Home's Keep rows and Remove rows
//! menus (`pq_tabs.rs`, docs/gaps.md G67), through the accessibility tree on an engine session:
//! each item runs `query.apply_step` with its kind, the forms ask the step's values, and the
//! preview shows the evaluated step. A date column's filter menu writes `#date` literals.

use std::path::Path;

use bicraft_engine::Session;
use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use super::backstage_tests::{engine_shell, steps};
use super::harness;
use crate::App;
use crate::pq_editor::{EditorTab, FilterMenu, Form};
use crate::registry::Tab as RTab;
use crate::shell::Caps;

const SALES: &str = "let\n    Source = #table(type table [Product = text, Qty = Int64.Type, City = text, Day = date], {{\"Bike\", 2, \"Paris\", #date(2024, 1, 31)}, {\"Car\", 1, \"Lyon\", #date(2024, 2, 1)}, {\"Pen\", 9, \"Paris\", #date(2024, 2, 1)}}),\n    Kept = Table.SelectRows(Source, each [Qty] > 0)\nin\n    Kept";

fn session() -> Session {
    let mut s = Session::new();
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        ("table.import", json!({"name": "Sales", "m": SALES})),
        ("report.new", json!({})),
    ] {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    s.dirty = false;
    s.report_dirty = false;
    s
}

fn opened(lang: &str) -> Harness<'static, App> {
    let mut app = App::new(lang, engine_shell(Caps::DESKTOP, session)).with_sync_render();
    app.open_path(Path::new("shop.pbip"));
    let mut h = harness(app, (1500.0, 950.0));
    for _ in 0..200 {
        h.step();
        if h.state().shown.is_some() && h.state().loaded.is_some() {
            break;
        }
    }
    steps(&mut h, 2);
    h.state_mut().ribbon_tab = RTab::Home;
    steps(&mut h, 3);
    click(
        &mut h,
        if lang == "fr-FR" {
            "Transformer les données"
        } else {
            "Transform data"
        },
    );
    until(&mut h, "the steps", |a| {
        a.cmd.pq.as_ref().is_some_and(|e| e.step.is_some())
    });
    h
}

fn click(h: &mut Harness<'_, App>, label: &str) {
    h.get_by_label(label).click();
    steps(h, 3);
}

/// The last node labelled `label` (an editor tab or column header shares a main window label).
fn last(h: &mut Harness<'_, App>, label: &str) {
    h.get_all_by_label(label).last().expect(label).click();
    steps(h, 3);
}

fn type_into(h: &mut Harness<'_, App>, label: &str, text: &str) {
    h.get_by(|n| n.role() == Role::TextInput && n.label().as_deref() == Some(label))
        .click();
    steps(h, 2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.get_by(|n| n.role() == Role::TextInput && n.label().as_deref() == Some(label))
        .type_text(text);
    steps(h, 2);
}

fn last_sent(h: &Harness<'_, App>, id: &str) -> Option<Value> {
    h.state()
        .sent
        .iter()
        .rev()
        .find(|c| c.id == id)
        .map(|c| c.params.clone())
}

fn until(h: &mut Harness<'_, App>, what: &str, done: impl Fn(&App) -> bool) {
    for _ in 0..400 {
        if done(h.state()) {
            steps(h, 2);
            return;
        }
        h.step();
    }
    panic!("never: {what}");
}

fn editor(h: &Harness<'_, App>) -> crate::pq_editor::Editor {
    h.state().cmd.pq.clone().expect("the editor is open")
}

/// The preview once read: (columns, rows).
fn preview(h: &mut Harness<'_, App>) -> (Vec<String>, Vec<Vec<Value>>) {
    until(h, "the preview", |a| {
        a.cmd
            .pq
            .as_ref()
            .is_some_and(|e| !e.busy && matches!(e.preview, Some(Ok(_))))
    });
    match &editor(h).preview {
        Some(Ok(p)) => (p.columns.iter().map(|c| c.0.clone()).collect(), p.rows.clone()),
        other => panic!("{other:?}"),
    }
}

fn last_step(h: &Harness<'_, App>) -> String {
    editor(h).step.unwrap_or_default()
}

/// The editor ribbon's Format menu (in the band of its Transpose button).
fn ribbon_format<'a>(h: &'a Harness<'_, App>) -> egui_kittest::Node<'a> {
    let band = h.get_by_label("Transpose").rect();
    h.get_all_by_label("Format")
        .find(|n| {
            let r = n.rect();
            r.min.y >= band.min.y - 1.0 && r.max.y <= band.min.y + crate::widgets::RIBBON_H
        })
        .expect("the ribbon's Format")
}

#[test]
fn home_remove_rows_asks_how_many_and_keep_rows_keeps_a_range() {
    let mut h = opened("en-US");
    preview(&mut h);
    click(&mut h, "Remove rows");
    click(&mut h, "Remove top rows");
    assert!(matches!(editor(&h).form, Some(Form::Params { .. })));
    type_into(&mut h, "Number of rows", "2");
    click(&mut h, "OK");
    let p = last_sent(&h, "query.apply_step").expect("OK runs query.apply_step");
    assert_eq!(p["step"], json!({"kind": "remove_top_rows", "count": 2}));
    let (_, rows) = preview(&mut h);
    assert_eq!(rows.len(), 1);
    assert_eq!(last_step(&h), "Removed Top Rows");
    click(&mut h, "Kept");
    preview(&mut h);
    click(&mut h, "Keep rows");
    click(&mut h, "Keep range of rows");
    type_into(&mut h, "First row", "2");
    type_into(&mut h, "Number of rows", "1");
    click(&mut h, "OK");
    let p = last_sent(&h, "query.apply_step").unwrap();
    assert_eq!(p["step"], json!({"kind": "keep_range", "first": 2, "count": 1}));
    assert_eq!(p["after"], "Kept");
    let (_, rows) = preview(&mut h);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0][0], json!("Car"));
    // a word that is not a number is refused in the window
    click(&mut h, "Remove rows");
    click(&mut h, "Remove bottom rows");
    type_into(&mut h, "Number of rows", "many");
    let before = h.state().sent.len();
    click(&mut h, "OK");
    assert_eq!(h.state().sent.len(), before);
    assert!(editor(&h).problem.is_some());
}

#[test]
fn transform_tab_formats_text_in_place_and_merges_columns() {
    let mut h = opened("en-US");
    preview(&mut h);
    last(&mut h, "Transform");
    assert_eq!(editor(&h).tab, EditorTab::Transform);
    // the main ribbon and the formula bar have a Format too: the editor ribbon's is in
    // Transpose's band
    assert!(ribbon_format(&h).accesskit_node().is_disabled());
    last(&mut h, "City");
    ribbon_format(&h).click();
    steps(&mut h, 3);
    click(&mut h, "UPPERCASE");
    let p = last_sent(&h, "query.apply_step").unwrap();
    assert_eq!(
        p["step"],
        json!({"kind": "column_op", "op": "upper", "columns": ["City"], "add": false})
    );
    let (cols, rows) = preview(&mut h);
    let city = cols.iter().position(|c| c == "City").unwrap();
    assert_eq!(rows[1][city], json!("LYON"));
    assert_eq!(last_step(&h), "Uppercased Text");
    // Extract › First characters asks its count
    last(&mut h, "Product");
    click(&mut h, "Extract");
    click(&mut h, "First characters");
    type_into(&mut h, "Count", "2");
    click(&mut h, "OK");
    let (cols, rows) = preview(&mut h);
    let product = cols.iter().position(|c| c == "Product").unwrap();
    assert_eq!(rows[0][product], json!("Bi"));
    // Merge columns needs two columns picked, in click order
    last(&mut h, "City");
    h.get_all_by_label("Product")
        .last()
        .unwrap()
        .click_modifiers(Modifiers::COMMAND);
    steps(&mut h, 3);
    assert_eq!(editor(&h).columns.len(), 2, "{:?}", editor(&h).columns);
    click(&mut h, "Merge columns");
    type_into(&mut h, "Separator", "-");
    click(&mut h, "OK");
    let p = last_sent(&h, "query.apply_step").unwrap();
    assert_eq!(p["step"]["kind"], "merge_columns");
    assert_eq!(p["step"]["delimiter"], "-");
    let (cols, rows) = preview(&mut h);
    let merged = cols
        .iter()
        .position(|c| c == "Merged")
        .expect("the merged column");
    assert_eq!(rows[0][merged], json!("PARIS-Bi"));
}

#[test]
fn transform_tab_dates_rounding_fill_and_table_shapes() {
    let mut h = opened("en-US");
    preview(&mut h);
    last(&mut h, "Transform");
    last(&mut h, "Day");
    click(&mut h, "Date");
    click(&mut h, "Year");
    let (cols, rows) = preview(&mut h);
    let day = cols.iter().position(|c| c == "Day").unwrap();
    assert_eq!(rows[0][day], json!(2024));
    assert_eq!(last_step(&h), "Extracted Year");
    last(&mut h, "Qty");
    click(&mut h, "Standard");
    click(&mut h, "Multiply");
    type_into(&mut h, "Value", "1,5");
    click(&mut h, "OK");
    let p = last_sent(&h, "query.apply_step").unwrap();
    assert_eq!(p["step"]["number"], json!(1.5));
    let (cols, rows) = preview(&mut h);
    let qty = cols.iter().position(|c| c == "Qty").unwrap();
    assert_eq!(rows[0][qty], json!(3));
    click(&mut h, "Reverse rows");
    let (_, rows) = preview(&mut h);
    assert_eq!(rows[0][0], json!("Pen"));
    click(&mut h, "Move");
    click(&mut h, "To beginning");
    let (cols, _) = preview(&mut h);
    assert_eq!(cols.first().map(String::as_str), Some("Qty"));
    click(&mut h, "Fill");
    click(&mut h, "Down");
    assert_eq!(
        last_sent(&h, "query.apply_step").unwrap()["step"],
        json!({"kind": "fill_down", "columns": ["Qty"]})
    );
    preview(&mut h);
    assert_eq!(last_step(&h), "Filled Down");
}

#[test]
fn add_column_tab_adds_index_conditional_and_extracted_columns() {
    let mut h = opened("en-US");
    preview(&mut h);
    last(&mut h, "Add column");
    assert_eq!(editor(&h).tab, EditorTab::AddColumn);
    click(&mut h, "Index column");
    click(&mut h, "From 1");
    let (cols, rows) = preview(&mut h);
    assert_eq!(cols.last().map(String::as_str), Some("Index"));
    assert_eq!(rows[2].last(), Some(&json!(3)));
    last(&mut h, "Qty");
    click(&mut h, "Conditional column");
    assert!(matches!(editor(&h).form, Some(Form::Conditional { .. })));
    type_into(&mut h, "New column name", "Size");
    if let Some(Form::Conditional { rules, .. }) = h.state_mut().cmd.pq.as_mut().and_then(|e| e.form.as_mut())
    {
        // "is greater than"
        rules[0].op = 2;
    }
    steps(&mut h, 2);
    type_into(&mut h, "Value", "5");
    type_into(&mut h, "Output", "big");
    type_into(&mut h, "Else", "small");
    click(&mut h, "OK");
    let p = last_sent(&h, "query.apply_step").unwrap();
    assert_eq!(
        p["step"],
        json!({"kind": "conditional_column", "name": "Size", "rules": [
            {"column": "Qty", "op": "greater", "value": 5.0, "output": "big"}], "otherwise": "small"})
    );
    let (cols, rows) = preview(&mut h);
    let size = cols.iter().position(|c| c == "Size").unwrap();
    assert_eq!(
        rows.iter().map(|r| r[size].clone()).collect::<Vec<_>>(),
        [json!("small"), json!("small"), json!("big")]
    );
    last(&mut h, "Day");
    click(&mut h, "Date");
    click(&mut h, "Name of month");
    let p = last_sent(&h, "query.apply_step").unwrap();
    assert_eq!(
        p["step"],
        json!({"kind": "column_op", "op": "month_name", "columns": ["Day"], "add": true})
    );
    let (cols, rows) = preview(&mut h);
    assert_eq!(cols.last().map(String::as_str), Some("Month Name"));
    assert_eq!(rows[0].last(), Some(&json!("January")));
    assert_eq!(last_step(&h), "Inserted Month Name");
    click(&mut h, "Duplicate column");
    let (cols, _) = preview(&mut h);
    assert_eq!(cols.last().map(String::as_str), Some("Day - Copy"));
}

#[test]
fn a_date_columns_filter_menu_writes_date_literals() {
    let mut h = opened("en-US");
    preview(&mut h);
    click(&mut h, "Filter Day");
    until(&mut h, "the values", |a| {
        a.cmd
            .pq
            .as_ref()
            .and_then(|e| e.filter.as_ref())
            .is_some_and(|f| matches!(f.values, Some(Ok(_))))
    });
    let f = editor(&h).filter.unwrap();
    assert_eq!(f.date_kind.as_deref(), Some("date"));
    let values: Vec<Value> = match &f.values {
        Some(Ok(v)) => v.iter().map(|x| x.0.clone()).collect(),
        other => panic!("{other:?}"),
    };
    assert_eq!(values, [json!("2024-01-31"), json!("2024-02-01")]);
    if let Some(f) = h.state_mut().cmd.pq.as_mut().and_then(|e| e.filter.as_mut())
        && let Some(Ok(v)) = f.values.as_mut()
    {
        v[0].1 = false;
    }
    steps(&mut h, 2);
    click(&mut h, "OK");
    let p = last_sent(&h, "query.apply_step").unwrap();
    assert_eq!(
        p["step"],
        json!({"kind": "filter_values", "column": "Day", "values": [{"date": "2024-02-01"}], "exclude": false})
    );
    let (_, rows) = preview(&mut h);
    assert_eq!(rows.len(), 2);
}

#[test]
fn filter_values_of_datetime_columns_are_datetime_literals() {
    let f = FilterMenu {
        column: "At".into(),
        ty: "dateTime".into(),
        values: Some(Ok(vec![
            (json!("2024-01-31"), true),
            (json!("2024-02-01T10:30:00"), true),
            (json!("2024-03-01"), false),
        ])),
        incomplete: false,
        search: String::new(),
        date_kind: Some("datetime".into()),
    };
    assert_eq!(
        crate::pq_editor::filter_step(&f).unwrap()["values"],
        json!([{"datetime": "2024-03-01T00:00:00"}])
    );
    assert_eq!(
        crate::pq_editor::date_value(&json!("2024-02-01T10:30:00"), Some("datetime")),
        json!({"datetime": "2024-02-01T10:30:00"})
    );
    assert_eq!(crate::pq_editor::date_value(&json!(3), Some("date")), json!(3));
    assert_eq!(crate::pq_editor::date_value(&json!("x"), None), json!("x"));
}

#[test]
fn the_step_settings_gear_reopens_the_form_and_rewrites_the_step() {
    let mut h = opened("en-US");
    preview(&mut h);
    // Kept is `Table.SelectRows(Source, each [Qty] > 0)`: a filter's settings
    match &editor(&h).form {
        None => {}
        other => panic!("{other:?}"),
    }
    click(&mut h, "Step settings");
    match editor(&h).form {
        Some(Form::Filter { column, op, value }) => {
            assert_eq!(column, "Qty");
            assert_eq!(crate::pq_editor::OPS[op].0, "greater");
            assert_eq!(value, "0");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(editor(&h).editing.as_deref(), Some("Kept"));
    type_into(&mut h, "Value", "5");
    let added = h
        .state()
        .sent
        .iter()
        .filter(|c| c.id == "query.apply_step")
        .count();
    click(&mut h, "OK");
    let p = last_sent(&h, "query.edit_step").expect("OK runs query.edit_step");
    assert_eq!(p["step"], "Kept");
    assert_eq!(
        p["settings"],
        json!({"kind": "filter_rows", "column": "Qty", "op": "greater", "value": 5.0})
    );
    assert_eq!(
        h.state()
            .sent
            .iter()
            .filter(|c| c.id == "query.apply_step")
            .count(),
        added,
        "no step is added"
    );
    let (_, rows) = preview(&mut h);
    assert_eq!(rows.len(), 1);
    assert!(editor(&h).editing.is_none());
    // Remove rows writes settings the gear reads back
    click(&mut h, "Remove rows");
    click(&mut h, "Remove top rows");
    type_into(&mut h, "Number of rows", "1");
    click(&mut h, "OK");
    preview(&mut h);
    click(&mut h, "Step settings");
    match editor(&h).form {
        Some(Form::Params { fields, .. }) => assert_eq!(fields[0].value, "1"),
        other => panic!("{other:?}"),
    }
    click(&mut h, "Cancel");
    assert!(editor(&h).editing.is_none());
    // the Source step has none
    click(&mut h, "Source");
    preview(&mut h);
    assert!(h.get_by_label("Step settings").accesskit_node().is_disabled());
}

#[test]
fn the_new_tabs_speak_french() {
    let mut h = opened("fr-FR");
    preview(&mut h);
    h.get_by_label("Supprimer les lignes");
    h.get_by_label("Conserver les lignes");
    last(&mut h, "Transformer");
    assert_eq!(editor(&h).tab, EditorTab::Transform);
    h.get_by_label("Transposer");
    h.get_by_label("Remplissage");
    h.get_by_label("Arrondi");
    last(&mut h, "Ajouter une colonne");
    assert_eq!(editor(&h).tab, EditorTab::AddColumn);
    h.get_by_label("Colonne conditionnelle");
    h.get_by_label("Colonne d'index");
    h.get_by_label("Dupliquer la colonne");
}

#[test]
fn number_scientific_information_date_and_between_delimiters_items() {
    let mut h = opened("en-US");
    preview(&mut h);
    last(&mut h, "Transform");
    last(&mut h, "Qty");
    click(&mut h, "Scientific");
    click(&mut h, "Square");
    let p = last_sent(&h, "query.apply_step").unwrap();
    assert_eq!(
        p["step"],
        json!({"kind": "column_op", "op": "power", "number": 2, "columns": ["Qty"], "add": false})
    );
    let (cols, rows) = preview(&mut h);
    let qty = cols.iter().position(|c| c == "Qty").unwrap();
    assert_eq!(
        rows.iter().map(|r| r[qty].clone()).collect::<Vec<_>>(),
        [json!(4), json!(1), json!(81)]
    );
    assert_eq!(last_step(&h), "Calculated Power");
    last(&mut h, "Information");
    click(&mut h, "Is even");
    let (_, rows) = preview(&mut h);
    assert_eq!(
        rows.iter().map(|r| r[qty].clone()).collect::<Vec<_>>(),
        [json!(true), json!(false), json!(false)]
    );
    last(&mut h, "Add column");
    last(&mut h, "Day");
    click(&mut h, "Date");
    click(&mut h, "Start of quarter");
    let p = last_sent(&h, "query.apply_step").unwrap();
    assert_eq!(
        p["step"],
        json!({"kind": "column_op", "op": "start_of_quarter", "columns": ["Day"], "add": true})
    );
    let (cols, _) = preview(&mut h);
    assert_eq!(cols.last().map(String::as_str), Some("Start of Quarter"));
    last(&mut h, "Product");
    click(&mut h, "Extract");
    click(&mut h, "Text between delimiters");
    type_into(&mut h, "Start delimiter", "B");
    type_into(&mut h, "End delimiter", "e");
    click(&mut h, "OK");
    let p = last_sent(&h, "query.apply_step").unwrap();
    assert_eq!(p["step"]["op"], "between_delimiters");
    assert_eq!(p["step"]["end"], "e");
    let (cols, rows) = preview(&mut h);
    assert_eq!(cols.last().map(String::as_str), Some("Text Between Delimiters"));
    assert_eq!(rows[0].last(), Some(&json!("ik")));
}
