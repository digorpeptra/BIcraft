//! View as (Model › Security › View as) through the accessibility tree (egui_kittest, no GPU),
//! over an engine session run in the test: the dialog's None, Other user and role check boxes
//! and OK, the banner and its Stop viewing, in English and French, for a reader too. Every
//! gesture runs `view.interact {action: "view_as"}` and never touches the report.

use std::path::Path;

use bicraft_engine::Session;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use super::backstage_tests::{engine_shell, steps};
use super::harness;
use crate::App;
use crate::shell::Caps;

/// Sales(Region, Manager, Amount) with roles East, West and Managers (Manager =
/// USERPRINCIPALNAME()), and a card of Total on the first page.
pub(super) fn rls_session() -> Session {
    let mut s = Session::new();
    let sales = r#"#table({"Region", "Manager", "Amount"}, {{"East", "alice@contoso.test", 100}, {"West", "bob@contoso.test", 1000}, {"East", "alice@contoso.test", 5}})"#;
    let run = |s: &mut Session, id: &str, p: Value| s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Region", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Manager", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])", "format_string": "#,0"}),
        ),
        ("role.add", json!({"name": "East"})),
        (
            "role.set_filter",
            json!({"role": "East", "table": "Sales", "filter_expression": "[Region] = \"East\""}),
        ),
        ("role.add", json!({"name": "West"})),
        (
            "role.set_filter",
            json!({"role": "West", "table": "Sales", "filter_expression": "[Region] = \"West\""}),
        ),
        ("role.add", json!({"name": "Managers"})),
        (
            "role.set_filter",
            json!({"role": "Managers", "table": "Sales", "filter_expression": "[Manager] = USERPRINCIPALNAME()"}),
        ),
        ("report.new", json!({})),
    ] {
        run(&mut s, id, p);
    }
    let page = s
        .report
        .as_ref()
        .map(|r| r.pages[0].name.clone())
        .unwrap_or_default();
    run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "card", "x": 40, "y": 40, "width": 300, "height": 160,
               "fields": [{"role": "Values", "field": "[Total]"}]}),
    );
    s.dirty = false;
    s.report_dirty = false;
    s
}

pub(super) fn app(lang: &str) -> Harness<'static, App> {
    let mut app = App::new(lang, engine_shell(Caps::DESKTOP, rls_session));
    app.open_path(Path::new("r.pbip"));
    let mut h = harness(app, (1400.0, 860.0));
    steps(&mut h, 6);
    h
}

/// The snapshot's View as state.
fn view_as(h: &Harness<'_, App>) -> Value {
    h.state()
        .loaded
        .as_ref()
        .and_then(|l| l.snap.pointer("/interaction/view_as").cloned())
        .unwrap_or(Value::Null)
}

/// The card's value as the snapshot evaluates it.
fn total(h: &Harness<'_, App>) -> f64 {
    h.state()
        .loaded
        .as_ref()
        .and_then(|l| {
            l.snap
                .pointer("/report/pages/0/visuals/0/data/rows/0/0")
                .and_then(Value::as_f64)
        })
        .unwrap_or(f64::NAN)
}

/// Model tab, then View as: the dialog opens.
fn open_dialog(h: &mut Harness<'_, App>, tab: &str, button: &str) {
    // the tab, unless its ribbon is shown already (a click on the shown tab folds the ribbon)
    if h.query_by_label(button).is_none() {
        h.get_all_by_label(tab)
            .find(|n| n.accesskit_node().role() == egui::accesskit::Role::Button)
            .unwrap_or_else(|| panic!("no {tab} tab"))
            .click();
        steps(h, 3);
    }
    h.get_by_label(button).click();
    steps(h, 3);
    assert!(h.state().cmd.view_as.is_some(), "the dialog did not open");
}

#[test]
fn checking_a_role_and_ok_views_the_report_as_that_role() {
    let mut h = app("en-US");
    assert_eq!(total(&h), 1105.0);
    open_dialog(&mut h, "Model", "View as");
    for label in [
        "View as roles",
        "None",
        "Other user",
        "East",
        "West",
        "Managers",
        "OK",
        "Cancel",
    ] {
        h.get_by_label(label);
    }
    h.get_by_label("East").click();
    steps(&mut h, 2);
    h.get_by_label("OK").click();
    steps(&mut h, 6);
    assert!(h.state().cmd.view_as.is_none(), "OK closes the dialog");
    assert_eq!(
        h.state().last_action,
        Some(json!({"action": "view_as", "roles": ["East"], "user": null}))
    );
    assert_eq!(view_as(&h), json!({"roles": ["East"], "user": null}));
    assert_eq!(total(&h), 105.0);
    // the banner says who the report is seen as
    h.get_by_label("Now viewing as: East");
    h.get_by_label("Stop viewing");
    // view state: nothing unsaved
    let snap = h.state().loaded.as_ref().map(|l| l.snap.clone()).unwrap();
    assert_eq!(
        snap["dirty"],
        json!({"model": false, "report": false, "queries": false})
    );
}

#[test]
fn other_user_names_the_user_the_roles_see() {
    let mut h = app("en-US");
    open_dialog(&mut h, "Model", "View as");
    h.get_by_label("Other user").click();
    steps(&mut h, 2);
    h.get_by_label("User name or email").click();
    steps(&mut h, 1);
    h.get_by_label("User name or email").type_text("bob@contoso.test");
    steps(&mut h, 2);
    h.get_by_label("Managers").click();
    steps(&mut h, 2);
    h.get_by_label("OK").click();
    steps(&mut h, 6);
    assert_eq!(
        view_as(&h),
        json!({"roles": ["Managers"], "user": "bob@contoso.test"})
    );
    assert_eq!(total(&h), 1000.0);
    h.get_by_label("Now viewing as: Managers (bob@contoso.test)");
}

#[test]
fn stop_viewing_on_the_banner_shows_the_data_as_stored() {
    let mut h = app("en-US");
    open_dialog(&mut h, "Model", "View as");
    h.get_by_label("West").click();
    steps(&mut h, 2);
    h.get_by_label("OK").click();
    steps(&mut h, 6);
    assert_eq!(total(&h), 1000.0);
    h.get_by_label("Stop viewing").click();
    steps(&mut h, 6);
    assert_eq!(
        h.state().last_action,
        Some(json!({"action": "view_as", "roles": null}))
    );
    assert_eq!(view_as(&h), Value::Null);
    assert_eq!(total(&h), 1105.0);
    assert!(h.query_by_label("Stop viewing").is_none(), "the banner stays");
}

#[test]
fn none_clears_the_roles_and_cancel_changes_nothing() {
    let mut h = app("en-US");
    open_dialog(&mut h, "Model", "View as");
    h.get_by_label("East").click();
    steps(&mut h, 2);
    h.get_by_label("Cancel").click();
    steps(&mut h, 4);
    assert!(h.state().cmd.view_as.is_none());
    assert_eq!(view_as(&h), Value::Null);
    assert_eq!(h.state().last_action, None);
    // on again, then None and OK: stopped
    open_dialog(&mut h, "Model", "View as");
    h.get_by_label("East").click();
    steps(&mut h, 2);
    h.get_by_label("OK").click();
    steps(&mut h, 6);
    assert_eq!(total(&h), 105.0);
    open_dialog(&mut h, "Model", "View as");
    // the dialog opens on the current state
    assert_eq!(
        h.state().cmd.view_as.as_ref().map(|d| d.roles.clone()),
        Some(vec![
            ("East".into(), true),
            ("West".into(), false),
            ("Managers".into(), false)
        ])
    );
    h.get_by_label("None").click();
    steps(&mut h, 2);
    assert!(
        h.state()
            .cmd
            .view_as
            .as_ref()
            .is_some_and(|d| d.roles.iter().all(|(_, on)| !on))
    );
    h.get_by_label("OK").click();
    steps(&mut h, 6);
    assert_eq!(view_as(&h), Value::Null);
    assert_eq!(total(&h), 1105.0);
}

#[test]
fn the_dialog_and_the_banner_in_french() {
    let mut h = app("fr-FR");
    open_dialog(&mut h, "Modèle", "Afficher en tant que");
    for label in [
        "Afficher en tant que rôles",
        "Aucun",
        "Autre utilisateur",
        "Annuler",
    ] {
        h.get_by_label(label);
    }
    h.get_by_label("East").click();
    steps(&mut h, 2);
    h.get_by_label("OK").click();
    steps(&mut h, 6);
    h.get_by_label("Affichage en tant que : East");
    h.get_by_label("Arrêter l'affichage");
}

#[test]
fn a_reader_may_view_as_a_role() {
    let mut h = app("en-US");
    h.state_mut().access = Some("reader".into());
    steps(&mut h, 2);
    open_dialog(&mut h, "Model", "View as");
    h.get_by_label("East").click();
    steps(&mut h, 2);
    h.get_by_label("OK").click();
    steps(&mut h, 6);
    assert_eq!(total(&h), 105.0);
    // the role buttons that edit the model stay greyed for a reader
    let n = h.get_by_label("New role");
    assert!(
        n.accesskit_node().is_disabled(),
        "New role is enabled for a reader"
    );
}
