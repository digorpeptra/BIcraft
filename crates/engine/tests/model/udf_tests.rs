//! `function.add`, `function.set`, `function.rename`, `function.remove`: the model's DAX
//! user-defined functions, their validation, the calls in measures, the evaluator, and the round
//! trips through TMDL, TMSL and PBIX.

use serde_json::{Value, json};

use bicraft_engine::Session;

use crate::tempdir::TempDir;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn shop() -> Session {
    let mut s = Session::new();
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.import",
            json!({"name": "Sales", "m": "#table(type table [Amount = number], {{10}, {5}, {7}})"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
    ] {
        run(&mut s, id, p);
    }
    s
}

fn functions(s: &Session) -> Value {
    s.model
        .as_ref()
        .unwrap()
        .extra
        .get("functions")
        .cloned()
        .unwrap_or(Value::Null)
}

fn value(s: &mut Session, query: &str) -> Value {
    let v = run(s, "dax.evaluate", json!({"query": query}));
    v["results"][0]["rows"][0].clone()
}

#[test]
fn add_from_parts_validates_raises_the_level_and_evaluates_in_a_measure() {
    let mut s = shop();
    let r = run(
        &mut s,
        "function.add",
        json!({"name": "Sales.AddTax", "description": "Adds the tax",
               "parameters": [{"name": "amount", "type": "scalar", "subtype": "numeric"},
                              {"name": "rate", "default": "0.1"}],
               "body": "amount * (1 + rate)"}),
    );
    assert_eq!(
        r["function"]["expression"],
        "(amount : SCALAR NUMERIC, rate = 0.1) => amount * (1 + rate)"
    );
    assert_eq!(r["function"]["parameters"][0]["mode"], "VAL");
    assert_eq!(r["function"]["parameters"][1]["default"], "0.1");
    let f = &functions(&s)[0];
    assert_eq!(f["name"], "Sales.AddTax");
    assert_eq!(f["description"], "Adds the tax");
    assert!(f["lineageTag"].is_string());
    assert!(s.model.as_ref().unwrap().compatibility_level >= Some(1702));
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "With Tax", "expression": "Sales.AddTax([Total])"}),
    );
    let row = value(
        &mut s,
        "EVALUATE ROW(\"x\", [With Tax], \"y\", Sales.AddTax(10, 1))",
    );
    let x = row["[x]"].as_f64().unwrap();
    assert!((x - 24.2).abs() < 1e-9, "{row}");
    assert_eq!(row["[y]"], 20.0, "{row}");
    // one undo takes the function back
    run(&mut s, "measure.remove", json!({"name": "With Tax"}));
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(functions(&s), Value::Null);
}

#[test]
fn bad_definitions_and_calls_are_refused() {
    let mut s = shop();
    for (p, why) in [
        (json!({"name": "SUM", "expression": "() => 1"}), "built-in"),
        (json!({"name": "My Func", "expression": "() => 1"}), "no spaces"),
        (
            json!({"name": "A..B", "expression": "() => 1"}),
            "consecutive periods",
        ),
        (
            json!({"name": "F", "expression": "(x) => x + [Nope]"}),
            "invalid DAX",
        ),
        (
            json!({"name": "F", "expression": "(x : NUMBER) => x"}),
            "invalid DAX",
        ),
        (json!({"name": "F", "expression": "x * 2"}), "invalid DAX"),
        (json!({"name": "F", "expression": "(x) => F(x)"}), "invalid DAX"),
        (
            json!({"name": "F", "body": "1", "parameters": [{"name": "a b"}]}),
            "parameter name",
        ),
        (
            json!({"name": "F", "body": "1", "parameters": [{"name": "x", "type": "Number"}]}),
            "unknown type",
        ),
        (
            json!({"name": "F", "expression": "() => 1", "body": "1"}),
            "not both",
        ),
        (json!({"name": "F"}), "required"),
        (json!({"name": "F", "expression": 1}), ""),
        (json!({"name": "F", "expression": "() => 1", "nope": 1}), ""),
    ] {
        let e = s.execute("function.add", &p).unwrap_err();
        assert!(e.contains(why), "{p}: {e}");
    }
    assert_eq!(functions(&s), Value::Null);
    run(
        &mut s,
        "function.add",
        json!({"name": "Twice", "expression": "(x : NUMERIC) => x * 2"}),
    );
    assert!(
        s.execute("function.add", &json!({"name": "twice", "expression": "() => 1"}))
            .is_err()
    );
    // calls in measures: arity and argument kinds
    for (expr, why) in [
        ("Twice()", "argument"),
        ("Twice(1, 2)", "argument"),
        ("Twice(Sales)", "scalar"),
    ] {
        let e = s
            .execute(
                "measure.add",
                &json!({"table": "Sales", "name": "M", "expression": expr}),
            )
            .unwrap_err();
        assert!(e.contains(why), "{expr}: {e}");
    }
    // mutual recursion through function.set
    run(
        &mut s,
        "function.add",
        json!({"name": "Other", "expression": "(x) => Twice(x)"}),
    );
    let e = s
        .execute(
            "function.set",
            &json!({"name": "Twice", "expression": "(x) => Other(x)"}),
        )
        .unwrap_err();
    assert!(e.contains("recursive"), "{e}");
    for bad in [
        json!({"name": "Nope", "body": "1"}),
        json!({"name": "Twice"}),
        json!({"name": "Twice", "hidden": "yes"}),
    ] {
        assert!(s.execute("function.set", &bad).is_err(), "{bad}");
    }
}

#[test]
fn set_rename_and_remove_follow_the_calls() {
    let mut s = shop();
    run(
        &mut s,
        "function.add",
        json!({"name": "Twice", "expression": "(x : NUMERIC) => x * 2"}),
    );
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Double", "expression": "Twice([Total])"}),
    );
    // the body alone: the parameters stay
    let r = run(
        &mut s,
        "function.set",
        json!({"name": "twice", "body": "x * 3", "hidden": true}),
    );
    assert_eq!(r["function"]["expression"], "(x : NUMERIC) => x * 3");
    assert_eq!(functions(&s)[0]["isHidden"], true);
    assert_eq!(value(&mut s, "EVALUATE ROW(\"x\", [Double])")["[x]"], 66.0);
    // a signature change that breaks the measure is reported
    let r = run(
        &mut s,
        "function.set",
        json!({"name": "Twice", "parameters": [{"name": "x"}, {"name": "y"}]}),
    );
    assert_eq!(r["broken"][0]["name"], "Double", "{r}");
    run(&mut s, "edit.undo", json!({}));
    // rename: the measure's call follows
    let r = run(
        &mut s,
        "function.rename",
        json!({"name": "Twice", "new_name": "Math.Triple"}),
    );
    assert_eq!(r["calls_rewritten"], 1, "{r}");
    let m = run(&mut s, "measure.get", json!({"name": "Double"}));
    assert_eq!(m["expression"], "Math.Triple([Total])");
    assert!(
        s.execute(
            "function.rename",
            &json!({"name": "Math.Triple", "new_name": "SUM"})
        )
        .is_err()
    );
    // remove: refused while called, unless force
    let e = s
        .execute("function.remove", &json!({"name": "Math.Triple"}))
        .unwrap_err();
    assert!(e.contains("measure"), "{e}");
    let r = run(
        &mut s,
        "function.remove",
        json!({"name": "Math.Triple", "force": true}),
    );
    assert_eq!(r["removed"], "Math.Triple");
    assert_eq!(functions(&s), Value::Null);
    assert!(
        s.execute("function.remove", &json!({"name": "Math.Triple"}))
            .is_err()
    );
}

#[test]
fn functions_round_trip_through_tmdl_tmsl_and_pbix() {
    let tmp = TempDir::new("engine-udf-save");
    for (format, file) in [
        ("pbip-tmdl", "Shop.pbip"),
        ("pbip-tmsl", "ShopT.pbip"),
        ("bim", "Shop.bim"),
        ("pbix", "Shop.pbix"),
    ] {
        let mut s = shop();
        run(
            &mut s,
            "function.add",
            json!({"name": "AddTax", "description": "Adds the tax",
                   "expression": "(amount : NUMERIC, rate : NUMERIC = 0.1) =>\n    amount * (1 + rate)"}),
        );
        run(
            &mut s,
            "measure.add",
            json!({"table": "Sales", "name": "With Tax", "expression": "AddTax([Total])"}),
        );
        let before = functions(&s);
        let path = tmp.path().join(file);
        run(&mut s, "file.save_as", json!({"path": path, "format": format}));
        let mut back = Session::new();
        run(&mut back, "file.open", json!({"path": path}));
        assert_eq!(back.model_error, None, "{format}");
        assert_eq!(functions(&back), before, "{format}");
        let v = run(
            &mut back,
            "dax.validate",
            json!({"expression": "AddTax(1, 2, 3)"}),
        );
        assert_eq!(v["valid"], false, "{format}: {v}");
    }
}
