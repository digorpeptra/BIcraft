//! Modeling › Manage relationships through the accessibility tree (egui_kittest, no GPU): the
//! list (`relationship.list`), its Active boxes (`relationship.set {active}`), Delete
//! (`relationship.remove`), Autodetect, and the Create / Edit relationship window: tables picked
//! in the combo boxes, columns by a click on the preview's header, the suggestion taken from the
//! review (`check: true`), the validation messages, OK (`relationship.add` / `relationship.set`);
//! in English and French.

use bicraft_engine::Session;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::backstage_tests::steps;
use super::model_sec_ui_tests::{app_with, click, last_sent};
use crate::App;

/// Sales(ProductKey, Region, Amount), Product(ProductKey, Name), Region(Region), Notes(Region:
/// E twice); Sales → Product.
fn session() -> Session {
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
            "table.import",
            json!({"name": "Notes", "m": "#table(type table [Region = text, Note = text], {{\"E\", \"x\"}, {\"E\", \"y\"}})"}),
        ),
        (
            "relationship.add",
            json!({"from": "Sales[ProductKey]", "to": "Product[ProductKey]", "name": "SalesProduct"}),
        ),
        ("report.new", json!({})),
    ] {
        s.execute(id, &p).unwrap();
    }
    s.dirty = false;
    s.report_dirty = false;
    s
}

fn rels(h: &Harness<'_, App>) -> Vec<Value> {
    h.state()
        .loaded
        .as_ref()
        .and_then(|l| {
            l.snap
                .pointer("/model/relationships")
                .and_then(Value::as_array)
                .cloned()
        })
        .unwrap_or_default()
}

/// The ribbon's Manage relationships button (the lowest widget of that label is the window's
/// title once it is open; the button is the first).
fn manage(h: &mut Harness<'_, App>, label: &str) {
    let r = h
        .get_all_by_label(label)
        .map(|n| n.rect())
        .min_by(|a, b| a.min.y.total_cmp(&b.min.y))
        .unwrap_or_else(|| panic!("no `{label}`"));
    h.hover_at(r.center());
    h.step();
    click_at(h, r.center());
    steps(h, 6);
}

fn click_at(h: &mut Harness<'_, App>, pos: egui::Pos2) {
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

/// Opens combo box `combo` and picks `item` in its list (the lowest widget of that label).
fn pick(h: &mut Harness<'_, App>, combo: &str, item: &str) {
    let at = h.get_by_role_and_label(Role::ComboBox, combo).rect();
    click_at(h, at.center());
    // the list opens under (or over) the combo box: the item nearest to it
    let r = h
        .get_all_by_label(item)
        .map(|n| n.rect())
        .filter(|r| (r.min.x - at.min.x).abs() < 40.0)
        .min_by(|a, b| (a.min.y - at.max.y).abs().total_cmp(&(b.min.y - at.max.y).abs()))
        .unwrap_or_else(|| panic!("no `{item}` in {combo}"));
    click_at(h, r.center());
    steps(h, 6);
}

#[test]
fn manage_lists_the_relationships_and_toggles_active() {
    let mut h = app_with("en-US", session);
    manage(&mut h, "Manage relationships");
    assert_eq!(last_sent(&h, "relationship.list"), Some(json!({})));
    assert!(h.state().cmd.rels.is_some(), "the list opens");
    h.get_by_label("Sales (ProductKey)");
    h.get_by_label("Product (ProductKey)");
    // the Active box runs relationship.set
    click(&mut h, "Active: Sales (ProductKey) → Product (ProductKey)");
    steps(&mut h, 6);
    assert_eq!(
        last_sent(&h, "relationship.set"),
        Some(json!({"name": "SalesProduct", "active": false}))
    );
    assert_eq!(rels(&h)[0]["active"], false);
    // the list was read again at the new revision
    let lists = h
        .state()
        .sent
        .iter()
        .filter(|c| c.id == "relationship.list")
        .count();
    assert!(lists >= 2, "{lists}");
    assert!(h.state().error.is_none(), "{:?}", h.state().error);
}

#[test]
fn manage_deletes_the_selected_relationship() {
    let mut h = app_with("en-US", session);
    manage(&mut h, "Manage relationships");
    // Delete is disabled until a row is selected
    click(&mut h, "Delete");
    assert_eq!(last_sent(&h, "relationship.remove"), None);
    click(&mut h, "Sales (ProductKey)");
    click(&mut h, "Delete");
    steps(&mut h, 6);
    assert_eq!(
        last_sent(&h, "relationship.remove"),
        Some(json!({"name": "SalesProduct"}))
    );
    assert!(rels(&h).is_empty());
    h.get_by_label_contains("No relationship yet");
}

#[test]
fn manage_opens_autodetect() {
    let mut h = app_with("en-US", session);
    manage(&mut h, "Manage relationships");
    click(&mut h, "Autodetect…");
    steps(&mut h, 4);
    assert!(h.state().cmd.autodetect.is_some(), "the Autodetect dialog opens");
    assert_eq!(last_sent(&h, "relationship.autodetect"), Some(json!({})));
}

#[test]
fn new_relationship_reads_a_preview_per_table() {
    let mut h = app_with("en-US", session);
    manage(&mut h, "Manage relationships");
    click(&mut h, "New…");
    assert!(h.state().cmd.rel_edit.is_some());
    pick(&mut h, "First table", "Product");
    // the preview was read through table.rows, and shows the first rows
    assert_eq!(
        last_sent(&h, "table.rows"),
        Some(json!({"table": "Product", "limit": 5}))
    );
    h.get_by_label("Bike");
    h.get_by_label("Car");
    pick(&mut h, "Second table", "Region");
    assert_eq!(
        last_sent(&h, "table.rows"),
        Some(json!({"table": "Region", "limit": 5}))
    );
    // no column picked yet: no review, OK disabled
    h.get_by_label("Select a column in each table.");
    assert_eq!(last_sent(&h, "relationship.add"), None);
}

#[test]
fn new_relationship_columns_from_the_header_then_ok() {
    let mut h = app_with("en-US", session);
    manage(&mut h, "Manage relationships");
    click(&mut h, "New…");
    pick(&mut h, "First table", "Sales");
    pick(&mut h, "Second table", "Region");
    // the preview's first rows
    assert!(
        h.get_all_by_label("W").count() >= 2,
        "Sales and Region rows are shown"
    );
    click(&mut h, "Sales[Region]");
    click(&mut h, "Region[Region]");
    steps(&mut h, 6);
    // the review ran with check, and its suggestion (many to one, single) is taken
    let p = last_sent(&h, "relationship.add").unwrap();
    assert_eq!(p["check"], true, "{p}");
    let e = h.state().cmd.rel_edit.clone().unwrap();
    assert_eq!(e.card, crate::relationships_dialog::Card::ManyToOne);
    assert!(!e.both && e.active);
    assert_eq!(
        h.get_by_role_and_label(Role::ComboBox, "Cardinality")
            .value()
            .as_deref(),
        Some("Many to one (*:1)")
    );
    click(&mut h, "OK");
    steps(&mut h, 6);
    let p = last_sent(&h, "relationship.add").unwrap();
    assert_eq!(
        p,
        json!({"from": "Sales[Region]", "to": "Region[Region]", "from_cardinality": "many",
               "to_cardinality": "one", "cross_filter": "single"})
    );
    assert!(h.state().cmd.rel_edit.is_none(), "the window closes");
    assert_eq!(rels(&h).len(), 2);
    h.get_by_label("Sales (Region)");
}

#[test]
fn the_window_shows_the_review_and_blocks_ok() {
    let mut h = app_with("en-US", session);
    manage(&mut h, "Manage relationships");
    click(&mut h, "New…");
    pick(&mut h, "First table", "Sales");
    pick(&mut h, "Second table", "Notes");
    click(&mut h, "Sales[Region]");
    click(&mut h, "Notes[Region]");
    steps(&mut h, 6);
    // neither column is unique: the suggestion is many to many, with its warning
    let e = h.state().cmd.rel_edit.clone().unwrap();
    assert_eq!(e.card, crate::relationships_dialog::Card::ManyToMany);
    h.get_by_label_contains("Warning: a many-to-many relationship");
    // many to one is refused: Notes[Region] has duplicates
    pick(&mut h, "Cardinality", "Many to one (*:1)");
    steps(&mut h, 6);
    h.get_by_label_contains("Error: one of the columns must have unique values");
    let before = rels(&h).len();
    click(&mut h, "OK");
    steps(&mut h, 4);
    assert_eq!(rels(&h).len(), before, "OK is disabled");
    // a second relationship Sales → Product cannot be active
    click(&mut h, "Cancel");
    click(&mut h, "New…");
    pick(&mut h, "First table", "Sales");
    pick(&mut h, "Second table", "Product");
    click(&mut h, "Sales[Amount]");
    click(&mut h, "Product[ProductKey]");
    steps(&mut h, 6);
    let e = h.state().cmd.rel_edit.clone().unwrap();
    assert!(!e.active, "made inactive: Sales → Product is already active");
    h.get_by_label_contains("Warning: the relationship is made inactive");
}

#[test]
fn edit_moves_an_end_and_sets_the_flags() {
    let mut h = app_with("en-US", session);
    manage(&mut h, "Manage relationships");
    click(&mut h, "Sales (ProductKey)");
    click(&mut h, "Edit…");
    let e = h.state().cmd.rel_edit.clone().unwrap();
    assert_eq!(e.sides[1].table, "Product");
    assert!(
        h.get_all_by_label("Edit relationship").count() >= 2,
        "the window's title"
    );
    pick(&mut h, "Cross-filter direction", "Both");
    click(&mut h, "Apply security filter in both directions");
    steps(&mut h, 6);
    click(&mut h, "OK");
    steps(&mut h, 6);
    let p = last_sent(&h, "relationship.set").unwrap();
    assert_eq!(
        p,
        json!({"name": "SalesProduct", "new_from": "Sales[ProductKey]", "new_to": "Product[ProductKey]",
               "security_filtering": "both", "from_cardinality": "many", "to_cardinality": "one",
               "cross_filter": "both", "active": true})
    );
    let r = &rels(&h)[0];
    assert_eq!(
        (r["cross_filter"].as_str(), r["security_filtering"].as_str()),
        (Some("both"), Some("both"))
    );
    assert!(h.state().error.is_none(), "{:?}", h.state().error);
}

#[test]
fn a_double_click_on_a_line_opens_the_edit_window() {
    let mut h = app_with("en-US", session);
    // the model view's double click hands relationship.set's values to the app
    let r = rels(&h)[0].clone();
    h.state_mut().model_view.form = Some(("relationship.set", crate::model_view::rel_values(&r)));
    steps(&mut h, 4);
    assert!(h.state().cmd.rel_edit.as_ref().is_some_and(|e| e.old.is_some()));
    assert!(h.state().form.is_none(), "no generic form");
}

#[test]
fn manage_relationships_speaks_french() {
    let mut h = app_with("fr-FR", session);
    manage(&mut h, "Gérer les relations");
    h.get_by_label("De : Table (Colonne)");
    h.get_by_label("Vers : Table (Colonne)");
    click(&mut h, "Nouvelle…");
    assert!(
        h.get_all_by_label("Nouvelle relation").count() >= 2,
        "the window's title"
    );
    pick(&mut h, "Première table", "Sales");
    pick(&mut h, "Deuxième table", "Region");
    click(&mut h, "Sales[Region]");
    click(&mut h, "Region[Region]");
    steps(&mut h, 6);
    assert_eq!(
        h.get_by_role_and_label(Role::ComboBox, "Cardinalité")
            .value()
            .as_deref(),
        Some("Plusieurs à un (*:1)")
    );
    h.get_by_label("Rendre cette relation active");
    click(&mut h, "OK");
    steps(&mut h, 6);
    assert_eq!(rels(&h).len(), 2);
}
