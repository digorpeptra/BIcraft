//! Storage mode in the Model view (egui_kittest, no GPU): the Properties pane's Advanced ›
//! Storage mode list sends `table.set_storage_mode`; switching a DirectQuery table to Import asks
//! first and offers to set the DirectQuery dimension tables to Dual, in English and French.

use bicraft_engine::Session;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::backstage_tests::steps;
use super::model_sec_ui_tests::{app_with, last_sent};
use crate::App;
use crate::model_view::Sel;

const SQL: &str =
    "let Source = Sql.Database(\"srv.example\", \"db\"), T = Source{[Schema=\"dbo\",Item=\"T\"]}[Data] in T";

/// Sales (DirectQuery) many-to-one Product (DirectQuery); Notes (Import, entered).
pub(super) fn session() -> Session {
    let mut s = Session::new();
    s.execute("model.new", &json!({"name": "Shop"})).unwrap();
    for (table, cols) in [
        ("Sales", &["ProductKey", "Amount"][..]),
        ("Product", &["ProductKey"][..]),
    ] {
        s.execute(
            "table.add",
            &json!({"name": table, "partition": {"source_type": "m", "expression": SQL}}),
        )
        .unwrap();
        for c in cols {
            s.execute(
                "column.add",
                &json!({"table": table, "name": c, "data_type": "int64"}),
            )
            .unwrap();
        }
        s.execute(
            "table.set_storage_mode",
            &json!({"table": table, "mode": "directQuery"}),
        )
        .unwrap();
    }
    for (id, p) in [
        (
            "relationship.add",
            json!({"from": "Sales[ProductKey]", "to": "Product[ProductKey]"}),
        ),
        (
            "table.import",
            json!({"name": "Notes", "m": "#table(type table [Id = Int64.Type], {{1}, {2}})"}),
        ),
        ("report.new", json!({})),
    ] {
        s.execute(id, &p).unwrap();
    }
    s.dirty = false;
    s.report_dirty = false;
    s
}

fn mode(h: &Harness<'_, App>, table: &str) -> String {
    h.state()
        .loaded
        .as_ref()
        .and_then(|l| l.snap.pointer("/model/tables"))
        .and_then(Value::as_array)
        .and_then(|a| a.iter().find(|t| t["name"] == table))
        .and_then(|t| t["storage_mode"].as_str())
        .unwrap_or_default()
        .to_string()
}

fn model_view(lang: &str, table: &str) -> Harness<'static, App> {
    let mut h = app_with(lang, session);
    h.state_mut().show_tab(crate::app::Tab::Model);
    steps(&mut h, 3);
    h.state_mut().model_view.sel = Some(Sel::Table(table.into()));
    steps(&mut h, 4);
    h
}

/// Picks `option` in the Properties pane's Storage mode list.
fn pick(h: &mut Harness<'static, App>, list: &str, option: &str) {
    let r = h
        .get_by_role_and_label(egui::accesskit::Role::ComboBox, list)
        .rect();
    super::canvas_ui_tests::click_with(h, r.center(), egui::Modifiers::NONE);
    steps(h, 2);
    // the option in the open list (the lowest node of that label)
    let r = h
        .get_all_by_label(option)
        .map(|n| n.rect())
        .max_by(|a, b| a.min.y.total_cmp(&b.min.y))
        .unwrap_or_else(|| panic!("no option `{option}`"));
    super::canvas_ui_tests::click_with(h, r.center(), egui::Modifiers::NONE);
    steps(h, 4);
}

fn until(h: &mut Harness<'_, App>, what: &str, f: impl Fn(&App) -> bool) {
    for _ in 0..200 {
        if f(h.state()) {
            return;
        }
        h.step();
    }
    panic!("timed out waiting for {what}");
}

#[test]
fn switching_a_direct_query_table_to_import_asks_and_sets_dual() {
    let mut h = model_view("en", "Sales");
    assert_eq!(mode(&h, "Sales"), "directQuery");
    pick(&mut h, "Storage mode", "Import");
    // nothing sent yet: the confirmation lists Product, ticked
    assert!(last_sent(&h, "table.set_storage_mode").is_none());
    let c = h.state().cmd.storage.clone().expect("the confirmation");
    assert_eq!(c.candidates, vec!["Product".to_string()]);
    assert!(c.dual);
    h.get_by_label("Set these tables to Dual");
    h.get_by_label("OK").click();
    steps(&mut h, 4);
    assert_eq!(
        last_sent(&h, "table.set_storage_mode"),
        Some(json!({"table": "Sales", "mode": "import", "dual_related": true}))
    );
    until(&mut h, "the new modes", |a| {
        a.loaded.as_ref().is_some_and(|l| {
            l.snap
                .pointer("/model/tables")
                .and_then(Value::as_array)
                .is_some_and(|t| {
                    t.iter()
                        .any(|t| t["name"] == "Product" && t["storage_mode"] == "dual")
                })
        })
    });
    assert_eq!(mode(&h, "Sales"), "import");
    assert!(h.state().cmd.storage.is_none());
}

#[test]
fn cancel_keeps_the_mode_and_dual_is_sent_at_once_in_french() {
    let mut h = model_view("fr", "Sales");
    pick(&mut h, "Mode de stockage", "Importation");
    h.get_by_label("Passer ces tables en mode double");
    h.get_by_label("Annuler").click();
    steps(&mut h, 4);
    assert!(h.state().cmd.storage.is_none());
    assert!(last_sent(&h, "table.set_storage_mode").is_none());
    assert_eq!(mode(&h, "Sales"), "directQuery");
    // Dual needs no confirmation
    h.state_mut().model_view.sel = Some(Sel::Table("Product".into()));
    steps(&mut h, 4);
    pick(&mut h, "Mode de stockage", "Double");
    assert_eq!(
        last_sent(&h, "table.set_storage_mode"),
        Some(json!({"table": "Product", "mode": "dual"}))
    );
    until(&mut h, "Product in Dual", |a| {
        a.loaded.as_ref().is_some_and(|l| {
            l.snap
                .pointer("/model/tables")
                .and_then(Value::as_array)
                .is_some_and(|t| {
                    t.iter()
                        .any(|t| t["name"] == "Product" && t["storage_mode"] == "dual")
                })
        })
    });
}
