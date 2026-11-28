//! `visual.analyze` (G62) and `visual.explain_anomaly` (G56): on a small synthetic star, the field
//! planted to explain a change, a different distribution or an anomaly ranks first, through the
//! model's relationships; points that cannot be analyzed are refused with a reason.

use serde_json::{Value, json};

use bicraft_engine::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Product(Key, Color) 1—* Sales(Year, Region, Channel, ProductKey, Amount) with `rows`, and a
/// visual of [Total] by `axis`.
fn star(rows: &str, visual_type: &str, axis: &str) -> (Session, String, String) {
    let mut s = Session::new();
    let product = r#"#table(type table [Key = Int64.Type, Color = text], {{1, "Red"}, {2, "Blue"}})"#;
    let sales = format!(
        "#table(type table [Year = Int64.Type, Region = text, Channel = text, ProductKey = Int64.Type, Amount = number], {{{rows}}})"
    );
    for (id, p) in [
        ("model.new", json!({"name": "Star"})),
        (
            "table.add",
            json!({"name": "Product", "partition": {"source_type": "m", "expression": product}}),
        ),
        (
            "column.add",
            json!({"table": "Product", "name": "Key", "data_type": "int64"}),
        ),
        (
            "column.add",
            json!({"table": "Product", "name": "Color", "data_type": "string"}),
        ),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Year", "data_type": "int64"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Region", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Channel", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "ProductKey", "data_type": "int64"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
        ),
        (
            "relationship.add",
            json!({"from": "Sales[ProductKey]", "to": "Product[Key]"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        ("report.new", json!({})),
    ] {
        run(&mut s, id, p);
    }
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    let v = run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": visual_type, "x": 0, "y": 0, "width": 400, "height": 300,
               "fields": [{"role": "Category", "field": axis}, {"role": "Y", "field": "[Total]"}]}),
    );
    let visual = v["visual"].as_str().unwrap().to_string();
    (s, page, visual)
}

/// The candidates' fields, best first, and their scores.
fn ranking(v: &Value) -> Vec<(String, f64)> {
    v["candidates"]
        .as_array()
        .unwrap_or_else(|| panic!("no candidates: {v}"))
        .iter()
        .map(|c| {
            (
                c["field"].as_str().unwrap().to_string(),
                c["score"].as_f64().unwrap(),
            )
        })
        .collect()
}

/// The North region grows from 2023 to 2024 on both channels and both products; South falls
/// from 2024 to 2025. The other regions stay flat.
const CHANGE: &str = r#"{2023, "North", "Web", 1, 10}, {2023, "North", "Shop", 2, 10},
    {2023, "South", "Web", 1, 10}, {2023, "South", "Shop", 2, 10},
    {2023, "East", "Web", 1, 10}, {2023, "East", "Shop", 2, 10},
    {2024, "North", "Web", 1, 40}, {2024, "North", "Shop", 2, 40},
    {2024, "South", "Web", 1, 10}, {2024, "South", "Shop", 2, 10},
    {2024, "East", "Web", 1, 10}, {2024, "East", "Shop", 2, 10},
    {2025, "North", "Web", 1, 40}, {2025, "North", "Shop", 2, 40},
    {2025, "South", "Web", 1, 2}, {2025, "South", "Shop", 2, 2},
    {2025, "East", "Web", 1, 10}, {2025, "East", "Shop", 2, 10}"#;

#[test]
fn the_planted_field_explains_an_increase_and_a_decrease() {
    let (mut s, page, visual) = star(CHANGE, "lineChart", "Sales[Year]");
    let up = run(
        &mut s,
        "visual.analyze",
        json!({"page": page, "visual": visual, "point": [{"c": 0, "v": 2024}]}),
    );
    assert_eq!(up["kind"], "change", "{up}");
    assert_eq!(up["change"]["direction"], "increase", "{up}");
    assert_eq!(up["change"]["previous_point"], 2023, "{up}");
    let r = ranking(&up);
    assert_eq!(r[0].0, "Sales[Region]", "{up}");
    assert_eq!(up["candidates"][0]["member"], "North", "{up}");
    // the related table's column is a candidate, and explains less than the planted field
    let color = r
        .iter()
        .find(|(f, _)| f == "Product[Color]")
        .unwrap_or_else(|| panic!("{up}"));
    assert!(color.1 < r[0].1, "{up}");
    let channel = r
        .iter()
        .find(|(f, _)| f == "Sales[Channel]")
        .unwrap_or_else(|| panic!("{up}"));
    assert!(channel.1 < r[0].1, "{up}");
    // the axis itself is never a candidate
    assert!(r.iter().all(|(f, _)| f != "Sales[Year]"), "{up}");

    let down = run(
        &mut s,
        "visual.analyze",
        json!({"page": page, "visual": visual, "point": [{"c": 0, "v": 2025}]}),
    );
    assert_eq!(down["change"]["direction"], "decrease", "{down}");
    assert_eq!(ranking(&down)[0].0, "Sales[Region]", "{down}");
    assert_eq!(down["candidates"][0]["member"], "South", "{down}");
    assert!(
        down["candidates"][0]["member_change"].as_f64().unwrap() < 0.0,
        "{down}"
    );

    // the candidates can be named; `top` cuts the list
    let named = run(
        &mut s,
        "visual.analyze",
        json!({"page": page, "visual": visual, "point": [{"c": 0, "v": 2024}],
               "fields": ["Product[Color]", "Sales[Region]"], "top": 1}),
    );
    assert_eq!(ranking(&named).len(), 1, "{named}");
    assert_eq!(ranking(&named)[0].0, "Sales[Region]", "{named}");
}

/// North sells only on the web; the other regions sell on both channels, half each. Products
/// are split the same way everywhere.
const DISTRIBUTION: &str = r#"{2024, "North", "Web", 1, 30}, {2024, "North", "Web", 2, 30},
    {2024, "South", "Web", 1, 15}, {2024, "South", "Shop", 2, 15},
    {2024, "East", "Web", 1, 15}, {2024, "East", "Shop", 2, 15}"#;

#[test]
fn the_planted_field_shows_where_the_distribution_differs() {
    let (mut s, page, visual) = star(DISTRIBUTION, "clusteredColumnChart", "Sales[Region]");
    let v = run(
        &mut s,
        "visual.analyze",
        json!({"page": page, "visual": visual, "point": [{"c": 0, "v": "North"}]}),
    );
    assert_eq!(v["kind"], "distribution", "{v}");
    let r = ranking(&v);
    assert_eq!(r[0].0, "Sales[Channel]", "{v}");
    assert_eq!(v["candidates"][0]["member"], "Web", "{v}");
    let top = &v["candidates"][0];
    assert!(
        top["share_at_point"].as_f64() > top["share_elsewhere"].as_f64(),
        "{v}"
    );
    let color = r
        .iter()
        .find(|(f, _)| f == "Product[Color]")
        .unwrap_or_else(|| panic!("{v}"));
    assert!(color.1 < r[0].1, "{v}");
}

#[test]
fn points_that_cannot_be_analyzed_are_refused() {
    let (mut s, page, visual) = star(CHANGE, "lineChart", "Sales[Year]");
    for bad in [
        // the first point has no previous one
        json!({"page": page, "visual": visual, "point": [{"c": 0, "v": 2023}]}),
        // no point
        json!({"page": page, "visual": visual}),
        // a point the visual does not show
        json!({"page": page, "visual": visual, "point": [{"c": 0, "v": 1999}]}),
        json!({"page": page, "visual": visual, "point": [{"c": 0, "v": 2024}], "kind": "nope"}),
        json!({"page": page, "visual": visual, "point": [{"c": 0, "v": 2024}], "fields": ["[Total]"]}),
        json!({"page": page, "visual": visual, "point": [{"c": 0, "v": 2024}], "top": 0}),
    ] {
        assert!(s.execute("visual.analyze", &bad).is_err(), "{bad}");
    }
    // a change on a categorical axis
    let (mut s, page, visual) = star(DISTRIBUTION, "clusteredColumnChart", "Sales[Region]");
    let e = s
        .execute(
            "visual.analyze",
            &json!({"page": page, "visual": visual, "point": [{"c": 0, "v": "North"}], "kind": "change"}),
        )
        .unwrap_err();
    assert!(e.contains("categorical"), "{e}");
}

/// Twelve years, each region selling 10 on the web (product 1) and 10 in shops (product 2); in
/// year 7 South sells 110 on each: the line's one anomaly, planted in the region.
fn spike() -> String {
    let mut rows = Vec::new();
    for year in 1..=12 {
        for region in ["North", "South", "East"] {
            let amount = if year == 7 && region == "South" { 110 } else { 10 };
            rows.push(format!(r#"{{{year}, "{region}", "Web", 1, {amount}}}"#));
            rows.push(format!(r#"{{{year}, "{region}", "Shop", 2, {amount}}}"#));
        }
    }
    rows.join(", ")
}

/// The spike's line chart of [Total] by year, its anomalies found.
fn anomalous() -> (Session, String, String) {
    let (mut s, page, visual) = star(&spike(), "lineChart", "Sales[Year]");
    run(
        &mut s,
        "visual.analytics",
        json!({"page": page, "visual": visual, "lines": [{"kind": "anomaly_detection"}]}),
    );
    (s, page, visual)
}

#[test]
fn the_planted_field_explains_an_anomaly() {
    let (mut s, page, visual) = anomalous();
    // no point: the anomaly furthest outside its range, the only one here
    let v = run(
        &mut s,
        "visual.explain_anomaly",
        json!({"page": page, "visual": visual}),
    );
    assert_eq!(v["compare"], "expected", "{v}");
    assert_eq!(v["anomaly"]["at"], 7, "{v}");
    assert_eq!(v["anomaly"]["direction"], "above", "{v}");
    assert!(
        v["anomaly"]["value"].as_f64() > v["anomaly"]["high"].as_f64(),
        "{v}"
    );
    assert_eq!(v["anomalies"], json!([7]), "{v}");
    let r = ranking(&v);
    assert_eq!(r[0].0, "Sales[Region]", "{v}");
    assert_eq!(v["candidates"][0]["member"], "South", "{v}");
    assert!(
        v["candidates"][0]["member_deviation"].as_f64().unwrap() > 0.0,
        "{v}"
    );
    for other in ["Sales[Channel]", "Product[Color]"] {
        let o = r
            .iter()
            .find(|(f, _)| f == other)
            .unwrap_or_else(|| panic!("{other}: {v}"));
        assert!(o.1 < r[0].1, "{other} explains less: {v}");
    }
    assert!(r.iter().all(|(f, _)| f != "Sales[Year]"), "{v}");
    assert!(v["estimate"].as_str().unwrap().contains("BIcraft"), "{v}");
    // the clicked anomaly gives the same answer
    let at = run(
        &mut s,
        "visual.explain_anomaly",
        json!({"page": page, "visual": visual, "point": [{"c": 0, "v": 7}]}),
    );
    assert_eq!(ranking(&at), r, "{at}");
    // against the previous point: visual.analyze's change, the same field first
    let prev = run(
        &mut s,
        "visual.explain_anomaly",
        json!({"page": page, "visual": visual, "point": [{"c": 0, "v": 7}], "compare": "previous"}),
    );
    assert_eq!(prev["compare"], "previous", "{prev}");
    assert_eq!(prev["change"]["previous_point"], 6, "{prev}");
    assert_eq!(ranking(&prev)[0].0, "Sales[Region]", "{prev}");
    assert_eq!(prev["anomaly"]["at"], 7, "{prev}");
}

#[test]
fn what_is_not_an_anomaly_is_not_explained() {
    let (mut s, page, visual) = anomalous();
    for bad in [
        // an ordinary point
        json!({"page": page, "visual": visual, "point": [{"c": 0, "v": 3}]}),
        // a point the visual does not show
        json!({"page": page, "visual": visual, "point": [{"c": 0, "v": 1999}]}),
        json!({"page": page, "visual": visual, "compare": "nope"}),
        json!({"page": page, "visual": visual, "top": 0}),
        json!({"page": page, "visual": visual, "fields": ["[Total]"]}),
        json!({"page": page, "visual": visual, "extra": 1}),
    ] {
        assert!(s.execute("visual.explain_anomaly", &bad).is_err(), "{bad}");
    }
    let e = s
        .execute(
            "visual.explain_anomaly",
            &json!({"page": page, "visual": visual, "point": [{"c": 0, "v": 3}]}),
        )
        .unwrap_err();
    assert!(e.contains("not an anomaly"), "{e}");
    // a line without anomaly detection
    let (mut s, page, visual) = star(&spike(), "lineChart", "Sales[Year]");
    let e = s
        .execute("visual.explain_anomaly", &json!({"page": page, "visual": visual}))
        .unwrap_err();
    assert!(e.contains("Find anomalies"), "{e}");
}
