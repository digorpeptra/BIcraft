//! The Modeling ribbon's model dialogs through the accessibility tree (egui_kittest, no GPU):
//! Relationships › Autodetect (the preview's list, Add running `relationship.autodetect` with
//! the ticked ones) and Manage roles (object-level security and row filters), in English and
//! French.

use std::path::Path;

use bicraft_engine::Session;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::backstage_tests::{engine_shell, steps};
use super::harness;
use crate::App;
use crate::registry::Tab as RTab;
use crate::shell::Caps;

/// Sales(ProductKey, Region, Amount), Product(ProductKey, Name), Region(Region), no
/// relationship: autodetect finds Sales → Product and Sales → Region.
pub(super) fn session() -> Session {
    let mut s = Session::new();
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.import",
            json!({"name": "Sales", "m": "#table(type table [ProductKey = number, Region = text, Amount = number], {{1, \"E\", 10}, {1, \"W\", 5}, {2, \"E\", 7}})"}),
        ),
        (
            "table.import",
            json!({"name": "Product", "m": "#table(type table [ProductKey = number, Name = text], {{1, \"Bike\"}, {2, \"Car\"}})"}),
        ),
        (
            "table.import",
            json!({"name": "Region", "m": "#table(type table [Region = text], {{\"E\"}, {\"W\"}})"}),
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

pub(super) fn app_with(lang: &str, session: fn() -> Session) -> Harness<'static, App> {
    let mut app = App::new(lang, engine_shell(Caps::DESKTOP, session)).with_sync_render();
    app.open_path(Path::new("shop.pbip"));
    let mut h = harness(app, (1280.0, 860.0));
    for _ in 0..200 {
        h.step();
        if h.state().shown.is_some() {
            break;
        }
    }
    steps(&mut h, 2);
    if let Some(v) = crate::ribbon::view_of(RTab::Model) {
        h.state_mut().show_tab(v);
    }
    h.state_mut().ribbon_tab = RTab::Model;
    steps(&mut h, 3);
    h
}

pub(super) fn click(h: &mut Harness<'_, App>, label: &str) {
    h.get_by_label(label).click();
    steps(h, 4);
}

pub(super) fn last_sent(h: &Harness<'_, App>, id: &str) -> Option<Value> {
    h.state()
        .sent
        .iter()
        .rev()
        .find(|c| c.id == id)
        .map(|c| c.params.clone())
}

fn relationships(h: &Harness<'_, App>) -> usize {
    h.state()
        .loaded
        .as_ref()
        .and_then(|l| {
            l.snap
                .pointer("/model/relationships")
                .and_then(Value::as_array)
                .map(Vec::len)
        })
        .unwrap_or(0)
}

/// The Model explorer's Model tab, then a right click on the row labelled `row` and its menu
/// item `item` (the lowest widget of that label: a ribbon button may share it).
pub(super) fn explorer_menu(h: &mut Harness<'_, App>, row: &str, item: &str) {
    h.state_mut().model_view.explorer.tab = crate::model_explorer::Tab::Model;
    steps(h, 3);
    let r = h.get_by_label(row).rect();
    super::backstage_tests::right_click(h, r.center());
    let at = h
        .get_all_by_label(item)
        .map(|n| n.rect())
        .max_by(|a, b| a.min.y.total_cmp(&b.min.y))
        .unwrap_or_else(|| panic!("no menu item `{item}`"));
    let pos = at.center();
    h.hover_at(pos);
    h.step();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        h.step();
    }
    steps(h, 4);
}

#[test]
fn autodetect_lists_the_candidates_and_adds_the_ticked_ones() {
    let mut h = app_with("en-US", session);
    explorer_menu(&mut h, "Relationships (0)", "Autodetect relationships");
    steps(&mut h, 4);
    // the preview ran without apply and changed nothing
    assert_eq!(last_sent(&h, "relationship.autodetect"), Some(json!({})));
    assert_eq!(relationships(&h), 0);
    let first = "Sales[ProductKey] → Product[ProductKey]  (many to one, single direction)";
    let second = "Sales[Region] → Region[Region]  (many to one, single direction)";
    h.get_by_label(first);
    // untick the second, then Add
    click(&mut h, second);
    click(&mut h, "Add (1)");
    steps(&mut h, 4);
    let p = last_sent(&h, "relationship.autodetect").unwrap();
    assert_eq!(
        p,
        json!({"apply": true, "only": [{"from": "Sales[ProductKey]", "to": "Product[ProductKey]"}]})
    );
    assert!(h.state().cmd.autodetect.is_none(), "the dialog closes");
    assert_eq!(relationships(&h), 1);
    assert!(h.state().error.is_none(), "{:?}", h.state().error);
}

#[test]
fn autodetect_speaks_french() {
    let mut h = app_with("fr-FR", session);
    explorer_menu(&mut h, "Relations (0)", "Détecter les relations");
    steps(&mut h, 4);
    h.get_by_label("Sales[Region] → Region[Region]  (plusieurs à un, un seul sens)");
    h.get_by_label("Annuler");
    click(&mut h, "Ajouter (2)");
    steps(&mut h, 4);
    assert_eq!(relationships(&h), 2);
}

/// The session of [`session`] with the role Readers (Sales filtered to East).
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

fn perm(h: &Harness<'_, App>, role: &str) -> Value {
    h.state()
        .loaded
        .as_ref()
        .and_then(|l| l.snap.pointer("/model/security").cloned())
        .and_then(|v| {
            v.as_array()?
                .iter()
                .find(|r| r["name"] == role)
                .map(|r| r["tables"].clone())
        })
        .unwrap_or(Value::Null)
}

#[test]
fn manage_roles_hides_a_column_and_a_table() {
    let mut h = app_with("en-US", roles_session);
    explorer_menu(&mut h, "Roles (1)", "Manage roles");
    assert!(h.state().cmd.roles.is_some(), "the dialog opens");
    // a column of Product: pick the table, open its columns, hide Name
    click(&mut h, "Table Product");
    click(&mut h, "Columns of Product");
    click(&mut h, "Hide Product[Name]");
    steps(&mut h, 4);
    assert_eq!(
        last_sent(&h, "role.set_permission"),
        Some(json!({"role": "Readers", "table": "Product", "column": "Name", "metadata_permission": "none"}))
    );
    let t = perm(&h, "Readers");
    assert!(
        t.as_array().unwrap().iter().any(|x| x["table"] == "Product"
            && x["columns"][0] == json!({"name": "Name", "metadata_permission": "none"})),
        "{t}"
    );
    // the whole Region table
    click(&mut h, "Table Region");
    click(&mut h, "Hide table Region");
    steps(&mut h, 4);
    assert_eq!(
        last_sent(&h, "role.set_permission"),
        Some(json!({"role": "Readers", "table": "Region", "metadata_permission": "none"}))
    );
    // and back: null removes the permission
    click(&mut h, "Hide table Region");
    steps(&mut h, 4);
    assert_eq!(
        last_sent(&h, "role.set_permission").unwrap()["metadata_permission"],
        Value::Null
    );
    assert!(h.state().error.is_none(), "{:?}", h.state().error);
}

#[test]
fn manage_roles_speaks_french() {
    let mut h = app_with("fr-FR", roles_session);
    explorer_menu(&mut h, "Rôles (1)", "Gérer les rôles");
    h.get_by_label("Masquer la table Sales");
    h.get_by_label("Fermer");
    click(&mut h, "Table Region");
    click(&mut h, "Masquer la table Region");
    steps(&mut h, 4);
    assert_eq!(last_sent(&h, "role.set_permission").unwrap()["table"], "Region");
}

/// The session of [`session`] with the perspective Lite (Sales[Amount] and [Total]).
fn persp_session() -> Session {
    let mut s = session();
    s.execute(
        "perspective.add",
        &json!({"name": "Lite", "tables": [{"table": "Sales", "columns": ["Amount"], "measures": ["Total"]}]}),
    )
    .unwrap();
    s.dirty = false;
    s
}

fn members(h: &Harness<'_, App>) -> Value {
    h.state()
        .loaded
        .as_ref()
        .and_then(|l| l.snap.pointer("/model/perspective_members").cloned())
        .unwrap_or(Value::Null)
}

/// Clicks the lowest widget labelled `label` (a combo box shares its label's text; a list item
/// its selected text).
fn click_lowest(h: &mut Harness<'_, App>, label: &str) {
    let at = h
        .get_all_by_label(label)
        .map(|n| n.rect())
        .max_by(|a, b| a.min.y.total_cmp(&b.min.y))
        .unwrap_or_else(|| panic!("no widget `{label}`"));
    h.hover_at(at.center());
    h.step();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: at.center(),
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        h.step();
    }
    steps(h, 4);
}

#[test]
fn the_perspectives_dialog_saves_the_ticked_members() {
    let mut h = app_with("en-US", persp_session);
    explorer_menu(&mut h, "Perspectives (1)", "Manage perspectives");
    assert!(h.state().cmd.perspective.is_some(), "the dialog opens");
    // the whole Region table, and Product's Name only
    click(&mut h, "Include table Region");
    click(&mut h, "Members of Product");
    click(&mut h, "Product[Name]");
    click(&mut h, "Save the perspective");
    steps(&mut h, 4);
    assert_eq!(
        last_sent(&h, "perspective.set"),
        Some(json!({"name": "Lite", "tables": [
            {"table": "Sales", "columns": ["Amount"], "measures": ["Total"], "hierarchies": []},
            {"table": "Product", "columns": ["Name"], "measures": [], "hierarchies": []},
            {"table": "Region", "columns": ["Region"], "measures": [], "hierarchies": []}
        ]}))
    );
    assert_eq!(members(&h)[0]["tables"].as_array().map(Vec::len), Some(3));
    // a new perspective
    h.get_by_label("New perspective name").click();
    steps(&mut h, 2);
    h.get_by_label("New perspective name").type_text("Empty");
    steps(&mut h, 2);
    click(&mut h, "Create");
    steps(&mut h, 4);
    assert_eq!(last_sent(&h, "perspective.add"), Some(json!({"name": "Empty"})));
    assert!(h.state().error.is_none(), "{:?}", h.state().error);
}

#[test]
fn the_model_view_filters_to_a_perspective() {
    let mut h = app_with("en-US", persp_session);
    let cards = |h: &Harness<'_, App>| -> Vec<String> {
        h.state()
            .model_view
            .diagram()
            .map(|d| d.cards.iter().map(|c| c.name.clone()).collect())
            .unwrap_or_default()
    };
    assert_eq!(cards(&h).len(), 3);
    click_lowest(&mut h, "Perspective");
    click_lowest(&mut h, "Lite");
    steps(&mut h, 4);
    assert_eq!(h.state().model_view.perspective.as_deref(), Some("Lite"));
    assert_eq!(cards(&h), ["Sales"]);
    click_lowest(&mut h, "Perspective");
    click_lowest(&mut h, "(All)");
    steps(&mut h, 4);
    assert_eq!(cards(&h).len(), 3);
}

#[test]
fn perspectives_speak_french() {
    let mut h = app_with("fr-FR", persp_session);
    explorer_menu(&mut h, "Perspectives (1)", "Gérer les perspectives");
    h.get_by_label("Inclure la table Sales");
    h.get_by_label("Enregistrer la perspective");
    h.get_by_label("Nom de la nouvelle perspective");
    // the explorer's menu of a perspective deletes it
    h.state_mut().cmd.perspective = None;
    steps(&mut h, 2);
    click_lowest(&mut h, "Perspectives (1)");
    explorer_menu(&mut h, "Lite", "Supprimer la perspective");
    assert_eq!(last_sent(&h, "perspective.remove"), Some(json!({"name": "Lite"})));
}
