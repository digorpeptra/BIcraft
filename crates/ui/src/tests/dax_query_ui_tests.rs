//! The DAX query view through the accessibility tree (egui_kittest, no GPU): the status bar's and
//! the ribbon's way in, the query typed in the editor and run with Ctrl+Enter, F5 or Run
//! (`dax.evaluate`), the rows and their count, an error inline (icon and word), the Update model
//! lenses of DEFINE MEASURE and Update model with changes (`dax.apply_define`), and the query
//! tabs (new, rename, duplicate, close, kept per tab); in English and French.

use egui::{Event, Key, Modifiers, PointerButton};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::Value;

use super::backstage_tests::{right_click, steps};
use super::model_sec_ui_tests::{app_with, click, last_sent, session};
use crate::App;
use crate::app::Tab as View;

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

/// The view, opened from the status bar's view icons.
fn view(lang: &str) -> Harness<'static, App> {
    let mut h = app_with(lang, session);
    let label = if lang == "fr-FR" {
        "Vue de requête DAX"
    } else {
        "DAX query view"
    };
    // the status bar's icon (the lowest widget of that label)
    let at = h
        .get_all_by_label(label)
        .map(|n| n.rect())
        .max_by(|a, b| a.min.y.total_cmp(&b.min.y))
        .unwrap();
    h.hover_at(at.center());
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos: at.center(),
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.step();
    }
    steps(&mut h, 4);
    assert_eq!(h.state().tab, View::Query, "the status bar opens the view");
    h
}

/// Types `text` into the editor (a click in it first).
fn type_query(h: &mut Harness<'_, App>, text: &str) {
    let label = if h.state().lang == crate::i18n::Lang::Fr {
        "Requête DAX"
    } else {
        "DAX query"
    };
    let r = h.get_by_label(label).rect();
    let pos = r.min + egui::vec2(10.0, 8.0);
    h.hover_at(pos);
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
    steps(h, 2);
    h.event(Event::Text(text.into()));
    steps(h, 3);
}

/// Steps until the tab shown has an answer.
fn wait_run(h: &mut Harness<'_, App>) {
    for _ in 0..200 {
        h.step();
        let q = h.state().cmd.dax_query.active_tab();
        if q.is_some_and(|q| !q.running && q.outcome.is_some()) {
            break;
        }
    }
    steps(h, 3);
}

fn measure(h: &Harness<'_, App>, name: &str) -> Option<String> {
    let snap = h.state().loaded.as_ref()?.snap.clone();
    snap["model"]["tables"]
        .as_array()?
        .iter()
        .flat_map(|t| t["measures"].as_array().cloned().unwrap_or_default())
        .find(|m| m["name"] == name)
        .and_then(|m| m["expression"].as_str().map(str::to_string))
}

fn names(h: &Harness<'_, App>) -> Vec<String> {
    h.state()
        .cmd
        .dax_query
        .tabs
        .iter()
        .map(|t| t.name.clone())
        .collect()
}

#[test]
fn the_status_bar_and_the_ribbon_open_the_view_with_a_first_tab() {
    let mut h = view("en-US");
    assert_eq!(names(&h), ["Query 1"]);
    h.get_by_label("Query 1");
    h.get_by_label("DAX query");
    h.get_by_label("Run");
    // back to the Model view, then Analyze › Run DAX query opens it again
    h.state_mut().show_tab(View::Model);
    steps(&mut h, 2);
    h.state_mut().ribbon_tab = crate::registry::Tab::Analyze;
    steps(&mut h, 3);
    click(&mut h, "Run DAX query");
    assert_eq!(h.state().tab, View::Query);
    assert_eq!(names(&h), ["Query 1"], "the same tab, kept");
}

#[test]
fn ctrl_enter_runs_the_query_and_the_grid_shows_its_rows_and_count() {
    let mut h = view("en-US");
    type_query(
        &mut h,
        "EVALUATE SUMMARIZECOLUMNS(Sales[Region], \"Sum\", [Total])",
    );
    press(&mut h, Key::Enter, Modifiers::COMMAND);
    wait_run(&mut h);
    let sent = last_sent(&h, "dax.evaluate").expect("Ctrl+Enter runs dax.evaluate");
    assert_eq!(
        sent["query"],
        "EVALUATE SUMMARIZECOLUMNS(Sales[Region], \"Sum\", [Total])"
    );
    // the text has no new line: Ctrl+Enter was the view's
    let q = h.state().cmd.dax_query.active_tab().unwrap();
    assert!(!q.text.contains('\n'), "{:?}", q.text);
    h.get_by_label("2 rows");
    h.get_by_label("Sales[Region]");
    h.get_by_label("[Sum]");
    h.get_by_label("E");
    h.get_by_label("17.0");
    // F5 runs it again (not a refresh)
    let before = h.state().sent.len();
    press(&mut h, Key::F5, Modifiers::NONE);
    wait_run(&mut h);
    let ids: Vec<&str> = h.state().sent[before..].iter().map(|c| c.id).collect();
    assert_eq!(ids, ["dax.evaluate"], "F5 runs the query");
}

#[test]
fn an_error_is_shown_inline_with_its_word() {
    let mut h = view("en-US");
    type_query(&mut h, "EVALUATE ROW(\"x\", NOSUCHFUNCTION(1))");
    click(&mut h, "Run");
    wait_run(&mut h);
    assert!(matches!(
        h.state().cmd.dax_query.active_tab().unwrap().outcome,
        Some(crate::dax_query::Outcome::Error(_))
    ));
    h.get_by_label("Error");
}

#[test]
fn lenses_add_or_overwrite_one_measure_and_the_bar_updates_them_all() {
    let mut h = view("en-US");
    type_query(
        &mut h,
        "DEFINE\n    MEASURE Sales[Total] = SUM(Sales[Amount]) * 2\n    MEASURE Sales[Count] = COUNTROWS(Sales)\nEVALUATE ROW(\"C\", [Count])",
    );
    h.get_by_label("Update model: Overwrite measure Total");
    h.get_by_label("Update model with changes (2)");
    click(&mut h, "Update model: Add new measure Count");
    steps(&mut h, 6);
    let p = last_sent(&h, "dax.apply_define").expect("the lens runs dax.apply_define");
    assert_eq!(p["measures"], serde_json::json!(["Count"]));
    assert_eq!(measure(&h, "Count").as_deref(), Some("COUNTROWS(Sales)"));
    assert_eq!(measure(&h, "Total").as_deref(), Some("SUM(Sales[Amount])"));
    h.get_by_label("Done");
    // Count is in the model now: its lens overwrites
    h.get_by_label("Update model: Overwrite measure Count");
    click(&mut h, "Update model with changes (2)");
    steps(&mut h, 6);
    let p = last_sent(&h, "dax.apply_define").unwrap();
    assert_eq!(p.get("measures"), None::<&Value>, "every measure");
    assert_eq!(measure(&h, "Total").as_deref(), Some("SUM(Sales[Amount]) * 2"));
    // one undo step for the bar's update
    crate::ribbon::invoke(h.state_mut(), "edit.undo", serde_json::json!({}));
    steps(&mut h, 6);
    assert_eq!(measure(&h, "Total").as_deref(), Some("SUM(Sales[Amount])"));
    assert_eq!(measure(&h, "Count").as_deref(), Some("COUNTROWS(Sales)"));
}

#[test]
fn an_unknown_table_is_an_inline_error_and_changes_nothing() {
    let mut h = view("en-US");
    type_query(&mut h, "DEFINE MEASURE Nowhere[X] = 1\nEVALUATE ROW(\"X\", [X])");
    click(&mut h, "Update model: Add new measure X");
    steps(&mut h, 6);
    let note = h
        .state()
        .cmd
        .dax_query
        .active_tab()
        .unwrap()
        .note
        .clone()
        .unwrap();
    assert!(!note.ok && note.text.contains("Nowhere"), "{note:?}");
    h.get_by_label("Error");
    assert_eq!(measure(&h, "X"), None);
}

#[test]
fn tabs_are_added_renamed_duplicated_and_closed_each_with_its_own_text() {
    let mut h = view("en-US");
    type_query(&mut h, "EVALUATE Sales");
    click(&mut h, "New query");
    assert_eq!(names(&h), ["Query 1", "Query 2"]);
    assert_eq!(h.state().cmd.dax_query.active, 1);
    assert_eq!(h.state().cmd.dax_query.active_tab().unwrap().text, "");
    // rename Query 2 from its menu
    let r = h.get_by_label("Query 2").rect();
    right_click(&mut h, r.center());
    click(&mut h, "Rename");
    h.get_by_label("Query name");
    press(&mut h, Key::A, Modifiers::COMMAND);
    h.event(Event::Text("Regions".into()));
    steps(&mut h, 2);
    press(&mut h, Key::Enter, Modifiers::NONE);
    assert_eq!(names(&h), ["Query 1", "Regions"]);
    // back to Query 1: its text is kept
    click(&mut h, "Query 1");
    assert_eq!(
        h.state().cmd.dax_query.active_tab().unwrap().text,
        "EVALUATE Sales"
    );
    // duplicate it from its menu
    let r = h.get_by_label("Query 1").rect();
    right_click(&mut h, r.center());
    click(&mut h, "Duplicate");
    assert_eq!(names(&h), ["Query 1", "Regions", "Query 2"]);
    assert_eq!(
        h.state().cmd.dax_query.active_tab().unwrap().text,
        "EVALUATE Sales"
    );
    click(&mut h, "Close Regions");
    assert_eq!(names(&h), ["Query 1", "Query 2"]);
    click(&mut h, "Close Query 1");
    click(&mut h, "Close Query 2");
    assert_eq!(names(&h), ["Query 1"], "the last one closed leaves an empty tab");
    assert_eq!(h.state().cmd.dax_query.active_tab().unwrap().text, "");
}

#[test]
fn the_view_in_french() {
    let mut h = view("fr-FR");
    assert_eq!(names(&h), ["Requête 1"]);
    h.get_by_label("Exécuter");
    h.get_by_label("Requête DAX");
    type_query(
        &mut h,
        "DEFINE MEASURE Sales[Nombre] = COUNTROWS(Sales)\nEVALUATE ROW(\"N\", [Nombre])",
    );
    h.get_by_label("Mettre à jour le modèle avec les modifications (1)");
    press(&mut h, Key::Enter, Modifiers::COMMAND);
    wait_run(&mut h);
    h.get_by_label("1 ligne");
    click(&mut h, "Mettre à jour le modèle : ajouter la mesure Nombre");
    steps(&mut h, 6);
    h.get_by_label("Terminé");
    assert_eq!(measure(&h, "Nombre").as_deref(), Some("COUNTROWS(Sales)"));
}

/// Steps until a command was sent, at most `n` frames.
fn wait_sent(h: &mut Harness<'_, App>, id: &str, n: usize) -> Option<Value> {
    for _ in 0..n {
        if let Some(p) = last_sent(h, id) {
            steps(h, 4);
            return Some(p);
        }
        h.step();
    }
    last_sent(h, id)
}

#[test]
fn format_query_lays_the_tab_out_through_dax_format() {
    let mut h = view("en-US");
    type_query(&mut h, "evaluate filter(Sales,Sales[Amount]>5)");
    click(&mut h, "Format");
    steps(&mut h, 6);
    let p = last_sent(&h, "dax.format").expect("Format query runs dax.format");
    assert_eq!(p["query"], "evaluate filter(Sales,Sales[Amount]>5)");
    assert_eq!(
        h.state().cmd.dax_query.active_tab().unwrap().text,
        "evaluate\n    filter(Sales, Sales[Amount] > 5)"
    );
    // a text that does not parse: an inline error, the text kept
    click(&mut h, "New query");
    type_query(&mut h, "EVALUATE (");
    click(&mut h, "Format");
    steps(&mut h, 6);
    let q = h.state().cmd.dax_query.active_tab().unwrap();
    assert_eq!(q.text, "EVALUATE (");
    assert!(q.note.as_ref().is_some_and(|n| !n.ok), "{:?}", q.note);
}

#[test]
fn the_text_is_checked_while_typing_and_its_first_error_underlined() {
    let mut h = view("en-US");
    type_query(&mut h, "EVALUATE ROW(\"x\", NOSUCHFUNCTION(1))");
    let p = wait_sent(&mut h, "dax.validate_query", 120).expect("the text is checked after a pause");
    assert_eq!(p["query"], "EVALUATE ROW(\"x\", NOSUCHFUNCTION(1))");
    steps(&mut h, 4);
    let q = h.state().cmd.dax_query.active_tab().unwrap();
    let under = crate::dax_query::underline(&q.diagnostics, &q.text).expect("an error to underline");
    assert_eq!(q.text.get(under), Some("NOSUCHFUNCTION"));
    let d = q.diagnostics.iter().find(|d| d.error).unwrap();
    h.get_by_label(&format!("Line 1, column 19: {}", d.message));
    // fixed: no error left
    let before = h.state().sent.len();
    h.state_mut().cmd.dax_query.tabs[0].text = "EVALUATE ROW(\"x\", 1)".into();
    for _ in 0..120 {
        h.step();
        if h.state().sent[before..]
            .iter()
            .any(|c| c.id == "dax.validate_query")
        {
            break;
        }
    }
    steps(&mut h, 4);
    let q = h.state().cmd.dax_query.active_tab().unwrap();
    assert!(q.diagnostics.iter().all(|d| !d.error), "{:?}", q.diagnostics);
}

#[test]
fn cancel_shows_while_a_query_runs_and_sends_dax_cancel() {
    let mut h = view("en-US");
    assert!(
        h.query_by_label("Cancel").is_none(),
        "no Cancel while nothing runs"
    );
    h.state_mut().cmd.dax_query.tabs[0].running = true;
    steps(&mut h, 2);
    click(&mut h, "Cancel");
    steps(&mut h, 4);
    assert!(last_sent(&h, "dax.cancel").is_some(), "Cancel runs dax.cancel");
}

#[test]
fn the_tabs_go_to_the_session_a_moment_after_they_change() {
    let mut h = view("en-US");
    type_query(&mut h, "EVALUATE Sales");
    let p = wait_sent(&mut h, "dax.set_queries", 200).expect("the tabs are handed to the session");
    assert_eq!(
        p,
        serde_json::json!({"tabs": [{"name": "Query 1", "text": "EVALUATE Sales"}], "default_tab": "Query 1"})
    );
    let snap = h.state().loaded.as_ref().unwrap().snap.clone();
    assert_eq!(snap["dax_queries"]["tabs"][0]["text"], "EVALUATE Sales");
    // unchanged: not sent again
    let before = h.state().sent.len();
    steps(&mut h, 120);
    assert!(!h.state().sent[before..].iter().any(|c| c.id == "dax.set_queries"));
}

#[test]
fn a_files_tabs_are_loaded_with_its_default_tab_shown() {
    let mut v = crate::dax_query::QueryView::default();
    v.ensure(crate::i18n::Lang::En);
    v.tabs[0].text = "typed".into();
    v.load(
        &serde_json::json!({
            "generation": 7,
            "tabs": [{"name": "A", "text": "EVALUATE {1}"}, {"name": "B", "text": ""}],
            "default_tab": "b",
        }),
        crate::i18n::Lang::En,
    );
    let names: Vec<&str> = v.tabs.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["A", "B"]);
    assert_eq!(v.active, 1);
    assert_eq!(v.tabs[0].text, "EVALUATE {1}");
    // a file without tabs: one empty tab
    v.load(
        &serde_json::json!({"generation": 8, "tabs": []}),
        crate::i18n::Lang::En,
    );
    assert_eq!(v.tabs.len(), 1);
    assert_eq!(v.tabs[0].text, "");
}
