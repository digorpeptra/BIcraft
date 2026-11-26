//! Home › Transform data, the Power Query editor, through the accessibility tree (egui_kittest,
//! no GPU), on an engine session (ROADMAP M11 lot 12): the queries listed by `query.list`, the
//! applied steps, the preview at a step (`query.preview {step}`), the transform buttons and forms
//! adding steps (`query.apply_step`), the formula bar (`query.edit_step`), renaming and deleting
//! a step, Close & Apply (`query.apply`, one undo step), and the Navigator's Transform data;
//! the Advanced editor, the column headers' type and filter menus, new, duplicated, referenced,
//! renamed and deleted queries, and the question asked before changes are discarded.

use std::path::{Path, PathBuf};

use bicraft_engine::Session;
use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use super::backstage_tests::{engine_shell, steps};
use super::harness;
use crate::App;
use crate::registry::Tab as RTab;
use crate::shell::Caps;

const SALES: &str = "let\n    // typed in\n    Source = #table(type table [Product = text, Qty = Int64.Type, City = text], {{\"Bike\", 2, \"Paris\"}, {\"Car\", 1, \"Lyon\"}, {\"Pen\", 9, \"Paris\"}}),\n    Kept = Table.SelectRows(Source, each [Qty] > 0)\nin\n    Kept";

/// Table Sales loaded by a two-step query, shared query Cities.
fn session() -> Session {
    let mut s = Session::new();
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        ("table.import", json!({"name": "Sales", "m": SALES})),
        (
            "expression.add",
            json!({"name": "Cities", "expression": "#table(type table [City = text, Country = text], {{\"Paris\", \"FR\"}, {\"Lyon\", \"FR\"}})"}),
        ),
        ("report.new", json!({})),
    ] {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    s.dirty = false;
    s.report_dirty = false;
    s
}

fn app(lang: &str) -> Harness<'static, App> {
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
    h
}

fn click(h: &mut Harness<'_, App>, label: &str) {
    h.get_by_label(label).click();
    steps(h, 3);
}

/// A click on preview column header `label` (drawn after the queries list).
fn header(h: &mut Harness<'_, App>, label: &str) {
    h.get_all_by_label(label).last().expect("the header").click();
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

fn count_sent(h: &Harness<'_, App>, id: &str) -> usize {
    h.state().sent.iter().filter(|c| c.id == id).count()
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

/// The preview's column names, once it is read.
fn preview_columns(h: &mut Harness<'_, App>) -> Vec<String> {
    until(h, "the preview", |a| {
        a.cmd
            .pq
            .as_ref()
            .is_some_and(|e| !e.busy && matches!(e.preview, Some(Ok(_))))
    });
    match &editor(h).preview {
        Some(Ok(p)) => p.columns.iter().map(|c| c.0.clone()).collect(),
        other => panic!("{other:?}"),
    }
}

fn step_names(h: &Harness<'_, App>) -> Vec<String> {
    let e = editor(h);
    e.selected
        .and_then(|i| e.queries.get(i))
        .and_then(|q| q.steps.clone())
        .unwrap_or_default()
        .into_iter()
        .map(|s| s.name)
        .collect()
}

fn table_columns(h: &Harness<'_, App>, table: &str) -> Vec<String> {
    h.state()
        .loaded
        .as_ref()
        .and_then(|l| l.snap.pointer("/model/tables").and_then(Value::as_array).cloned())
        .unwrap_or_default()
        .iter()
        .find(|t| t["name"] == table)
        .and_then(|t| t["columns"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|c| c["name"].as_str().map(str::to_string))
        .collect()
}

/// Opens the editor with Home › Transform data, on Sales at its last step.
fn opened(lang: &str, button: &str) -> Harness<'static, App> {
    let mut h = app(lang);
    click(&mut h, button);
    assert_eq!(count_sent(&h, "query.list"), 1);
    until(&mut h, "the steps", |a| {
        a.cmd.pq.as_ref().is_some_and(|e| e.step.is_some())
    });
    h
}

#[test]
fn transform_data_lists_the_queries_their_steps_and_previews_the_last_step() {
    let mut h = opened("en-US", "Transform data");
    let e = editor(&h);
    let names: Vec<&str> = e.queries.iter().map(|q| q.name.as_str()).collect();
    assert_eq!(names, ["Sales", "Cities"]);
    assert_eq!(e.selected, Some(0));
    assert_eq!(step_names(&h), ["Source", "Kept"]);
    assert_eq!(e.step.as_deref(), Some("Kept"));
    assert_eq!(e.formula, "Table.SelectRows(Source, each [Qty] > 0)");
    assert_eq!(preview_columns(&mut h), ["Product", "Qty", "City"]);
    assert_eq!(last_sent(&h, "query.preview").unwrap()["step"], "Kept");
    h.get_by_label("Applied steps");
    h.get_by_label("Pen");
    // another query: its steps are read
    click(&mut h, "Cities");
    until(&mut h, "Cities' step", |a| {
        a.cmd
            .pq
            .as_ref()
            .is_some_and(|e| e.step.as_deref() == Some("Source"))
    });
    assert_eq!(preview_columns(&mut h), ["City", "Country"]);
}

#[test]
fn a_click_on_a_step_previews_the_query_at_it() {
    let mut h = opened("en-US", "Transform data");
    preview_columns(&mut h);
    let before = count_sent(&h, "query.preview");
    click(&mut h, "Source");
    assert_eq!(count_sent(&h, "query.preview"), before + 1);
    assert_eq!(last_sent(&h, "query.preview").unwrap()["step"], "Source");
    assert!(editor(&h).formula.starts_with("#table(type table"));
    preview_columns(&mut h);
}

#[test]
fn remove_columns_adds_a_step_after_the_one_shown() {
    let mut h = opened("en-US", "Transform data");
    preview_columns(&mut h);
    // a transform needs a column picked first
    assert!(h.get_by_label("Remove columns").accesskit_node().is_disabled());
    click(&mut h, "City");
    assert_eq!(editor(&h).columns, ["City"]);
    click(&mut h, "Remove columns");
    let p = last_sent(&h, "query.apply_step").expect("Remove columns runs query.apply_step");
    assert_eq!(p["step"], json!({"kind": "remove_columns", "columns": ["City"]}));
    assert_eq!(p["after"], "Kept");
    assert_eq!(preview_columns(&mut h), ["Product", "Qty"]);
    assert_eq!(step_names(&h), ["Source", "Kept", "Removed Columns"]);
    assert_eq!(editor(&h).step.as_deref(), Some("Removed Columns"));
    assert!(editor(&h).queries[0].changed());
    // nothing written yet
    assert_eq!(table_columns(&h, "Sales"), ["Product", "Qty", "City"]);
}

#[test]
fn filter_rows_asks_its_comparison_and_value() {
    let mut h = opened("en-US", "Transform data");
    preview_columns(&mut h);
    click(&mut h, "Qty");
    click(&mut h, "Filter rows");
    assert!(matches!(
        editor(&h).form,
        Some(crate::pq_editor::Form::Filter { .. })
    ));
    type_into(&mut h, "Value", "2");
    click(&mut h, "OK");
    let p = last_sent(&h, "query.apply_step").expect("OK runs query.apply_step");
    // a number for a number column
    assert_eq!(
        p["step"],
        json!({"kind": "filter_rows", "column": "Qty", "op": "equals", "value": 2.0})
    );
    preview_columns(&mut h);
    assert_eq!(step_names(&h).last().map(String::as_str), Some("Filtered Rows"));
    match &editor(&h).preview {
        Some(Ok(p)) => assert_eq!(p.rows.len(), 1),
        other => panic!("{other:?}"),
    }
    assert!(editor(&h).form.is_none());
}

#[test]
fn rename_change_type_and_sort_columns() {
    let mut h = opened("en-US", "Transform data");
    preview_columns(&mut h);
    click(&mut h, "Product");
    click(&mut h, "Rename column");
    type_into(&mut h, "New name", "Item");
    click(&mut h, "OK");
    assert_eq!(preview_columns(&mut h), ["Item", "Qty", "City"]);
    click(&mut h, "Qty");
    click(&mut h, "Change type");
    click(&mut h, "OK");
    let p = last_sent(&h, "query.apply_step").unwrap();
    assert_eq!(
        p["step"],
        json!({"kind": "change_type", "columns": [{"column": "Qty", "type": "text"}]})
    );
    preview_columns(&mut h);
    click(&mut h, "Item");
    click(&mut h, "Sort descending");
    preview_columns(&mut h);
    assert_eq!(
        step_names(&h),
        ["Source", "Kept", "Renamed Columns", "Changed Type", "Sorted Rows"]
    );
    match &editor(&h).preview {
        Some(Ok(p)) => assert_eq!(p.rows[0][0], json!("Pen")),
        other => panic!("{other:?}"),
    }
}

#[test]
fn group_by_merge_and_append_use_their_forms() {
    let mut h = opened("en-US", "Transform data");
    preview_columns(&mut h);
    click(&mut h, "City");
    click(&mut h, "Merge queries");
    click(&mut h, "OK");
    let p = last_sent(&h, "query.apply_step").unwrap();
    assert_eq!(p["step"]["kind"], "merge");
    assert_eq!(p["step"]["other"], "Cities");
    assert_eq!(preview_columns(&mut h), ["Product", "Qty", "City", "Cities"]);
    header(&mut h, "Cities");
    click(&mut h, "Expand column");
    type_into(&mut h, "Columns (comma-separated)", "Country");
    click(&mut h, "OK");
    assert_eq!(
        preview_columns(&mut h),
        ["Product", "Qty", "City", "Cities.Country"]
    );
    click(&mut h, "City");
    click(&mut h, "Group by");
    click(&mut h, "OK");
    assert_eq!(preview_columns(&mut h), ["City", "Count"]);
    click(&mut h, "Append queries");
    click(&mut h, "OK");
    let p = last_sent(&h, "query.apply_step").unwrap();
    assert_eq!(p["step"], json!({"kind": "append", "others": ["Cities"]}));
    preview_columns(&mut h);
    assert_eq!(
        step_names(&h)[2..],
        [
            "Merged Queries",
            "Expanded Cities",
            "Grouped Rows",
            "Appended Query"
        ]
    );
}

#[test]
fn the_formula_bar_edits_the_step_shown() {
    let mut h = opened("en-US", "Transform data");
    preview_columns(&mut h);
    type_into(
        &mut h,
        "Formula of the step",
        "Table.SelectRows(Source, each [Qty] > 5)",
    );
    click(&mut h, "Apply formula");
    let p = last_sent(&h, "query.edit_step").expect("Apply formula runs query.edit_step");
    assert_eq!(p["step"], "Kept");
    assert_eq!(p["formula"], "Table.SelectRows(Source, each [Qty] > 5)");
    preview_columns(&mut h);
    match &editor(&h).preview {
        Some(Ok(p)) => assert_eq!(p.rows.len(), 1),
        other => panic!("{other:?}"),
    }
    // the rest of the text is the user's
    assert_eq!(editor(&h).queries[0].draft, SALES.replace("> 0)", "> 5)"));
}

#[test]
fn rename_and_delete_a_step() {
    let mut h = opened("en-US", "Transform data");
    preview_columns(&mut h);
    click(&mut h, "Rename step");
    type_into(&mut h, "New step name", "Positive");
    click(&mut h, "Rename");
    let p = last_sent(&h, "query.edit_step").unwrap();
    assert_eq!(p["new_name"], "Positive");
    preview_columns(&mut h);
    assert_eq!(step_names(&h), ["Source", "Positive"]);
    click(&mut h, "Delete step");
    let p = last_sent(&h, "query.remove_step").expect("Delete step runs query.remove_step");
    assert_eq!(p["step"], "Positive");
    preview_columns(&mut h);
    assert_eq!(step_names(&h), ["Source"]);
    // one step left: it cannot go
    assert!(h.get_by_label("Delete step").accesskit_node().is_disabled());
    click(&mut h, "Insert step after");
    preview_columns(&mut h);
    assert_eq!(step_names(&h), ["Source", "Custom"]);
    assert_eq!(editor(&h).formula, "Source");
}

#[test]
fn close_and_apply_writes_the_query_back_in_one_undo_step() {
    let mut h = opened("en-US", "Transform data");
    preview_columns(&mut h);
    click(&mut h, "City");
    click(&mut h, "Remove columns");
    preview_columns(&mut h);
    click(&mut h, "Close & Apply");
    let p = last_sent(&h, "query.apply").expect("Close & Apply runs query.apply");
    assert_eq!(
        p["queries"].as_array().map(Vec::len),
        Some(1),
        "only the changed query: {p}"
    );
    assert_eq!(p["queries"][0]["name"], "Sales");
    until(&mut h, "the editor closes", |a| a.cmd.pq.is_none());
    until(&mut h, "the table follows its query", |a| {
        a.loaded.as_ref().is_some_and(|l| {
            l.snap
                .pointer("/model/tables/0/columns")
                .and_then(Value::as_array)
                .is_some_and(|c| c.len() == 2)
        })
    });
    assert_eq!(table_columns(&h, "Sales"), ["Product", "Qty"]);
    assert_eq!(count_sent(&h, "query.apply"), 1);
}

#[test]
fn the_navigators_transform_data_opens_the_items_as_new_queries() {
    let dir: PathBuf = std::env::temp_dir().join(format!("bicraft-ui-pq-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("stock.csv"), "Item,Count\nBike,4\nCar,2\n").unwrap();
    let mut h = app("en-US");
    click(&mut h, "Get data");
    // the ribbon's Transform data out of the way
    h.state_mut().ribbon_tab = RTab::Insert;
    steps(&mut h, 2);
    click(&mut h, "Text/CSV");
    type_into(&mut h, "File path", &dir.join("stock.csv").display().to_string());
    click(&mut h, "Connect");
    until(&mut h, "the item", |a| {
        a.cmd
            .get_data
            .as_ref()
            .and_then(|d| d.nav.as_ref())
            .is_some_and(|n| n.checked.len() == 1)
    });
    click(&mut h, "Transform data");
    assert!(h.state().cmd.get_data.is_none(), "the Navigator closes");
    assert_eq!(count_sent(&h, "source.load"), 0, "nothing is loaded yet");
    until(&mut h, "the new query's step", |a| {
        a.cmd.pq.as_ref().is_some_and(|e| e.step.is_some())
    });
    let e = editor(&h);
    let q = e.selected.and_then(|i| e.queries.get(i)).unwrap();
    assert_eq!(q.name, "stock");
    assert_eq!(q.source, "new");
    assert_eq!(preview_columns(&mut h), ["Item", "Count"]);
    click(&mut h, "Close & Apply");
    let p = last_sent(&h, "query.apply").unwrap();
    assert_eq!(p["queries"][0]["name"], "stock");
    assert!(p["data_root"].as_str().is_some());
    until(&mut h, "the editor closes", |a| a.cmd.pq.is_none());
    until(&mut h, "the table is loaded", |a| {
        a.loaded.as_ref().is_some_and(|l| {
            l.snap
                .pointer("/model/tables")
                .and_then(Value::as_array)
                .is_some_and(|t| t.iter().any(|t| t["name"] == "stock"))
        })
    });
}

fn checkbox(h: &mut Harness<'_, App>, label: &str) {
    h.get_by(|n| n.role() == Role::CheckBox && n.label().as_deref() == Some(label))
        .click();
    steps(h, 3);
}

fn query_names(h: &Harness<'_, App>) -> Vec<String> {
    editor(h).queries.iter().map(|q| q.name.clone()).collect()
}

#[test]
fn the_advanced_editor_shows_the_whole_query_and_takes_it_only_when_it_parses() {
    let mut h = opened("en-US", "Transform data");
    preview_columns(&mut h);
    click(&mut h, "Advanced editor");
    assert_eq!(editor(&h).advanced.as_deref(), Some(SALES));
    // a text that does not parse: the error shown, the draft as it was
    type_into(&mut h, "Query M", "let Source = in Source");
    click(&mut h, "Done");
    assert_eq!(
        last_sent(&h, "query.steps").unwrap()["expression"],
        "let Source = in Source"
    );
    until(&mut h, "the check", |a| {
        a.cmd.pq.as_ref().is_some_and(|e| !e.busy)
    });
    let e = editor(&h);
    assert!(e.problem.is_some(), "the parse error is shown");
    assert!(e.advanced.is_some(), "the Advanced editor stays open");
    assert_eq!(e.queries[0].draft, SALES);
    // a text that parses: taken as the draft, its steps listed
    let m = "let\n    Source = #table({\"A\"}, {{1}, {2}}),\n    Big = Table.SelectRows(Source, each [A] > 1)\nin\n    Big";
    type_into(&mut h, "Query M", m);
    click(&mut h, "Done");
    assert_eq!(preview_columns(&mut h), ["A"]);
    let e = editor(&h);
    assert!(e.advanced.is_none());
    assert_eq!(e.queries[0].draft, m);
    assert_eq!(step_names(&h), ["Source", "Big"]);
    assert!(e.queries[0].changed());
}

/// The editor's Format button (several "Format" labels may be on screen: the editor's is
/// drawn last).
fn click_format(h: &mut Harness<'_, App>, label: &str) {
    h.get_all_by(|n| n.role() == Role::Button && n.label().as_deref() == Some(label))
        .last()
        .expect("a Format button")
        .click();
    steps(h, 3);
}

#[test]
fn format_lays_out_the_advanced_editors_text_and_the_formula() {
    let mut h = opened("en-US", "Transform data");
    preview_columns(&mut h);
    click(&mut h, "Advanced editor");
    type_into(&mut h, "Query M", "let a=1, // one\nb=a+1 in b");
    click_format(&mut h, "Format");
    assert_eq!(
        last_sent(&h, "m.format").expect("Format runs m.format")["expression"],
        "let a=1, // one\nb=a+1 in b"
    );
    until(&mut h, "the formatted text", |a| {
        a.cmd.pq.as_ref().is_some_and(|e| !e.busy)
    });
    let e = editor(&h);
    assert_eq!(
        e.advanced.as_deref(),
        Some("let\n    a = 1, // one\n    b = a + 1\nin\n    b")
    );
    // nothing is taken until Done
    assert_eq!(e.queries[0].draft, SALES);
    assert!(e.problem.is_none());
    // a text that does not parse stays as typed, the error shown
    type_into(&mut h, "Query M", "let a = in a");
    click_format(&mut h, "Format");
    until(&mut h, "the refusal", |a| {
        a.cmd.pq.as_ref().is_some_and(|e| !e.busy)
    });
    let e = editor(&h);
    assert_eq!(e.advanced.as_deref(), Some("let a = in a"));
    assert!(e.problem.is_some());
    click(&mut h, "Cancel");
    // the formula bar: the step's formula formatted, not applied
    type_into(
        &mut h,
        "Formula of the step",
        "Table.SelectRows(Source,each [Qty]>5)",
    );
    click_format(&mut h, "Format");
    until(&mut h, "the formatted formula", |a| {
        a.cmd.pq.as_ref().is_some_and(|e| !e.busy)
    });
    assert_eq!(editor(&h).formula, "Table.SelectRows(Source, each [Qty] > 5)");
    assert_eq!(count_sent(&h, "query.edit_step"), 0);
}

/// The labels of an M editor's open completion list.
fn m_list(a: &crate::m_box::MAssist) -> Vec<String> {
    a.completion
        .as_ref()
        .map(|(items, _, _)| items.iter().map(|i| i.label.clone()).collect())
        .unwrap_or_default()
}

/// M11.35: the Advanced editor and the formula bar complete M (the catalogue's functions, the
/// steps, the queries, the preview's columns inside `[ ]`), show the signature of the call
/// being typed and say what the name under the pointer is.
#[test]
fn the_m_editors_complete_sign_and_describe() {
    let mut h = opened("en-US", "Transform data");
    preview_columns(&mut h);
    click(&mut h, "Advanced editor");
    type_into(&mut h, "Query M", "let\n    S = Table.SelectR");
    assert_eq!(
        m_list(&editor(&h).m_advanced).first().map(String::as_str),
        Some("Table.SelectRows")
    );
    assert!(
        h.query_all_by_label("function").next().is_some(),
        "the list says what each item is"
    );
    h.key_press(Key::Enter);
    steps(&mut h, 3);
    let e = editor(&h);
    assert_eq!(e.advanced.as_deref(), Some("let\n    S = Table.SelectRows("));
    assert!(e.m_advanced.completion.is_none());
    // the signature of the call being typed
    assert!(
        h.query_by_label("Table.SelectRows(table, condition)").is_some(),
        "the signature shows"
    );
    // the preview's columns inside [ ], the window's queries, Esc closes the list
    type_into(&mut h, "Query M", "let\n    S = Table.SelectRows(Sales, each [Q");
    assert_eq!(m_list(&editor(&h).m_advanced), vec!["Qty"]);
    h.key_press(Key::Tab);
    steps(&mut h, 3);
    assert_eq!(
        editor(&h).advanced.as_deref(),
        Some("let\n    S = Table.SelectRows(Sales, each [Qty]")
    );
    type_into(&mut h, "Query M", "let\n    S = Cit");
    assert_eq!(m_list(&editor(&h).m_advanced), vec!["Cities"]);
    h.key_press(Key::Escape);
    steps(&mut h, 2);
    assert!(editor(&h).m_advanced.completion.is_none());
    assert!(
        editor(&h).advanced.is_some(),
        "Esc closes the list, not the editor"
    );
    // what the name under the pointer is
    type_into(&mut h, "Query M", "Table.RowCount(Sales)");
    let r = h
        .get_by(|n| n.role() == Role::TextInput && n.label().as_deref() == Some("Query M"))
        .rect();
    h.hover_at(r.left_top() + egui::vec2(12.0, 10.0));
    steps(&mut h, 2);
    assert!(
        h.query_by_label("Table.RowCount(table)").is_some(),
        "the hover shows the signature"
    );
    click(&mut h, "Cancel");
    // the formula bar: the steps before the one shown, the signature
    type_into(&mut h, "Formula of the step", "Table.SelectRows(Sou");
    let e = editor(&h);
    assert_eq!(m_list(&e.m_formula).first().map(String::as_str), Some("Source"));
    h.key_press(Key::Enter);
    steps(&mut h, 3);
    assert_eq!(editor(&h).formula, "Table.SelectRows(Source");
    type_into(&mut h, "Formula of the step", "Text.Upper(");
    assert!(h.query_by_label("Text.Upper(text, optional culture)").is_some());
    assert_eq!(count_sent(&h, "query.edit_step"), 0, "nothing is applied");
}

#[test]
fn a_column_headers_type_menu_changes_its_type() {
    let mut h = opened("en-US", "Transform data");
    preview_columns(&mut h);
    click(&mut h, "Change type of Qty");
    click(&mut h, "Text");
    let p = last_sent(&h, "query.apply_step").expect("the type menu runs query.apply_step");
    assert_eq!(
        p["step"],
        json!({"kind": "change_type", "columns": [{"column": "Qty", "type": "text"}]})
    );
    assert_eq!(p["after"], "Kept");
    preview_columns(&mut h);
    assert_eq!(step_names(&h).last().map(String::as_str), Some("Changed Type"));
}

#[test]
fn a_column_headers_filter_menu_lists_its_values_and_filters_by_them() {
    let mut h = opened("en-US", "Transform data");
    preview_columns(&mut h);
    click(&mut h, "Filter City");
    let p = last_sent(&h, "query.preview").unwrap();
    assert_eq!(p["limit"], 1000, "the values of the first 1000 rows");
    assert_eq!(p["step"], "Kept");
    until(&mut h, "the values", |a| {
        a.cmd
            .pq
            .as_ref()
            .and_then(|e| e.filter.as_ref())
            .is_some_and(|f| matches!(f.values, Some(Ok(_))))
    });
    let f = editor(&h).filter.unwrap();
    let values: Vec<Value> = match f.values {
        Some(Ok(v)) => v.into_iter().map(|x| x.0).collect(),
        other => panic!("{other:?}"),
    };
    assert_eq!(values, [json!("Lyon"), json!("Paris")]);
    checkbox(&mut h, "Lyon");
    click(&mut h, "OK");
    let p = last_sent(&h, "query.apply_step").expect("OK runs query.apply_step");
    assert_eq!(
        p["step"],
        json!({"kind": "filter_values", "column": "City", "values": ["Paris"], "exclude": false})
    );
    preview_columns(&mut h);
    match &editor(&h).preview {
        Some(Ok(p)) => assert_eq!(p.rows.len(), 2),
        other => panic!("{other:?}"),
    }
    assert!(editor(&h).filter.is_none());
    // Text filters › begins with… asks its value, as Filter rows does
    click(&mut h, "Filter Product");
    click(&mut h, "Text filters");
    click(&mut h, "begins with…");
    match editor(&h).form {
        Some(crate::pq_editor::Form::Filter { column, op, .. }) => {
            assert_eq!(column, "Product");
            assert_eq!(crate::pq_editor::OPS[op].0, "begins_with");
        }
        other => panic!("{other:?}"),
    }
    assert!(editor(&h).filter.is_none());
    type_into(&mut h, "Value", "B");
    click(&mut h, "OK");
    let p = last_sent(&h, "query.apply_step").unwrap();
    assert_eq!(
        p["step"],
        json!({"kind": "filter_rows", "column": "Product", "op": "begins_with", "value": "B"})
    );
}

#[test]
fn new_duplicate_and_reference_queries_join_the_drafts() {
    let mut h = opened("en-US", "Transform data");
    preview_columns(&mut h);
    click(&mut h, "Duplicate");
    let p = last_sent(&h, "query.new").expect("Duplicate runs query.new");
    assert_eq!(p["kind"], "duplicate");
    assert_eq!(p["of"], "Sales");
    assert_eq!(p["expression"], SALES);
    preview_columns(&mut h);
    assert_eq!(query_names(&h), ["Sales", "Cities", "Sales (2)"]);
    assert_eq!(editor(&h).selected, Some(2));
    // the queries list's entry (the window is drawn over the Data pane)
    h.get_all_by_label("Sales").last().expect("Sales").click();
    steps(&mut h, 3);
    preview_columns(&mut h);
    click(&mut h, "Reference");
    preview_columns(&mut h);
    let e = editor(&h);
    assert_eq!(e.queries[3].name, "Sales (3)");
    assert_eq!(e.queries[3].draft, "let\n    Source = Sales\nin\n    Source");
    click(&mut h, "New query");
    assert_eq!(last_sent(&h, "query.new").unwrap()["kind"], "blank");
    until(&mut h, "the blank query", |a| {
        a.cmd.pq.as_ref().is_some_and(|e| e.queries.len() == 5 && !e.busy)
    });
    assert_eq!(editor(&h).queries[4].name, "Query1");
    // nothing in the model until Close & Apply
    assert_eq!(count_sent(&h, "query.apply"), 0);
    assert!(editor(&h).has_changes());
}

#[test]
fn rename_and_delete_queries_in_the_drafts_then_close_and_apply_once() {
    let mut h = opened("en-US", "Transform data");
    preview_columns(&mut h);
    click(&mut h, "Rename query");
    type_into(&mut h, "New query name", "Orders");
    click(&mut h, "Rename");
    let p = last_sent(&h, "query.rename").expect("Rename runs query.rename");
    assert_eq!(p["name"], "Sales");
    assert_eq!(p["new_name"], "Orders");
    assert_eq!(p["queries"].as_array().map(Vec::len), Some(2));
    until(&mut h, "the rename", |a| {
        a.cmd.pq.as_ref().is_some_and(|e| e.queries[0].name == "Orders")
    });
    assert!(editor(&h).queries[0].renamed());
    // Cities from its context menu: asked first
    h.get_by_label("Cities").click_secondary();
    steps(&mut h, 3);
    // the context menu's entry (drawn last)
    header(&mut h, "Delete query");
    assert_eq!(editor(&h).confirm, Some(crate::pq_editor::Confirm::Delete(1)));
    assert_eq!(query_names(&h), ["Orders", "Cities"]);
    click(&mut h, "Keep it");
    assert!(editor(&h).confirm.is_none());
    h.get_by_label("Cities").click_secondary();
    steps(&mut h, 3);
    header(&mut h, "Delete query");
    click(&mut h, "Delete");
    assert_eq!(query_names(&h), ["Orders"]);
    assert_eq!(editor(&h).removed, ["Cities"]);
    preview_columns(&mut h);
    click(&mut h, "Close & Apply");
    let p = last_sent(&h, "query.apply").expect("Close & Apply runs query.apply");
    assert_eq!(p["renamed"], json!([{"name": "Sales", "new_name": "Orders"}]));
    assert_eq!(p["removed"], json!(["Cities"]));
    assert_eq!(p["queries"], json!([]));
    until(&mut h, "the editor closes", |a| a.cmd.pq.is_none());
    until(&mut h, "the table is renamed", |a| {
        a.loaded
            .as_ref()
            .is_some_and(|l| l.snap.pointer("/model/tables/0/name") == Some(&json!("Orders")))
    });
    assert_eq!(count_sent(&h, "query.apply"), 1);
}

#[test]
fn closing_with_changes_asks_first() {
    let mut h = opened("en-US", "Transform data");
    preview_columns(&mut h);
    click(&mut h, "City");
    click(&mut h, "Remove columns");
    preview_columns(&mut h);
    click(&mut h, "Discard changes");
    assert_eq!(editor(&h).confirm, Some(crate::pq_editor::Confirm::Close));
    click(&mut h, "Keep editing");
    assert!(editor(&h).confirm.is_none());
    assert!(editor(&h).queries[0].changed(), "the draft is kept");
    click(&mut h, "Discard changes");
    click(&mut h, "Discard");
    assert!(h.state().cmd.pq.is_none());
    assert_eq!(count_sent(&h, "query.apply"), 0);
    // without changes, Close closes at once
    click(&mut h, "Transform data");
    until(&mut h, "the steps", |a| {
        a.cmd.pq.as_ref().is_some_and(|e| e.step.is_some())
    });
    click(&mut h, "Close");
    assert!(h.state().cmd.pq.is_none());
}

#[test]
fn the_editor_speaks_french() {
    let mut h = opened("fr-FR", "Transformer les données");
    preview_columns(&mut h);
    h.get_by_label("Étapes appliquées");
    h.get_by_label("Fermer");
    h.get_by_label("Supprimer les colonnes");
    h.get_by_label("Renommer l'étape");
    h.get_by_label("Éditeur avancé");
    assert!(h.get_all_by_label("Mettre en forme").next().is_some());
    h.get_by_label("Nouvelle requête");
    h.get_by_label("Dupliquer");
    h.get_by_label("Filtrer City");
    h.get_by_label("Modifier le type de Qty");
}

/// Opens the editor's View tab (its label is also the main ribbon's: the editor's is drawn last).
fn view_tab(h: &mut Harness<'_, App>) {
    header(h, "View");
    assert_eq!(editor(h).tab, crate::pq_editor::EditorTab::View);
}

fn profiled(h: &mut Harness<'_, App>) -> crate::pq_editor::Profile {
    until(h, "the profile", |a| {
        a.cmd
            .pq
            .as_ref()
            .is_some_and(|e| matches!(e.profiled, Some(Ok(_))))
    });
    match editor(h).profiled {
        Some(Ok(p)) => p,
        other => panic!("{other:?}"),
    }
}

#[test]
fn view_column_quality_and_distribution_profile_each_column_under_its_header() {
    let mut h = opened("en-US", "Transform data");
    preview_columns(&mut h);
    view_tab(&mut h);
    assert_eq!(count_sent(&h, "query.profile"), 0, "profiling is off at first");
    click(&mut h, "Column quality");
    let p = last_sent(&h, "query.profile").expect("Column quality runs query.profile");
    assert_eq!(p["step"], "Kept");
    assert_eq!(p["full"], false);
    assert!(p.get("column").is_none());
    let pr = profiled(&mut h);
    assert_eq!(pr.rows, 3);
    let qty = pr.columns.iter().find(|c| c.name == "Qty").expect("Qty");
    assert_eq!((qty.valid, qty.error, qty.empty), (1.0, 0.0, 0.0));
    h.get_by_label("Quality of Qty: Valid 100%, Error 0%, Empty 0%");
    assert!(h.query_by_label_contains("Distribution of Qty").is_none());
    click(&mut h, "Column distribution");
    profiled(&mut h);
    // Paris twice, Lyon once
    h.get_by_label("Distribution of City: 2 distinct, 1 unique");
    // another step: profiled again
    let before = count_sent(&h, "query.profile");
    click(&mut h, "Source");
    assert_eq!(count_sent(&h, "query.profile"), before + 1);
    assert_eq!(last_sent(&h, "query.profile").unwrap()["step"], "Source");
    // both off: no profile
    click(&mut h, "Column quality");
    click(&mut h, "Column distribution");
    assert!(editor(&h).profiled.is_none());
    assert!(h.query_by_label_contains("Quality of").is_none());
}

#[test]
fn view_column_profile_shows_the_picked_columns_statistics_in_a_bottom_pane() {
    let mut h = opened("en-US", "Transform data");
    preview_columns(&mut h);
    view_tab(&mut h);
    click(&mut h, "Column profile");
    profiled(&mut h);
    h.get_by_label("Click a column header to see its profile.");
    header(&mut h, "Qty");
    let p = last_sent(&h, "query.profile").unwrap();
    assert_eq!(p["column"], "Qty");
    let pr = profiled(&mut h);
    let stats = &pr.column.expect("the column profile")["statistics"];
    assert_eq!(stats["max"], 9.0);
    h.get_by_label("Column statistics · Qty");
    h.get_by_label("Value distribution");
    h.get_by_label("Standard deviation");
}

#[test]
fn profiling_reads_the_top_1000_rows_until_the_entire_data_set_is_asked() {
    let mut h = opened("en-US", "Transform data");
    preview_columns(&mut h);
    view_tab(&mut h);
    click(&mut h, "Column quality");
    profiled(&mut h);
    click(&mut h, "Profile the entire data set");
    assert_eq!(last_sent(&h, "query.profile").unwrap()["full"], true);
    profiled(&mut h);
    // the status line under the preview switches back
    click(&mut h, "Column profiling based on the entire data set");
    assert_eq!(last_sent(&h, "query.profile").unwrap()["full"], false);
    profiled(&mut h);
    h.get_by_label("Column profiling based on top 1000 rows");
}

#[test]
fn the_column_profile_command_opens_the_editor_with_profiling_on() {
    let mut h = app("en-US");
    assert!(crate::pq_editor::intercept(
        h.state_mut(),
        "query.profile",
        &json!({})
    ));
    until(&mut h, "the steps", |a| {
        a.cmd.pq.as_ref().is_some_and(|e| e.step.is_some())
    });
    let e = editor(&h);
    assert_eq!(e.tab, crate::pq_editor::EditorTab::View);
    assert!(e.quality && e.distribution && e.profile);
    profiled(&mut h);
    // a call with its values is the command's, not the window's
    assert!(!crate::pq_editor::intercept(
        h.state_mut(),
        "query.profile",
        &json!({"name": "Sales"})
    ));
}

#[test]
fn the_view_tab_speaks_french() {
    let mut h = opened("fr-FR", "Transformer les données");
    preview_columns(&mut h);
    header(&mut h, "Affichage");
    click(&mut h, "Qualité de colonne");
    profiled(&mut h);
    h.get_by_label("Qualité de Qty: Valide 100 %, Erreur 0 %, Vide 0 %");
    h.get_by_label("Profiler l'ensemble du jeu de données");
    h.get_by_label("Profilage de colonne basé sur les 1000 premières lignes");
    h.get_by_label("Distribution de colonne");
    h.get_by_label("Profil de colonne");
}

fn graph(h: &mut Harness<'_, App>) -> crate::pq_deps::Graph {
    until(h, "the dependencies", |a| {
        a.cmd
            .pq
            .as_ref()
            .and_then(|e| e.deps.as_ref())
            .is_some_and(|d| matches!(d.graph, Some(Ok(_))))
    });
    match editor(h).deps.and_then(|d| d.graph) {
        Some(Ok(g)) => g,
        other => panic!("{other:?}"),
    }
}

fn zoom(h: &Harness<'_, App>) -> f32 {
    editor(h).deps.expect("the dependencies view").zoom
}

#[test]
fn view_query_dependencies_draws_the_queries_and_the_tables_they_load() {
    let mut h = opened("en-US", "Transform data");
    preview_columns(&mut h);
    view_tab(&mut h);
    assert_eq!(count_sent(&h, "query.dependencies"), 0);
    click(&mut h, "Query dependencies");
    let p = last_sent(&h, "query.dependencies").expect("the button runs query.dependencies");
    let names: Vec<&str> = p["queries"]
        .as_array()
        .expect("the drafts")
        .iter()
        .filter_map(|q| q["name"].as_str())
        .collect();
    assert!(names.contains(&"Sales") && names.contains(&"Cities"), "{p}");
    let g = graph(&mut h);
    assert!(
        g.edges
            .iter()
            .any(|&(a, b)| g.nodes[a].name == "Sales" && g.nodes[b].kind == "model_table"),
        "{g:?}"
    );
    h.get_by_label("Query (loaded) Sales");
    h.get_by_label("Model table Sales");
    h.get_by_label("Query Cities");
    // zoom and fit
    let fitted = zoom(&h);
    click(&mut h, "Zoom in");
    assert!(zoom(&h) > fitted);
    click(&mut h, "Zoom out");
    click(&mut h, "Zoom out");
    assert!(zoom(&h) < fitted);
    click(&mut h, "Fit");
    assert!((zoom(&h) - fitted).abs() < 0.01, "{} {fitted}", zoom(&h));
    // closed: back to the preview
    click(&mut h, "Close dependencies");
    assert!(editor(&h).deps.is_none());
    h.get_by_label("Applied steps");
}

#[test]
fn the_query_dependencies_command_opens_the_editor_on_the_diagram() {
    let mut h = app("en-US");
    assert!(crate::pq_editor::intercept(
        h.state_mut(),
        "query.dependencies",
        &json!({})
    ));
    let g = graph(&mut h);
    assert!(g.nodes.iter().any(|n| n.name == "Cities"), "{g:?}");
    assert_eq!(editor(&h).tab, crate::pq_editor::EditorTab::View);
    // a call with its queries is the command's, not the window's
    assert!(!crate::pq_editor::intercept(
        h.state_mut(),
        "query.dependencies",
        &json!({"queries": []})
    ));
}

#[test]
fn the_dependencies_view_speaks_french() {
    let mut h = opened("fr-FR", "Transformer les données");
    preview_columns(&mut h);
    header(&mut h, "Affichage");
    click(&mut h, "Dépendances de requête");
    graph(&mut h);
    h.get_by_label("Requête (chargée) Sales");
    h.get_by_label("Table du modèle Sales");
    h.get_by_label("Zoom avant");
    h.get_by_label("Ajuster");
    h.get_by_label("Fermer les dépendances");
}

#[test]
fn the_ribbon_groups_icon_buttons_in_one_band() {
    // as the main ribbon: small icon buttons stacked in columns, big ones for the main actions,
    // every button in one band of the ribbon's height (no wrapped rows of text buttons)
    let h = opened("en-US", "Transform data");
    let r = |l: &str| h.get_by_label(l).rect();
    let (remove, keep, rename) = (r("Remove columns"), r("Keep columns"), r("Rename column"));
    assert!((remove.min.x - keep.min.x).abs() < 1.0, "{remove:?} {keep:?}");
    assert!(keep.min.y > remove.max.y - 1.0 && rename.min.y > keep.max.y - 1.0);
    let close = r("Close");
    assert!(
        close.height() > 2.0 * remove.height(),
        "Close is a big button: {close:?}"
    );
    let labels = [
        "Close",
        "Advanced editor",
        "Change type",
        "Filter rows",
        "Remove duplicates",
        "Group by",
        "Append queries",
        "Pivot column",
        "Custom column",
    ];
    let top = labels.iter().map(|l| r(l).min.y).fold(f32::MAX, f32::min);
    let bottom = labels.iter().map(|l| r(l).max.y).fold(f32::MIN, f32::max);
    assert!(bottom - top <= crate::widgets::RIBBON_H, "{top} {bottom}");
}
