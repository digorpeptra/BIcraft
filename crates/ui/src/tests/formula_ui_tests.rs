//! The DAX formula bar on an engine session (`formula.rs`): a measure picked in the Data pane
//! shows its DAX; Enter commits through the engine, Esc reverts, Ctrl+Z restores; a new measure;
//! a rename through the name part; the live check's error; autocomplete; a reader's read-only
//! bar; a calculated column and a calculated table; the same frames in both shells; a
//! 10,000-line expression.

use std::path::Path;
use std::time::Instant;

use bicraft_engine::Session;
use egui::accesskit::Role;
use egui::{Event, Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::backstage_tests::{engine_shell, steps};
use super::frame;
use crate::App;
use crate::formula::{Kind, Target};
use crate::model_view::Sel;
use crate::registry::Tab as RTab;
use crate::shell::Caps;

/// Sales(Category, Amount, Double = Amount * 2) with [Total]; Big, a calculated table; a page
/// with a card of [Total].
fn session() -> Session {
    session_with(None)
}

fn session_with(extra: Option<(&str, String)>) -> Session {
    let mut s = Session::new();
    let sales = r#"#table({"Category", "Amount"}, {{"Wheels", 100}, {"Toys", 5}, {"Games", 20}})"#;
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
            "column.add",
            json!({"table": "Sales", "name": "Double", "data_type": "double", "expression": "Sales[Amount] * 2"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])", "format_string": "#,0"}),
        ),
        (
            "table.add",
            json!({"name": "Big", "partition": {"source_type": "calculated", "expression": "FILTER(Sales, Sales[Amount] > 10)"}}),
        ),
        ("report.new", json!({})),
    ] {
        s.execute(id, &p).unwrap();
    }
    if let Some((name, expression)) = extra {
        s.execute(
            "measure.add",
            &json!({"table": "Sales", "name": name, "expression": expression, "force": true}),
        )
        .unwrap();
    }
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    s.execute(
        "visual.add",
        &json!({"page": page, "visual_type": "card", "x": 40, "y": 40, "width": 240, "height": 120,
                "fields": [{"role": "Values", "field": "[Total]"}]}),
    )
    .unwrap();
    s.dirty = false;
    s.report_dirty = false;
    s
}

fn open_with(caps: Caps, reader: bool, make: impl Fn() -> Session + 'static) -> Harness<'static, App> {
    let mut app = App::new("en-US", engine_shell(caps, make)).with_sync_render();
    app.open_path(Path::new("shop.pbip"));
    // 60 frames a second, as a window draws while typing (kittest's default is 4)
    let mut h = Harness::builder()
        .with_size(egui::vec2(1400.0, 860.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(
            |ui, app: &mut App| {
                app.logic(ui.ctx());
                app.ui(ui);
            },
            app,
        );
    steps(&mut h, 6);
    if reader {
        h.state_mut().access = Some("reader".into());
    }
    for _ in 0..200 {
        h.step();
        if h.state().shown.is_some() {
            break;
        }
    }
    h.state_mut().panes.open.insert("t:Sales".into());
    steps(&mut h, 2);
    h
}

fn open(reader: bool) -> Harness<'static, App> {
    open_with(Caps::DESKTOP, reader, session)
}

fn press(h: &mut Harness<'_, App>, key: Key, modifiers: Modifiers) {
    for pressed in [true, false] {
        h.event_modifiers(
            Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers,
            },
            modifiers,
        );
    }
    steps(h, 3);
}

fn type_text(h: &mut Harness<'_, App>, s: &str) {
    h.event(Event::Text(s.into()));
    steps(h, 2);
}

/// The text the bar shows.
fn bar_text(h: &Harness<'_, App>) -> String {
    h.state().formula.text.clone()
}

/// Puts the cursor in the editor, at the end of its text.
fn focus_end(h: &mut Harness<'_, App>) {
    // near its top left: a long text's node reaches below the window
    let r = h.get_by_role(Role::MultilineTextInput).rect();
    let pos = r.min + egui::vec2(8.0, 6.0);
    h.hover_at(pos);
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
    steps(h, 2);
    let end = h.state().formula.text.chars().count();
    h.state_mut().formula.cursor_to = Some(end);
    steps(h, 2);
    assert!(crate::formula::has_focus(&h.ctx), "the editor has the keyboard");
}

/// Measure `name`'s expression in the latest snapshot.
fn expression(h: &Harness<'_, App>, name: &str) -> Option<String> {
    let snap = h.state().loaded.as_ref().unwrap().snap.clone();
    snap["model"]["tables"]
        .as_array()?
        .iter()
        .flat_map(|t| t["measures"].as_array().cloned().unwrap_or_default())
        .find(|m| m["name"] == name)
        .and_then(|m| m["expression"].as_str().map(str::to_string))
}

fn sent_ids(h: &Harness<'_, App>) -> Vec<&'static str> {
    h.state().sent.iter().map(|c| c.id).collect()
}

/// Waits (frames) until `ok`.
fn until(h: &mut Harness<'_, App>, what: &str, ok: impl Fn(&App) -> bool) {
    for _ in 0..400 {
        if ok(h.state()) {
            return;
        }
        h.step();
    }
    panic!("never: {what}");
}

fn pick_total(h: &mut Harness<'_, App>) {
    h.get_by_label("Total").click();
    steps(h, 4);
    until(h, "Total's DAX", |a| {
        a.formula.text == "Total = SUM(Sales[Amount])"
    });
}

#[test]
fn a_measure_picked_in_the_data_pane_shows_its_dax() {
    let mut h = open(false);
    assert!(h.state().formula.target.is_none(), "no bar without a selection");
    assert!(h.query_by_role(Role::MultilineTextInput).is_none());
    pick_total(&mut h);
    assert_eq!(
        h.state().formula.target,
        Some(Target {
            kind: Kind::Measure,
            table: "Sales".into(),
            name: "Total".into(),
            new: false
        })
    );
    assert!(
        sent_ids(&h).contains(&"measure.get"),
        "the full expression is read"
    );
    // its home table and format next to it
    assert!(h.query_by_label_contains("Sales · Format #,0").is_some());
    // the live check ran on the loaded text: valid, nothing under the bar
    until(&mut h, "the check", |a| {
        a.sent.iter().any(|c| c.id == "dax.validate")
    });
    assert!(h.state().formula.diagnostics.is_empty());
}

#[test]
fn enter_commits_esc_reverts_and_undo_restores() {
    let mut h = open(false);
    pick_total(&mut h);
    // Esc: the edit goes, the model keeps its measure
    focus_end(&mut h);
    type_text(&mut h, " * 2");
    assert_eq!(bar_text(&h), "Total = SUM(Sales[Amount]) * 2");
    press(&mut h, Key::Escape, Modifiers::NONE);
    assert_eq!(bar_text(&h), "Total = SUM(Sales[Amount])");
    assert!(!sent_ids(&h).contains(&"measure.set"));
    assert!(h.state().formula.target.is_some(), "Esc on an edit keeps the bar");
    // Shift+Enter and Alt+Enter add lines; Enter commits
    focus_end(&mut h);
    press(&mut h, Key::Enter, Modifiers::SHIFT);
    type_text(&mut h, "* 2");
    press(&mut h, Key::Enter, Modifiers::ALT);
    type_text(&mut h, "+ 1");
    assert_eq!(bar_text(&h), "Total = SUM(Sales[Amount])\n* 2\n+ 1");
    press(&mut h, Key::Enter, Modifiers::NONE);
    steps(&mut h, 4);
    assert_eq!(sent_ids(&h).iter().filter(|i| **i == "measure.set").count(), 1);
    assert_eq!(
        expression(&h, "Total").as_deref(),
        Some("SUM(Sales[Amount])\n* 2\n+ 1")
    );
    assert!(!crate::formula::has_focus(&h.ctx), "the commit leaves the editor");
    assert!(!h.state().formula.dirty());
    // Ctrl+Z: the engine's undo, the bar shows the previous expression again
    press(&mut h, Key::Z, Modifiers::COMMAND);
    steps(&mut h, 4);
    assert!(sent_ids(&h).contains(&"edit.undo"));
    assert_eq!(expression(&h, "Total").as_deref(), Some("SUM(Sales[Amount])"));
    until(&mut h, "the old text", |a| {
        a.formula.text == "Total = SUM(Sales[Amount])"
    });
}

#[test]
fn a_new_measure_is_written_in_the_bar() {
    let mut h = open(false);
    h.state_mut().ribbon_tab = RTab::Home;
    steps(&mut h, 3);
    h.get_by_label("New measure").click();
    steps(&mut h, 3);
    let t = h.state().formula.target.clone().expect("the bar is open");
    assert_eq!(
        (t.kind, t.table.as_str(), t.name.as_str(), t.new),
        (Kind::Measure, "Sales", "Measure", true)
    );
    assert_eq!(bar_text(&h), "Measure = ");
    assert!(h.state().form.is_none(), "no form");
    assert!(crate::formula::has_focus(&h.ctx), "focused");
    type_text(&mut h, "[Total] / 2");
    press(&mut h, Key::Enter, Modifiers::NONE);
    steps(&mut h, 4);
    let add = h
        .state()
        .sent
        .iter()
        .rev()
        .find(|c| c.id == "measure.add")
        .expect("measure.add");
    assert_eq!(
        add.params,
        json!({"table": "Sales", "name": "Measure", "expression": "[Total] / 2", "force": true})
    );
    assert_eq!(expression(&h, "Measure").as_deref(), Some("[Total] / 2"));
    let t = h.state().formula.target.clone().unwrap();
    assert!(!t.new, "the bar now edits the new measure");
    // the next one gets the next free name; Esc drops it
    h.get_by_label("New measure").click();
    steps(&mut h, 3);
    assert_eq!(bar_text(&h), "Measure 2 = ");
    press(&mut h, Key::Escape, Modifiers::NONE);
    assert!(h.state().formula.target.is_none());
    assert_eq!(expression(&h, "Measure 2"), None);
}

#[test]
fn the_name_part_renames_and_the_report_follows() {
    let mut h = open(false);
    pick_total(&mut h);
    focus_end(&mut h);
    // the name: before the first =
    press(&mut h, Key::Home, Modifiers::COMMAND);
    for _ in 0.."Total".len() {
        press(&mut h, Key::Delete, Modifiers::NONE);
    }
    type_text(&mut h, "Sales total");
    assert_eq!(bar_text(&h), "Sales total = SUM(Sales[Amount])");
    press(&mut h, Key::Enter, Modifiers::NONE);
    steps(&mut h, 4);
    let set = h
        .state()
        .sent
        .iter()
        .rev()
        .find(|c| c.id == "measure.set")
        .unwrap();
    assert_eq!(set.params["new_name"], "Sales total");
    assert_eq!(set.params["update_report"], true);
    assert_eq!(expression(&h, "Total"), None);
    assert_eq!(
        expression(&h, "Sales total").as_deref(),
        Some("SUM(Sales[Amount])")
    );
    let snap = h.state().loaded.as_ref().unwrap().snap.clone();
    assert_eq!(
        snap["report"]["pages"][0]["visuals"][0]["fields"][0]["text"],
        "[Sales total]"
    );
    assert_eq!(h.state().formula.target.as_ref().unwrap().name, "Sales total");
    until(&mut h, "the reloaded text", |a| {
        a.formula.text == "Sales total = SUM(Sales[Amount])"
    });
    // one undo step for the rename and the expression
    press(&mut h, Key::Z, Modifiers::COMMAND);
    steps(&mut h, 4);
    assert_eq!(expression(&h, "Total").as_deref(), Some("SUM(Sales[Amount])"));
}

#[test]
fn the_live_check_underlines_the_error() {
    let mut h = open(false);
    pick_total(&mut h);
    focus_end(&mut h);
    let checks = |h: &Harness<'_, App>| sent_ids(h).iter().filter(|i| **i == "dax.validate").count();
    let before = checks(&h);
    press(&mut h, Key::Backspace, Modifiers::NONE);
    press(&mut h, Key::Backspace, Modifiers::NONE);
    type_text(&mut h, "x])");
    // no check while typing goes on: one after the pause
    assert_eq!(checks(&h), before, "the check waits for typing to stop");
    until(&mut h, "the diagnostic", |a| !a.formula.diagnostics.is_empty());
    assert_eq!(checks(&h), before + 1);
    let d = h.state().formula.diagnostics[0].clone();
    let text = bar_text(&h);
    assert_eq!(
        &text[d.span.clone()],
        "Sales[Amountx]",
        "the span is in the bar's text"
    );
    steps(&mut h, 2);
    assert!(
        h.query_by_label_contains("(line 1, column").is_some(),
        "the message under the bar"
    );
    // Enter: as Desktop, the measure is saved with its error, which stays under the bar
    press(&mut h, Key::Enter, Modifiers::NONE);
    steps(&mut h, 4);
    let p = h
        .state()
        .sent
        .iter()
        .rev()
        .find(|c| c.id == "measure.set")
        .map(|c| c.params.clone())
        .unwrap();
    assert_eq!(p["force"], true, "{p}");
    assert_eq!(expression(&h, "Total").as_deref(), Some("SUM(Sales[Amountx])"));
    assert!(!h.state().formula.dirty());
    let e = h.state().formula.error.clone().unwrap_or_default();
    assert!(e.starts_with("Saved with an error:"), "{e}");
    steps(&mut h, 2);
    assert!(h.query_by_label_contains("Saved with an error:").is_some());
}

/// A click away from the bar commits the edit, as Desktop does; a click on the bar's own
/// buttons (Cancel) does not (docs/gaps.md G45).
#[test]
fn a_click_elsewhere_commits_the_edit() {
    let mut h = open(false);
    pick_total(&mut h);
    // Cancel: no commit
    focus_end(&mut h);
    type_text(&mut h, " * 3");
    h.get_by_label("Cancel (Esc)").click();
    steps(&mut h, 4);
    assert!(!sent_ids(&h).contains(&"measure.set"), "Cancel does not commit");
    assert_eq!(bar_text(&h), "Total = SUM(Sales[Amount])");
    // a click on the page: committed
    focus_end(&mut h);
    type_text(&mut h, " * 3");
    let pos = egui::pos2(900.0, 700.0);
    h.hover_at(pos);
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.step();
    }
    steps(&mut h, 4);
    assert!(!crate::formula::has_focus(&h.ctx));
    assert_eq!(sent_ids(&h).iter().filter(|i| **i == "measure.set").count(), 1);
    assert_eq!(expression(&h, "Total").as_deref(), Some("SUM(Sales[Amount]) * 3"));
    assert!(!h.state().formula.dirty());
    // another object picked while editing: the edit is committed first
    focus_end(&mut h);
    type_text(&mut h, " + 1");
    h.get_by_label("Double").click();
    steps(&mut h, 6);
    assert_eq!(
        expression(&h, "Total").as_deref(),
        Some("SUM(Sales[Amount]) * 3 + 1")
    );
}

/// Format DAX lays the expression out in BIcraft's formula style (after SQLBI's public rules)
/// through `dax.format`; the text changes on the bar and the commit stays with the user.
#[test]
fn format_dax_lays_the_formula_out() {
    let mut h = open(false);
    pick_total(&mut h);
    h.get_by_label("Format DAX").click();
    steps(&mut h, 4);
    let p = h
        .state()
        .sent
        .iter()
        .rev()
        .find(|c| c.id == "dax.format")
        .map(|c| c.params.clone())
        .unwrap();
    assert_eq!(p, json!({"query": "SUM(Sales[Amount])", "style": "formula"}));
    assert_eq!(bar_text(&h), "Total = SUM ( Sales[Amount] )");
    assert!(h.state().formula.dirty());
    assert!(!sent_ids(&h).contains(&"measure.set"));
    focus_end(&mut h);
    press(&mut h, Key::Enter, Modifiers::NONE);
    steps(&mut h, 4);
    assert_eq!(expression(&h, "Total").as_deref(), Some("SUM ( Sales[Amount] )"));
    // a long expression breaks over lines, and the bar expands
    focus_end(&mut h);
    type_text(
        &mut h,
        " + calculate(sumx(filter(Sales, Sales[Amount] > 10), Sales[Amount] * 2), all(Sales[Category]))",
    );
    h.get_by_label("Format DAX").click();
    steps(&mut h, 4);
    assert!(
        bar_text(&h).contains("CALCULATE (\n    SUMX ("),
        "{}",
        bar_text(&h)
    );
    assert!(h.state().formula.expanded);
    // text that does not parse: the error under the bar, the text unchanged
    focus_end(&mut h);
    type_text(&mut h, " +");
    let before = bar_text(&h);
    h.get_by_label("Format DAX").click();
    steps(&mut h, 4);
    assert_eq!(bar_text(&h), before);
    assert!(
        h.state()
            .formula
            .error
            .as_deref()
            .is_some_and(|e| e.contains("does not parse"))
    );
}

/// The autocomplete shows the selected function's signature and BIcraft's description of it,
/// in the UI's language.
#[test]
fn the_completion_describes_the_selected_function() {
    let want = Session::new()
        .execute("dax.functions", &json!({"query": "DIVIDE"}))
        .unwrap();
    let f = want["functions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == "DIVIDE")
        .unwrap()
        .clone();
    let en = f["description"].as_str().unwrap().to_string();
    let fr = f["description_fr"].as_str().unwrap().to_string();
    assert!(!en.is_empty() && !fr.is_empty() && en != fr);
    let mut h = open(false);
    pick_total(&mut h);
    until(&mut h, "the function list", |a| {
        a.sent.iter().any(|c| c.id == "dax.functions")
    });
    focus_end(&mut h);
    type_text(&mut h, " + DIVI");
    steps(&mut h, 2);
    assert!(h.query_by_label_contains(&en).is_some(), "the description");
    assert!(
        h.query_by_label_contains("DIVIDE(numerator, denominator[, alternate_result])")
            .is_some()
    );
    let func = crate::dax_text::Func {
        name: "DIVIDE".into(),
        signature: "DIVIDE(a, b)".into(),
        params: Vec::new(),
        description: (en.clone(), fr.clone()),
    };
    assert_eq!(
        crate::formula::function_detail(&func, crate::i18n::Lang::Fr),
        format!("DIVIDE(a, b)\n{fr}")
    );
    let bare = crate::dax_text::Func {
        description: (en.clone(), String::new()),
        ..func
    };
    assert_eq!(
        crate::formula::function_detail(&bare, crate::i18n::Lang::Fr),
        format!("DIVIDE(a, b)\n{en}")
    );
}

#[test]
fn autocomplete_inserts_a_column_and_shows_the_signature() {
    let mut h = open(false);
    pick_total(&mut h);
    until(&mut h, "the function list", |a| {
        a.sent.iter().any(|c| c.id == "dax.functions")
    });
    focus_end(&mut h);
    type_text(&mut h, " + Amou");
    let c = h.state().formula.completion.clone().expect("the list is open");
    assert_eq!(c.items[0].insert, "'Sales'[Amount]");
    h.get_by_label("'Sales'[Amount]");
    press(&mut h, Key::Enter, Modifiers::NONE);
    assert_eq!(bar_text(&h), "Total = SUM(Sales[Amount]) + 'Sales'[Amount]");
    assert!(h.state().formula.completion.is_none());
    assert!(
        !sent_ids(&h).contains(&"measure.set"),
        "Enter took the suggestion, not the commit"
    );
    // a function: its signature while the arguments are typed
    type_text(&mut h, " + DIVI");
    let c = h.state().formula.completion.clone().expect("functions");
    assert_eq!(c.items[0].insert, "DIVIDE(");
    press(&mut h, Key::Tab, Modifiers::NONE);
    assert!(bar_text(&h).ends_with("+ DIVIDE("));
    type_text(&mut h, "1, ");
    steps(&mut h, 2);
    assert!(
        h.query_by_label("DIVIDE(numerator, denominator[, alternate_result])")
            .is_some(),
        "the signature help"
    );
    // [ lists the measures; a table's [ its columns
    type_text(&mut h, "[To");
    let c = h.state().formula.completion.clone().expect("measures");
    assert_eq!(c.items[0].insert, "[Total]");
    press(&mut h, Key::Escape, Modifiers::NONE);
    assert!(h.state().formula.completion.is_none(), "Esc closes the list only");
    assert!(crate::formula::has_focus(&h.ctx));
    assert!(h.state().formula.dirty());
}

#[test]
fn a_reader_sees_the_dax_read_only() {
    let mut h = open(true);
    pick_total(&mut h);
    let sent = h.state().sent.len();
    h.get_by_role(Role::MultilineTextInput).click();
    steps(&mut h, 2);
    type_text(&mut h, "zz");
    press(&mut h, Key::Enter, Modifiers::NONE);
    assert_eq!(bar_text(&h), "Total = SUM(Sales[Amount])");
    assert!(
        h.state().sent[sent..]
            .iter()
            .all(|c| c.id == "dax.validate" || c.id == "dax.functions"),
        "a reader commits nothing"
    );
    assert!(h.query_by_label_contains("Read-only:").is_some());
    // New measure is greyed for a reader: no bar
    h.state_mut().formula = Default::default();
    crate::ribbon::invoke(h.state_mut(), "measure.add", json!({}));
    steps(&mut h, 2);
    assert!(h.state().formula.target.is_none());
}

#[test]
fn a_calculated_column_and_a_calculated_table() {
    let mut h = open(false);
    // the model diagram's selection
    h.state_mut().show_tab(crate::app::Tab::Model);
    steps(&mut h, 3);
    h.state_mut().model_view.sel = Some(Sel::Column {
        table: "Sales".into(),
        name: "Double".into(),
    });
    steps(&mut h, 3);
    until(&mut h, "the column's DAX", |a| {
        a.formula.text == "Double = Sales[Amount] * 2"
    });
    focus_end(&mut h);
    press(&mut h, Key::Backspace, Modifiers::NONE);
    type_text(&mut h, "3");
    press(&mut h, Key::Enter, Modifiers::NONE);
    steps(&mut h, 4);
    let set = h
        .state()
        .sent
        .iter()
        .rev()
        .find(|c| c.id == "column.set")
        .expect("column.set");
    assert_eq!(
        set.params,
        json!({"table": "Sales", "name": "Double", "expression": "Sales[Amount] * 3", "force": true})
    );
    assert!(h.state().formula.error.is_none(), "{:?}", h.state().formula.error);
    until(&mut h, "the reloaded column", |a| {
        a.formula.text == "Double = Sales[Amount] * 3" && !a.formula.loading
    });
    // a data column has no formula: the bar closes
    h.state_mut().model_view.sel = Some(Sel::Column {
        table: "Sales".into(),
        name: "Amount".into(),
    });
    steps(&mut h, 3);
    assert!(h.state().formula.target.is_none());
    // a calculated table, by the diagram's selection: its partition's DAX; the new expression
    // and the new name are one command (one undo step), and the columns follow the expression
    h.state_mut().model_view.sel = Some(Sel::Table("Big".into()));
    steps(&mut h, 3);
    until(&mut h, "the table's DAX", |a| {
        a.formula.text == "Big = FILTER(Sales, Sales[Amount] > 10)"
    });
    focus_end(&mut h);
    press(&mut h, Key::Backspace, Modifiers::NONE);
    press(&mut h, Key::Backspace, Modifiers::NONE);
    press(&mut h, Key::Backspace, Modifiers::NONE);
    type_text(&mut h, "50)");
    press(&mut h, Key::Home, Modifiers::COMMAND);
    type_text(&mut h, "Very");
    press(&mut h, Key::Enter, Modifiers::NONE);
    steps(&mut h, 6);
    let ids = sent_ids(&h);
    assert!(!ids.contains(&"table.rename"), "{ids:?}");
    let set = h
        .state()
        .sent
        .iter()
        .rev()
        .find(|c| c.id == "partition.set")
        .expect("partition.set");
    assert_eq!(set.params["new_name"], "VeryBig");
    assert_eq!(set.params["expression"], "FILTER(Sales, Sales[Amount] > 50)");
    assert!(h.state().formula.error.is_none(), "{:?}", h.state().formula.error);
    assert_eq!(h.state().formula.target.as_ref().unwrap().name, "VeryBig");
    until(&mut h, "the renamed table", |a| {
        a.formula.text == "VeryBig = FILTER(Sales, Sales[Amount] > 50)"
    });
    let columns = |h: &Harness<'_, App>, table: &str| -> Vec<String> {
        let snap = h.state().loaded.as_ref().unwrap().snap.clone();
        snap["model"]["tables"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == table)
            .map(|t| {
                t["columns"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(|c| c["name"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    assert!(
        columns(&h, "VeryBig").contains(&"Amount".to_string()),
        "the columns come from the expression: {:?}",
        columns(&h, "VeryBig")
    );
    // one Ctrl+Z takes the name and the columns back (the table had none before)
    press(&mut h, Key::Z, Modifiers::COMMAND);
    steps(&mut h, 4);
    assert_eq!(sent_ids(&h).iter().filter(|i| **i == "edit.undo").count(), 1);
    until(&mut h, "the old table", |a| {
        let snap = a.loaded.as_ref().unwrap().snap.clone();
        let names: Vec<String> = snap["model"]["tables"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|t| t["name"].as_str().map(str::to_string))
            .collect();
        names.contains(&"Big".to_string()) && !names.contains(&"VeryBig".to_string())
    });
    assert!(columns(&h, "Big").is_empty(), "{:?}", columns(&h, "Big"));
}

/// Ctrl+H opens find and replace on the bar: the matches are marked, Replace all changes the
/// text as typing would (the bar is dirty, nothing is committed), Enter in the editor commits;
/// moving the keyboard to the find row does not commit; Esc closes it.
#[test]
fn find_and_replace_in_the_bar() {
    let mut h = open(false);
    pick_total(&mut h);
    focus_end(&mut h);
    type_text(&mut h, " + SUM(Sales[Amount])");
    press(&mut h, Key::H, Modifiers::COMMAND);
    steps(&mut h, 2);
    assert!(h.state().formula.find.is_some(), "Ctrl+H opens the row");
    assert!(
        h.ctx.memory(|m| m.has_focus(crate::formula::find_id())),
        "the find field has the keyboard"
    );
    assert!(
        !sent_ids(&h).contains(&"measure.set"),
        "going to the find row does not commit"
    );
    type_text(&mut h, "amount");
    assert_eq!(h.state().formula.find.as_ref().unwrap().needle, "amount");
    steps(&mut h, 2);
    assert!(h.query_by_label("1 of 2").is_some(), "the match count");
    let t = crate::theme::Tokens::of(h.state().prefs.theme);
    let marked: Vec<String> = h
        .state()
        .formula
        .sections()
        .into_iter()
        .filter(|(_, bg, _)| *bg == t.text_selection || *bg == t.accent_soft)
        .map(|(s, _, _)| s)
        .collect();
    assert_eq!(marked, ["Amount", "Amount"], "both matches are marked");
    // Enter: the next match
    press(&mut h, Key::Enter, Modifiers::NONE);
    assert!(h.query_by_label("2 of 2").is_some());
    // letter case
    h.state_mut().formula.find.as_mut().unwrap().case = true;
    steps(&mut h, 2);
    assert!(h.query_by_label("No match").is_some());
    h.state_mut().formula.find.as_mut().unwrap().case = false;
    h.state_mut().formula.find.as_mut().unwrap().replacement = "Price".into();
    steps(&mut h, 2);
    h.get_by_label("Replace all").click();
    steps(&mut h, 3);
    assert_eq!(bar_text(&h), "Total = SUM(Sales[Price]) + SUM(Sales[Price])");
    assert!(h.state().formula.dirty());
    assert!(
        !sent_ids(&h).contains(&"measure.set"),
        "Replace all commits nothing"
    );
    // Esc: the row closes, the editor has the keyboard again; Enter commits
    h.ctx.memory_mut(|m| m.request_focus(crate::formula::find_id()));
    steps(&mut h, 2);
    press(&mut h, Key::Escape, Modifiers::NONE);
    steps(&mut h, 2);
    assert!(h.state().formula.find.is_none());
    assert!(
        h.ctx.memory(|m| m.has_focus(crate::formula::editor_id())),
        "the editor has the keyboard again"
    );
    press(&mut h, Key::Enter, Modifiers::NONE);
    steps(&mut h, 4);
    assert_eq!(
        expression(&h, "Total").as_deref(),
        Some("SUM(Sales[Price]) + SUM(Sales[Price])")
    );
}

/// The bracket next to the cursor and its partner are marked (a background and a line).
#[test]
fn the_bracket_pair_at_the_cursor_is_marked() {
    let mut h = open(false);
    pick_total(&mut h);
    focus_end(&mut h);
    let t = crate::theme::Tokens::of(h.state().prefs.theme);
    let marked = |h: &Harness<'_, App>| -> Vec<String> {
        h.state()
            .formula
            .sections()
            .into_iter()
            .filter(|(_, bg, _)| *bg == t.selected)
            .map(|(s, _, _)| s)
            .collect()
    };
    // after the closing `)`
    assert_eq!(marked(&h), ["(", ")"]);
    // inside the name: nothing
    h.state_mut().formula.cursor_to = Some(2);
    steps(&mut h, 3);
    assert!(marked(&h).is_empty());
    // before `[`: the reference's own brackets
    let at = bar_text(&h).find('[').unwrap();
    h.state_mut().formula.cursor_to = Some(at);
    steps(&mut h, 3);
    assert_eq!(marked(&h), ["[", "]"]);
}

/// The error is underlined with a wavy line in the theme's error colour, not a straight one.
#[test]
fn the_error_underline_is_wavy() {
    let mut h = open(false);
    pick_total(&mut h);
    focus_end(&mut h);
    press(&mut h, Key::Backspace, Modifiers::NONE);
    press(&mut h, Key::Backspace, Modifiers::NONE);
    type_text(&mut h, "x])");
    until(&mut h, "the diagnostic", |a| !a.formula.diagnostics.is_empty());
    steps(&mut h, 3);
    let err = crate::theme::Tokens::of(h.state().prefs.theme).err;
    assert!(
        h.state().formula.sections().iter().all(|(_, _, u)| *u != err),
        "no straight underline in the error colour"
    );
    fn waves(s: &egui::Shape, err: egui::Color32, out: &mut Vec<Vec<egui::Pos2>>) {
        match s {
            egui::Shape::Vec(v) => v.iter().for_each(|s| waves(s, err, out)),
            egui::Shape::Path(p) if p.stroke.color == egui::epaint::ColorMode::Solid(err) => {
                out.push(p.points.clone());
            }
            _ => {}
        }
    }
    let mut found = Vec::new();
    for c in &h.output().shapes {
        waves(&c.shape, err, &mut found);
    }
    let wavy = found.iter().any(|pts| {
        pts.len() > 4 && {
            let (lo, hi) = pts
                .iter()
                .fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.y), hi.max(p.y)));
            let (x0, x1) = (pts[0].x, pts[pts.len() - 1].x);
            hi > lo && hi - lo < 4.0 && x1 > x0
        }
    });
    assert!(wavy, "a wavy line in the error colour: {found:?}");
}

#[test]
fn the_data_pane_menu_and_a_double_click_open_the_editor() {
    let mut h = open(false);
    // right click Total, Edit measure: on the bar, focused
    h.get_by_label("Total").click_secondary();
    steps(&mut h, 3);
    h.get_by_label("Edit measure").click();
    steps(&mut h, 4);
    until(&mut h, "Total's DAX", |a| {
        a.formula.text == "Total = SUM(Sales[Amount])"
    });
    assert!(crate::formula::has_focus(&h.ctx));
    // a data column's menu has no Edit
    h.get_by_label("Amount").click_secondary();
    steps(&mut h, 3);
    assert!(h.query_by_label("Edit formula").is_none());
    press(&mut h, Key::Escape, Modifiers::NONE);
    h.get_by_label("Double").click_secondary();
    steps(&mut h, 3);
    h.get_by_label("Edit formula").click();
    steps(&mut h, 4);
    until(&mut h, "the column's DAX", |a| {
        a.formula.text == "Double = Sales[Amount] * 2"
    });
    // the diagram: a double click on a measure row
    h.state_mut().show_tab(crate::app::Tab::Model);
    h.state_mut().formula = Default::default();
    // selected first (the bar shows), as after the double click's first click
    h.state_mut().model_view.sel = Some(Sel::Measure {
        table: "Sales".into(),
        name: "Total".into(),
    });
    steps(&mut h, 6);
    h.ctx
        .memory_mut(|m| m.surrender_focus(crate::formula::editor_id()));
    steps(&mut h, 2);
    assert!(
        !crate::formula::has_focus(&h.ctx),
        "a selection alone does not focus"
    );
    // well after the menu's clicks (egui would count a triple click)
    steps(&mut h, 60);
    let pos = {
        let a = h.state();
        let d = a.model_view.diagram().expect("the diagram");
        let card = d.cards.iter().find(|c| c.name == "Sales").expect("Sales");
        let row = card.measures.first().expect("a measure row");
        a.model_view.screen_pos(row.rect.center())
    };
    h.hover_at(pos);
    h.step();
    for _ in 0..2 {
        for pressed in [true, false] {
            h.event(Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
            h.step();
        }
    }
    steps(&mut h, 4);
    until(&mut h, "Total from the diagram", |a| {
        a.formula.text == "Total = SUM(Sales[Amount])"
    });
    assert!(crate::formula::has_focus(&h.ctx));
}

/// The bar draws the same frames in the desktop and the browser shells: collapsed, expanded,
/// light and dark.
#[test]
fn the_bar_draws_the_same_frames_in_both_shells() {
    for dark in [false, true] {
        let mut desktop = open_with(Caps::DESKTOP, false, session);
        let mut browser = open_with(Caps::BROWSER, false, session);
        for h in [&mut desktop, &mut browser] {
            if dark {
                h.state_mut().set_theme(crate::theme::Brightness::Dark);
            }
            pick_total(h);
            until(h, "the check", |a| a.sent.iter().any(|c| c.id == "dax.validate"));
            steps(h, 3);
        }
        for expanded in [false, true] {
            for h in [&mut desktop, &mut browser] {
                h.state_mut().formula.expanded = expanded;
                steps(h, 3);
            }
            let (a, b) = (frame(&desktop), frame(&browser));
            assert!(a.0 == b.0, "dark {dark} expanded {expanded}: the meshes differ");
            assert!(
                a.1 == b.1,
                "dark {dark} expanded {expanded}: the font atlases differ"
            );
        }
    }
}

/// A 10,000-line measure: loaded whole (the snapshot caps expressions), laid out once, typing in
/// it lays it out once per keystroke.
#[test]
fn a_large_expression_stays_responsive() {
    let big: String = std::iter::once("0".to_string())
        .chain((1..10_000).map(|i| format!("+ {i} // line {i}")))
        .collect::<Vec<_>>()
        .join("\n");
    let lines = big.lines().count();
    assert_eq!(lines, 10_000);
    let expected = format!("Big sum = {big}");
    let mut h = open_with(Caps::DESKTOP, false, move || {
        session_with(Some(("Big sum", big.clone())))
    });
    h.get_by_label("Big sum").click();
    steps(&mut h, 4);
    until(&mut h, "the whole text", |a| {
        a.formula.text.len() == expected.len()
    });
    assert_eq!(bar_text(&h), expected);
    h.state_mut().formula.expanded = true;
    steps(&mut h, 3);
    // frames without a change reuse the layout
    let t0 = Instant::now();
    steps(&mut h, 10);
    let idle = t0.elapsed().as_secs_f64() * 100.0;
    focus_end(&mut h);
    let t0 = Instant::now();
    for c in ["+", " ", "1"] {
        type_text(&mut h, c);
    }
    let typing = t0.elapsed().as_secs_f64() * 1000.0 / 6.0;
    let layout = h.state().formula.layout_ms;
    println!(
        "10,000 lines: {idle:.1} ms per idle frame, {typing:.1} ms per frame while typing (layout {layout:.1} ms)"
    );
    let tail: String = bar_text(&h)
        .chars()
        .rev()
        .take(30)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    assert!(tail.ends_with("+ 9999 // line 9999+ 1"), "{tail:?}");
    // generous bounds: debug builds on a busy CI machine
    assert!(idle < 1000.0, "idle frames take {idle:.0} ms");
    assert!(typing < 5000.0, "typing takes {typing:.0} ms a frame");
}

/// Damaged snapshots and hostile picks never break the bar.
#[test]
fn damaged_snapshots_never_break_the_bar() {
    let mut h = open(false);
    for snap in [
        json!({"model": {"tables": [{"name": "Sales", "measures": [{"name": 3}, {"name": "Total"}],
                                       "columns": [{"name": "Double", "calculated": "yes"}]}]}}),
        json!({"model": {"tables": 7}}),
        json!({"model": null, "revision": "x"}),
        Value::Null,
    ] {
        crate::formula::pick(&h.ctx, Kind::Measure, Some("Sales"), "Total", true);
        h.state_mut().loaded.as_mut().unwrap().snap = std::sync::Arc::new(snap);
        steps(&mut h, 3);
        crate::formula::pick(&h.ctx, Kind::Column, None, "\u{0}", true);
        crate::formula::pick(&h.ctx, Kind::Table, Some("'"), "", false);
        steps(&mut h, 3);
    }
}
