//! Tests of `view.interact` `focus` and `show_table`: the reader's focus mode and Show as a
//! table are view state (the snapshot's `interaction.focus` / `show_table`, `view.interactions`),
//! never the report or the files; `reset` and a page change leave them; hostile parameters are
//! errors that leave the state alone.

use serde_json::{Value, json};

use crate::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// A column chart on the first page; a second page.
fn session() -> (Session, String, String, String) {
    let mut s = Session::new();
    let sales = r#"#table({"Category", "Amount"}, {{"Wheels", 100}, {"Toys", 5}})"#;
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Category", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        ("report.new", json!({})),
        ("page.add", json!({"display_name": "Detail"})),
    ] {
        run(&mut s, id, p);
    }
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    let second = s.report.as_ref().unwrap().pages[1].name.clone();
    let r = run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "clusteredColumnChart", "x": 20, "y": 20,
               "width": 300, "height": 200,
               "fields": [{"role": "Category", "field": "Sales[Category]"}, {"role": "Y", "field": "[Total]"}]}),
    );
    let chart = r["visual"].as_str().unwrap().to_string();
    s.dirty = false;
    s.report_dirty = false;
    (s, page, second, chart)
}

fn interaction(s: &Session) -> Value {
    s.snapshot()["interaction"].clone()
}

#[test]
fn focus_and_show_table_are_view_state() {
    let (mut s, page, second, chart) = session();
    let report = s.report.clone();
    let rev = s.revision;
    assert_eq!(interaction(&s)["focus"], Value::Null);
    let out = run(
        &mut s,
        "view.interact",
        json!({"action": "focus", "page": page, "visual": chart}),
    );
    assert_eq!(out["changed"], true);
    let i = interaction(&s);
    assert_eq!(i["focus"], json!({"page": page, "visual": chart}));
    assert_eq!(i["show_table"], false);
    assert_eq!(
        run(&mut s, "view.interactions", json!({}))["focus"],
        json!({"page": page, "visual": chart, "show_table": false})
    );
    // the same again changes nothing
    let again = run(
        &mut s,
        "view.interact",
        json!({"action": "focus", "page": page, "visual": chart}),
    );
    assert_eq!(again["changed"], false);
    // Show as a table; off keeps the focus
    run(
        &mut s,
        "view.interact",
        json!({"action": "show_table", "page": page, "visual": chart}),
    );
    assert_eq!(interaction(&s)["show_table"], true);
    // focusing the visual shown as a table keeps its table
    run(
        &mut s,
        "view.interact",
        json!({"action": "focus", "page": page, "visual": chart}),
    );
    assert_eq!(interaction(&s)["show_table"], true);
    run(
        &mut s,
        "view.interact",
        json!({"action": "show_table", "page": page, "visual": chart, "on": false}),
    );
    let i = interaction(&s);
    assert_eq!(
        (i["focus"]["visual"].as_str(), i["show_table"].as_bool()),
        (Some(chart.as_str()), Some(false))
    );
    // null leaves focus mode
    run(
        &mut s,
        "view.interact",
        json!({"action": "focus", "page": page, "visual": null}),
    );
    assert_eq!(interaction(&s)["focus"], Value::Null);
    // reset leaves it
    run(
        &mut s,
        "view.interact",
        json!({"action": "show_table", "page": page, "visual": chart}),
    );
    let out = run(&mut s, "view.interact", json!({"action": "reset"}));
    assert_eq!(out["changed"], true);
    assert_eq!(interaction(&s)["focus"], Value::Null);
    // a page change leaves it
    run(
        &mut s,
        "view.interact",
        json!({"action": "focus", "page": page, "visual": chart}),
    );
    run(&mut s, "view.interact", json!({"action": "page", "page": second}));
    assert_eq!(interaction(&s)["focus"], Value::Null);
    // the report, the revision and the dirty flags never changed
    assert_eq!(s.report, report);
    assert_eq!(s.revision, rev);
    assert!(!s.dirty && !s.report_dirty);
}

#[test]
fn hostile_focus_params_are_errors() {
    let (mut s, page, _, chart) = session();
    run(
        &mut s,
        "view.interact",
        json!({"action": "focus", "page": page, "visual": chart}),
    );
    let before = run(&mut s, "view.interactions", json!({}));
    for p in [
        json!({"action": "focus", "page": page, "visual": "nope"}),
        json!({"action": "focus", "page": "nope", "visual": chart}),
        json!({"action": "focus", "page": page, "visual": 7}),
        json!({"action": "focus", "page": page, "visual": chart, "extra": 1}),
        json!({"action": "show_table", "page": page}),
        json!({"action": "show_table", "page": page, "visual": chart, "on": "yes"}),
        json!({"action": "show_table", "page": [], "visual": chart}),
        json!({"action": "show_table", "page": page, "visual": {"x": 1}}),
    ] {
        assert!(s.execute("view.interact", &p).is_err(), "{p} accepted");
    }
    assert_eq!(run(&mut s, "view.interactions", json!({})), before);
}
