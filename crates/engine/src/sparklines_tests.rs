//! Tests of the Data format (`visual.set_data_format`) and sparklines (`visual.add_sparkline`,
//! `visual.edit_sparkline`): the commands, what the preview computes, the PBIR / PBIX round trip
//! and hostile parameters.

use serde_json::{Value, json};

use crate::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Runs a command that must fail and checks the report did not change.
fn fails(s: &mut Session, id: &str, params: Value) -> String {
    let before = s.report.clone();
    let err = s
        .execute(id, &params)
        .expect_err(&format!("{id} {params} should fail"));
    assert_eq!(s.report, before, "{id} changed the report although it failed");
    err
}

/// Sales(Product, Region, Month, Amount): Bike sells 100 in month 1 and 50 in month 2 (North,
/// South), 25 more in month 3; Car 300 in month 1; Ball 50 in month 2.
fn shop() -> Session {
    let mut s = Session::new();
    let sales = r#"#table({"Product", "Region", "Month", "Amount"}, {{"Bike", "North", 1, 100}, {"Bike", "South", 2, 50}, {"Bike", "South", 3, 25}, {"Car", "North", 1, 300}, {"Ball", "South", 2, 50}})"#;
    let steps = [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Product", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Region", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Month", "data_type": "int64"}),
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
        ("page.add", json!({"display_name": "Main"})),
    ];
    for (id, p) in steps {
        run(&mut s, id, p);
    }
    s
}

fn add_visual(s: &mut Session, visual_type: &str, fields: Value) -> String {
    let r = run(
        s,
        "visual.add",
        json!({"page": "Main", "visual_type": visual_type, "x": 0, "y": 0, "width": 400, "height": 240,
               "fields": fields}),
    );
    r["visual"].as_str().unwrap_or_default().to_string()
}

fn table(s: &mut Session) -> String {
    add_visual(
        s,
        "tableEx",
        json!([{"role": "Values", "field": "Sales[Product]"},
               {"role": "Values", "field": "Sum(Sales[Amount])"}]),
    )
}

/// The snapshot entry of a visual.
fn visual(s: &Session, name: &str) -> Value {
    let snap = s.snapshot();
    for p in snap["report"]["pages"].as_array().cloned().unwrap_or_default() {
        for v in p["visuals"].as_array().cloned().unwrap_or_default() {
            if v["name"] == name {
                return v;
            }
        }
    }
    panic!("visual {name} not in the snapshot")
}

fn data(s: &Session, name: &str) -> Value {
    let d = visual(s, name)["data"].clone();
    assert_eq!(d["status"], "ok", "{d}");
    d
}

/// Row `key` (first cell) of `d`.
fn row<'a>(d: &'a Value, key: &str) -> &'a Value {
    d["rows"]
        .as_array()
        .and_then(|rows| rows.iter().find(|r| r[0] == key))
        .unwrap_or_else(|| panic!("no row {key} in {d}"))
}

fn the_visual(s: &Session, name: &str) -> bicraft_report::Visual {
    s.report
        .as_ref()
        .and_then(|r| r.pages.iter().flat_map(|p| &p.visuals).find(|v| v.name == name))
        .cloned()
        .unwrap_or_else(|| panic!("no visual {name}"))
}

#[test]
fn data_format_writes_the_projection_format_and_the_values_show_it() {
    let mut s = shop();
    let t = table(&mut s);
    let set = |s: &mut Session, p: Value| {
        let mut base = json!({"page": "Main", "visual": t, "role": "Values", "field": "Sum(Sales[Amount])"});
        if let (Some(b), Some(o)) = (base.as_object_mut(), p.as_object()) {
            b.extend(o.clone());
        }
        run(s, "visual.set_data_format", base)
    };
    let cases = [
        (json!({"type": "decimal", "decimals": 1}), "#,0.0"),
        (json!({"type": "whole", "thousands": false}), "0"),
        // as Power BI Desktop 2.159.1231.0's card writes or recognizes them
        // (docs/formats/sparklines.md "Data format in Desktop")
        (json!({"type": "percentage", "decimals": 0}), "0%"),
        (json!({"type": "percentage", "thousands": true}), "#,0.00%"),
        (
            json!({"type": "currency", "currency_symbol": "€"}),
            "#,0.00 \"€\";-#,0.00 \"€\";#,0.00 \"€\"",
        ),
        (
            json!({"type": "currency", "decimals": 1}),
            "\\$#,0.0;(\\$#,0.0);\\$#,0.0",
        ),
        (
            json!({"type": "currency", "thousands": false}),
            "\\$0.00;(\\$0.00);\\$0.00",
        ),
        (
            json!({"type": "currency", "currency_symbol": "CHF", "decimals": 0}),
            "\"CHF \"#,0",
        ),
        (json!({"type": "scientific", "decimals": 3}), "0.000E+000"),
        (json!({"type": "general"}), "G"),
        (json!({"type": "date"}), "d"),
        (json!({"type": "date", "date_format": "Long Date"}), "D"),
        (json!({"type": "date", "date_format": "dd/MM/yyyy"}), "dd/MM/yyyy"),
        (json!({"format": "0.0 \"u\""}), "0.0 \"u\""),
    ];
    for (p, want) in cases {
        let out = set(&mut s, p.clone());
        assert_eq!(out["format"], want, "{p}");
        // the projection's visual-scoped format, the snapshot and the data column
        let v = the_visual(&s, &t);
        let q = bicraft_report::query::effective(&v);
        assert_eq!(
            bicraft_report::query::projection_formats(q.as_ref()),
            vec![None, Some(want.to_string())]
        );
        assert_eq!(visual(&s, &t)["fields"][1]["format"], want);
        let d = data(&s, &t);
        assert_eq!(d["columns"][1]["format_string"], want, "{d}");
        assert_eq!(d["columns"][1]["visual_format"], true);
        assert!(d["columns"][0].get("visual_format").is_none());
    }
    // auto removes it; the same again is refused
    let out = set(&mut s, json!({"type": "auto"}));
    assert_eq!(out["format"], Value::Null);
    assert!(visual(&s, &t)["fields"][1].get("format").is_none());
    let e = fails(
        &mut s,
        "visual.set_data_format",
        json!({"page": "Main", "visual": t, "role": "Values", "index": 1, "type": "auto"}),
    );
    assert!(e.contains("no data format"), "{e}");
    // the model is untouched
    let m = s.model.as_ref().unwrap();
    assert!(m.tables[0].column("Amount").unwrap().format_string.is_none());
}

#[test]
fn data_format_refuses_what_it_cannot_do() {
    let mut s = shop();
    let t = table(&mut s);
    let base = json!({"page": "Main", "visual": t, "role": "Values", "field": "Sum(Sales[Amount])"});
    let with = |extra: Value| {
        let mut p = base.clone();
        if let (Some(b), Some(o)) = (p.as_object_mut(), extra.as_object()) {
            b.extend(o.clone());
        }
        p
    };
    for (extra, why) in [
        (json!({}), "give `type`"),
        (json!({"type": "nope"}), "must be one of"),
        (json!({"type": "whole", "decimals": 2}), "does not apply"),
        (json!({"type": "decimal", "decimals": 99}), "0 to 15"),
        (json!({"type": "date", "currency_symbol": "$"}), "currency only"),
        (json!({"type": "decimal", "format": "0"}), "not both"),
        (json!({"format": "x".repeat(300)}), "at most 255"),
        (json!({"format": "a\u{7}b"}), "control"),
        (json!({"type": "currency", "currency_symbol": ""}), "1 to 10"),
        (json!({"type": "decimal", "field": "Sales[Nope]"}), "not bound"),
        (json!({"type": "decimal", "role": "Bad role"}), "data role"),
        (json!({"type": "decimal", "bogus": 1}), "bogus"),
    ] {
        let e = fails(&mut s, "visual.set_data_format", with(extra.clone()));
        assert!(e.contains(why), "{extra}: {e}");
    }
}

/// Adds a sparkline of Sum(Amount) by Month to `t`.
fn spark(s: &mut Session, t: &str, extra: Value) -> Value {
    let mut p = json!({"page": "Main", "visual": t, "y": "Sales[Amount]", "x": "Sales[Month]"});
    if let (Some(b), Some(o)) = (p.as_object_mut(), extra.as_object()) {
        b.extend(o.clone());
    }
    run(s, "visual.add_sparkline", p)
}

#[test]
fn a_table_sparkline_has_each_rows_points_in_x_order() {
    let mut s = shop();
    let t = table(&mut s);
    let out = spark(
        &mut s,
        &t,
        json!({"chart_type": "column", "color": "#118DFF", "markers": ["highest", "last"], "width": 2}),
    );
    assert_eq!(
        out["query_ref"], "SparklineData(Sum(Sales.Amount)_[Sales.Month])",
        "{out}"
    );
    assert_eq!(out["index"], 0);
    // the query: a SparklineData projection in Values, its look selected by its queryRef
    let v = the_visual(&s, &t);
    let q = bicraft_report::query::effective(&v).unwrap();
    let p = &q["queryState"]["Values"]["projections"][2];
    // as Desktop 2.159.1231.0's Add dialog writes it: no PointsPerSparkline, "Point"
    assert!(
        p["field"]["SparklineData"].get("PointsPerSparkline").is_none(),
        "{q}"
    );
    assert_eq!(p["field"]["SparklineData"]["ApplyCalculationGroupTo"], "Point");
    assert_eq!(
        p["field"]["SparklineData"]["Measure"]["Aggregation"]["Function"],
        0
    );
    let objects = &v.visual_extra["objects"]["sparklines"][0];
    assert_eq!(objects["selector"]["metadata"], out["query_ref"]);
    assert_eq!(
        objects["properties"]["chartType"]["expr"]["Literal"]["Value"],
        "'column'"
    );
    assert_eq!(
        objects["properties"]["markers"]["expr"]["Literal"]["Value"],
        "18D"
    );
    // the snapshot names it
    let f = &visual(&s, &t)["fields"][2];
    assert_eq!(f["kind"], "sparkline", "{f}");
    assert_eq!(f["label"], "Sum of Amount by Month");
    assert_eq!(f["sparkline"]["x"], "Sales[Month]");
    // the data: a third column with the points of each product, in month order
    let d = data(&s, &t);
    assert_eq!(d["columns"].as_array().map(Vec::len), Some(3), "{d}");
    assert_eq!(d["columns"][2]["kind"], "sparkline");
    assert_eq!(d["columns"][2]["sparkline"]["chart_type"], "column");
    assert_eq!(d["columns"][2]["sparkline"]["color"], "#118DFF");
    assert_eq!(d["columns"][2]["sparkline"]["markers"], 18.0);
    assert_eq!(
        row(&d, "Bike")[2],
        json!({"sparkline": [[1, 100], [2, 50], [3, 25]]}),
        "{d}"
    );
    assert_eq!(row(&d, "Car")[2], json!({"sparkline": [[1, 300]]}));
    assert_eq!(row(&d, "Ball")[2], json!({"sparkline": [[2, 50]]}));
    // the values beside it are unchanged
    assert_eq!(row(&d, "Bike")[1], json!(175));
    // the auto title leaves it out
    assert_eq!(visual(&s, &t)["auto_title"], "Product and Sum of Amount");
}

#[test]
fn a_matrix_sparkline_follows_its_rows_and_columns() {
    let mut s = shop();
    let m = add_visual(
        &mut s,
        "pivotTable",
        json!([{"role": "Rows", "field": "Sales[Product]"},
               {"role": "Values", "field": "[Total]"}]),
    );
    run(
        &mut s,
        "visual.add_sparkline",
        json!({"page": "Main", "visual": m, "y": "[Total]", "x": "Sales[Month]"}),
    );
    let d = data(&s, &m);
    assert_eq!(
        row(&d, "Bike")[2],
        json!({"sparkline": [[1, 100], [2, 50], [3, 25]]}),
        "{d}"
    );
    // subtotals and totals have the sparkline in its place
    for l in d["subtotals"].as_array().into_iter().flatten() {
        for r in l["data"].as_array().into_iter().flatten() {
            assert_eq!(r.as_array().map(Vec::len), Some(3), "{l}");
            assert!(r[2].get("sparkline").is_some(), "{l}");
        }
    }
    // a measure is not summarized again
    let e = fails(
        &mut s,
        "visual.add_sparkline",
        json!({"page": "Main", "visual": m, "y": "[Total]", "summarization": "Average", "x": "Sales[Region]"}),
    );
    assert!(e.contains("not summarized"), "{e}");
}

/// Desktop 2.159.1231.0 draws a sparkline in a table's and a matrix's Total row
/// (docs/formats/sparklines.md "Seen in Desktop"): the measure per X value in the total's
/// filter context, and in a matrix's subtotals, per member of the levels they total.
#[test]
fn the_total_row_and_subtotals_have_their_sparklines() {
    let mut s = shop();
    let t = table(&mut s);
    spark(&mut s, &t, json!({}));
    let d = data(&s, &t);
    let tot = d["totals"].as_array().cloned().unwrap_or_default();
    assert_eq!(tot.len(), 3, "{d}");
    assert_eq!(tot[1], json!(525), "{d}");
    assert_eq!(tot[2], json!({"sparkline": [[1, 400], [2, 100], [3, 25]]}), "{d}");
    // a matrix: Product in rows, Region in columns
    let m = add_visual(
        &mut s,
        "pivotTable",
        json!([{"role": "Rows", "field": "Sales[Product]"},
               {"role": "Columns", "field": "Sales[Region]"},
               {"role": "Values", "field": "[Total]"}]),
    );
    run(
        &mut s,
        "visual.add_sparkline",
        json!({"page": "Main", "visual": m, "y": "[Total]", "x": "Sales[Month]"}),
    );
    let d = data(&s, &m);
    let level = |r: u64, c: u64| -> Vec<Value> {
        d["subtotals"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|l| l["rows"] == r && l["columns"] == c)
            .and_then(|l| l["data"].as_array().cloned())
            .unwrap_or_else(|| panic!("no level {r},{c} in {d}"))
    };
    let find = |rows: &[Value], at: usize, key: &str| -> Value {
        rows.iter()
            .find(|r| r[at] == key)
            .map(|r| r[3].clone())
            .unwrap_or_else(|| panic!("no {key} in {rows:?}"))
    };
    // a product's row total, over its regions
    assert_eq!(
        find(&level(1, 0), 0, "Bike"),
        json!({"sparkline": [[1, 100], [2, 50], [3, 25]]})
    );
    // a region's column total, over its products
    assert_eq!(
        find(&level(0, 1), 1, "South"),
        json!({"sparkline": [[2, 100], [3, 25]]})
    );
    // the grand total
    assert_eq!(
        level(0, 0)[0][3],
        json!({"sparkline": [[1, 400], [2, 100], [3, 25]]})
    );
}

#[test]
fn edit_sparkline_changes_axis_and_look_and_keeps_its_query_ref() {
    let mut s = shop();
    let t = table(&mut s);
    let added = spark(&mut s, &t, json!({}));
    let out = run(
        &mut s,
        "visual.edit_sparkline",
        json!({"page": "Main", "visual": t, "summarization": "Max", "chart_type": "line", "marker_shape": "diamond"}),
    );
    assert_eq!(out["query_ref"], added["query_ref"]);
    assert_eq!(out["y"], "Max(Sales[Amount])", "{out}");
    let d = data(&s, &t);
    assert_eq!(
        row(&d, "Bike")[2],
        json!({"sparkline": [[1, 100], [2, 50], [3, 25]]})
    );
    assert_eq!(d["columns"][2]["sparkline"]["marker_shape"], "diamond");
    // X by region: Bike has North 100 and South 75
    run(
        &mut s,
        "visual.edit_sparkline",
        json!({"page": "Main", "visual": t, "x": "Sales[Region]", "summarization": "Sum", "color": null}),
    );
    let d = data(&s, &t);
    assert_eq!(
        row(&d, "Bike")[2],
        json!({"sparkline": [["North", 100], ["South", 75]]}),
        "{d}"
    );
    // nothing to change, a second index
    let e = fails(
        &mut s,
        "visual.edit_sparkline",
        json!({"page": "Main", "visual": t}),
    );
    assert!(e.contains("nothing to change"), "{e}");
    let e = fails(
        &mut s,
        "visual.edit_sparkline",
        json!({"page": "Main", "visual": t, "index": 3, "chart_type": "line"}),
    );
    assert!(e.contains("past"), "{e}");
    // removing it is visual.unbind
    run(
        &mut s,
        "visual.unbind",
        json!({"page": "Main", "visual": t, "role": "Values", "index": 2}),
    );
    assert_eq!(data(&s, &t)["columns"].as_array().map(Vec::len), Some(2));
}

#[test]
fn add_sparkline_refuses_what_it_cannot_do() {
    let mut s = shop();
    let t = table(&mut s);
    let chart = add_visual(
        &mut s,
        "clusteredColumnChart",
        json!([{"role": "Category", "field": "Sales[Product]"}, {"role": "Y", "field": "[Total]"}]),
    );
    let p = |v: &str, extra: Value| {
        let mut p = json!({"page": "Main", "visual": v, "y": "Sales[Amount]", "x": "Sales[Month]"});
        if let (Some(b), Some(o)) = (p.as_object_mut(), extra.as_object()) {
            b.extend(o.clone());
        }
        p
    };
    for (v, extra, why) in [
        (&chart, json!({}), "table or a matrix"),
        (&t, json!({"x": "[Total]"}), "is a column"),
        (&t, json!({"x": "Sales[Nope]"}), "no column"),
        (&t, json!({"summarization": "Bogus"}), "summarization"),
        (&t, json!({"chart_type": "pie"}), "chart_type"),
        (&t, json!({"color": "red"}), "colour"),
        (&t, json!({"width": 99}), "0.5 to 10"),
        (&t, json!({"markers": ["peak"]}), "markers"),
        (&t, json!({"markers": "all"}), "markers"),
        (&t, json!({"marker_shape": "star"}), "marker_shape"),
        (
            &t,
            json!({"apply_calculation_group_to": "half"}),
            "apply_calculation_group_to",
        ),
        (&t, json!({"bogus": true}), "bogus"),
    ] {
        let e = fails(&mut s, "visual.add_sparkline", p(v, extra.clone()));
        assert!(e.contains(why), "{extra}: {e}");
    }
    spark(&mut s, &t, json!({}));
    let e = fails(&mut s, "visual.add_sparkline", p(&t, json!({})));
    assert!(e.contains("already has a sparkline"), "{e}");
    for x in ["Sales[Region]", "Sales[Product]", "Sales[Amount]"] {
        spark(&mut s, &t, json!({"x": x, "summarization": "Count"}));
    }
    spark(&mut s, &t, json!({"summarization": "Min"}));
    let e = fails(
        &mut s,
        "visual.add_sparkline",
        p(&t, json!({"summarization": "Max"})),
    );
    assert!(e.contains("already has 5"), "{e}");
    // the X axis on the visual's own rows: each row has one point, flagged
    let d = data(&s, &t);
    let product = d["columns"]
        .as_array()
        .and_then(|c| c.iter().find(|c| c["name"] == "Count of Amount by Product"))
        .unwrap_or_else(|| panic!("{d}"));
    assert!(
        product["reason"]
            .as_str()
            .unwrap_or_default()
            .contains("one point"),
        "{product}"
    );
    // the data format refuses a sparkline
    let e = fails(
        &mut s,
        "visual.set_data_format",
        json!({"page": "Main", "visual": t, "role": "Values", "index": 2, "type": "decimal"}),
    );
    assert!(e.contains("sparkline"), "{e}");
}

#[test]
fn sparklines_and_formats_survive_pbip_and_pbix() {
    let mut s = shop();
    let t = table(&mut s);
    spark(&mut s, &t, json!({"chart_type": "column"}));
    run(
        &mut s,
        "visual.set_data_format",
        json!({"page": "Main", "visual": t, "role": "Values", "index": 1, "type": "decimal", "decimals": 1}),
    );
    let orig = the_visual(&s, &t);
    let dir = crate::tempdir::TempDir::new("engine-sparklines");
    let pbip = dir.path().join("Shop.pbip");
    let pbix = dir.path().join("Shop.pbix");
    run(&mut s, "file.save_as", json!({"path": pbip.to_string_lossy()}));
    run(&mut s, "file.save_as", json!({"path": pbix.to_string_lossy()}));
    for file in [pbip, pbix] {
        let mut back = Session::new();
        run(&mut back, "file.open", json!({"path": file.to_string_lossy()}));
        let v = the_visual(&back, &t);
        assert_eq!(v.fields, orig.fields, "{}", file.display());
        let q = bicraft_report::query::effective(&v);
        assert_eq!(
            bicraft_report::query::projection_formats(q.as_ref()),
            vec![None, Some("#,0.0".to_string()), None],
            "{}",
            file.display()
        );
        let d = data(&back, &t);
        assert_eq!(d["columns"][1]["format_string"], "#,0.0", "{}", file.display());
        assert_eq!(
            d["columns"][2]["sparkline"]["chart_type"],
            "column",
            "{}",
            file.display()
        );
        assert_eq!(
            row(&d, "Car")[2],
            json!({"sparkline": [[1, 300]]}),
            "{}",
            file.display()
        );
    }
}

/// Hostile parameters give an error, never a panic, and leave the report alone.
#[test]
fn hostile_parameters_are_refused() {
    let mut s = shop();
    let t = table(&mut s);
    for id in [
        "visual.set_data_format",
        "visual.add_sparkline",
        "visual.edit_sparkline",
    ] {
        for p in [
            json!(null),
            json!([]),
            json!({"page": 1}),
            json!({"page": "Main", "visual": t, "y": {"a": 1}, "x": [1]}),
            json!({"page": "Main", "visual": t, "role": "Values", "index": -1, "type": "decimal"}),
            json!({"page": "Main", "visual": t, "markers": [1, 2, 3, 4, 5, 6, 7], "y": "Sales[Amount]", "x": "Sales[Month]"}),
            json!({"page": "Main", "visual": t, "width": "wide", "y": "Sales[Amount]", "x": "Sales[Month]"}),
            json!({"page": "Nope", "visual": t, "role": "Values", "index": 0, "type": "decimal", "y": "Sales[Amount]", "x": "Sales[Month]"}),
        ] {
            let _ = fails(&mut s, id, p);
        }
    }
}
