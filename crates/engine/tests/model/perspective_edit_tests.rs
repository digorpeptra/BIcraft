//! Authoring perspectives: `perspective.add`, `perspective.set` (members, rename, description)
//! and `perspective.remove`, with tables, columns, measures and hierarchies; the snapshot's
//! `perspective_members`; renames and removals still follow; TMDL, TMSL and PBIX saves keep them.

use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Sales(Amount, Region, City) with [Total] and the hierarchy Geo; Product(Name, Cost).
fn shop() -> Session {
    let mut s = Session::new();
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.import",
            json!({"name": "Sales", "m": "#table(type table [Amount = number, Region = text, City = text], {{1.5, \"N\", \"A\"}, {2, \"S\", \"B\"}})"}),
        ),
        (
            "table.import",
            json!({"name": "Product", "m": "#table(type table [Name = text, Cost = number], {{\"X\", 1}})"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        (
            "hierarchy.add",
            json!({"table": "Sales", "name": "Geo", "levels": ["Region", "City"]}),
        ),
    ] {
        run(&mut s, id, p);
    }
    s
}

fn persp(s: &Session) -> Value {
    s.model
        .as_ref()
        .unwrap()
        .extra
        .get("perspectives")
        .cloned()
        .unwrap_or(Value::Null)
}

#[test]
fn add_set_and_remove_a_perspective() {
    let mut s = shop();
    let v = run(
        &mut s,
        "perspective.add",
        json!({"name": "Lite", "tables": [
            {"table": "sales", "columns": ["amount", "Region"], "measures": ["Total"], "hierarchies": ["Geo"]},
            "Product"
        ]}),
    );
    assert_eq!(
        v["members"],
        json!({"tables": 2, "columns": 4, "measures": 1, "hierarchies": 1})
    );
    assert_eq!(
        persp(&s),
        json!([{"name": "Lite", "tables": [
            {"name": "Sales", "columns": [{"name": "Amount"}, {"name": "Region"}],
             "measures": [{"name": "Total"}], "hierarchies": [{"name": "Geo"}]},
            {"name": "Product", "columns": [{"name": "Name"}, {"name": "Cost"}]}
        ]}])
    );
    // the snapshot lists the members
    let snap = s.snapshot();
    assert_eq!(snap["model"]["perspectives"], json!(["Lite"]));
    assert_eq!(
        snap["model"]["perspective_members"][0]["tables"][0],
        json!({"name": "Sales", "columns": ["Amount", "Region"], "measures": ["Total"], "hierarchies": ["Geo"]})
    );
    // a duplicate name, a missing member, a table twice
    assert!(s.execute("perspective.add", &json!({"name": "lite"})).is_err());
    for bad in [
        json!({"name": "X", "tables": [{"table": "Sales", "columns": ["Nope"]}]}),
        json!({"name": "X", "tables": [{"table": "Nope"}]}),
        json!({"name": "X", "tables": ["Sales", "sales"]}),
        json!({"name": "X", "tables": [{"table": "Sales", "colour": 1}]}),
    ] {
        assert!(s.execute("perspective.add", &bad).is_err(), "{bad}");
    }
    // set: new members, a rename and a description, one undo step each
    let v = run(
        &mut s,
        "perspective.set",
        json!({"name": "Lite", "new_name": "Sales view", "description": "For sales",
               "tables": [{"table": "Sales", "all": true}]}),
    );
    assert_eq!(v["renamed"], "Lite");
    assert_eq!(
        persp(&s),
        json!([{"name": "Sales view", "description": "For sales", "tables": [
            {"name": "Sales", "columns": [{"name": "Amount"}, {"name": "Region"}, {"name": "City"}],
             "measures": [{"name": "Total"}], "hierarchies": [{"name": "Geo"}]}
        ]}])
    );
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(persp(&s)[0]["name"], "Lite");
    run(&mut s, "edit.redo", json!({}));
    assert!(
        s.execute("perspective.set", &json!({"name": "Sales view"}))
            .is_err()
    );
    assert!(
        s.execute("perspective.set", &json!({"name": "Nope", "tables": []}))
            .is_err()
    );
    // renames and removals follow, as for perspectives a file brought
    run(
        &mut s,
        "measure.rename",
        json!({"table": "Sales", "name": "Total", "new_name": "Sum"}),
    );
    assert_eq!(persp(&s)[0]["tables"][0]["measures"], json!([{"name": "Sum"}]));
    run(&mut s, "perspective.remove", json!({"name": "SALES VIEW"}));
    assert_eq!(persp(&s), Value::Null);
    assert!(
        s.execute("perspective.remove", &json!({"name": "Sales view"}))
            .is_err()
    );
}

#[test]
fn perspectives_are_saved_to_tmdl_tmsl_and_pbix() {
    let tmp = TempDir::new("engine-perspective-edit-save");
    for (format, file) in [
        ("pbip-tmdl", "Tmdl.pbip"),
        ("pbip-tmsl", "Tmsl.pbip"),
        ("pbix", "shop.pbix"),
    ] {
        let mut s = shop();
        run(
            &mut s,
            "perspective.add",
            json!({"name": "Lite", "description": "Short", "tables": [
                {"table": "Sales", "columns": ["Amount"], "measures": ["Total"], "hierarchies": ["Geo"]},
                {"table": "Product", "all": true}
            ]}),
        );
        let path = tmp.path().join(file);
        run(&mut s, "file.save_as", json!({"path": path, "format": format}));
        let mut back = Session::new();
        run(&mut back, "file.open", json!({"path": path}));
        assert_eq!(back.model_error, None, "{format}");
        let members = &back.snapshot()["model"]["perspective_members"];
        assert_eq!(
            members,
            &json!([{"name": "Lite", "tables": [
                {"name": "Sales", "columns": ["Amount"], "measures": ["Total"], "hierarchies": ["Geo"]},
                {"name": "Product", "columns": ["Name", "Cost"], "measures": [], "hierarchies": []}
            ]}]),
            "{format}"
        );
        // and it can be edited after the round trip
        run(
            &mut back,
            "perspective.set",
            json!({"name": "Lite", "tables": ["Product"]}),
        );
    }
}
