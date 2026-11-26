//! The Incremental refresh dialog (egui_kittest, no GPU): a table's menu opens it; the switch
//! stays off with the reason until the model has RangeStart and RangeEnd and the query filters on
//! them; Apply sends `table.set_refresh_policy` with the periods shown in the review sentences, and
//! `table.remove_refresh_policy` when the switch is turned off; English and French.

use bicraft_engine::Session;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::backstage_tests::steps;
use super::model_sec_ui_tests::{app_with, last_sent};
use crate::App;
use crate::model_view::Sel;

const PARAM: &str = "#datetime(2024, 1, 1, 0, 0, 0) meta [IsParameterQuery=true, Type=\"DateTime\", IsParameterQueryRequired=true]";
const FILTERED: &str = "let Source = Sql.Database(\"srv.example\", \"db\"), Orders = Source{[Schema=\"dbo\",Item=\"Orders\"]}[Data], Filtered = Table.SelectRows(Orders, each [OrderDate] >= RangeStart and [OrderDate] < RangeEnd) in Filtered";

/// Orders (Import, filtered on RangeStart and RangeEnd), with the parameters when `params`.
fn build(params: bool) -> Session {
    let mut s = Session::new();
    s.execute("model.new", &json!({"name": "Shop"})).unwrap();
    if params {
        for n in ["RangeStart", "RangeEnd"] {
            s.execute("expression.add", &json!({"name": n, "expression": PARAM}))
                .unwrap();
        }
    }
    s.execute(
        "table.add",
        &json!({"name": "Orders", "partition": {"source_type": "m", "expression": FILTERED}}),
    )
    .unwrap();
    for (c, ty) in [
        ("OrderDate", "dateTime"),
        ("Modified", "dateTime"),
        ("Amount", "int64"),
    ] {
        s.execute(
            "column.add",
            &json!({"table": "Orders", "name": c, "data_type": ty}),
        )
        .unwrap();
    }
    s.execute("report.new", &json!({})).unwrap();
    s.dirty = false;
    s.report_dirty = false;
    s
}

pub(super) fn ready() -> Session {
    build(true)
}

fn bare() -> Session {
    build(false)
}

fn policy(h: &Harness<'_, App>) -> Option<Value> {
    h.state()
        .loaded
        .as_ref()
        .and_then(|l| l.snap.pointer("/model/tables"))
        .and_then(Value::as_array)
        .and_then(|a| a.iter().find(|t| t["name"] == "Orders"))
        .and_then(|t| t.get("refresh_policy"))
        .cloned()
}

fn until(h: &mut Harness<'_, App>, what: &str, f: impl Fn(&Harness<'_, App>) -> bool) {
    for _ in 0..200 {
        if f(h) {
            return;
        }
        h.step();
    }
    panic!("timed out waiting for {what}");
}

/// The table's menu item, as the Data pane and the diagram run it.
fn open(h: &mut Harness<'static, App>) {
    let params = crate::menus::item_params("table.set_refresh_policy", Some(&Sel::Table("Orders".into())));
    assert_eq!(params, json!({"table": "Orders"}));
    crate::ribbon::invoke(h.state_mut(), "table.set_refresh_policy", params);
    steps(h, 3);
}

#[test]
fn the_menu_opens_the_dialog_and_apply_sets_then_removes_the_policy() {
    let mut h = app_with("en", ready);
    open(&mut h);
    let d = h.state().cmd.refresh_policy.clone().expect("the dialog");
    assert!(!d.on && !d.had_policy);
    // nothing sent by opening it
    assert!(last_sent(&h, "table.set_refresh_policy").is_none());
    h.get_by_role_and_label(Role::CheckBox, "Incrementally refresh this table")
        .click();
    steps(&mut h, 2);
    h.get_by_label("Archive data starting 5 years before refresh date");
    h.get_by_label("Incrementally refresh data starting 10 days before refresh date");
    h.get_by_label("Apply").click();
    steps(&mut h, 3);
    assert_eq!(
        last_sent(&h, "table.set_refresh_policy"),
        Some(
            json!({"table": "Orders", "store": {"n": 5, "unit": "year"}, "refresh": {"n": 10, "unit": "day"}})
        )
    );
    assert!(h.state().cmd.refresh_policy.is_none());
    until(&mut h, "the policy", |h| policy(h).is_some());
    // opened again: filled from the policy; switching it off removes it
    open(&mut h);
    let d = h.state().cmd.refresh_policy.clone().expect("the dialog again");
    assert!(d.on && d.had_policy);
    h.get_by_role_and_label(Role::CheckBox, "Incrementally refresh this table")
        .click();
    steps(&mut h, 2);
    h.get_by_label("Apply removes the table's incremental refresh policy.");
    h.get_by_label("Apply").click();
    steps(&mut h, 3);
    assert_eq!(
        last_sent(&h, "table.remove_refresh_policy"),
        Some(json!({"table": "Orders"}))
    );
    until(&mut h, "no policy", |h| policy(h).is_none());
}

#[test]
fn without_range_parameters_the_switch_is_off_with_the_reason_in_french() {
    let mut h = app_with("fr", bare);
    open(&mut h);
    assert!(h.state().cmd.refresh_policy.is_some());
    h.get_by_label(
        "Créez d'abord les paramètres Date/Heure RangeStart et RangeEnd (éditeur Power Query › Gérer les paramètres).",
    );
    h.get_by_role_and_label(Role::CheckBox, "Actualiser cette table de manière incrémentielle")
        .click();
    steps(&mut h, 2);
    assert!(!h.state().cmd.refresh_policy.clone().unwrap().on);
    h.get_by_label("Annuler").click();
    steps(&mut h, 2);
    assert!(h.state().cmd.refresh_policy.is_none());
    assert!(last_sent(&h, "table.set_refresh_policy").is_none());
}

#[test]
fn the_review_sentences_follow_the_settings() {
    let d = crate::refresh_policy_dialog::Dialog {
        table: "Orders".into(),
        on: true,
        store_n: "1".into(),
        store_unit: "quarter",
        refresh_n: "3".into(),
        refresh_unit: "month",
        realtime: true,
        complete: false,
        detect: true,
        detect_column: Some("Modified".into()),
        had_policy: false,
    };
    let fr = crate::refresh_policy_dialog::summary(&d, crate::i18n::Lang::Fr);
    assert_eq!(
        fr,
        vec![
            "Archiver les données à partir de 1 trimestre avant la date d'actualisation".to_string(),
            "Actualiser les données de manière incrémentielle à partir de 3 mois avant la date d'actualisation".to_string(),
            "Obtenir les dernières données en temps réel avec DirectQuery".to_string(),
        ]
    );
    let call = crate::refresh_policy_dialog::apply_call(&d).unwrap();
    assert_eq!(
        call.params,
        json!({"table": "Orders", "store": {"n": 1, "unit": "quarter"}, "refresh": {"n": 3, "unit": "month"},
               "realtime": true, "only_complete_periods": true, "detect_changes": "Modified"})
    );
}
