//! `measure.quick` (Quick measures): each calculation writes an ordinary measure that the DAX
//! binder accepts and the evaluator computes, on a small synthetic model (a sales table and a
//! date table related to it); refusals change nothing; one undo step.

use serde_json::{Value, json};

use bicraft_engine::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Sales(Date, Product, Amount, Qty) related to Dates(Date) (every day of 2024 and 2025), and
/// [Total].
fn shop() -> Session {
    let mut s = Session::new();
    let sales = r#"#table(type table [Date = date, Product = text, Amount = number, Qty = Int64.Type, Customer = text], {
        {#date(2024, 1, 5), "Bike", 100, 1, "Ann"}, {#date(2024, 2, 5), "Bike", 50, 2, "Bob"},
        {#date(2024, 4, 10), "Car", 1000, 1, "Ann"}, {#date(2025, 1, 7), "Ball", 5, 5, "Cid"},
        {#date(2025, 3, 9), "Kite", 42, 2, "Bob"}, {#date(2025, 8, 1), "Doll", 18, 3, "Dan"}})"#;
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Date", "data_type": "dateTime"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Product", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Qty", "data_type": "int64"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Customer", "data_type": "string"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        (
            "table.import",
            json!({"name": "Dates", "dax": "CALENDAR(DATE(2024, 1, 1), DATE(2025, 12, 31))"}),
        ),
        (
            "relationship.add",
            json!({"from": "Sales[Date]", "to": "Dates[Date]"}),
        ),
    ] {
        run(&mut s, id, p);
    }
    s
}

/// Adds quick measure `kind` and returns its name.
fn quick(s: &mut Session, kind: &str, params: Value) -> String {
    let r = run(s, "measure.quick", json!({"kind": kind, "params": params}));
    r["measure"].as_str().unwrap().to_string()
}

/// The value of `expression` in an empty filter context.
fn value(s: &mut Session, expression: &str) -> Value {
    let q = format!("EVALUATE ROW(\"v\", {expression})");
    let v = run(s, "dax.evaluate", json!({ "query": q }));
    v["results"][0]["rows"][0]["[v]"].clone()
}

fn on(s: &mut Session, measure: &str, from: &str, to: &str) -> Value {
    value(
        s,
        &format!("CALCULATE([{measure}], DATESBETWEEN('Dates'[Date], DATE({from}), DATE({to})))"),
    )
}

fn close(v: &Value, want: f64) -> bool {
    v.as_f64().is_some_and(|x| (x - want).abs() < 1e-9)
}

#[test]
fn aggregates_per_category() {
    let mut s = shop();
    let r = run(
        &mut s,
        "measure.quick",
        json!({"kind": "average_per_category", "params": {"base": "[Total]", "category": "Sales[Product]"}}),
    );
    assert_eq!(r["measure"], "Total average per Product");
    assert_eq!(r["table"], "Sales");
    let m = s
        .model
        .as_ref()
        .unwrap()
        .measure("Total average per Product")
        .unwrap()
        .1
        .clone();
    assert_eq!(m.expression, r["expression"].as_str().unwrap());
    // the product totals 150, 1000, 5, 42, 18
    assert!(close(&value(&mut s, "[Total average per Product]"), 243.0));
    let max = quick(
        &mut s,
        "max_per_category",
        json!({"base": "[Total]", "category": "Sales[Product]"}),
    );
    assert!(close(&value(&mut s, &format!("[{max}]")), 1000.0));
    let min = quick(
        &mut s,
        "min_per_category",
        json!({"base": "[Total]", "category": "Sales[Product]"}),
    );
    assert!(close(&value(&mut s, &format!("[{min}]")), 5.0));
    let var = quick(
        &mut s,
        "variance_per_category",
        json!({"base": "[Total]", "category": "Sales[Product]"}),
    );
    let mean = 243.0;
    let want = [150.0f64, 1000.0, 5.0, 42.0, 18.0]
        .iter()
        .map(|x| (x - mean) * (x - mean))
        .sum::<f64>()
        / 5.0;
    assert!(close(&value(&mut s, &format!("[{var}]")), want));
    // weighted by quantity: Σ(total × qty) / Σ qty per product
    let w = quick(
        &mut s,
        "weighted_average_per_category",
        json!({"base": "[Total]", "weight": "Sales[Qty]", "category": "Sales[Product]"}),
    );
    assert_eq!(w, "Total weighted by Sum of Qty per Product");
    let want = (150.0 * 3.0 + 1000.0 + 5.0 * 5.0 + 42.0 * 2.0 + 18.0 * 3.0) / 14.0;
    assert!(close(&value(&mut s, &format!("[{w}]")), want));
}

#[test]
fn filtered_values_and_their_differences() {
    let mut s = shop();
    let f = quick(
        &mut s,
        "filtered_value",
        json!({"base": "[Total]", "filter": "Sales[Product]", "values": ["Bike", "Car"]}),
    );
    assert_eq!(f, "Total for Bike, Car");
    assert!(close(&value(&mut s, &format!("[{f}]")), 1150.0));
    let d = quick(
        &mut s,
        "difference_from_filtered_value",
        json!({"base": "[Total]", "filter": "Sales[Product]", "values": ["Bike"]}),
    );
    assert!(close(&value(&mut s, &format!("[{d}]")), 1215.0 - 150.0));
    let p = quick(
        &mut s,
        "percentage_difference_from_filtered_value",
        json!({"base": "Sum(Sales[Amount])", "filter": "Sales[Product]", "values": ["Bike"]}),
    );
    assert_eq!(p, "Sum of Amount % difference from Bike");
    assert!(close(&value(&mut s, &format!("[{p}]")), (1215.0 - 150.0) / 150.0));
    let m = s.model.as_ref().unwrap().measure(&p).unwrap().1.clone();
    assert_eq!(m.format_string.as_deref(), Some("0.00%"));
}

#[test]
fn time_intelligence() {
    let mut s = shop();
    let p = json!({"base": "[Total]", "date": "Dates[Date]"});
    let ytd = quick(&mut s, "year_to_date", p.clone());
    assert_eq!(ytd, "Total YTD");
    // on 2024-02-05: January and February 2024
    assert!(close(&on(&mut s, &ytd, "2024, 2, 5", "2024, 2, 5"), 150.0));
    assert!(close(&on(&mut s, &ytd, "2025, 12, 31", "2025, 12, 31"), 65.0));
    let qtd = quick(&mut s, "quarter_to_date", p.clone());
    assert!(close(&on(&mut s, &qtd, "2024, 4, 30", "2024, 4, 30"), 1000.0));
    let mtd = quick(&mut s, "month_to_date", p.clone());
    assert!(close(&on(&mut s, &mtd, "2024, 2, 28", "2024, 2, 28"), 50.0));
    // 2025 against 2024
    let yoy = quick(&mut s, "year_over_year_change", p.clone());
    assert_eq!(yoy, "Total YoY%");
    assert!(close(
        &on(&mut s, &yoy, "2025, 1, 1", "2025, 12, 31"),
        (65.0 - 1150.0) / 1150.0
    ));
    let mom = quick(&mut s, "month_over_month_change", p.clone());
    // February 2024 (50) against January (100)
    assert!(close(&on(&mut s, &mom, "2024, 2, 1", "2024, 2, 29"), -0.5));
    let qoq = quick(&mut s, "quarter_over_quarter_change", p);
    // Q2 2024 (1000) against Q1 (150)
    assert!(close(
        &on(&mut s, &qoq, "2024, 4, 1", "2024, 6, 30"),
        (1000.0 - 150.0) / 150.0
    ));
    // the average of the days with sales from three months back (excluded) to the last date
    let roll = quick(
        &mut s,
        "rolling_average",
        json!({"base": "[Total]", "date": "Dates[Date]", "period": "month", "before": 3}),
    );
    assert_eq!(roll, "Total rolling average");
    assert!(close(
        &on(&mut s, &roll, "2024, 4, 10", "2024, 4, 10"),
        (50.0 + 1000.0) / 2.0
    ));
    // two days back and one on, around 2025-03-08: only 2025-03-09 sells
    let days = quick(
        &mut s,
        "rolling_average",
        json!({"base": "[Total]", "date": "Dates[Date]", "period": "day", "before": 2, "after": 1}),
    );
    assert!(close(&on(&mut s, &days, "2025, 3, 8", "2025, 3, 8"), 42.0));
}

#[test]
fn totals_and_arithmetic() {
    let mut s = shop();
    let run_total = quick(
        &mut s,
        "running_total",
        json!({"base": "[Total]", "field": "Sales[Product]"}),
    );
    assert_eq!(run_total, "Total running total in Product");
    // Ball 5, Bike 150, Car 1000, Doll 18, Kite 42: up to Car, 1155
    assert!(close(
        &value(
            &mut s,
            &format!("CALCULATE([{run_total}], Sales[Product] = \"Car\")")
        ),
        1155.0
    ));
    let down = quick(
        &mut s,
        "running_total",
        json!({"base": "[Total]", "field": "Sales[Product]", "descending": true}),
    );
    assert!(close(
        &value(&mut s, &format!("CALCULATE([{down}], Sales[Product] = \"Doll\")")),
        60.0
    ));
    let all = quick(
        &mut s,
        "total_for_category_unfiltered",
        json!({"base": "[Total]", "category": "Sales[Product]"}),
    );
    assert!(close(
        &value(&mut s, &format!("CALCULATE([{all}], Sales[Product] = \"Car\")")),
        1215.0
    ));
    let sel = quick(
        &mut s,
        "total_for_category_filtered",
        json!({"base": "[Total]", "category": "Sales[Product]"}),
    );
    // the products selected (a filter of the query, as a slicer's), whatever the row's product
    let q = format!(
        "EVALUATE SUMMARIZECOLUMNS(Sales[Product], TREATAS({{\"Bike\", \"Car\"}}, Sales[Product]), \"v\", [{sel}])"
    );
    let rows = run(&mut s, "dax.evaluate", json!({ "query": q }))["results"][0]["rows"].clone();
    let rows = rows.as_array().cloned().unwrap_or_default();
    assert_eq!(rows.len(), 2, "{rows:?}");
    for r in &rows {
        assert!(close(&r["[v]"], 1150.0), "{rows:?}");
    }
    for (kind, want) in [
        ("addition", 1215.0 + 14.0),
        ("subtraction", 1215.0 - 14.0),
        ("multiplication", 1215.0 * 14.0),
        ("division", 1215.0 / 14.0),
        ("percentage_difference", (1215.0 - 14.0) / 14.0),
    ] {
        let m = quick(&mut s, kind, json!({"base": "[Total]", "second": "Sales[Qty]"}));
        assert!(close(&value(&mut s, &format!("[{m}]")), want), "{kind}");
    }
}

#[test]
fn stars_and_lists() {
    let mut s = shop();
    let star = quick(
        &mut s,
        "star_rating",
        json!({"base": "[Total]", "max_value": 2000}),
    );
    assert_eq!(star, "Total star rating");
    // 1215 / 2000 × 5 = 3.04 → 3 stars of 5
    assert_eq!(value(&mut s, &format!("[{star}]")), json!("★★★☆☆"));
    assert_eq!(
        value(&mut s, &format!("CALCULATE([{star}], Sales[Product] = \"Nope\")")),
        Value::Null
    );
    let list = quick(&mut s, "concatenated_list", json!({"field": "Sales[Product]"}));
    assert_eq!(list, "List of Product values");
    assert_eq!(
        value(&mut s, &format!("[{list}]")),
        json!("Ball, Bike, Car, etc.")
    );
    assert_eq!(
        value(
            &mut s,
            &format!("CALCULATE([{list}], Sales[Product] IN {{\"Kite\", \"Car\"}})")
        ),
        json!("Car, Kite")
    );
}

#[test]
fn names_tables_undo_and_refusals() {
    let mut s = shop();
    let before = s.model.clone();
    let a = quick(
        &mut s,
        "year_to_date",
        json!({"base": "[Total]", "date": "Dates[Date]"}),
    );
    let b = quick(
        &mut s,
        "year_to_date",
        json!({"base": "[Total]", "date": "Dates[Date]"}),
    );
    assert_eq!((a.as_str(), b.as_str()), ("Total YTD", "Total YTD 2"));
    let r = run(
        &mut s,
        "measure.quick",
        json!({"kind": "year_to_date", "name": "Cumul", "table": "Dates",
               "params": {"base": "[Total]", "date": "Dates[Date]"}}),
    );
    assert_eq!(
        (r["measure"].clone(), r["table"].clone()),
        (json!("Cumul"), json!("Dates"))
    );
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.model, before);
    for (p, why) in [
        (json!({"kind": "nope", "params": {}}), "unknown quick measure"),
        (
            json!({"kind": "year_to_date", "params": {"base": "[Total]"}}),
            "params.date",
        ),
        (
            json!({"kind": "year_to_date", "params": {"base": "[Total]", "date": "Sales[Product]"}}),
            "not a date column",
        ),
        (
            json!({"kind": "average_per_category", "params": {"base": "[Nope]", "category": "Sales[Product]"}}),
            "params.base",
        ),
        (
            json!({"kind": "average_per_category", "params": {"base": "[Total]", "category": "[Total]"}}),
            "not a column",
        ),
        (
            json!({"kind": "filtered_value", "params": {"base": "[Total]", "filter": "Sales[Product]", "values": []}}),
            "params.values",
        ),
        (
            json!({"kind": "filtered_value", "params": {"base": "[Total]", "filter": "Sales[Amount]", "values": ["x"]}}),
            "does not fit",
        ),
        (
            json!({"kind": "star_rating", "params": {"base": "[Total]", "max_value": 0}}),
            "greater than",
        ),
        (
            json!({"kind": "year_to_date", "params": {"base": "[Total]", "date": "Dates[Date]", "extra": 1}}),
            "unknown parameter",
        ),
        (
            json!({"kind": "year_to_date", "name": "Total", "params": {"base": "[Total]", "date": "Dates[Date]"}}),
            "already exists",
        ),
    ] {
        let e = s.execute("measure.quick", &p).unwrap_err();
        assert!(e.contains(why), "{p}: {e}");
        assert_eq!(s.model, before, "{p}");
    }
}

#[test]
fn sales_from_new_customers() {
    let mut s = shop();
    let n = quick(
        &mut s,
        "sales_from_new_customers",
        json!({"base": "[Total]", "customer": "Sales[Customer]", "date": "Dates[Date]"}),
    );
    assert_eq!(n, "Total from new customers");
    // 2024: Ann and Bob buy for the first time (100 + 50 + 1000); 2025: Cid and Dan (5 + 18),
    // Bob bought before
    assert!(close(&on(&mut s, &n, "2024, 1, 1", "2024, 12, 31"), 1150.0));
    assert!(close(&on(&mut s, &n, "2025, 1, 1", "2025, 12, 31"), 23.0));
    assert!(close(&on(&mut s, &n, "2024, 2, 1", "2024, 2, 29"), 50.0));
    // Ann's second purchase is no new customer's
    assert_eq!(on(&mut s, &n, "2024, 4, 1", "2024, 4, 30"), Value::Null);
    assert!(close(&value(&mut s, &format!("[{n}]")), 1215.0));
    // other filters do not make an old customer new: Bob's kite in 2025
    assert_eq!(
        value(
            &mut s,
            &format!(
                "CALCULATE([{n}], Sales[Product] = \"Kite\", DATESBETWEEN('Dates'[Date], DATE(2025, 1, 1), DATE(2025, 12, 31)))"
            )
        ),
        Value::Null
    );
    // on the sales table's own dates (gaps between them): the same customers
    let f = quick(
        &mut s,
        "sales_from_new_customers",
        json!({"base": "[Total]", "customer": "Sales[Customer]", "date": "Sales[Date]"}),
    );
    assert!(close(
        &value(
            &mut s,
            &format!("CALCULATE([{f}], DATESBETWEEN(Sales[Date], DATE(2025, 1, 1), DATE(2025, 12, 31)))")
        ),
        23.0
    ));
}

#[test]
fn correlation_coefficient() {
    let mut s = shop();
    let c = quick(
        &mut s,
        "correlation_coefficient",
        json!({"category": "Sales[Product]", "base": "[Total]", "second": "Sales[Qty]"}),
    );
    assert_eq!(c, "Total and Sum of Qty correlation per Product");
    // Ball, Bike, Car, Doll, Kite: totals and quantities
    let x = [5.0f64, 150.0, 1000.0, 18.0, 42.0];
    let y = [5.0f64, 3.0, 1.0, 3.0, 2.0];
    let pearson = |x: &[f64], y: &[f64]| {
        let n = x.len() as f64;
        let (mx, my) = (x.iter().sum::<f64>() / n, y.iter().sum::<f64>() / n);
        let cov: f64 = x.iter().zip(y).map(|(a, b)| (a - mx) * (b - my)).sum();
        let vx: f64 = x.iter().map(|a| (a - mx) * (a - mx)).sum();
        let vy: f64 = y.iter().map(|b| (b - my) * (b - my)).sum();
        cov / (vx.sqrt() * vy.sqrt())
    };
    let got = value(&mut s, &format!("[{c}]"));
    let want = pearson(&x, &y);
    assert!(v_close(&got, want), "{got} instead of {want}");
    // the categories of the context only: 2025 sells Ball, Kite and Doll
    let got = on(&mut s, &c, "2025, 1, 1", "2025, 12, 31");
    let want = pearson(&[5.0, 18.0, 42.0], &[5.0, 3.0, 2.0]);
    assert!(v_close(&got, want), "{got} instead of {want}");
    // one category: no correlation (blank, not an error)
    assert_eq!(
        value(&mut s, &format!("CALCULATE([{c}], Sales[Product] = \"Car\")")),
        Value::Null
    );
}

fn v_close(v: &Value, want: f64) -> bool {
    v.as_f64().is_some_and(|x| (x - want).abs() < 1e-9)
}

#[test]
fn fiscal_year_to_date() {
    let mut s = shop();
    let r = run(
        &mut s,
        "measure.quick",
        json!({"kind": "year_to_date", "params": {"base": "[Total]", "date": "Dates[Date]", "year_end_month": 6}}),
    );
    assert!(
        r["expression"].as_str().is_some_and(|e| e.contains("\"6/30\"")),
        "{r}"
    );
    let m = r["measure"].as_str().unwrap().to_string();
    // the fiscal year from July 2024 to June 2025: on 2025-03-31, 5 + 42
    assert!(close(&on(&mut s, &m, "2025, 3, 31", "2025, 3, 31"), 47.0));
    // July 2023 to June 2024: every 2024 sale up to April
    assert!(close(&on(&mut s, &m, "2024, 4, 30", "2024, 4, 30"), 1150.0));
    // February ends on the 28th
    let r = run(
        &mut s,
        "measure.quick",
        json!({"kind": "year_to_date", "params": {"base": "[Total]", "date": "Dates[Date]", "year_end_month": 2}}),
    );
    assert!(
        r["expression"].as_str().is_some_and(|e| e.contains("\"2/28\"")),
        "{r}"
    );
    for bad in [json!(0), json!(13), json!(6.5)] {
        let e = s
            .execute(
                "measure.quick",
                &json!({"kind": "year_to_date", "params": {"base": "[Total]", "date": "Dates[Date]", "year_end_month": bad}}),
            )
            .unwrap_err();
        assert!(e.contains("year_end_month"), "{e}");
    }
    // only the year-to-date total takes it
    let e = s
        .execute(
            "measure.quick",
            &json!({"kind": "quarter_to_date", "params": {"base": "[Total]", "date": "Dates[Date]", "year_end_month": 6}}),
        )
        .unwrap_err();
    assert!(e.contains("unknown parameter"), "{e}");
}

/// Time intelligence on a fact table's own date column: with auto date/time, the measure walks
/// the auto date table's every date (`T[C].[Date]`); without, it warns that only the dates the
/// column holds are seen.
#[test]
fn time_intelligence_on_a_fact_date_column() {
    let mut s = shop();
    let r = run(
        &mut s,
        "measure.quick",
        json!({"kind": "year_over_year_change", "params": {"base": "[Total]", "date": "Sales[Date]"}}),
    );
    let w = r["warnings"].to_string();
    assert!(w.contains("only sees the dates"), "{r}");
    // a marked date table: no warning
    run(
        &mut s,
        "table.set",
        json!({"name": "Dates", "data_category": "Time", "key_column": "Date"}),
    );
    let r = run(
        &mut s,
        "measure.quick",
        json!({"kind": "month_to_date", "params": {"base": "[Total]", "date": "Dates[Date]"}}),
    );
    assert!(!r["warnings"].to_string().contains("only sees the dates"), "{r}");
    // auto date/time on Sales[Date]: a local date table with every date, related as Desktop does
    run(
        &mut s,
        "table.import",
        json!({"name": "LocalDateTable_1", "dax": "CALENDAR(DATE(2024, 1, 1), DATE(2025, 12, 31))"}),
    );
    run(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[Date]", "to": "LocalDateTable_1[Date]"}),
    );
    {
        let m = s.model.as_mut().unwrap();
        let rel = m
            .relationships
            .iter()
            .find(|r| r.to_table == "LocalDateTable_1")
            .unwrap()
            .name
            .clone();
        let c = m
            .tables
            .iter_mut()
            .find(|t| t.name == "Sales")
            .unwrap()
            .column_mut("Date")
            .unwrap();
        c.extra.insert(
            "variations".into(),
            json!([{"name": "Variation", "relationship": rel, "isDefault": true}]),
        );
    }
    let r = run(
        &mut s,
        "measure.quick",
        json!({"kind": "year_over_year_change", "params": {"base": "[Total]", "date": "Sales[Date]"}}),
    );
    assert!(
        r["expression"]
            .as_str()
            .is_some_and(|e| e.contains("'Sales'[Date].[Date]")),
        "{r}"
    );
    assert!(!r["warnings"].to_string().contains("only sees the dates"), "{r}");
    let yoy = r["measure"].as_str().unwrap().to_string();
    // 2025 (65) against all of 2024 (1150), although no 2025 sale has a 2024 date a year before
    let got = value(
        &mut s,
        &format!(
            "CALCULATE([{yoy}], DATESBETWEEN('LocalDateTable_1'[Date], DATE(2025, 1, 1), DATE(2025, 12, 31)))"
        ),
    );
    assert!(close(&got, (65.0 - 1150.0) / 1150.0), "{got}");
    let ytd = quick(
        &mut s,
        "year_to_date",
        json!({"base": "[Total]", "date": "Sales[Date]"}),
    );
    let got = value(
        &mut s,
        &format!(
            "CALCULATE([{ytd}], DATESBETWEEN('LocalDateTable_1'[Date], DATE(2024, 3, 1), DATE(2024, 3, 31)))"
        ),
    );
    assert!(close(&got, 150.0), "{got}");
}
