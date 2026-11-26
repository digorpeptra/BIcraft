//! `dax.apply_define` (the DAX query view's Update model): a query's DEFINE MEASURE blocks add new
//! measures and overwrite changed ones in one undo step; unknown tables, a measure of another
//! table, a query that does not parse and invalid DAX change nothing; what it writes is saved to
//! PBIP (TMDL) and PBIX.

use serde_json::{Value, json};

use bicraft_engine::Session;

use crate::tempdir::TempDir;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Sales(Amount) with [Total] = SUM(Sales[Amount]), and an empty Product table.
fn shop() -> Session {
    let mut s = Session::new();
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.import",
            json!({"name": "Sales", "m": "#table(type table [Amount = number], {{10}, {5}, {7}})"}),
        ),
        (
            "table.import",
            json!({"name": "Product", "m": "#table(type table [Name = text], {{\"Bike\"}})"}),
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

fn expression(s: &mut Session, name: &str) -> Option<String> {
    let m = s.execute("measure.get", &json!({"name": name})).ok()?;
    m.get("expression").and_then(Value::as_str).map(str::to_string)
}

const QUERY: &str = "DEFINE\n    MEASURE Sales[Total] = SUM(Sales[Amount]) * 2\n    MEASURE Sales[Count] = COUNTROWS(Sales)\n    MEASURE Sales[Average] = DIVIDE([Total], [Count])\nEVALUATE ROW(\"T\", [Total], \"A\", [Average])";

#[test]
fn every_define_measure_is_added_or_overwritten_in_one_undo_step() {
    let mut s = shop();
    let r = run(&mut s, "dax.apply_define", json!({"query": QUERY}));
    assert_eq!(r["added"], 2, "{r}");
    assert_eq!(r["updated"], 1, "{r}");
    assert_eq!(r["changed"], true);
    assert_eq!(
        r["changes"],
        json!([
            {"table": "Sales", "measure": "Total", "action": "updated"},
            {"table": "Sales", "measure": "Count", "action": "added"},
            {"table": "Sales", "measure": "Average", "action": "added"},
        ])
    );
    assert_eq!(
        expression(&mut s, "Total").as_deref(),
        Some("SUM(Sales[Amount]) * 2")
    );
    assert_eq!(expression(&mut s, "Count").as_deref(), Some("COUNTROWS(Sales)"));
    assert_eq!(
        expression(&mut s, "Average").as_deref(),
        Some("DIVIDE([Total], [Count])")
    );
    // the measures now evaluate from the model, without the DEFINE block
    let v = run(
        &mut s,
        "dax.evaluate",
        json!({"query": "EVALUATE ROW(\"A\", [Average])"}),
    );
    assert_eq!(v["results"][0]["rows"][0]["[A]"], json!(44.0 / 3.0), "{v}");
    // one undo step takes every change back
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(expression(&mut s, "Total").as_deref(), Some("SUM(Sales[Amount])"));
    assert_eq!(expression(&mut s, "Count"), None);
    assert_eq!(expression(&mut s, "Average"), None);
}

#[test]
fn only_the_measures_named_are_applied_and_an_unchanged_one_is_no_edit() {
    let mut s = shop();
    let r = run(
        &mut s,
        "dax.apply_define",
        json!({"query": QUERY, "measures": ["count"]}),
    );
    assert_eq!(
        r["changes"],
        json!([{"table": "Sales", "measure": "Count", "action": "added"}])
    );
    assert_eq!(expression(&mut s, "Total").as_deref(), Some("SUM(Sales[Amount])"));
    let again = run(
        &mut s,
        "dax.apply_define",
        json!({"query": QUERY, "measures": ["Count"]}),
    );
    assert_eq!(again["changed"], false, "{again}");
    assert_eq!(again["unchanged"], 1);
}

#[test]
fn refusals_change_nothing() {
    let mut s = shop();
    for (query, measures, why) in [
        (
            "DEFINE MEASURE Nowhere[X] = 1 EVALUATE ROW(\"X\", [X])",
            None,
            "no table named `Nowhere`",
        ),
        (
            "DEFINE MEASURE Product[Total] = 1 EVALUATE ROW(\"X\", [Total])",
            None,
            "is in table `Sales`",
        ),
        ("DEFINE MEASURE Sales[X] = ( EVALUATE", None, "does not parse"),
        ("EVALUATE Sales", None, "defines no measure"),
        (QUERY, Some(json!(["Missing"])), "no measure named `Missing`"),
        (QUERY, Some(json!("Count")), "list of measure names"),
        (QUERY, Some(json!([])), "is empty"),
        (
            "DEFINE MEASURE Sales[Bad] = SUM(Sales[NoSuchColumn]) EVALUATE ROW(\"X\", 1)",
            None,
            "invalid DAX",
        ),
        (
            "DEFINE MEASURE Sales[Amount] = 1 EVALUATE ROW(\"X\", 1)",
            None,
            "has a column named `Amount`",
        ),
    ] {
        let mut p = json!({"query": query});
        if let Some(m) = measures {
            p["measures"] = m;
        }
        let e = s.execute("dax.apply_define", &p).unwrap_err();
        assert!(e.contains(why), "{query}: {e}");
        assert_eq!(expression(&mut s, "Total").as_deref(), Some("SUM(Sales[Amount])"));
        assert_eq!(expression(&mut s, "X"), None);
        assert_eq!(expression(&mut s, "Bad"), None);
    }
    for bad in [json!({}), json!({"query": 1}), json!({"query": QUERY, "nope": 1})] {
        assert!(s.execute("dax.apply_define", &bad).is_err(), "{bad}");
    }
    // force applies invalid DAX anyway, with a warning
    let r = run(
        &mut s,
        "dax.apply_define",
        json!({"query": "DEFINE MEASURE Sales[Bad] = SUM(Sales[NoSuchColumn]) EVALUATE ROW(\"X\", 1)", "force": true}),
    );
    assert_eq!(r["added"], 1, "{r}");
}

#[test]
fn what_it_writes_is_saved_to_pbip_and_pbix() {
    let tmp = TempDir::new("engine-apply-define-save");
    for (format, file) in [("pbip-tmdl", "Shop.pbip"), ("pbix", "Shop.pbix")] {
        let mut s = shop();
        run(&mut s, "dax.apply_define", json!({"query": QUERY}));
        let path = tmp.path().join(file);
        run(&mut s, "file.save_as", json!({"path": path, "format": format}));
        let mut back = Session::new();
        run(&mut back, "file.open", json!({"path": path}));
        assert_eq!(back.model_error, None, "{format}");
        assert_eq!(
            expression(&mut back, "Total").as_deref(),
            Some("SUM(Sales[Amount]) * 2"),
            "{format}"
        );
        assert_eq!(
            expression(&mut back, "Average").as_deref(),
            Some("DIVIDE([Total], [Count])"),
            "{format}"
        );
    }
}
