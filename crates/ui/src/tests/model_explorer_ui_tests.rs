//! The Model explorer's objects in the Properties pane (G61) through the accessibility tree
//! (egui_kittest, no GPU): a calculation item, a role, a shared expression, a culture and a
//! hierarchy are selected in the tree and edited by their engine commands; the search box; a
//! layout tab's Add related tables and Remove from layout.

use bicraft_engine::Session;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::backstage_tests::steps;
use super::model_sec_ui_tests::{app_with, last_sent, session};
use crate::App;
use crate::model_explorer::Tab;
use crate::model_view::{Item, Sel};

/// The shop of [`session`] with a calculation group Time (Current, YTD), the role Readers, the
/// culture fr-FR, the parameter Base, the hierarchy Product › Products and the relationship
/// Sales → Product.
fn rich() -> Session {
    let mut s = session();
    for (id, p) in [
        ("calculation_group.add", json!({"name": "Time"})),
        (
            "calculation_item.add",
            json!({"group": "Time", "name": "Current", "expression": "SELECTEDMEASURE()"}),
        ),
        (
            "calculation_item.add",
            json!({"group": "Time", "name": "YTD", "expression": "SELECTEDMEASURE()"}),
        ),
        ("role.add", json!({"name": "Readers"})),
        ("culture.add", json!({"name": "fr-FR"})),
        (
            "expression.add",
            json!({"name": "Base", "expression": "\"x\" meta [IsParameterQuery = true, Type = \"Text\", IsParameterQueryRequired = true]"}),
        ),
        (
            "hierarchy.add",
            json!({"table": "Product", "name": "Products", "levels": ["Name", "ProductKey"]}),
        ),
        (
            "relationship.add",
            json!({"from": "Sales[ProductKey]", "to": "Product[ProductKey]"}),
        ),
    ] {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    s.dirty = false;
    s
}

fn model_view(lang: &str) -> Harness<'static, App> {
    let mut h = app_with(lang, rich);
    h.state_mut().show_tab(crate::app::Tab::Model);
    steps(&mut h, 4);
    h
}

/// A left click on the explorer's row labelled `label`: the right-most widget of that label
/// (the explorer is the right-most pane; the diagram and the Properties pane may share it).
fn row(h: &mut Harness<'static, App>, label: &str) {
    let at = h
        .get_all_by_label(label)
        .map(|n| n.rect())
        .max_by(|a, b| a.min.x.total_cmp(&b.min.x))
        .unwrap_or_else(|| panic!("no row `{label}`"));
    super::canvas_ui_tests::click_with(h, at.center(), egui::Modifiers::NONE);
    steps(h, 4);
}

/// A right click on the explorer's row `label`, then its menu item `item` (the lowest widget of
/// that label).
fn menu(h: &mut Harness<'static, App>, label: &str, item: &str) {
    let at = h
        .get_all_by_label(label)
        .map(|n| n.rect())
        .max_by(|a, b| a.min.x.total_cmp(&b.min.x))
        .unwrap_or_else(|| panic!("no row `{label}`"));
    super::backstage_tests::right_click(h, at.center());
    let at = h
        .get_all_by_label(item)
        .map(|n| n.rect())
        .max_by(|a, b| a.min.y.total_cmp(&b.min.y))
        .unwrap_or_else(|| panic!("no menu item `{item}`"));
    super::canvas_ui_tests::click_with(h, at.center(), egui::Modifiers::NONE);
    steps(h, 6);
}

fn model_tab(h: &mut Harness<'static, App>) {
    h.state_mut().model_view.explorer.tab = Tab::Model;
    steps(h, 3);
}

/// Types `text` into the Properties pane's text field `label` (its text replaced), then Enter.
fn type_field(h: &mut Harness<'static, App>, label: &str, text: &str) {
    h.get_by_role_and_label(Role::TextInput, label).click();
    steps(h, 2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.key_press(egui::Key::Backspace);
    h.get_by_role_and_label(Role::TextInput, label).type_text(text);
    steps(h, 2);
    h.key_press(egui::Key::Enter);
    steps(h, 4);
}

/// Picks `option` in the Properties pane's list `list`.
fn pick(h: &mut Harness<'static, App>, list: &str, option: &str) {
    let r = h.get_by_role_and_label(Role::ComboBox, list).rect();
    super::canvas_ui_tests::click_with(h, r.center(), egui::Modifiers::NONE);
    steps(h, 2);
    let r = h
        .get_all_by_label(option)
        .map(|n| n.rect())
        .max_by(|a, b| a.min.y.total_cmp(&b.min.y))
        .unwrap_or_else(|| panic!("no option `{option}`"));
    super::canvas_ui_tests::click_with(h, r.center(), egui::Modifiers::NONE);
    steps(h, 4);
}

fn snap(h: &Harness<'_, App>, pointer: &str) -> Value {
    h.state()
        .loaded
        .as_ref()
        .and_then(|l| l.snap.pointer(pointer).cloned())
        .unwrap_or(Value::Null)
}

#[test]
fn a_calculation_item_is_selected_and_edited_in_the_properties_pane() {
    let mut h = model_view("en-US");
    model_tab(&mut h);
    row(&mut h, "Calculation groups (1)");
    row(&mut h, "Time");
    row(&mut h, "YTD");
    assert_eq!(
        h.state().model_view.sel,
        Some(Sel::Item(Item::CalcItem {
            group: "Time".into(),
            name: "YTD".into()
        }))
    );
    // its pane, not its form
    assert!(h.state().form.is_none(), "no form opens");
    h.get_by_label("Format string expression");
    type_field(&mut h, "Description", "Year to date");
    assert_eq!(
        last_sent(&h, "calculation_item.set"),
        Some(json!({"group": "Time", "name": "YTD", "description": "Year to date"}))
    );
    let items = snap(&h, "/model/tables");
    let time = items
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "Time")
        .unwrap();
    assert_eq!(time["calculation_item_details"][1]["description"], "Year to date");
    assert!(h.state().error.is_none(), "{:?}", h.state().error);
}

#[test]
fn a_role_takes_its_permission_and_description_in_the_properties_pane() {
    let mut h = model_view("en-US");
    model_tab(&mut h);
    row(&mut h, "Roles (1)");
    row(&mut h, "Readers");
    assert_eq!(
        h.state().model_view.sel,
        Some(Sel::Item(Item::Role("Readers".into())))
    );
    pick(&mut h, "Model permission", "readRefresh");
    assert_eq!(
        last_sent(&h, "role.set"),
        Some(json!({"name": "Readers", "model_permission": "readRefresh"}))
    );
    type_field(&mut h, "Description", "Read only");
    assert_eq!(
        last_sent(&h, "role.set"),
        Some(json!({"name": "Readers", "description": "Read only"}))
    );
    let r = snap(&h, "/model/security/0");
    assert_eq!(r["model_permission"], "readRefresh");
    assert_eq!(r["description"], "Read only");
}

#[test]
fn a_shared_expression_and_a_culture_show_their_properties_in_french() {
    let mut h = model_view("fr-FR");
    model_tab(&mut h);
    row(&mut h, "Expressions partagées (1)");
    row(&mut h, "Base");
    assert_eq!(
        h.state().model_view.sel,
        Some(Sel::Item(Item::Expression("Base".into())))
    );
    h.get_by_label("Paramètre");
    type_field(&mut h, "Groupe de requêtes", "Paramètres");
    assert_eq!(
        last_sent(&h, "expression.set"),
        Some(json!({"name": "Base", "query_group": "Paramètres"}))
    );
    assert_eq!(snap(&h, "/model/expression_details/0/query_group"), "Paramètres");
    row(&mut h, "Cultures (1)");
    row(&mut h, "fr-FR");
    assert_eq!(
        h.state().model_view.sel,
        Some(Sel::Item(Item::Culture("fr-FR".into())))
    );
    h.get_by_label("Schéma linguistique");
    // the data sources' branch is there, empty
    h.get_by_label("Sources de données (0)");
}

#[test]
fn a_hierarchy_is_listed_under_its_table_and_hidden_from_the_pane() {
    let mut h = model_view("en-US");
    row(&mut h, "Product");
    row(&mut h, "Products");
    assert_eq!(
        h.state().model_view.sel,
        Some(Sel::Item(Item::Hierarchy {
            table: "Product".into(),
            name: "Products".into()
        }))
    );
    // its levels under it: ProductKey as a column of Product and as a level of Products
    assert_eq!(h.query_all_by_label("ProductKey").count(), 2);
    h.get_by_label("Name › ProductKey");
    h.get_by_label("Is hidden").click();
    steps(&mut h, 4);
    assert_eq!(
        last_sent(&h, "hierarchy.set"),
        Some(json!({"table": "Product", "name": "Products", "hidden": true}))
    );
    assert!(h.state().error.is_none(), "{:?}", h.state().error);
}

#[test]
fn the_search_box_keeps_the_matching_rows() {
    let mut h = model_view("en-US");
    let rows = |h: &Harness<'_, App>| -> Vec<Sel> { h.state().model_view.explorer.order.clone() };
    assert!(rows(&h).contains(&Sel::Table("Product".into())));
    h.get_by_label("Search the model").click();
    steps(&mut h, 2);
    h.get_by_label("Search the model").type_text("reg");
    steps(&mut h, 4);
    // Region by its name; Sales by its column Region, alone; Product and Time not at all
    assert_eq!(
        rows(&h),
        vec![
            Sel::Table("Sales".into()),
            Sel::Column {
                table: "Sales".into(),
                name: "Region".into()
            },
            Sel::Table("Region".into()),
            Sel::Column {
                table: "Region".into(),
                name: "Region".into()
            },
        ]
    );
}

#[test]
fn a_layout_tab_adds_related_tables_and_removes_one() {
    let mut h = model_view("en-US");
    crate::ribbon::invoke(
        h.state_mut(),
        "diagram.add",
        json!({"name": "Orders", "tables": ["Sales"]}),
    );
    steps(&mut h, 6);
    h.state_mut().model_view.layout = Some("Orders".into());
    steps(&mut h, 4);
    menu(&mut h, "Sales", "Add related tables");
    let p = last_sent(&h, "diagram.move").expect("Add related tables");
    assert_eq!(p["diagram"], "Orders");
    assert_eq!(p["tables"][0]["table"], "Product");
    let tables = |h: &Harness<'_, App>| -> Vec<String> {
        snap(h, "/model/diagram/layouts/1/nodes")
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(|n| n["table"].as_str().map(str::to_string))
            .collect()
    };
    assert_eq!(tables(&h), ["Sales", "Product"]);
    menu(&mut h, "Product", "Remove from layout");
    assert_eq!(
        last_sent(&h, "diagram.remove_tables"),
        Some(json!({"tables": ["Product"], "diagram": "Orders"}))
    );
    assert_eq!(tables(&h), ["Sales"]);
    assert!(h.state().error.is_none(), "{:?}", h.state().error);
}

/// A thin report's session: a report, no model, a package whose `Connections` part names a live
/// connection to a model published in the Power BI service.
fn thin() -> Session {
    let mut s = super::backstage_tests::report_session(1);
    s.model = None;
    let mut package = bicraft_opc::Package::default();
    let conn = r#"{"Version":4,"Connections":[{"Name":"EntityDataSource","ConnectionString":"Data Source=powerbi://api.powerbi.com/v1.0/myorg/Finance;Initial Catalog=Sales Model","ConnectionType":"pbiServiceXmlaStyleLive"}]}"#;
    package.put("Connections", conn.as_bytes().to_vec(), true);
    s.source = Some(bicraft_engine::Source::Package {
        path: "Thin.pbix".into(),
        package,
    });
    s
}

/// The Model view of a thin report says where its model is and that BIcraft does not open it, a
/// known limitation not pursued by the maintainers' decision (docs/gaps.md G12), in English and
/// in French.
#[test]
fn a_thin_report_says_its_model_is_published() {
    let h = app_with("en-US", thin);
    h.get_by_label_contains("published in the Power BI service");
    h.get_by_label_contains("powerbi://api.powerbi.com/v1.0/myorg/Finance");
    h.get_by_label_contains("Sales Model");
    h.get_by_label_contains("not pursued");
    let h = app_with("fr-FR", thin);
    h.get_by_label_contains("publié dans le service Power BI");
    h.get_by_label_contains("Sales Model");
}
