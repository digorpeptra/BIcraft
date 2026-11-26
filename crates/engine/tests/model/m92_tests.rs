//! Tests of the M9.2 authoring commands: `relationship.set`, calculation groups and items, date
//! tables (`table.set {data_category, key_column}`) and `hierarchy.set`, on PBIP (TMSL and TMDL)
//! and PBIX.

use std::path::Path;

use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Runs a command that must fail, checks the model did not change, and returns the error.
fn fails(s: &mut Session, id: &str, params: Value) -> String {
    let before = s.model.clone();
    let err = s
        .execute(id, &params)
        .expect_err(&format!("{id} {params} should fail"));
    assert_eq!(s.model, before, "{id} changed the model although it failed");
    err
}

fn open(path: &Path) -> Session {
    let mut s = Session::new();
    run(&mut s, "file.open", json!({ "path": path }));
    assert_eq!(s.model_error, None);
    s
}

/// Sales(Key, Product, Date, Amount), Product(Product, Name, Category), Category(Category),
/// Dates(Date): every table with data (CSV literals), Sales → Product and Sales → Dates active.
fn shop() -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Shop"}));
    for (name, csv) in [
        (
            "Sales",
            "Key,Product,Date,Amount\n1,a,2026-01-01,10\n2,b,2026-01-02,20\n3,a,2026-01-03,5\n",
        ),
        (
            "Product",
            "Product,Name,Category\na,Apple,Fruit\nb,Bread,Bakery\n",
        ),
        ("Category", "Category\nFruit\nBakery\n"),
        (
            "Dates",
            "Date,Month\n2026-01-01,Jan\n2026-01-02,Jan\n2026-01-03,Jan\n",
        ),
        ("Gaps", "Date\n2026-01-01\n2026-01-03\n"),
    ] {
        run(&mut s, "table.import", json!({"name": name, "csv": csv}));
    }
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
    );
    run(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[Product]", "to": "Product[Product]"}),
    );
    run(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[Date]", "to": "Dates[Date]"}),
    );
    s
}

fn rel<'s>(s: &'s Session, from: &str) -> &'s bicraft_model::Relationship {
    s.model
        .as_ref()
        .unwrap()
        .relationships
        .iter()
        .find(|r| format!("{}[{}]", r.from_table, r.from_column) == from)
        .unwrap()
}

// --- relationship.set -------------------------------------------------------------------------

#[test]
fn relationship_set_changes_in_place_and_keeps_the_name() {
    let mut s = shop();
    let name = rel(&s, "Sales[Product]").name.clone();
    let count = s.model.as_ref().unwrap().relationships.len();
    let v = run(
        &mut s,
        "relationship.set",
        json!({"name": name, "cross_filter": "both", "security_filtering": "both"}),
    );
    assert_eq!(v["name"], name.as_str());
    assert_eq!(v["cross_filter"], "both");
    assert_eq!(v["changed"], true);
    let r = rel(&s, "Sales[Product]");
    assert_eq!(r.name, name);
    assert_eq!(r.cross_filtering_behavior.as_deref(), Some("bothDirections"));
    assert_eq!(r.extra["securityFilteringBehavior"], "bothDirections");
    assert_eq!(s.model.as_ref().unwrap().relationships.len(), count);
    // By ends, back to single (security both then needs both: refused first).
    let err = fails(
        &mut s,
        "relationship.set",
        json!({"from": "Sales[Product]", "to": "Product[Product]", "cross_filter": "single"}),
    );
    assert!(err.contains("security filtering in both directions"), "{err}");
    run(
        &mut s,
        "relationship.set",
        json!({"from": "Sales[Product]", "to": "Product[Product]", "cross_filter": "single",
               "security_filtering": "single"}),
    );
    let r = rel(&s, "Sales[Product]");
    assert_eq!(r.cross_filtering_behavior, None);
    assert!(!r.extra.contains_key("securityFilteringBehavior"));
    // Deactivate, then undo restores it.
    run(&mut s, "relationship.set", json!({"name": name, "active": false}));
    assert!(!rel(&s, "Sales[Product]").is_active);
    run(&mut s, "edit.undo", json!({}));
    assert!(rel(&s, "Sales[Product]").is_active);
}

#[test]
fn relationship_set_refuses_what_desktop_refuses() {
    let mut s = shop();
    let name = rel(&s, "Sales[Product]").name.clone();
    // Nothing to change, unknown values, unknown relationship.
    fails(&mut s, "relationship.set", json!({"name": name}));
    fails(
        &mut s,
        "relationship.set",
        json!({"name": name, "cross_filter": "sideways"}),
    );
    fails(
        &mut s,
        "relationship.set",
        json!({"name": "nope", "active": true}),
    );
    // One-to-many is the relationship with its ends swapped.
    let err = fails(
        &mut s,
        "relationship.set",
        json!({"name": name, "from_cardinality": "one", "to_cardinality": "many"}),
    );
    assert!(err.contains("swap the ends"), "{err}");
    // A second active relationship between the same tables.
    run(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[Key]", "to": "Product[Name]", "active": false}),
    );
    let err = fails(
        &mut s,
        "relationship.set",
        json!({"from": "Sales[Key]", "to": "Product[Name]", "active": true}),
    );
    assert!(err.contains("already the active one"), "{err}");
    // An ambiguous path: Category filters Sales directly and through Product.
    run(
        &mut s,
        "relationship.add",
        json!({"from": "Product[Category]", "to": "Category[Category]"}),
    );
    run(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[Product]", "to": "Category[Category]", "active": false}),
    );
    let err = fails(
        &mut s,
        "relationship.set",
        json!({"from": "Sales[Product]", "to": "Category[Category]", "active": true}),
    );
    assert!(err.contains("ambiguous"), "{err}");
}

#[test]
fn relationship_set_checks_a_new_one_side_against_the_data() {
    let mut s = shop();
    // Sales[Product] has "a" twice: it cannot become a "one" side.
    let err = fails(
        &mut s,
        "relationship.set",
        json!({"from": "Sales[Product]", "to": "Product[Product]", "from_cardinality": "one"}),
    );
    assert!(err.contains("duplicate"), "{err}");
    // Sales[Date] is unique: one-to-one is accepted, checked against the data.
    let v = run(
        &mut s,
        "relationship.set",
        json!({"from": "Sales[Date]", "to": "Dates[Date]", "from_cardinality": "one", "cross_filter": "both"}),
    );
    assert_eq!(v["data_checked"], json!(["Sales[Date]"]));
    assert_eq!(rel(&s, "Sales[Date]").from_cardinality.as_deref(), Some("one"));
    // Many-to-many needs no unique side.
    run(
        &mut s,
        "relationship.set",
        json!({"from": "Sales[Product]", "to": "Product[Product]", "to_cardinality": "many"}),
    );
    assert_eq!(rel(&s, "Sales[Product]").to_cardinality.as_deref(), Some("many"));
}

/// Regression: a one-to-one relationship with single-direction cross filtering was accepted, and
/// Power BI Desktop 2.158 then refused to open the PBIX or PBIP. One-to-one is both directions.
#[test]
fn one_to_one_relationships_cross_filter_both_ways() {
    let mut s = shop();
    let one_to_one = json!({"from": "Gaps[Date]", "to": "Dates[Date]",
                            "from_cardinality": "one", "to_cardinality": "one"});
    let mut single = one_to_one.clone();
    single["cross_filter"] = json!("single");
    let err = fails(&mut s, "relationship.add", single);
    assert!(err.contains("both directions"), "{err}");
    let v = run(&mut s, "relationship.add", one_to_one);
    assert!(v.to_string().contains("both directions"), "{v}");
    let r = rel(&s, "Gaps[Date]");
    assert_eq!(r.cross_filtering_behavior.as_deref(), Some("bothDirections"));
    let err = fails(
        &mut s,
        "relationship.set",
        json!({"from": "Gaps[Date]", "to": "Dates[Date]", "cross_filter": "single"}),
    );
    assert!(err.contains("both directions"), "{err}");
    // Making a single-direction relationship one-to-one turns it both ways.
    let v = run(
        &mut s,
        "relationship.set",
        json!({"from": "Sales[Date]", "to": "Dates[Date]", "from_cardinality": "one", "active": false}),
    );
    assert_eq!(v["cross_filter"], "both", "{v}");
    let err = fails(
        &mut s,
        "relationship.set",
        json!({"from": "Sales[Date]", "to": "Dates[Date]", "from_cardinality": "one", "cross_filter": "single"}),
    );
    assert!(err.contains("both directions"), "{err}");
    // model.validate flags one in an existing file, and relationship.set repairs it.
    assert_eq!(run(&mut s, "model.validate", json!({}))["valid"], true);
    if let Some(r) = s
        .model
        .as_mut()
        .unwrap()
        .relationships
        .iter_mut()
        .find(|r| r.from_table == "Gaps")
    {
        r.cross_filtering_behavior = None;
    }
    let v = run(&mut s, "model.validate", json!({}));
    assert_eq!(v["valid"], false, "{v}");
    assert_eq!(v["relationships"]["valid"], false, "{v}");
    assert_eq!(v["relationships"]["invalid"][0]["from"], "Gaps[Date]", "{v}");
    run(
        &mut s,
        "relationship.set",
        json!({"from": "Gaps[Date]", "to": "Dates[Date]", "active": true}),
    );
    let v = run(&mut s, "model.validate", json!({}));
    assert_eq!(v["relationships"]["valid"], true, "{v}");
}

// --- calculation groups -----------------------------------------------------------------------

fn add_group(s: &mut Session) {
    let v = run(
        s,
        "calculation_group.add",
        json!({"name": "Time Calc", "column": "Calculation"}),
    );
    assert_eq!(v["precedence"], 0);
    run(
        s,
        "calculation_item.add",
        json!({"group": "Time Calc", "name": "Current", "expression": "SELECTEDMEASURE()"}),
    );
    run(
        s,
        "calculation_item.add",
        json!({"group": "Time Calc", "name": "Double", "expression": "SELECTEDMEASURE() * 2",
               "format_string_expression": "\"0.00\""}),
    );
}

fn items(s: &Session, group: &str) -> Vec<(String, i64)> {
    s.model.as_ref().unwrap().table(group).unwrap().extra["calculationGroup"]["calculationItems"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            (
                i["name"].as_str().unwrap().to_string(),
                i["ordinal"].as_i64().unwrap(),
            )
        })
        .collect()
}

#[test]
fn calculation_groups_and_items() {
    let mut s = shop();
    add_group(&mut s);
    let m = s.model.as_ref().unwrap();
    assert_eq!(m.extra["discourageImplicitMeasures"], true);
    let t = m.table("Time Calc").unwrap();
    assert_eq!(t.partitions[0].source_type, "calculationGroup");
    assert_eq!(t.columns[0].name, "Calculation");
    assert_eq!(t.columns[0].source_column.as_deref(), Some("Name"));
    assert_eq!(t.columns[0].sort_by_column.as_deref(), Some("Ordinal"));
    assert!(t.columns[1].is_hidden);
    assert_eq!(
        items(&s, "Time Calc"),
        vec![("Current".to_string(), 0), ("Double".to_string(), 1)]
    );
    // Insert at a position: later items move down.
    run(
        &mut s,
        "calculation_item.add",
        json!({"group": "Time Calc", "name": "Half", "expression": "SELECTEDMEASURE() / 2", "ordinal": 0}),
    );
    assert_eq!(
        items(&s, "Time Calc"),
        vec![
            ("Half".to_string(), 0),
            ("Current".to_string(), 1),
            ("Double".to_string(), 2)
        ]
    );
    // Move, rename, change the expression and the format string.
    run(
        &mut s,
        "calculation_item.set",
        json!({"group": "Time Calc", "name": "Half", "ordinal": 2, "new_name": "Halved",
               "expression": "DIVIDE(SELECTEDMEASURE(), 2)", "format_string_expression": "SELECTEDMEASUREFORMATSTRING()"}),
    );
    assert_eq!(
        items(&s, "Time Calc"),
        vec![
            ("Current".to_string(), 0),
            ("Double".to_string(), 1),
            ("Halved".to_string(), 2)
        ]
    );
    // Invalid DAX is refused, duplicate names too.
    let err = fails(
        &mut s,
        "calculation_item.add",
        json!({"group": "Time Calc", "name": "Bad", "expression": "SUM(Nope[x])"}),
    );
    assert!(err.contains("invalid DAX"), "{err}");
    fails(
        &mut s,
        "calculation_item.add",
        json!({"group": "Time Calc", "name": "current", "expression": "1"}),
    );
    fails(
        &mut s,
        "calculation_item.add",
        json!({"group": "Sales", "name": "X", "expression": "1"}),
    );
    // A second group needs another precedence.
    let err = fails(
        &mut s,
        "calculation_group.add",
        json!({"name": "Other", "precedence": 0}),
    );
    assert!(err.contains("precedence"), "{err}");
    let v = run(&mut s, "calculation_group.add", json!({"name": "Other"}));
    assert_eq!(v["precedence"], 1);
    run(
        &mut s,
        "calculation_group.set",
        json!({"name": "Other", "precedence": 5, "no_selection_expression": "SELECTEDMEASURE()"}),
    );
    // model.validate covers the items.
    let v = run(&mut s, "model.validate", json!({}));
    assert_eq!(v["valid"], true, "{v}");
    run(
        &mut s,
        "calculation_item.remove",
        json!({"group": "Time Calc", "name": "Double"}),
    );
    assert_eq!(items(&s, "Time Calc").len(), 2);
    run(&mut s, "calculation_group.remove", json!({"name": "Other"}));
    assert!(s.model.as_ref().unwrap().table("Other").is_none());
    fails(&mut s, "calculation_group.remove", json!({"name": "Sales"}));
}

fn assert_group_round_trips(format: &str) {
    let dir = TempDir::new(&format!("m92-cg-{format}"));
    let mut s = shop();
    add_group(&mut s);
    run(
        &mut s,
        "project.new",
        json!({"dir": dir.path(), "name": "Shop", "format": format}),
    );
    let s2 = open(&dir.join("Shop.pbip"));
    let m = s2.model.as_ref().unwrap();
    assert_eq!(m.extra["discourageImplicitMeasures"], true);
    assert_eq!(
        items(&s2, "Time Calc"),
        vec![("Current".to_string(), 0), ("Double".to_string(), 1)]
    );
    assert_eq!(
        m.table("Time Calc").unwrap().extra["calculationGroup"]["calculationItems"][1]["formatStringDefinition"]
            ["expression"],
        "\"0.00\""
    );
    assert_eq!(
        m.table("Time Calc").unwrap().partitions[0].source_type,
        "calculationGroup"
    );
    // Edit the reopened project and save it again.
    let mut s3 = open(&dir.join("Shop.pbip"));
    run(
        &mut s3,
        "calculation_item.set",
        json!({"group": "Time Calc", "name": "Double", "expression": "SELECTEDMEASURE() * 3"}),
    );
    run(&mut s3, "file.save", json!({}));
    let s4 = open(&dir.join("Shop.pbip"));
    assert_eq!(
        s4.model.as_ref().unwrap().table("Time Calc").unwrap().extra["calculationGroup"]["calculationItems"]
            [1]["expression"],
        "SELECTEDMEASURE() * 3"
    );
}

#[test]
fn calculation_groups_round_trip_through_pbip_tmsl() {
    assert_group_round_trips("tmsl");
}

#[test]
fn calculation_groups_round_trip_through_pbip_tmdl() {
    assert_group_round_trips("tmdl");
}

// --- date tables ------------------------------------------------------------------------------

#[test]
fn mark_as_date_table() {
    let mut s = shop();
    let v = run(
        &mut s,
        "table.set",
        json!({"name": "Dates", "data_category": "time", "key_column": "date"}),
    );
    assert_eq!(v["data_category"], "Time");
    assert_eq!(v["key_column"], "Date");
    assert_eq!(v["data_checked"], "Dates[Date]");
    let t = s.model.as_ref().unwrap().table("Dates").unwrap();
    assert_eq!(t.extra["dataCategory"], "Time");
    assert_eq!(t.columns[0].extra["isKey"], true);
    // Not a date column, gaps, no key column.
    let err = fails(
        &mut s,
        "table.set",
        json!({"name": "Dates", "data_category": "Time", "key_column": "Month"}),
    );
    assert!(err.contains("not a date column"), "{err}");
    let err = fails(
        &mut s,
        "table.set",
        json!({"name": "Gaps", "data_category": "Time", "key_column": "Date"}),
    );
    assert!(err.contains("not contiguous"), "{err}");
    let err = fails(
        &mut s,
        "table.set",
        json!({"name": "Gaps", "data_category": "Time"}),
    );
    assert!(err.contains("key_column"), "{err}");
    // Unmark: the key goes with the category.
    run(
        &mut s,
        "table.set",
        json!({"name": "Dates", "data_category": null}),
    );
    let t = s.model.as_ref().unwrap().table("Dates").unwrap();
    assert!(!t.extra.contains_key("dataCategory"));
    assert!(!t.columns[0].extra.contains_key("isKey"));
}

// --- hierarchy.set ----------------------------------------------------------------------------

#[test]
fn hierarchy_set_renames_reorders_and_hides() {
    let mut s = shop();
    run(
        &mut s,
        "hierarchy.add",
        json!({"table": "Product", "name": "Catalog", "levels": ["Category", "Name"]}),
    );
    let before = s.model.as_ref().unwrap().table("Product").unwrap().extra["hierarchies"][0]["levels"]
        [0]["lineageTag"]
        .clone();
    let v = run(
        &mut s,
        "hierarchy.set",
        json!({"table": "Product", "name": "catalog", "new_name": "Products", "hidden": true,
               "levels": ["Name", {"column": "Category", "name": "Group"}, "Product"]}),
    );
    assert_eq!(v["hierarchy"], "Products");
    let h = &s.model.as_ref().unwrap().table("Product").unwrap().extra["hierarchies"][0];
    assert_eq!(h["name"], "Products");
    assert_eq!(h["isHidden"], true);
    let levels: Vec<(&str, &str, i64)> = h["levels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| {
            (
                l["name"].as_str().unwrap(),
                l["column"].as_str().unwrap(),
                l["ordinal"].as_i64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        levels,
        vec![
            ("Name", "Name", 0),
            ("Group", "Category", 1),
            ("Product", "Product", 2)
        ]
    );
    // The kept level kept its lineage tag.
    assert_eq!(h["levels"][1]["lineageTag"], before);
    run(
        &mut s,
        "hierarchy.set",
        json!({"table": "Product", "name": "Products", "rename_levels": {"group": "Category group"}, "hidden": false}),
    );
    let h = &s.model.as_ref().unwrap().table("Product").unwrap().extra["hierarchies"][0];
    assert_eq!(h["levels"][1]["name"], "Category group");
    assert!(h.get("isHidden").is_none());
    fails(
        &mut s,
        "hierarchy.set",
        json!({"table": "Product", "name": "Products", "levels": ["Name", "Name"]}),
    );
    fails(
        &mut s,
        "hierarchy.set",
        json!({"table": "Product", "name": "Nope", "hidden": true}),
    );
    fails(
        &mut s,
        "hierarchy.set",
        json!({"table": "Product", "name": "Products"}),
    );
}

// --- PBIX -------------------------------------------------------------------------------------

/// The shop saved as a PBIX (built from scratch), reopened.
fn shop_pbix(dir: &TempDir) -> std::path::PathBuf {
    let path = dir.join("Shop.pbix");
    let mut s = shop();
    run(&mut s, "file.save_as", json!({"path": path}));
    path
}

#[test]
fn pbix_relationship_set_round_trips() {
    let dir = TempDir::new("m92-pbix-rel");
    let path = shop_pbix(&dir);
    let mut s = open(&path);
    let name = rel(&s, "Sales[Product]").name.clone();
    run(
        &mut s,
        "relationship.set",
        json!({"name": name, "cross_filter": "both", "active": false}),
    );
    run(&mut s, "file.save", json!({"backup": false}));
    let s2 = open(&path);
    let r = rel(&s2, "Sales[Product]");
    assert_eq!(r.name, name);
    assert!(!r.is_active);
    assert_eq!(r.cross_filtering_behavior.as_deref(), Some("bothDirections"));

    // A cardinality change rebuilds the storage under the same name.
    let mut s3 = open(&path);
    let date = rel(&s3, "Sales[Date]").name.clone();
    run(
        &mut s3,
        "relationship.set",
        json!({"name": date, "from_cardinality": "one", "cross_filter": "both"}),
    );
    run(&mut s3, "file.save", json!({"backup": false}));
    let s4 = open(&path);
    let r = rel(&s4, "Sales[Date]");
    assert_eq!(r.name, date);
    assert_eq!(r.from_cardinality.as_deref(), Some("one"));
}

#[test]
fn pbix_date_table_and_hierarchy_set_round_trip() {
    let dir = TempDir::new("m92-pbix-date");
    let path = shop_pbix(&dir);
    let mut s = open(&path);
    let v = run(
        &mut s,
        "table.set",
        json!({"name": "Dates", "data_category": "Time", "key_column": "Date"}),
    );
    // Checked against the stored data.
    assert_eq!(v["data_checked"], "Dates[Date]");
    run(
        &mut s,
        "hierarchy.add",
        json!({"table": "Product", "name": "Catalog", "levels": ["Category", "Name"]}),
    );
    run(&mut s, "file.save", json!({"backup": false}));
    let mut s2 = open(&path);
    let t = s2.model.as_ref().unwrap().table("Dates").unwrap();
    assert_eq!(t.extra["dataCategory"], "Time");
    assert_eq!(t.column("Date").unwrap().extra["isKey"], true);
    run(
        &mut s2,
        "hierarchy.set",
        json!({"table": "Product", "name": "Catalog", "new_name": "Products", "hidden": true,
               "rename_levels": {"Name": "Item"}}),
    );
    run(&mut s2, "file.save", json!({"backup": false}));
    let s3 = open(&path);
    let h = &s3.model.as_ref().unwrap().table("Product").unwrap().extra["hierarchies"][0];
    assert_eq!(h["name"], "Products");
    assert_eq!(h["isHidden"], true);
    assert_eq!(h["levels"][1]["name"], "Item");
}

#[test]
fn pbix_calculation_groups_round_trip() {
    let dir = TempDir::new("m92-pbix-cg");
    let path = shop_pbix(&dir);
    let mut s = open(&path);
    add_group(&mut s);
    run(&mut s, "file.save", json!({"backup": false}));
    let mut s2 = open(&path);
    assert_eq!(
        items(&s2, "Time Calc"),
        vec![("Current".to_string(), 0), ("Double".to_string(), 1)]
    );
    assert_eq!(
        s2.model.as_ref().unwrap().extra["discourageImplicitMeasures"],
        true
    );
    // Items added, changed and removed on the saved group.
    run(
        &mut s2,
        "calculation_item.add",
        json!({"group": "Time Calc", "name": "Half", "expression": "SELECTEDMEASURE() / 2"}),
    );
    run(
        &mut s2,
        "calculation_item.set",
        json!({"group": "Time Calc", "name": "Current", "expression": "SELECTEDMEASURE() + 0"}),
    );
    run(
        &mut s2,
        "calculation_item.remove",
        json!({"group": "Time Calc", "name": "Double"}),
    );
    run(&mut s2, "file.save", json!({"backup": false}));
    let s3 = open(&path);
    let names: Vec<String> = items(&s3, "Time Calc").into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, vec!["Current", "Half"]);
}

#[test]
fn relationship_names_are_given_and_changed() {
    let mut s = shop();
    let v = run(
        &mut s,
        "relationship.add",
        json!({"from": "Product[Category]", "to": "Category[Category]", "name": "Product Category"}),
    );
    assert_eq!(v["name"], "Product Category");
    assert_eq!(rel(&s, "Product[Category]").name, "Product Category");
    // names are unique (case-insensitive) and checked like other names
    let e = fails(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[Date]", "to": "Gaps[Date]", "active": false, "name": "product category"}),
    );
    assert!(e.contains("already exists"), "{e}");
    let e = fails(
        &mut s,
        "relationship.set",
        json!({"name": "Product Category", "new_name": " padded"}),
    );
    assert!(e.contains("whitespace"), "{e}");
    let other = rel(&s, "Sales[Product]").name.clone();
    let e = fails(
        &mut s,
        "relationship.set",
        json!({"name": other, "new_name": "PRODUCT CATEGORY"}),
    );
    assert!(e.contains("already exists"), "{e}");
    // a rename alone, by ends; everything else and the position are kept
    let before = s.model.as_ref().unwrap().relationships.clone();
    let v = run(
        &mut s,
        "relationship.set",
        json!({"from": "Sales[Product]", "to": "Product[Product]", "new_name": "Sales Product"}),
    );
    assert_eq!(v["renamed"], other.as_str());
    let after = &s.model.as_ref().unwrap().relationships;
    assert_eq!(after.len(), before.len());
    for (a, b) in before.iter().zip(after) {
        if a.name == other {
            assert_eq!(b.name, "Sales Product");
            assert_eq!(
                bicraft_model::Relationship {
                    name: a.name.clone(),
                    ..b.clone()
                },
                *a
            );
        } else {
            assert_eq!(a, b);
        }
    }
    // a case-only rename of itself is allowed
    run(
        &mut s,
        "relationship.set",
        json!({"name": "Sales Product", "new_name": "sales product"}),
    );
    assert_eq!(rel(&s, "Sales[Product]").name, "sales product");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(rel(&s, "Sales[Product]").name, "Sales Product");
    // PBIP round trips keep the names
    for format in ["tmsl", "tmdl"] {
        let dir = TempDir::new(&format!("m92-rel-names-{format}"));
        run(
            &mut s,
            "project.new",
            json!({"dir": dir.path(), "name": "Shop", "format": format}),
        );
        let back = open(&dir.join("Shop.pbip"));
        assert_eq!(rel(&back, "Sales[Product]").name, "Sales Product", "{format}");
        assert_eq!(
            rel(&back, "Product[Category]").name,
            "Product Category",
            "{format}"
        );
    }
}
