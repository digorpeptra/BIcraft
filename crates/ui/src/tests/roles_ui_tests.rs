//! Manage security roles through the accessibility tree (egui_kittest, no GPU): the default
//! editor's conditions saved as DAX, the DAX editor (colours, completion, Check without saving,
//! the errors with their words), the switch back that warns when the filter would be lost,
//! Revert, rename, New and Delete, in English and French.

use egui::{Event, Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::backstage_tests::steps;
use super::model_sec_ui_tests::{app_with, click, explorer_menu, last_sent, session};
use crate::App;
use crate::roles_dialog::Mode;
use bicraft_engine::Session;

/// [`session`] with the role Readers (Sales filtered to East).
fn roles_session() -> Session {
    let mut s = session();
    for (id, p) in [
        ("role.add", json!({"name": "Readers"})),
        (
            "role.set_filter",
            json!({"role": "Readers", "table": "Sales", "filter_expression": "[Region] = \"E\""}),
        ),
    ] {
        s.execute(id, &p).unwrap();
    }
    s.dirty = false;
    s
}

fn open(lang: &str) -> Harness<'static, App> {
    let mut h = app_with(lang, roles_session);
    let (row, item) = if lang == "fr-FR" {
        ("Rôles (1)", "Gérer les rôles")
    } else {
        ("Roles (1)", "Manage roles")
    };
    explorer_menu(&mut h, row, item);
    assert!(h.state().cmd.roles.is_some(), "the dialog opens");
    h
}

fn until(h: &mut Harness<'_, App>, what: &str, ok: impl Fn(&App) -> bool) {
    for _ in 0..300 {
        if ok(h.state()) {
            return;
        }
        h.step();
    }
    panic!("timed out waiting for {what}");
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

/// The lowest widget labelled `label` (a combo box's list is drawn over the dialog).
fn click_last(h: &mut Harness<'_, App>, label: &str) {
    let at = h
        .get_all_by_label(label)
        .last()
        .map(|n| n.rect())
        .unwrap_or_else(|| panic!("no `{label}`"));
    let pos = at.center();
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
    steps(h, 3);
}

/// `table`'s filter of `role` in the latest snapshot ("" when none).
fn filter(h: &Harness<'_, App>, role: &str, table: &str) -> String {
    h.state()
        .loaded
        .as_ref()
        .and_then(|l| l.snap.pointer("/model/security").cloned())
        .and_then(|v| {
            let r = v.as_array()?.iter().find(|r| r["name"] == role)?.clone();
            let t = r["tables"]
                .as_array()?
                .iter()
                .find(|t| t["table"] == table)?
                .clone();
            t["filter_expression"].as_str().map(str::to_string)
        })
        .unwrap_or_default()
}

fn editor(h: &Harness<'_, App>) -> crate::roles_dialog::Editor {
    h.state()
        .cmd
        .roles
        .as_ref()
        .and_then(|d| d.editor.clone())
        .expect("an editor")
}

#[test]
fn the_default_editor_writes_the_conditions_as_dax_and_saves_them() {
    let mut h = open("en-US");
    // Sales' saved filter reads back as one condition
    let e = editor(&h);
    assert_eq!((e.table.as_str(), e.mode), ("Sales", Mode::Default));
    assert_eq!(e.rows.len(), 1);
    assert_eq!(
        (e.rows[0].column.as_str(), e.rows[0].value.as_str()),
        ("Region", "E")
    );
    h.get_by_label("Table Sales");
    // Product: no filter yet; New adds a condition on its first column
    click(&mut h, "Table Product");
    assert_eq!(editor(&h).table, "Product");
    click(&mut h, "New condition");
    assert_eq!(editor(&h).rows.len(), 1);
    click(&mut h, "Column of condition 1");
    click_last(&mut h, "Name");
    h.get_by_label("Value of condition 1").click();
    steps(&mut h, 2);
    type_text(&mut h, "Bike");
    // a second condition joined by Or
    click(&mut h, "New condition");
    click(&mut h, "And or Or of condition 2");
    click_last(&mut h, "Or");
    click(&mut h, "Condition 2");
    click_last(&mut h, "is greater than");
    h.get_by_label("Value of condition 2").click();
    steps(&mut h, 2);
    type_text(&mut h, "1");
    let dax = "[Name] = \"Bike\" || [ProductKey] > 1";
    h.get_by_label(&format!("DAX of the conditions: {dax}"));
    click(&mut h, "Save filter Product");
    until(&mut h, "the save", |a| {
        a.cmd
            .roles
            .as_ref()
            .and_then(|d| d.editor.as_ref())
            .is_some_and(|e| !e.saving)
    });
    steps(&mut h, 4);
    assert_eq!(
        last_sent(&h, "role.set_filter"),
        Some(json!({"role": "Readers", "table": "Product", "filter_expression": dax}))
    );
    assert_eq!(filter(&h, "Readers", "Product"), dax);
    assert_eq!(editor(&h).saved, dax);
    // a condition deleted, then Revert brings the saved filter back
    click(&mut h, "Delete condition 2");
    assert_eq!(editor(&h).rows.len(), 1);
    click(&mut h, "Revert");
    assert_eq!(editor(&h).rows.len(), 2);
    assert!(h.state().error.is_none(), "{:?}", h.state().error);
}

#[test]
fn the_dax_editor_completes_checks_without_saving_and_shows_the_errors() {
    let mut h = open("en-US");
    click(&mut h, "Switch to DAX editor");
    assert_eq!(editor(&h).mode, Mode::Dax);
    assert_eq!(editor(&h).dax.text, "[Region] = \"E\"");
    until(&mut h, "the function list", |a| {
        a.sent.iter().any(|c| c.id == "dax.functions")
    });
    // the editor has the keyboard (Switch focuses it); the cursor at the end
    steps(&mut h, 3);
    let end = editor(&h).dax.text.chars().count();
    if let Some(e) = h.state_mut().cmd.roles.as_mut().and_then(|d| d.editor.as_mut()) {
        e.dax.cursor_to = Some(end);
    }
    steps(&mut h, 2);
    // a bare column of the table: the row context's completion
    type_text(&mut h, " && [Amo");
    let c = editor(&h).dax.completion.clone().expect("the list is open");
    assert_eq!(c.items[0].insert, "[Amount]");
    press(&mut h, Key::Enter, Modifiers::NONE);
    assert_eq!(editor(&h).dax.text, "[Region] = \"E\" && [Amount]");
    assert!(editor(&h).dax.completion.is_none());
    type_text(&mut h, " > [Nope]");
    let typed = "[Region] = \"E\" && [Amount] > [Nope]";
    assert_eq!(editor(&h).dax.text, typed);
    click(&mut h, "Check");
    until(&mut h, "the check", |a| {
        a.cmd
            .roles
            .as_ref()
            .and_then(|d| d.editor.as_ref())
            .is_some_and(|e| !e.checking)
    });
    steps(&mut h, 3);
    assert_eq!(
        last_sent(&h, "role.check_filter"),
        Some(json!({"role": "Readers", "table": "Sales", "filter_expression": typed}))
    );
    let v = editor(&h).verdict.expect("a verdict");
    assert!(!v.valid, "{v:?}");
    assert!(
        v.diags.iter().any(|d| d.error && d.message.contains("Nope")),
        "{v:?}"
    );
    // the error in words, with its place
    assert!(
        h.query_by_label_contains("(line 1, column ").is_some(),
        "the error is listed"
    );
    h.get_by_label("Error:");
    // nothing saved
    assert!(last_sent(&h, "role.set_filter").is_none());
    assert_eq!(filter(&h, "Readers", "Sales"), "[Region] = \"E\"");
    // a valid filter checks clean
    if let Some(e) = h.state_mut().cmd.roles.as_mut().and_then(|d| d.editor.as_mut()) {
        e.dax.text = "[Amount] > 5".into();
    }
    steps(&mut h, 2);
    click(&mut h, "Check");
    until(&mut h, "the second check", |a| {
        a.cmd
            .roles
            .as_ref()
            .and_then(|d| d.editor.as_ref())
            .is_some_and(|e| !e.checking)
    });
    steps(&mut h, 3);
    h.get_by_label("No errors found.");
    assert!(editor(&h).verdict.is_some_and(|v| v.valid));
}

#[test]
fn switching_back_warns_before_a_dax_only_filter_is_lost() {
    let mut h = open("en-US");
    click(&mut h, "Switch to DAX editor");
    // a filter the default editor shows: the switch back keeps it
    click(&mut h, "Switch to default editor");
    let e = editor(&h);
    assert_eq!((e.mode, e.rows.len()), (Mode::Default, 1));
    click(&mut h, "Switch to DAX editor");
    if let Some(e) = h.state_mut().cmd.roles.as_mut().and_then(|d| d.editor.as_mut()) {
        e.dax.text = "[Region] = USERPRINCIPALNAME()".into();
    }
    steps(&mut h, 2);
    click(&mut h, "Switch to default editor");
    assert!(editor(&h).confirm_switch, "the warning shows");
    h.get_by_label("Warning");
    click_last(&mut h, "Cancel");
    let e = editor(&h);
    assert_eq!((e.mode, e.confirm_switch), (Mode::Dax, false));
    assert_eq!(e.dax.text, "[Region] = USERPRINCIPALNAME()");
    click(&mut h, "Switch to default editor");
    click(&mut h, "Switch anyway");
    let e = editor(&h);
    assert_eq!((e.mode, e.rows.len()), (Mode::Default, 0));
}

#[test]
fn a_filter_the_default_editor_cannot_show_opens_in_the_dax_editor() {
    let mut h = app_with("en-US", || {
        let mut s = roles_session();
        s.execute(
            "role.set_filter",
            &json!({"role": "Readers", "table": "Sales", "filter_expression": "[Region] = USERNAME()"}),
        )
        .unwrap();
        s
    });
    explorer_menu(&mut h, "Roles (1)", "Manage roles");
    assert_eq!(editor(&h).mode, Mode::Dax);
    h.get_by_label("DAX filter of Sales");
    // the tables list marks the filtered table with an icon and a word
    h.get_by_label("filtered");
}

#[test]
fn roles_are_renamed_added_and_deleted() {
    let mut h = open("en-US");
    click(&mut h, "Rename");
    h.get_by_label("New name of Readers").click();
    steps(&mut h, 2);
    press(&mut h, Key::A, Modifiers::COMMAND);
    type_text(&mut h, "East, West");
    h.get_by_label("A role name cannot contain a comma.");
    press(&mut h, Key::A, Modifiers::COMMAND);
    type_text(&mut h, "East readers");
    click(&mut h, "Apply");
    steps(&mut h, 6);
    assert_eq!(
        last_sent(&h, "role.rename"),
        Some(json!({"name": "Readers", "new_name": "East readers"}))
    );
    let roles = |h: &Harness<'_, App>| {
        h.state()
            .loaded
            .as_ref()
            .and_then(|l| l.snap.pointer("/model/roles").cloned())
            .unwrap_or(Value::Null)
    };
    assert_eq!(roles(&h), json!(["East readers"]));
    assert_eq!(
        h.state()
            .cmd
            .roles
            .as_ref()
            .and_then(|d| d.role.clone())
            .as_deref(),
        Some("East readers")
    );
    // the filter stays with the renamed role
    assert_eq!(editor(&h).saved, "[Region] = \"E\"");
    // New
    h.get_by_label("New role name").click();
    steps(&mut h, 2);
    type_text(&mut h, "Auditors");
    click(&mut h, "New");
    steps(&mut h, 6);
    assert_eq!(last_sent(&h, "role.add"), Some(json!({"name": "Auditors"})));
    assert_eq!(roles(&h), json!(["East readers", "Auditors"]));
    // Delete the chosen one (the new role)
    click(&mut h, "Delete");
    steps(&mut h, 6);
    assert_eq!(last_sent(&h, "role.remove"), Some(json!({"name": "Auditors"})));
    assert_eq!(roles(&h), json!(["East readers"]));
    assert!(h.state().error.is_none(), "{:?}", h.state().error);
}

#[test]
fn manage_security_roles_speaks_french() {
    let mut h = open("fr-FR");
    h.get_by_label("Gérer les rôles de sécurité");
    h.get_by_label("Sélectionner des tables");
    h.get_by_label("Passer à l'éditeur DAX");
    h.get_by_label("Vérifier");
    h.get_by_label("Enregistrer le filtre Sales");
    h.get_by_label("Colonne de la condition 1");
    h.get_by_label("Renommer");
    click(&mut h, "Table Product");
    click(&mut h, "Nouvelle condition");
    h.get_by_label("Valeur de la condition 1");
    click(&mut h, "Supprimer la condition 1");
    click(&mut h, "Passer à l'éditeur DAX");
    h.get_by_label("Filtre DAX de Product");
    click(&mut h, "Vérifier");
    until(&mut h, "the check", |a| {
        a.cmd
            .roles
            .as_ref()
            .and_then(|d| d.editor.as_ref())
            .is_some_and(|e| !e.checking)
    });
    steps(&mut h, 3);
    h.get_by_label("Aucun filtre : les membres du rôle voient toutes les lignes.");
}
