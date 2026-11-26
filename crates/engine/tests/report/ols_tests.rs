//! Object-level security: `role.set_permission` writes a table's or a column's
//! `metadataPermission` (and removes it), refuses a table that would break a relationship chain,
//! and View as applies it: what a role hides fails as a missing object in `dax.evaluate` and in
//! visuals, the rest works; RLS and OLS from different roles are refused; TMDL, TMSL and PBIX
//! saves keep it.

use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Sales(ProductKey, Amount) many-to-one Product(ProductKey, Name, Cost); [Total] and [Cost
/// total]; roles NoCost (hides Product[Cost]), NoProduct (hides Product) and East (a row filter).
fn shop() -> Session {
    let mut s = Session::new();
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.import",
            json!({"name": "Sales", "m": "#table(type table [ProductKey = number, Amount = number], {{1, 10}, {1, 5}, {2, 7}})"}),
        ),
        (
            "table.import",
            json!({"name": "Product", "m": "#table(type table [ProductKey = number, Name = text, Cost = number], {{1, \"Bike\", 4}, {2, \"Car\", 300}})"}),
        ),
        (
            "relationship.add",
            json!({"from": "Sales[ProductKey]", "to": "Product[ProductKey]"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Cost total", "expression": "SUM(Product[Cost])"}),
        ),
        ("role.add", json!({"name": "NoCost"})),
        (
            "role.set_permission",
            json!({"role": "NoCost", "table": "Product", "column": "cost", "metadata_permission": "none"}),
        ),
        ("role.add", json!({"name": "NoProduct"})),
        (
            "role.set_permission",
            json!({"role": "NoProduct", "table": "Product", "metadata_permission": "none"}),
        ),
        ("role.add", json!({"name": "East"})),
        (
            "role.set_filter",
            json!({"role": "East", "table": "Sales", "filter_expression": "[ProductKey] = 1"}),
        ),
    ] {
        run(&mut s, id, p);
    }
    s
}

fn perms(s: &Session, role: &str) -> Value {
    let r = s
        .model
        .as_ref()
        .unwrap()
        .roles
        .iter()
        .find(|r| r.name == role)
        .unwrap();
    serde_json::to_value(&r.table_permissions).unwrap()
}

fn eval(s: &mut Session, q: &str) -> Result<Value, String> {
    s.execute("dax.evaluate", &json!({"query": q}))
}

fn view_as(s: &mut Session, roles: &[&str]) {
    run(s, "view.interact", json!({"action": "view_as", "roles": roles}));
}

#[test]
fn set_permission_writes_and_removes_metadata_permission() {
    let mut s = shop();
    assert_eq!(
        perms(&s, "NoCost"),
        json!([{"table": "Product", "extra": {"columnPermissions": [{"name": "Cost", "metadataPermission": "none"}]}}])
    );
    assert_eq!(
        perms(&s, "NoProduct"),
        json!([{"table": "Product", "extra": {"metadataPermission": "none"}}])
    );
    // read is written as is; null removes it, and the emptied permission goes
    run(
        &mut s,
        "role.set_permission",
        json!({"role": "NoProduct", "table": "Product", "metadata_permission": "read"}),
    );
    assert_eq!(perms(&s, "NoProduct")[0]["extra"]["metadataPermission"], "read");
    run(
        &mut s,
        "role.set_permission",
        json!({"role": "NoCost", "table": "Product", "column": "Cost", "metadata_permission": null}),
    );
    assert_eq!(perms(&s, "NoCost"), json!([]));
    // a row filter keeps its table permission
    run(
        &mut s,
        "role.set_permission",
        json!({"role": "East", "table": "Sales", "column": "Amount", "metadata_permission": "none"}),
    );
    run(
        &mut s,
        "role.set_permission",
        json!({"role": "East", "table": "Sales", "column": "Amount", "metadata_permission": null}),
    );
    assert_eq!(perms(&s, "East")[0]["filter_expression"], "[ProductKey] = 1");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(
        perms(&s, "East")[0]["extra"]["columnPermissions"][0]["name"],
        "Amount"
    );
    for bad in [
        json!({"role": "Nobody", "table": "Sales", "metadata_permission": "none"}),
        json!({"role": "East", "table": "Nope", "metadata_permission": "none"}),
        json!({"role": "East", "table": "Sales", "column": "Nope", "metadata_permission": "none"}),
        json!({"role": "East", "table": "Sales", "metadata_permission": "write"}),
        json!({"role": "East", "table": "Sales"}),
    ] {
        assert!(s.execute("role.set_permission", &bad).is_err(), "{bad}");
    }
}

#[test]
fn a_table_in_a_relationship_chain_cannot_be_secured() {
    let mut s = shop();
    run(
        &mut s,
        "table.import",
        json!({"name": "Supplier", "m": "#table(type table [SupplierKey = number], {{1}})"}),
    );
    run(
        &mut s,
        "column.add",
        json!({"table": "Product", "name": "SupplierKey", "expression": "1", "data_type": "int64"}),
    );
    run(
        &mut s,
        "relationship.add",
        json!({"from": "Product[SupplierKey]", "to": "Supplier[SupplierKey]"}),
    );
    let e = s
        .execute(
            "role.set_permission",
            &json!({"role": "East", "table": "Product", "metadata_permission": "none"}),
        )
        .unwrap_err();
    assert!(e.contains("relationship chain"), "{e}");
    // its columns can be
    run(
        &mut s,
        "role.set_permission",
        json!({"role": "East", "table": "Product", "column": "Cost", "metadata_permission": "none"}),
    );
}

#[test]
fn view_as_hides_what_the_role_secures() {
    let mut s = shop();
    view_as(&mut s, &["NoCost"]);
    let ok = eval(&mut s, "EVALUATE ROW(\"t\", [Total])").unwrap();
    assert!(ok.to_string().contains("22"), "{ok}");
    // the column, and a measure that reads it, fail as a missing object
    let e = eval(&mut s, "EVALUATE ROW(\"c\", SUM(Product[Cost]))").unwrap_err();
    assert!(e.contains("cannot find column 'Product'[Cost]"), "{e}");
    let e = eval(&mut s, "EVALUATE ROW(\"c\", [Cost total])").unwrap_err();
    assert!(e.contains("object-level security"), "{e}");
    // the other columns of the table still work
    let names = eval(&mut s, "EVALUATE VALUES(Product[Name])").unwrap();
    assert!(names.to_string().contains("Bike"), "{names}");
    // a whole table
    view_as(&mut s, &["NoProduct"]);
    let e = eval(&mut s, "EVALUATE VALUES(Product[Name])").unwrap_err();
    assert!(e.contains("cannot find table 'Product'"), "{e}");
    let e = eval(&mut s, "EVALUATE ROW(\"n\", COUNTROWS(Product))").unwrap_err();
    assert!(e.contains("object-level security"), "{e}");
    // stopped: everything is back
    run(
        &mut s,
        "view.interact",
        json!({"action": "view_as", "roles": null}),
    );
    let all = eval(&mut s, "EVALUATE ROW(\"c\", [Cost total])").unwrap();
    assert!(all.to_string().contains("304"), "{all}");
}

#[test]
fn roles_unite_and_rls_with_ols_from_another_role_is_refused() {
    let mut s = shop();
    // an object is hidden only when every role hides it: NoCost grants the table, NoProduct
    // grants the Cost column (it does not secure it on its own)
    view_as(&mut s, &["NoCost", "NoProduct"]);
    let names = eval(&mut s, "EVALUATE VALUES(Product[Name])").unwrap();
    assert!(names.to_string().contains("Bike"), "{names}");
    let cost = eval(&mut s, "EVALUATE ROW(\"c\", [Cost total])").unwrap();
    assert!(cost.to_string().contains("304"), "{cost}");
    // a row filter in one role and object-level security in another
    view_as(&mut s, &["East", "NoCost"]);
    let e = eval(&mut s, "EVALUATE ROW(\"t\", [Total])").unwrap_err();
    assert!(e.contains("cannot be combined"), "{e}");
}

#[test]
fn object_level_security_is_saved_to_tmdl_tmsl_and_pbix() {
    let tmp = TempDir::new("engine-ols-save");
    for (format, file) in [
        ("pbip-tmdl", "Tmdl.pbip"),
        ("pbip-tmsl", "Tmsl.pbip"),
        ("pbix", "shop.pbix"),
    ] {
        let mut s = shop();
        let path = tmp.path().join(file);
        run(&mut s, "file.save_as", json!({"path": path, "format": format}));
        let mut back = Session::new();
        run(&mut back, "file.open", json!({"path": path}));
        assert_eq!(back.model_error, None, "{format}");
        assert_eq!(perms(&back, "NoCost"), perms(&s, "NoCost"), "{format}");
        assert_eq!(perms(&back, "NoProduct"), perms(&s, "NoProduct"), "{format}");
        view_as(&mut back, &["NoCost"]);
        assert!(
            eval(&mut back, "EVALUATE ROW(\"c\", SUM(Product[Cost]))").is_err(),
            "{format}"
        );
    }
}
