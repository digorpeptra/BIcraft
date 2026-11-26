//! The Manage security roles dialog's commands: `role.rename` (unique, no comma, View as
//! follows, one undo step) and `role.check_filter` (the DAX checked as a row filter, nothing
//! changed); TMDL, TMSL and PBIX keep a renamed role with its filter.

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

/// Sales(Key, Region, Amount), Region(Region) with the role Readers (Sales filtered to E) and
/// the role Auditors.
fn shop() -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Shop"}));
    for (name, csv) in [
        ("Sales", "Key,Region,Amount\n1,E,10\n2,W,20\n3,E,5\n"),
        ("Region", "Region\nE\nW\n"),
    ] {
        run(&mut s, "table.import", json!({"name": name, "csv": csv}));
    }
    run(&mut s, "role.add", json!({"name": "Readers"}));
    run(&mut s, "role.add", json!({"name": "Auditors"}));
    run(
        &mut s,
        "role.set_filter",
        json!({"role": "Readers", "table": "Sales", "filter_expression": "[Region] = \"E\""}),
    );
    s
}

fn names(s: &Session) -> Vec<String> {
    s.model
        .as_ref()
        .unwrap()
        .roles
        .iter()
        .map(|r| r.name.clone())
        .collect()
}

#[test]
fn a_role_is_renamed_with_its_filter_and_undone_in_one_step() {
    let mut s = shop();
    let v = run(
        &mut s,
        "role.rename",
        json!({"name": "readers", "new_name": "East readers"}),
    );
    assert_eq!(v["role"], "East readers", "{v}");
    assert_eq!(v["old_name"], "Readers", "{v}");
    assert_eq!(names(&s), ["East readers", "Auditors"]);
    let r = &s.model.as_ref().unwrap().roles[0];
    assert_eq!(
        r.table_permissions[0].filter_expression.as_deref(),
        Some("[Region] = \"E\"")
    );
    // a change of case only is a rename too
    run(
        &mut s,
        "role.rename",
        json!({"name": "East readers", "new_name": "EAST readers"}),
    );
    assert_eq!(names(&s)[0], "EAST readers");
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(names(&s), ["Readers", "Auditors"]);
}

#[test]
fn a_rename_is_refused_for_a_taken_name_a_comma_or_no_role() {
    let mut s = shop();
    let e = fails(
        &mut s,
        "role.rename",
        json!({"name": "Readers", "new_name": "auditors"}),
    );
    assert!(e.contains("already exists"), "{e}");
    let e = fails(
        &mut s,
        "role.rename",
        json!({"name": "Readers", "new_name": "London,Paris"}),
    );
    assert!(e.contains("comma"), "{e}");
    let e = fails(&mut s, "role.rename", json!({"name": "Nobody", "new_name": "X"}));
    assert!(e.contains("no role"), "{e}");
    let e = fails(&mut s, "role.rename", json!({"name": "Readers", "new_name": " "}));
    assert!(e.contains("empty"), "{e}");
    // role.add follows the same rule
    let e = fails(&mut s, "role.add", json!({"name": "a,b"}));
    assert!(e.contains("comma"), "{e}");
}

#[test]
fn view_as_follows_a_renamed_role() {
    let mut s = shop();
    run(&mut s, "report.new", json!({}));
    run(
        &mut s,
        "view.interact",
        json!({"action": "view_as", "roles": ["Readers"]}),
    );
    run(
        &mut s,
        "role.rename",
        json!({"name": "Readers", "new_name": "East"}),
    );
    assert_eq!(
        s.interactions.view_as().map(|v| v.roles.clone()),
        Some(vec!["East".to_string()])
    );
}

#[test]
fn check_filter_reports_errors_and_warnings_without_changing_anything() {
    let mut s = shop();
    let before = s.model.clone();
    let ok = run(
        &mut s,
        "role.check_filter",
        json!({"role": "Readers", "table": "sales", "filter_expression": "[Region] = \"W\" || [Amount] > 10"}),
    );
    assert_eq!(
        (ok["valid"].clone(), ok["errors"].clone()),
        (json!(true), json!(0)),
        "{ok}"
    );
    assert_eq!(ok["table"], "Sales");
    // an unknown column: an error with its position
    let bad = run(
        &mut s,
        "role.check_filter",
        json!({"table": "Sales", "filter_expression": "[Regio] = \"E\""}),
    );
    assert_eq!(bad["valid"], false, "{bad}");
    assert!(bad["errors"].as_u64().unwrap() >= 1, "{bad}");
    let d = &bad["diagnostics"][0];
    assert_eq!(d["severity"], "error", "{bad}");
    assert_eq!(
        (d["line"].clone(), d["column"].clone()),
        (json!(1), json!(1)),
        "{bad}"
    );
    // a syntax error
    let bad = run(
        &mut s,
        "role.check_filter",
        json!({"table": "Sales", "filter_expression": "[Region] = "}),
    );
    assert_eq!(bad["valid"], false, "{bad}");
    // a filter that is not True/False: Desktop's run-time error, a warning here
    let w = run(
        &mut s,
        "role.check_filter",
        json!({"table": "Sales", "filter_expression": "[Amount] + 1"}),
    );
    assert!(w["warnings"].as_u64().unwrap() >= 1, "{w}");
    assert!(w.to_string().contains("True/False"), "{w}");
    // the query-only INFO functions are refused, as role.set_filter refuses them
    let i = run(
        &mut s,
        "role.check_filter",
        json!({"table": "Sales", "filter_expression": "COUNTROWS(INFO.TABLES()) > 0"}),
    );
    assert_eq!(i["valid"], false, "{i}");
    // no filter: every row
    let e = run(
        &mut s,
        "role.check_filter",
        json!({"table": "Sales", "filter_expression": null}),
    );
    assert_eq!(
        (e["valid"].clone(), e["empty"].clone()),
        (json!(true), json!(true))
    );
    assert_eq!(s.model, before, "the check changes nothing");
    assert!(
        s.execute(
            "role.check_filter",
            &json!({"table": "Nope", "filter_expression": "TRUE()"})
        )
        .is_err()
    );
    assert!(
        s.execute(
            "role.check_filter",
            &json!({"role": "Nope", "table": "Sales", "filter_expression": "TRUE()"})
        )
        .is_err()
    );
    assert!(
        s.execute(
            "role.check_filter",
            &json!({"table": "Sales", "filter_expression": 3})
        )
        .is_err()
    );
    assert!(s.execute("role.check_filter", &json!({"table": "Sales"})).is_ok());
}

fn assert_renamed(s: &Session, what: &str) {
    let m = s.model.as_ref().unwrap();
    let r = m
        .roles
        .iter()
        .find(|r| r.name == "East readers")
        .unwrap_or_else(|| panic!("{what}: no renamed role in {:?}", m.roles));
    assert_eq!(
        r.table_permissions[0].filter_expression.as_deref(),
        Some("[Region] = \"E\""),
        "{what}"
    );
    assert!(!m.roles.iter().any(|r| r.name == "Readers"), "{what}");
}

#[test]
fn tmdl_tmsl_and_pbix_keep_a_renamed_role() {
    for format in ["tmdl", "tmsl"] {
        let dir = TempDir::new(&format!("roles-rename-{format}"));
        let mut s = shop();
        run(
            &mut s,
            "project.new",
            json!({"dir": dir.path(), "name": "Shop", "format": format}),
        );
        let mut s = open(&dir.join("Shop.pbip"));
        run(
            &mut s,
            "role.rename",
            json!({"name": "Readers", "new_name": "East readers"}),
        );
        run(&mut s, "file.save", json!({}));
        assert_renamed(&open(&dir.join("Shop.pbip")), format);
    }
    let dir = TempDir::new("roles-rename-pbix");
    let path = dir.join("Shop.pbix");
    let mut s = shop();
    run(&mut s, "file.save_as", json!({"path": path}));
    let mut s = open(&path);
    run(
        &mut s,
        "role.rename",
        json!({"name": "Readers", "new_name": "East readers"}),
    );
    run(&mut s, "file.save", json!({"backup": false}));
    let s2 = open(&path);
    assert_renamed(&s2, "pbix");
    // View as finds the renamed role in the file, not the old name
    let mut s2 = open(&path);
    run(
        &mut s2,
        "view.interact",
        json!({"action": "view_as", "roles": ["East readers"]}),
    );
    assert!(
        s2.execute(
            "view.interact",
            &json!({"action": "view_as", "roles": ["Readers"]})
        )
        .is_err()
    );
}

/// The role in the snapshot's `model.security` (the Properties pane reads it).
fn security(s: &Session, role: &str) -> Value {
    s.snapshot()["model"]["security"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == role)
        .cloned()
        .unwrap_or(Value::Null)
}

/// `role.set` (G61, the Properties pane of a role): description and model permission, one undo
/// step, kept by TMDL and TMSL; hostile params change nothing.
#[test]
fn role_set_writes_the_description_and_permission_in_one_undo_step() {
    let mut s = shop();
    let out = run(
        &mut s,
        "role.set",
        json!({"name": "readers", "description": "East only", "model_permission": "readrefresh"}),
    );
    assert_eq!(
        out["model_permission"], "readRefresh",
        "the permission is spelt as TMSL spells it: {out}"
    );
    let r = security(&s, "Readers");
    assert_eq!(r["description"], "East only");
    assert_eq!(r["model_permission"], "readRefresh");
    // the filter is untouched
    assert_eq!(r["tables"][0]["filter_expression"], "[Region] = \"E\"");
    run(
        &mut s,
        "role.set",
        json!({"name": "Readers", "description": null}),
    );
    assert_eq!(security(&s, "Readers")["description"], Value::Null);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(security(&s, "Readers")["description"], "East only");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(security(&s, "Readers")["description"], Value::Null);
    assert_eq!(security(&s, "Readers")["model_permission"], "read");
    for p in [
        json!({"name": "Readers"}),
        json!({"name": "Nobody", "description": "x"}),
        json!({"name": "Readers", "model_permission": "owner"}),
        json!({"name": "Readers", "description": 3}),
        json!({"name": "Readers", "description": "x".repeat(10_001)}),
        json!({"name": "Readers", "description": "x", "extra": 1}),
        json!({"description": "x"}),
        Value::Null,
    ] {
        fails(&mut s, "role.set", p);
    }
    for format in ["tmdl", "tmsl"] {
        let dir = TempDir::new(&format!("roles-set-{format}"));
        let mut s = shop();
        run(
            &mut s,
            "project.new",
            json!({"dir": dir.path(), "name": "Shop", "format": format}),
        );
        let mut s = open(&dir.join("Shop.pbip"));
        run(
            &mut s,
            "role.set",
            json!({"name": "Auditors", "description": "Audit team", "model_permission": "none"}),
        );
        run(&mut s, "file.save", json!({}));
        let again = open(&dir.join("Shop.pbip"));
        let r = security(&again, "Auditors");
        assert_eq!(r["description"], "Audit team", "{format}");
        assert_eq!(r["model_permission"], "none", "{format}");
    }
}
