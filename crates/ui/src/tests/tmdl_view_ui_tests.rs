//! The TMDL view and the Model explorer's Functions through the accessibility tree
//! (egui_kittest, no GPU): the status bar's way in, Script model in the view, the Model
//! explorer and the Data pane (`tmdl.script`: the empty tab is filled, else a new tab opens),
//! Apply (`tmdl.apply`) changing the model, an error with its line and column and Go to, the
//! script tabs; the Functions node with its New, Edit, Rename and Delete items
//! (`function.*`); in English and French.

use bicraft_engine::Session;
use egui::{Event, Modifiers, PointerButton};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::backstage_tests::steps;
use super::model_sec_ui_tests::{app_with, click, explorer_menu, last_sent, session};
use crate::App;
use crate::app::Tab as View;

/// [`session`] with the function `Twice` and a measure calling it.
fn udf_session() -> Session {
    let mut s = session();
    for (id, p) in [
        (
            "function.add",
            json!({"name": "Twice", "expression": "(x : NUMERIC) => x * 2", "description": "Doubles"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Double", "expression": "Twice([Total])"}),
        ),
    ] {
        s.execute(id, &p).unwrap();
    }
    s.dirty = false;
    s
}

/// Clicks the lowest widget labelled `label` (the status bar's icon, a menu item).
fn click_lowest(h: &mut Harness<'_, App>, label: &str) {
    let at = h
        .get_all_by_label(label)
        .map(|n| n.rect())
        .max_by(|a, b| a.min.y.total_cmp(&b.min.y))
        .unwrap_or_else(|| panic!("no widget `{label}`"));
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
    steps(h, 4);
}

/// The view, opened from the status bar's view icons.
fn view(lang: &str, s: fn() -> Session) -> Harness<'static, App> {
    let mut h = app_with(lang, s);
    click_lowest(&mut h, if lang == "fr-FR" { "Vue TMDL" } else { "TMDL view" });
    assert_eq!(h.state().tab, View::Tmdl, "the status bar opens the view");
    h
}

/// Steps until the engine answered (the snapshot and the tab's answer are there).
fn settle(h: &mut Harness<'_, App>) {
    for _ in 0..100 {
        h.step();
        if h.state().cmd.tmdl.active_tab().is_none_or(|t| !t.applying) {
            break;
        }
    }
    steps(h, 4);
}

fn texts(h: &Harness<'_, App>) -> Vec<(String, String)> {
    h.state()
        .cmd
        .tmdl
        .tabs
        .iter()
        .map(|t| (t.name.clone(), t.text.clone()))
        .collect()
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

fn functions(h: &Harness<'_, App>) -> Value {
    h.state()
        .loaded
        .as_ref()
        .and_then(|l| l.snap.pointer("/model/functions").cloned())
        .unwrap_or(Value::Null)
}

fn set_text(h: &mut Harness<'_, App>, text: &str) {
    let v = &mut h.state_mut().cmd.tmdl;
    let i = v.active;
    if let Some(t) = v.tabs.get_mut(i) {
        t.text = text.to_string();
    }
    steps(h, 2);
}

#[test]
fn script_model_fills_the_empty_tab_then_the_explorer_opens_a_new_tab() {
    let mut h = view("en-US", session);
    h.get_by_label("TMDL script");
    h.get_by_label("Apply");
    assert_eq!(texts(&h), [("Script 1".to_string(), String::new())]);
    // Script model: the whole model, in the empty tab
    click(&mut h, "Script model");
    settle(&mut h);
    assert_eq!(last_sent(&h, "tmdl.script"), Some(json!({})));
    let t = texts(&h);
    assert_eq!(t.len(), 1, "the empty tab is filled");
    assert!(t[0].1.starts_with("createOrReplace"), "{}", t[0].1);
    assert!(t[0].1.contains("\ttable Sales"), "{}", t[0].1);
    // the Model explorer's Script TMDL on the model: a new tab, shown
    h.state_mut().show_tab(View::Model);
    steps(&mut h, 3);
    // the model's row (the right-most "Shop": the title bar has the name too)
    h.state_mut().model_view.explorer.tab = crate::model_explorer::Tab::Model;
    steps(&mut h, 3);
    let r = h
        .get_all_by_label("Shop")
        .map(|n| n.rect())
        .max_by(|a, b| a.min.x.total_cmp(&b.min.x))
        .unwrap();
    super::backstage_tests::right_click(&mut h, r.center());
    click_lowest(&mut h, "Script TMDL");
    settle(&mut h);
    assert_eq!(h.state().tab, View::Tmdl);
    let t = texts(&h);
    assert_eq!(t.len(), 2);
    assert_eq!(t[1].0, "Script 2");
    assert_eq!(h.state().cmd.tmdl.active, 1);
}

#[test]
fn apply_replaces_a_measure_in_one_step_and_says_done() {
    let mut h = view("en-US", session);
    set_text(
        &mut h,
        "createOrReplace\n\n\tref table Sales\n\n\t\tmeasure Total = SUM(Sales[Amount]) * 2\n\n\t\tmeasure Half = [Total] / 2\n",
    );
    click(&mut h, "Apply");
    settle(&mut h);
    assert!(last_sent(&h, "tmdl.apply").is_some());
    assert_eq!(measure(&h, "Total").as_deref(), Some("SUM(Sales[Amount]) * 2"));
    assert_eq!(measure(&h, "Half").as_deref(), Some("[Total] / 2"));
    let note = h
        .state()
        .cmd
        .tmdl
        .active_tab()
        .and_then(|t| t.note.clone())
        .unwrap();
    assert!(note.ok, "{note:?}");
    assert_eq!(note.text, "Script applied: 1 added, 1 replaced.");
    h.get_by_label("Done");
    // one undo takes it all back
    h.state_mut().ribbon_tab = crate::registry::Tab::Home;
    steps(&mut h, 3);
    click(&mut h, "Undo");
    steps(&mut h, 4);
    assert_eq!(measure(&h, "Total").as_deref(), Some("SUM(Sales[Amount])"));
    assert_eq!(measure(&h, "Half"), None);
}

#[test]
fn an_error_has_its_line_and_column_goes_there_and_changes_nothing() {
    let mut h = view("en-US", session);
    set_text(
        &mut h,
        "createOrReplace\n\n\tref table Sales\n\n\t\tmeasure Bad = SUM(Sales[Nope])\n",
    );
    click(&mut h, "Apply");
    settle(&mut h);
    let note = h
        .state()
        .cmd
        .tmdl
        .active_tab()
        .and_then(|t| t.note.clone())
        .unwrap();
    assert!(!note.ok);
    assert_eq!(note.at, Some((5, 21)), "{}", note.text);
    assert!(note.text.contains("nothing was applied"), "{}", note.text);
    assert_eq!(measure(&h, "Bad"), None);
    h.get_by_label("Error");
    click(&mut h, "Go to line 5, column 21");
    let id = egui::Id::unique(("bicraft-tmdl-editor", h.state().cmd.tmdl.tabs[0].id));
    let at = h.ctx.memory(|m| m.has_focus(id));
    assert!(at, "Go to gives the editor the focus");
    let cursor = egui::TextEdit::load_state(&h.ctx, id)
        .and_then(|s| s.cursor.char_range())
        .map(|r| r.primary.index.0);
    assert_eq!(cursor, Some(crate::tmdl_view::char_at(&texts(&h)[0].1, 5, 21)));
}

#[test]
fn script_tabs_are_added_renamed_duplicated_and_closed() {
    let mut h = view("en-US", session);
    set_text(&mut h, "createOrReplace\n");
    click(&mut h, "New script");
    assert_eq!(texts(&h).len(), 2);
    let r = h.get_by_label("Script 1").rect();
    super::backstage_tests::right_click(&mut h, r.center());
    click_lowest(&mut h, "Duplicate");
    let t = texts(&h);
    assert_eq!(t.len(), 3);
    assert_eq!(t[2], ("Script 3".to_string(), "createOrReplace\n".to_string()));
    click(&mut h, "Close Script 2");
    let names: Vec<String> = texts(&h).into_iter().map(|t| t.0).collect();
    assert_eq!(names, ["Script 1", "Script 3"]);
    let id = h.state().cmd.tmdl.tabs[1].id;
    assert!(h.state_mut().cmd.tmdl.rename(id, "Measures"));
    steps(&mut h, 2);
    h.get_by_label("Measures");
}

#[test]
fn the_model_explorer_lists_functions_and_edits_renames_and_scripts_them() {
    let mut h = app_with("en-US", udf_session);
    explorer_menu(&mut h, "Functions (1)", "New function");
    assert_eq!(h.state().form.as_ref().map(|f| f.id), Some("function.add"));
    h.state_mut().form = None;
    steps(&mut h, 2);
    click(&mut h, "Functions (1)");
    // Edit function: the form has its definition
    explorer_menu(&mut h, "Twice", "Edit function");
    let f = h.state().form.as_ref().unwrap();
    assert_eq!(f.id, "function.set");
    let text = |k: &str| f.input(k).map(|i| i.text.clone());
    assert_eq!(text("name").as_deref(), Some("Twice"));
    assert_eq!(text("expression").as_deref(), Some("(x : NUMERIC) => x * 2"));
    assert_eq!(text("description").as_deref(), Some("Doubles"));
    h.state_mut().form = None;
    steps(&mut h, 2);
    // Script TMDL: the function in the TMDL view
    explorer_menu(&mut h, "Twice", "Script TMDL");
    settle(&mut h);
    assert_eq!(h.state().tab, View::Tmdl);
    assert_eq!(
        last_sent(&h, "tmdl.script"),
        Some(json!({"objects": [{"kind": "function", "name": "Twice"}]}))
    );
    let t = texts(&h);
    assert!(
        t[0].1.contains("function Twice = (x : NUMERIC) => x * 2"),
        "{}",
        t[0].1
    );
    // Delete function: refused while [Double] calls it, the reason shown
    h.state_mut().show_tab(View::Model);
    steps(&mut h, 3);
    explorer_menu(&mut h, "Twice", "Delete function");
    steps(&mut h, 4);
    assert_eq!(functions(&h)[0]["name"], "Twice", "still there");
}

#[test]
fn the_data_pane_scripts_a_measure() {
    let mut h = app_with("en-US", udf_session);
    h.state_mut().show_tab(View::Report);
    steps(&mut h, 4);
    // the Data pane's Sales, opened
    if h.query_by_label("Double").is_none() {
        click_lowest(&mut h, "Sales");
    }
    let r = h
        .get_all_by_label("Double")
        .map(|n| n.rect())
        .max_by(|a, b| a.min.x.total_cmp(&b.min.x))
        .unwrap();
    super::backstage_tests::right_click(&mut h, r.center());
    click_lowest(&mut h, "Script TMDL");
    settle(&mut h);
    assert_eq!(h.state().tab, View::Tmdl);
    assert_eq!(
        last_sent(&h, "tmdl.script"),
        Some(json!({"objects": [{"kind": "measure", "table": "Sales", "name": "Double"}]}))
    );
    assert!(texts(&h)[0].1.contains("measure Double = Twice([Total])"));
}

#[test]
fn the_view_and_the_functions_speak_french() {
    let mut h = view("fr-FR", udf_session);
    h.get_by_label("Script TMDL");
    h.get_by_label("Appliquer");
    h.get_by_label("Scripter le modèle");
    h.get_by_label("Nouveau script");
    assert_eq!(texts(&h)[0].0, "Script 1");
    set_text(
        &mut h,
        "createOrReplace\n\n\tref table Nope\n\n\t\tmeasure X = 1\n",
    );
    click(&mut h, "Appliquer");
    settle(&mut h);
    h.get_by_label("Erreur");
    h.get_by_label("Aller à la ligne 3, colonne 2");
    h.state_mut().show_tab(View::Model);
    steps(&mut h, 3);
    explorer_menu(&mut h, "Fonctions (1)", "Nouvelle fonction");
    assert_eq!(h.state().form.as_ref().map(|f| f.id), Some("function.add"));
}
