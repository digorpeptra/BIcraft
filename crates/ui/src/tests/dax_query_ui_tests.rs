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

fn text(h: &Harness<'_, App>) -> String {
    h.state().cmd.dax_query.active_tab().unwrap().text.clone()
}

/// The editor's selection as the view keeps it, in characters (start, end).
fn editor_selection(h: &Harness<'_, App>) -> Option<(usize, usize)> {
    let v = &h.state().cmd.dax_query;
    let (id, r) = v.sel.clone()?;
    (Some(id) == v.active_tab().map(|q| q.id)).then_some((r.start, r.end))
}

#[test]
fn ctrl_slash_and_comment_toggle_the_line_comment_of_the_selected_lines() {
    let mut h = view("en-US");
    type_query(&mut h, "EVALUATE\n    ROW(\"a\", 1)");
    // the cursor's line
    press(&mut h, Key::Slash, Modifiers::COMMAND);
    assert_eq!(text(&h), "EVALUATE\n    // ROW(\"a\", 1)");
    press(&mut h, Key::Slash, Modifiers::COMMAND);
    assert_eq!(
        text(&h),
        "EVALUATE\n    ROW(\"a\", 1)",
        "a second press uncomments"
    );
    // every selected line, at their common indent; the tool bar's button does the same
    press(&mut h, Key::A, Modifiers::COMMAND);
    click(&mut h, "Comment");
    assert_eq!(text(&h), "// EVALUATE\n//     ROW(\"a\", 1)");
    assert_eq!(
        editor_selection(&h),
        Some((0, text(&h).chars().count())),
        "the lines stay selected"
    );
    press(&mut h, Key::Slash, Modifiers::COMMAND);
    assert_eq!(text(&h), "EVALUATE\n    ROW(\"a\", 1)");
    // the editor kept the keyboard and the selection: typing replaces the lines
    h.event(Event::Text("x".into()));
    steps(&mut h, 2);
    assert_eq!(text(&h), "x");
}

#[test]
fn find_and_replace_counts_steps_and_replaces_with_match_case_and_whole_word() {
    let mut h = view("en-US");
    type_query(&mut h, "amount Amount AmountX amount");
    press(&mut h, Key::F, Modifiers::COMMAND);
    assert!(
        h.state().cmd.dax_query.find.is_some(),
        "Ctrl+F opens the find row"
    );
    assert!(h.ctx.memory(|m| m.has_focus(crate::dax_query_tools::find_id())));
    h.event(Event::Text("amount".into()));
    steps(&mut h, 3);
    h.get_by_label("1 of 4");
    // Enter: the next match, selected in the editor
    press(&mut h, Key::Enter, Modifiers::NONE);
    h.get_by_label("2 of 4");
    assert_eq!(editor_selection(&h), Some((7, 13)));
    press(&mut h, Key::Enter, Modifiers::SHIFT);
    h.get_by_label("1 of 4");
    click(&mut h, "Whole word");
    h.get_by_label("1 of 3");
    click(&mut h, "Match case");
    h.get_by_label("1 of 2");
    // Ctrl+H: the replace field
    press(&mut h, Key::H, Modifiers::COMMAND);
    assert!(
        h.ctx
            .memory(|m| m.has_focus(crate::dax_query_tools::replace_id()))
    );
    h.event(Event::Text("total".into()));
    steps(&mut h, 2);
    click(&mut h, "Replace");
    assert_eq!(text(&h), "total Amount AmountX amount");
    click(&mut h, "Whole word");
    click(&mut h, "Match case");
    click(&mut h, "Replace all");
    assert_eq!(text(&h), "total total totalX total");
    h.get_by_label("No match");
    // Esc in the find field closes the row; the editor has the keyboard again
    press(&mut h, Key::F, Modifiers::COMMAND);
    press(&mut h, Key::Escape, Modifiers::NONE);
    let id = crate::dax_query::editor_id(h.state().cmd.dax_query.active_tab().unwrap().id);
    assert!(h.ctx.memory(|m| m.has_focus(id)));
    assert!(h.state().cmd.dax_query.find.is_none(), "Esc closes the find row");
    assert_eq!(
        h.state().sent.iter().filter(|c| c.id == "dax.evaluate").count(),
        0
    );
}

#[test]
fn the_data_pane_inserts_references_at_the_cursor() {
    let mut h = view("en-US");
    h.get_by_label("Data pane");
    type_query(&mut h, "EVALUATE ROW(\"x\", SUM())");
    // the cursor inside SUM( )
    press(&mut h, Key::ArrowLeft, Modifiers::NONE);
    press(&mut h, Key::ArrowLeft, Modifiers::NONE);
    click(&mut h, "Sales");
    click(&mut h, "Amount");
    assert_eq!(text(&h), "EVALUATE ROW(\"x\", SUM('Sales'[Amount]))");
    // the editor has the keyboard back, its cursor after the reference
    assert_eq!(editor_selection(&h), Some((37, 37)));
    h.event(Event::Text(" + ".into()));
    steps(&mut h, 2);
    click(&mut h, "Total");
    assert_eq!(text(&h), "EVALUATE ROW(\"x\", SUM('Sales'[Amount] + [Total]))");
    // the search keeps the matching fields, every table unrolled
    h.get_by_label("Search the fields").click();
    steps(&mut h, 2);
    h.event(Event::Text("name".into()));
    steps(&mut h, 3);
    h.get_by_label("Name");
    assert!(h.query_by_label("Amount").is_none());
    // the tool bar hides the pane
    click(&mut h, "Data pane");
    assert!(h.query_by_label("Name").is_none());
}

/// Right-clicks the Data pane row `label` and picks `item` in its menu.
fn quick_query(h: &mut Harness<'_, App>, label: &str, item: &str) {
    let r = h.get_by_label(label).rect();
    right_click(h, r.center());
    click(h, item);
    wait_run(h);
}

#[test]
fn quick_queries_open_a_tab_each_and_run() {
    let mut h = view("en-US");
    quick_query(&mut h, "Sales", "Evaluate top 100 rows");
    assert_eq!(names(&h), ["Query 1", "Query 2"]);
    assert_eq!(text(&h), crate::dax_query_tools::top_rows("Sales"));
    assert_eq!(last_sent(&h, "dax.evaluate").unwrap()["query"], text(&h));
    h.get_by_label("3 rows");
    click(&mut h, "Product");
    quick_query(&mut h, "Name", "Show distinct values");
    assert_eq!(names(&h).len(), 3);
    assert_eq!(
        text(&h),
        crate::dax_query_tools::distinct_values("Product", "Name")
    );
    h.get_by_label("2 rows");
    h.get_by_label("Bike");
    click(&mut h, "Sales");
    quick_query(&mut h, "Total", "Evaluate measure");
    assert_eq!(text(&h), crate::dax_query_tools::evaluate_measure("Total"));
    h.get_by_label("1 row");
    h.get_by_label("22.0");
    // Shift+F10 opens a row's menu from the keyboard
    h.get_by_label("Product").focus();
    steps(&mut h, 2);
    press(&mut h, Key::F10, Modifiers::SHIFT);
    h.get_by_label("Evaluate top 100 rows");
}

#[test]
fn the_tools_in_french() {
    let mut h = view("fr-FR");
    h.get_by_label("Commenter");
    h.get_by_label("Volet Données");
    click(&mut h, "Rechercher et remplacer");
    h.get_by_label("Mot entier");
    h.get_by_label("Respecter la casse");
    h.get_by_label("Tout remplacer");
    let r = h.get_by_label("Sales").rect();
    right_click(&mut h, r.center());
    h.get_by_label("Évaluer les 100 premières lignes");
    h.get_by_label("Insérer dans la requête");
}

/// Shop with [Count] and [Average] = DIVIDE([Total], [Count]).
fn shop_with_measures() -> bicraft_engine::Session {
    let mut s = super::model_sec_ui_tests::session();
    for (name, expression) in [
        ("Count", "COUNTROWS(Sales)"),
        ("Average", "DIVIDE([Total], [Count])"),
    ] {
        s.execute(
            "measure.add",
            &serde_json::json!({"table": "Sales", "name": name, "expression": expression}),
        )
        .unwrap();
    }
    s.dirty = false;
    s
}

#[test]
fn define_quick_queries_write_the_measures_with_their_references_and_run() {
    let mut h = super::model_sec_ui_tests::app_with("en-US", shop_with_measures);
    h.state_mut().show_tab(View::Query);
    steps(&mut h, 4);
    quick_query(&mut h, "Sales", "Define all measures");
    let p = last_sent(&h, "dax.define_query").expect("the menu runs dax.define_query");
    assert_eq!(p, serde_json::json!({"table": "Sales"}));
    assert_eq!(names(&h), ["Query 1", "Query 2"]);
    let q = text(&h);
    for m in ["Total", "Count", "Average"] {
        assert!(q.contains(&format!("MEASURE 'Sales'[{m}] =")), "{q}");
    }
    assert_eq!(last_sent(&h, "dax.evaluate").unwrap()["query"], q, "run");
    h.get_by_label("1 row");
    // its lenses: every measure is already in the model
    h.get_by_label("Update model: Overwrite measure Average");
    // a measure with the measures it references
    click(&mut h, "Sales");
    quick_query(&mut h, "Average", "Define with references");
    assert_eq!(
        last_sent(&h, "dax.define_query").unwrap(),
        serde_json::json!({"measure": "Average", "references": true})
    );
    assert_eq!(names(&h).len(), 3);
    let q = text(&h);
    let at = |m: &str| q.find(&format!("MEASURE 'Sales'[{m}]")).expect(m);
    assert!(at("Total") < at("Average") && at("Count") < at("Average"), "{q}");
    assert!(q.contains("ROW(\"Average\", [Average])"), "{q}");
    h.get_by_label("1 row");
    assert!(
        matches!(
            h.state().cmd.dax_query.active_tab().unwrap().outcome,
            Some(crate::dax_query::Outcome::Rows { .. })
        ),
        "the query evaluates"
    );
}

/// Shop with a hierarchy on Product and a calculation group.
fn shop_with_groups() -> bicraft_engine::Session {
    let mut s = super::model_sec_ui_tests::session();
    for (id, p) in [
        (
            "hierarchy.add",
            serde_json::json!({"table": "Product", "name": "Catalog", "levels": ["Name", "ProductKey"]}),
        ),
        ("calculation_group.add", serde_json::json!({"name": "Scale"})),
        (
            "calculation_item.add",
            serde_json::json!({"group": "Scale", "name": "Doubled", "expression": "SELECTEDMEASURE() * 2"}),
        ),
    ] {
        s.execute(id, &p).unwrap();
    }
    s.dirty = false;
    s
}

#[test]
fn the_data_pane_lists_hierarchies_and_calculation_items() {
    let mut h = super::model_sec_ui_tests::app_with("en-US", shop_with_groups);
    h.state_mut().show_tab(View::Query);
    steps(&mut h, 4);
    type_query(&mut h, "EVALUATE ROW(\"x\", 1)");
    press(&mut h, Key::End, Modifiers::COMMAND);
    click(&mut h, "Product");
    click(&mut h, "Catalog");
    // a level inserts its column
    let levels: Vec<_> = h.get_all_by_label("Name").collect();
    assert_eq!(levels.len(), 2, "the column and the hierarchy's level");
    let level = levels
        .iter()
        .map(|n| n.rect())
        .max_by(|a, b| a.min.y.total_cmp(&b.min.y))
        .unwrap();
    h.hover_at(level.center());
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos: level.center(),
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
    steps(&mut h, 3);
    assert_eq!(text(&h), "EVALUATE ROW(\"x\", 1)'Product'[Name]");
    // a calculation item inserts the filter that picks it
    click(&mut h, "Scale");
    click(&mut h, "Doubled");
    assert_eq!(
        text(&h),
        "EVALUATE ROW(\"x\", 1)'Product'[Name]'Scale'[Name] = \"Doubled\""
    );
}

#[test]
fn a_field_dragged_into_the_editor_lands_where_it_is_dropped() {
    let mut h = view("en-US");
    type_query(&mut h, "EVALUATE ROW(\"x\", SUM())");
    click(&mut h, "Sales");
    let from = h.get_by_label("Amount").rect().center();
    // the drop point: just after `SUM(` (a little before the boundary, which the drop rounds to),
    // found from the editor's left edge and its glyph width
    let editor = h.get_by_label("DAX query").rect();
    let char_w = h.ctx.fonts_mut(|f| f.glyph_width(&crate::formula::mono(), 'M'));
    let col = "EVALUATE ROW(\"x\", SUM(".chars().count() as f32;
    let to = egui::pos2(editor.min.x + 4.0 + char_w * (col - 0.3), editor.min.y + 14.0);
    h.hover_at(from);
    h.step();
    h.event(Event::PointerButton {
        pos: from,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    for k in 1..=12 {
        h.event(Event::PointerMoved(from + (to - from) * (k as f32 / 12.0)));
        h.step();
    }
    h.event(Event::PointerButton {
        pos: to,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    steps(&mut h, 4);
    assert_eq!(text(&h), "EVALUATE ROW(\"x\", SUM('Sales'[Amount]))");
    assert_eq!(editor_selection(&h), Some((37, 37)), "the cursor after it");
}

#[test]
fn a_lens_sits_on_its_own_line_above_its_measure() {
    let mut h = view("en-US");
    type_query(
        &mut h,
        "DEFINE\n    MEASURE Sales[Count] = COUNTROWS(Sales)\n    MEASURE Sales[Total] = 1\nEVALUATE ROW(\"C\", [Count])",
    );
    let editor = h.get_by_label("DAX query").rect();
    let add = h.get_by_label("Update model: Add new measure Count").rect();
    let over = h.get_by_label("Update model: Overwrite measure Total").rect();
    // from the measure's column, not at the right end of its line
    assert!(add.max.x < editor.center().x, "{add:?} in {editor:?}");
    assert!((add.min.x - over.min.x).abs() < 1.0, "both at MEASURE's column");
    // each in a gap of its own: a line of text and a lens between the two
    let row = h.ctx.fonts_mut(|f| f.row_height(&crate::formula::mono()));
    assert!(over.min.y - add.max.y > row, "{add:?} {over:?}");
}
