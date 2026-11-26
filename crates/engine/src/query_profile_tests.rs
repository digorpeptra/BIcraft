//! `query.profile` (`query_profile.rs`): column quality, distribution and a column profile over
//! the first 1000 rows or the whole query, at a step, with error values counted.

use serde_json::{Value, json};

use super::*;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Qty has an error (text "x" typed as a number), a null and a repeated value; City has an empty
/// text and a null.
const SALES: &str = "let\n    Source = #table({\"Product\", \"Qty\", \"City\"}, {{\"Bike\", \"2\", \"Paris\"}, {\"Car\", \"x\", \"Lyon\"}, {\"Pen\", \"9\", \"\"}, {\"Kite\", null, \"Paris\"}, {\"Cup\", \"2\", null}}),\n    Typed = Table.TransformColumnTypes(Source, {{\"Product\", type text}, {\"Qty\", Int64.Type}, {\"City\", type text}}),\n    Kept = Table.SelectRows(Typed, each [Product] <> \"Cup\")\nin\n    Kept";

fn session() -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Shop"}));
    run(
        &mut s,
        "expression.add",
        json!({"name": "Sales", "expression": SALES}),
    );
    s
}

fn col<'a>(v: &'a Value, name: &str) -> &'a Value {
    v["columns"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name)
        .unwrap_or_else(|| panic!("no column {name}: {v}"))
}

#[test]
fn profile_counts_valid_error_and_empty_values_and_distinct_and_unique_ones() {
    let mut s = session();
    let v = run(&mut s, "query.profile", json!({"name": "Sales", "step": "Typed"}));
    assert_eq!(v["rows"], 5);
    assert_eq!(v["full"], false);
    assert_eq!(v["truncated"], false);
    assert_eq!(v["step"], "Typed");
    let q = col(&v, "Qty");
    assert_eq!(q["data_type"], "int64");
    assert_eq!(
        (
            q["count"].clone(),
            q["valid"].clone(),
            q["errors"].clone(),
            q["empty"].clone()
        ),
        (json!(5), json!(3), json!(1), json!(1))
    );
    assert_eq!(q["error_share"], 0.2);
    assert_eq!(q["valid_share"], 0.6);
    // 2, 9 and null: three distinct values, 9 and null seen once
    assert_eq!(q["distinct"], 3);
    assert_eq!(q["unique"], 2);
    assert_eq!(q["distribution"][0], json!({"value": 2, "count": 2}));
    let c = col(&v, "City");
    // "" and null are both empty
    assert_eq!(
        (c["valid"].clone(), c["empty"].clone(), c["errors"].clone()),
        (json!(3), json!(2), json!(0))
    );
    // at the last step the row with Cup is gone
    let v = run(&mut s, "query.profile", json!({"name": "Sales"}));
    assert_eq!(v["rows"], 4);
    assert_eq!(col(&v, "Product")["unique"], 4);
    assert!(v["column"].is_null());
}

#[test]
fn a_column_profile_has_statistics_and_the_value_distribution() {
    let mut s = session();
    let v = run(
        &mut s,
        "query.profile",
        json!({"name": "Sales", "step": "Typed", "column": "Qty"}),
    );
    let st = &v["column"]["statistics"];
    assert_eq!(v["column"]["name"], "Qty");
    assert_eq!((st["min"].clone(), st["max"].clone()), (json!(2.0), json!(9.0)));
    assert_eq!(st["errors"], 1);
    assert_eq!(st["even"], 2);
    assert_eq!(st["odd"], 1);
    assert_eq!(st["zero"], 0);
    assert!((st["average"].as_f64().unwrap() - 13.0 / 3.0).abs() < 1e-9);
    assert_eq!(v["column"]["values"][0], json!({"value": 2, "count": 2}));
    let v = run(
        &mut s,
        "query.profile",
        json!({"name": "Sales", "column": "city"}),
    );
    let st = &v["column"]["statistics"];
    assert_eq!(
        (st["min"].clone(), st["max"].clone(), st["empty_string"].clone()),
        (json!("Lyon"), json!("Paris"), json!(1))
    );
    assert!(
        s.execute("query.profile", &json!({"name": "Sales", "column": "Nope"}))
            .is_err()
    );
}

#[test]
fn profiling_reads_the_top_1000_rows_unless_the_entire_data_set_is_asked() {
    let mut s = session();
    s.dirty = false;
    let m = "let Source = Table.FromColumns({List.Numbers(1, 1500)}, {\"N\"}) in Source";
    let v = run(&mut s, "query.profile", json!({"expression": m}));
    assert_eq!(v["rows"], 1000);
    assert_eq!(v["truncated"], true);
    assert_eq!(col(&v, "N")["distinct"], 1000);
    let v = run(
        &mut s,
        "query.profile",
        json!({"expression": m, "full": true, "column": "N"}),
    );
    assert_eq!(v["rows"], 1500);
    assert_eq!(v["truncated"], false);
    assert_eq!(v["column"]["statistics"]["max"], 1500.0);
    // nothing changed
    assert!(!s.dirty);
}

#[test]
fn hostile_profile_params_are_errors() {
    let mut s = session();
    for bad in [
        json!({}),
        json!({"name": "Nope"}),
        json!({"name": "Sales", "full": "yes"}),
        json!({"name": "Sales", "step": "Nope"}),
        json!({"name": "Sales", "column": 3}),
        json!({"name": "Sales", "other": 1}),
        json!({"expression": "1"}),
    ] {
        assert!(s.execute("query.profile", &bad).is_err(), "{bad}");
    }
}
