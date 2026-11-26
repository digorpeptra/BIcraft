//! The Create and Edit relationship dialogs' review (`relationship.add` / `relationship.set` with
//! `check: true`, and the same rules without it): the cardinality the data suggests, a "one" side
//! with duplicates, no unique side, the many-to-many warning, a second active relationship or an
//! ambiguous path made inactive (refused with `active: true`), "Assume referential integrity" on
//! DirectQuery tables only, an edit that moves an end; TMDL, TMSL and PBIX keep what they write.

use std::path::Path;

use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

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

/// Sales(Key, Product, Region, Amount), Product(Product, Name, Region), Region(Region),
/// Twin(Key): every table with data; no relationship.
fn shop() -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Shop"}));
    for (name, csv) in [
        (
            "Sales",
            "Key,Product,Region,Amount\n1,a,E,10\n2,b,W,20\n3,a,E,5\n",
        ),
        ("Product", "Product,Name,Region\na,Apple,E\nb,Bread,E\n"),
        ("Region", "Region\nE\nW\n"),
        ("Twin", "Key\n1\n2\n3\n"),
    ] {
        run(&mut s, "table.import", json!({"name": name, "csv": csv}));
    }
    s
}

fn codes(v: &Value, k: &str) -> Vec<String> {
    v[k].as_array()
        .into_iter()
        .flatten()
        .map(|f| f["code"].as_str().unwrap_or_default().to_string())
        .collect()
}

fn rels(s: &Session) -> &[bicraft_model::Relationship] {
    &s.model.as_ref().unwrap().relationships
}

#[test]
fn the_check_suggests_from_the_data_and_changes_nothing() {
    let mut s = shop();
    let before = s.model.clone();
    let history = run(&mut s, "edit.history", json!({}));
    s.dirty = false;
    let revision = s.revision;
    let v = run(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[Product]", "to": "Product[Product]", "check": true}),
    );
    assert_eq!(v["valid"], true, "{v}");
    assert_eq!(v["from_unique"], false);
    assert_eq!(v["to_unique"], true);
    assert_eq!(
        v["suggested"],
        json!({"cardinality": "many_to_one", "cross_filter": "single"})
    );
    assert_eq!(v["can_be_active"], true);
    // both unique: one-to-one, both directions
    let v = run(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[Key]", "to": "Twin[Key]", "check": true}),
    );
    assert_eq!(
        v["suggested"],
        json!({"cardinality": "one_to_one", "cross_filter": "both"})
    );
    // the unique side first: one-to-many
    let v = run(
        &mut s,
        "relationship.add",
        json!({"from": "Product[Product]", "to": "Sales[Product]", "check": true}),
    );
    assert_eq!(v["suggested"]["cardinality"], "one_to_many");
    assert_eq!(v["valid"], false, "the many side as `to` of a many-to-one: {v}");
    assert_eq!(codes(&v, "errors"), ["not_unique"]);
    assert_eq!(s.model, before, "a check changes nothing");
    assert_eq!(run(&mut s, "edit.history", json!({})), history);
    assert_eq!(
        s.revision, revision,
        "no new revision: the previews keep their caches"
    );
    assert!(!s.dirty);
}

#[test]
fn a_one_side_must_be_unique_unless_many_to_many() {
    let mut s = shop();
    // Product[Region] holds E twice: it cannot be the "one" side
    let err = fails(
        &mut s,
        "relationship.add",
        json!({"from": "Region[Region]", "to": "Product[Region]", "active": false}),
    );
    assert!(err.contains("duplicate"), "{err}");
    // neither Sales[Region] nor Product[Region] is unique
    let v = run(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[Region]", "to": "Product[Region]", "check": true}),
    );
    assert_eq!(codes(&v, "errors"), ["no_unique_side"], "{v}");
    assert_eq!(v["suggested"]["cardinality"], "many_to_many");
    let err = fails(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[Region]", "to": "Product[Region]"}),
    );
    assert!(
        err.contains("one of the columns must have unique values"),
        "{err}"
    );
    // many-to-many is allowed, with its warning
    let p = json!({"from": "Sales[Region]", "to": "Product[Region]",
                   "from_cardinality": "many", "to_cardinality": "many"});
    let mut check = p.clone();
    check["check"] = json!(true);
    let v = run(&mut s, "relationship.add", check);
    assert_eq!(v["valid"], true, "{v}");
    assert_eq!(codes(&v, "warnings"), ["many_to_many"]);
    let v = run(&mut s, "relationship.add", p);
    assert!(v.to_string().contains("many-to-many"), "{v}");
    assert_eq!(rels(&s).len(), 1);
}

#[test]
fn a_second_active_path_is_made_inactive_or_refused() {
    let mut s = shop();
    run(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[Product]", "to": "Product[Product]"}),
    );
    run(
        &mut s,
        "relationship.add",
        json!({"from": "Product[Region]", "to": "Region[Region]"}),
    );
    // Sales → Region directly makes Region filter Sales by two paths
    let v = run(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[Region]", "to": "Region[Region]", "check": true}),
    );
    assert_eq!(v["can_be_active"], false, "{v}");
    assert_eq!(v["active"], false);
    assert_eq!(codes(&v, "warnings"), ["made_inactive"]);
    let err = fails(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[Region]", "to": "Region[Region]", "active": true}),
    );
    assert!(err.contains("ambiguous"), "{err}");
    let v = run(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[Region]", "to": "Region[Region]"}),
    );
    assert_eq!(v["active"], false, "{v}");
    assert!(v.to_string().contains("made inactive"), "{v}");
    // a second relationship between the same two tables
    let v = run(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[Key]", "to": "Product[Name]", "check": true}),
    );
    assert_eq!(v["can_be_active"], false);
    let err = fails(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[Key]", "to": "Product[Name]", "active": true}),
    );
    assert!(err.contains("already the active one"), "{err}");
    // the inactive Sales → Region made active by an edit: refused; its check says why
    let v = run(
        &mut s,
        "relationship.set",
        json!({"from": "Sales[Region]", "to": "Region[Region]", "active": true, "check": true}),
    );
    assert_eq!(codes(&v, "errors"), ["ambiguous"], "{v}");
    let err = fails(
        &mut s,
        "relationship.set",
        json!({"from": "Sales[Region]", "to": "Region[Region]", "active": true}),
    );
    assert!(err.contains("ambiguous"), "{err}");
    // once Product → Region is inactive, it can be
    run(
        &mut s,
        "relationship.set",
        json!({"from": "Product[Region]", "to": "Region[Region]", "active": false}),
    );
    run(
        &mut s,
        "relationship.set",
        json!({"from": "Sales[Region]", "to": "Region[Region]", "active": true}),
    );
}

#[test]
fn security_filter_and_referential_integrity_rules() {
    let mut s = shop();
    let err = fails(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[Product]", "to": "Product[Product]", "security_filtering": "both"}),
    );
    assert!(err.contains("cross filtering in both directions"), "{err}");
    let err = fails(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[Product]", "to": "Product[Product]", "assume_referential_integrity": true}),
    );
    assert!(err.contains("DirectQuery"), "{err}");
    run(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[Product]", "to": "Product[Product]", "cross_filter": "both",
               "security_filtering": "both"}),
    );
    let r = &rels(&s)[0];
    assert_eq!(r.extra["securityFilteringBehavior"], "bothDirections");
    // hostile values are errors
    for p in [
        json!({"from": "Sales[Key]", "to": "Twin[Key]", "check": "yes"}),
        json!({"from": "Sales[Key]", "to": "Twin[Key]", "assume_referential_integrity": 1}),
        json!({"from": "Sales[Key]", "to": "Twin[Key]", "security_filtering": "up"}),
        json!({"from": "Sales[Key]", "to": "Sales[Product]"}),
        json!({"from": "Sales[Nope]", "to": "Twin[Key]"}),
    ] {
        fails(&mut s, "relationship.add", p);
    }
    fails(
        &mut s,
        "relationship.set",
        json!({"from": "Sales[Product]", "to": "Product[Product]", "new_to": "Nope[X]"}),
    );
}

/// Sales and Product queried live (DirectQuery): referential integrity is allowed.
fn live() -> Session {
    const SQL: &str = "let Source = Sql.Database(\"srv.example\", \"db\"), T = Source{[Schema=\"dbo\",Item=\"T\"]}[Data] in T";
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Live"}));
    for (table, cols) in [
        ("Sales", &["ProductKey", "Amount"][..]),
        ("Product", &["ProductKey"][..]),
    ] {
        run(
            &mut s,
            "table.add",
            json!({"name": table, "partition": {"source_type": "m", "expression": SQL}}),
        );
        for c in cols {
            run(
                &mut s,
                "column.add",
                json!({"table": table, "name": c, "data_type": "int64"}),
            );
        }
        run(
            &mut s,
            "table.set_storage_mode",
            json!({"table": table, "mode": "directQuery"}),
        );
    }
    run(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[ProductKey]", "to": "Product[ProductKey]",
               "assume_referential_integrity": true, "cross_filter": "both",
               "security_filtering": "both"}),
    );
    s
}

fn assert_flags(s: &Session) {
    let r = &rels(s)[0];
    assert_eq!(r.extra["relyOnReferentialIntegrity"], true, "{r:?}");
    assert_eq!(r.extra["securityFilteringBehavior"], "bothDirections", "{r:?}");
    assert_eq!(r.cross_filtering_behavior.as_deref(), Some("bothDirections"));
}

#[test]
fn referential_integrity_and_security_round_trip_through_tmdl_and_tmsl() {
    for format in ["tmdl", "tmsl"] {
        let dir = TempDir::new(&format!("rels-flags-{format}"));
        let mut s = live();
        let v = run(&mut s, "relationship.list", json!({}));
        assert_eq!(
            v["relationships"][0]["rely_on_referential_integrity"], true,
            "{v}"
        );
        run(
            &mut s,
            "project.new",
            json!({"dir": dir.path(), "name": "Live", "format": format}),
        );
        let mut s2 = open(&dir.join("Live.pbip"));
        assert_flags(&s2);
        // switched off and saved again
        run(
            &mut s2,
            "relationship.set",
            json!({"from": "Sales[ProductKey]", "to": "Product[ProductKey]",
                   "assume_referential_integrity": false}),
        );
        run(&mut s2, "file.save", json!({}));
        let s3 = open(&dir.join("Live.pbip"));
        assert!(
            !rels(&s3)[0].extra.contains_key("relyOnReferentialIntegrity"),
            "{format}"
        );
    }
}

#[test]
fn an_edit_moves_an_end_checked_against_the_data() {
    let mut s = shop();
    run(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[Key]", "to": "Twin[Key]"}),
    );
    let name = rels(&s)[0].name.clone();
    // the "one" end moved to a column with duplicates: refused
    let err = fails(
        &mut s,
        "relationship.set",
        json!({"name": name, "new_to": "Product[Region]"}),
    );
    assert!(err.contains("duplicate"), "{err}");
    // the check of the edit dialog answers without changing anything
    let v = run(
        &mut s,
        "relationship.set",
        json!({"name": name, "new_from": "Sales[Product]", "new_to": "Product[Product]", "check": true}),
    );
    assert_eq!(v["valid"], true, "{v}");
    assert_eq!(v["to"], "Product[Product]");
    assert_eq!(rels(&s)[0].to_table, "Twin");
    let v = run(
        &mut s,
        "relationship.set",
        json!({"name": name, "new_from": "Sales[Product]", "new_to": "Product[Product]"}),
    );
    assert_eq!(v["data_checked"], json!(["Product[Product]"]), "{v}");
    let r = &rels(&s)[0];
    assert_eq!(
        (r.name.as_str(), r.from_column.as_str(), r.to_table.as_str()),
        (name.as_str(), "Product", "Product")
    );
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(rels(&s)[0].to_table, "Twin");
}

#[test]
fn pbix_keeps_a_moved_end_and_the_security_filter() {
    let dir = TempDir::new("rels-pbix");
    let path = dir.join("Shop.pbix");
    let mut s = shop();
    run(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[Key]", "to": "Twin[Key]", "name": "Keys"}),
    );
    run(&mut s, "file.save_as", json!({"path": path}));
    let mut s = open(&path);
    run(
        &mut s,
        "relationship.set",
        json!({"name": "Keys", "new_from": "Sales[Product]", "new_to": "Product[Product]",
               "cross_filter": "both", "security_filtering": "both"}),
    );
    run(&mut s, "file.save", json!({"backup": false}));
    let s2 = open(&path);
    let r = &rels(&s2)[0];
    assert_eq!(r.name, "Keys");
    assert_eq!(
        (
            r.from_table.as_str(),
            r.from_column.as_str(),
            r.to_table.as_str(),
            r.to_column.as_str()
        ),
        ("Sales", "Product", "Product", "Product")
    );
    assert_eq!(r.extra["securityFilteringBehavior"], "bothDirections");
    // the relationship filters: Apple's sales through the moved end
    let mut s2 = open(&path);
    let v = run(
        &mut s2,
        "dax.evaluate",
        json!({"query": "EVALUATE ROW(\"n\", CALCULATE(COUNTROWS(Sales), Product[Name] = \"Apple\"))"}),
    );
    assert!(v.to_string().contains('2'), "{v}");
}
