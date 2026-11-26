//! Tests of the general preview evaluator (filter context, CALCULATE, time intelligence), on a
//! synthetic model: a 2023-2024 calendar, a small sales table and a product table.
//!
//! Expected values are computed by hand from the rows of `shop()` and were confirmed with Power BI
//! Desktop on the same model saved as a PBIP; the date-shift rules (`DATEADD` runs,
//! `DATESINPERIOD` month ends…) were checked against Desktop too.

use serde_json::{Value, json};

use super::{Interval, fiscal_year, serial, shift_blocks, shift_eom, ymd};
use crate::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn calendar() -> String {
    let mut rows = Vec::new();
    for y in 2023..=2024 {
        for m in 1..=12 {
            let days = match m {
                2 if y == 2024 => 29,
                2 => 28,
                4 | 6 | 9 | 11 => 30,
                _ => 31,
            };
            for d in 1..=days {
                rows.push(format!("{{#date({y},{m},{d}), {y}, {m}, {}}}", y * 100 + m));
            }
        }
    }
    format!(
        "#table({{\"Date\", \"Year\", \"Month\", \"YearMonth\"}}, {{{}}})",
        rows.join(", ")
    )
}

/// Sales (8 rows over 2023-2024; "Kite" has no product row), Product, Calendar.
fn shop() -> Session {
    let mut s = Session::new();
    let sales = r#"#table({"Date", "Product", "Channel", "Amount", "ShipDate"}, {
        {#date(2023,1,10), "Bike", "Web", 100, #date(2023,1,13)},
        {#date(2023,2,15), "Ball", "Store", 20, #date(2023,2,18)},
        {#date(2023,6,30), "Bike", "Store", 200, #date(2023,7,3)},
        {#date(2023,12,31), "Kite", "App", 50, #date(2024,1,3)},
        {#date(2024,1,5), "Bike", "Web", 150, #date(2024,1,8)},
        {#date(2024,2,29), "Ball", "App", 30, #date(2024,3,3)},
        {#date(2024,3,1), "Bike", "Store", 300, #date(2024,3,4)},
        {#date(2024,7,15), "Ball", "Web", 40, #date(2024,7,18)}})"#;
    let product =
        r#"#table({"Product", "Category"}, {{"Bike", "Wheels"}, {"Ball", "Toys"}, {"Car", "Wheels"}})"#;
    let mut steps = vec![
        ("model.new", json!({"name": "Shop"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
        ),
        (
            "table.add",
            json!({"name": "Product", "partition": {"source_type": "m", "expression": product}}),
        ),
        (
            "table.add",
            json!({"name": "Calendar", "partition": {"source_type": "m", "expression": calendar()}}),
        ),
    ];
    for (t, c, ty) in [
        ("Sales", "Date", "dateTime"),
        ("Sales", "Product", "string"),
        ("Sales", "Channel", "string"),
        ("Sales", "Amount", "double"),
        ("Sales", "ShipDate", "dateTime"),
        ("Product", "Product", "string"),
        ("Product", "Category", "string"),
        ("Calendar", "Date", "dateTime"),
        ("Calendar", "Year", "int64"),
        ("Calendar", "Month", "int64"),
        ("Calendar", "YearMonth", "int64"),
    ] {
        steps.push(("column.add", json!({"table": t, "name": c, "data_type": ty})));
    }
    for (from, to, active) in [
        ("Sales[Date]", "Calendar[Date]", true),
        ("Sales[ShipDate]", "Calendar[Date]", false),
        ("Sales[Product]", "Product[Product]", true),
    ] {
        steps.push((
            "relationship.add",
            json!({"from": from, "to": to, "active": active}),
        ));
    }
    for (name, expr) in [
        ("Total", "SUM(Sales[Amount])"),
        ("YTD", "CALCULATE([Total], DATESYTD('Calendar'[Date]))"),
        ("PY", "CALCULATE([Total], SAMEPERIODLASTYEAR('Calendar'[Date]))"),
        (
            "PYTD",
            "CALCULATE([Total], DATESYTD(SAMEPERIODLASTYEAR('Calendar'[Date]), \"31/12\"))",
        ),
        (
            "Delta",
            "VAR a = [Total] VAR b = CALCULATE([Total], SAMEPERIODLASTYEAR('Calendar'[Date])) RETURN IF(NOT ISBLANK(b), a - b)",
        ),
        (
            "Share",
            "VAR v = [Total] VAR t = CALCULATE([Total], REMOVEFILTERS(Product)) RETURN DIVIDE(v, t)",
        ),
        ("AllCat", "CALCULATE([Total], ALL(Product[Category]))"),
        ("Toys", "CALCULATE([Total], Product[Category] = \"Toys\")"),
        (
            "Online",
            "CALCULATE([Total], KEEPFILTERS(Sales[Channel] IN {\"Web\", \"App\"}))",
        ),
        ("Big", "CALCULATE([Total], Sales[Amount] >= 100)"),
        (
            "R12",
            "CALCULATE([Total], DATESINPERIOD('Calendar'[Date], MAX('Calendar'[Date]), -12, MONTH))",
        ),
        ("Twice", "SUMX(Sales, Sales[Amount] * 2)"),
        ("CatSum", "SUMX(VALUES(Product[Category]), [Total])"),
        (
            "Pick",
            "IF(HASONEVALUE(Product[Category]), SELECTEDVALUE(Product[Category]), \"many\")",
        ),
        ("Level", "SWITCH(TRUE(), [Total] > 500, 2, [Total] > 0, 1, 0)"),
        ("Last", "LASTDATE('Calendar'[Date])"),
        ("LastSale", "MAX(Sales[Date])"),
        (
            "Shipped",
            "CALCULATE([Total], USERELATIONSHIP(Sales[ShipDate], 'Calendar'[Date]))",
        ),
        ("YTD2", "TOTALYTD([Total], 'Calendar'[Date])"),
        ("PrevMonth", "CALCULATE([Total], PREVIOUSMONTH('Calendar'[Date]))"),
        (
            "Upto",
            "CALCULATE([Total], FILTER(ALL('Calendar'[Date]), 'Calendar'[Date] <= MAX('Calendar'[Date])))",
        ),
        (
            "LongMonths",
            "COUNTROWS(FILTER(VALUES('Calendar'[YearMonth]), COUNTROWS(DATESMTD('Calendar'[Date])) > 30))",
        ),
        ("Hard", "RAND()"),
    ] {
        steps.push((
            "measure.add",
            json!({"table": "Sales", "name": name, "expression": expr}),
        ));
    }
    steps.extend([
        ("report.new", json!({})),
        ("page.add", json!({"display_name": "Main"})),
    ]);
    for (id, p) in steps {
        run(&mut s, id, p);
    }
    s
}

fn add_visual(s: &mut Session, visual_type: &str, fields: &[&str]) -> String {
    let fields: Vec<Value> = fields
        .iter()
        .map(|f| json!({"role": "Values", "field": f}))
        .collect();
    let r = run(
        s,
        "visual.add",
        json!({"page": "Main", "visual_type": visual_type, "x": 0, "y": 0, "width": 100, "height": 100,
               "fields": fields}),
    );
    r.get("visual")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| panic!("no visual name in {r}"))
}

fn data(s: &Session, visual: &str) -> Value {
    let snap = s.snapshot();
    for p in snap["report"]["pages"].as_array().cloned().unwrap_or_default() {
        for v in p["visuals"].as_array().cloned().unwrap_or_default() {
            if v["name"] == visual {
                return v["data"].clone();
            }
        }
    }
    panic!("visual {visual} not in the snapshot")
}

fn rows(d: &Value) -> Vec<Value> {
    assert_eq!(d["status"], "ok", "{d}");
    d["rows"].as_array().cloned().unwrap_or_default()
}

/// A card's single value.
fn card(s: &mut Session, measure: &str) -> Value {
    let v = add_visual(s, "card", &[measure]);
    let r = rows(&data(s, &v));
    assert_eq!(r.len(), 1, "{r:?}");
    r[0][0].clone()
}

/// The row whose first cell is `key`.
fn row_of(rows: &[Value], key: Value) -> Value {
    rows.iter()
        .find(|r| r[0] == key)
        .cloned()
        .unwrap_or_else(|| panic!("no row {key} in {rows:?}"))
}

fn close(a: &Value, b: f64) -> bool {
    a.as_f64()
        .is_some_and(|a| (a - b).abs() <= 1e-9 * b.abs().max(1.0))
}

#[test]
fn year_to_date_and_previous_year_by_year() {
    let mut s = shop();
    let v = add_visual(
        &mut s,
        "tableEx",
        &["Calendar[Year]", "[Total]", "[YTD]", "[PY]", "[PYTD]", "[YTD2]"],
    );
    assert_eq!(
        rows(&data(&s, &v)),
        vec![
            json!([2023, 370, 370, null, null, 370]),
            json!([2024, 520, 520, 370, 370, 520]),
        ]
    );
}

#[test]
fn time_intelligence_by_month() {
    let mut s = shop();
    let v = add_visual(
        &mut s,
        "tableEx",
        &[
            "Calendar[YearMonth]",
            "[Total]",
            "[YTD]",
            "[PY]",
            "[R12]",
            "[PrevMonth]",
            "[Upto]",
        ],
    );
    let d = data(&s, &v);
    let r = rows(&d);
    // YTD and the rolling 12 months are never blank from the first sale on
    assert_eq!(d["total_rows"], 24);
    // February 2024 (a whole month) shifts to the whole of February 2023
    assert_eq!(
        row_of(&r, json!(202402)),
        json!([202402, 30, 180, 20, 430, 150, 550])
    );
    // no sale in March 2023: the cumulative measures still show
    assert_eq!(
        row_of(&r, json!(202303)),
        json!([202303, null, 120, null, 120, 20, 120])
    );
    assert_eq!(
        row_of(&r, json!(202403)),
        json!([202403, 300, 480, null, 730, 30, 850])
    );
    assert_eq!(
        row_of(&r, json!(202306)),
        json!([202306, 200, 320, null, 320, null, 320])
    );
    assert_eq!(
        row_of(&r, json!(202407)),
        json!([202407, 40, 520, null, 570, null, 890])
    );
}

#[test]
fn cards_use_the_last_date_in_context() {
    let mut s = shop();
    assert_eq!(card(&mut s, "[YTD]"), json!(520));
    assert_eq!(card(&mut s, "[PY]"), json!(370));
    // 890 - 370
    assert_eq!(card(&mut s, "[Delta]"), json!(520));
    assert_eq!(card(&mut s, "[Last]"), json!("2024-12-31"));
    assert_eq!(card(&mut s, "[LastSale]"), json!("2024-07-15"));
}

#[test]
fn removefilters_all_and_overriding_filters() {
    let mut s = shop();
    let v = add_visual(
        &mut s,
        "tableEx",
        &["Product[Category]", "[Total]", "[Share]", "[AllCat]", "[Toys]"],
    );
    let r = rows(&data(&s, &v));
    assert_eq!(r.len(), 3, "{r:?}");
    // the blank row: sales of a product missing from Product
    assert_eq!(r[0][0], Value::Null);
    assert_eq!(r[0][1], json!(50));
    assert!(close(&r[0][2], 50.0 / 890.0), "{r:?}");
    assert_eq!(r[1][0], "Toys");
    assert!(close(&r[1][2], 90.0 / 890.0), "{r:?}");
    assert_eq!(r[2][0], "Wheels");
    assert!(close(&r[2][2], 750.0 / 890.0), "{r:?}");
    for row in &r {
        assert_eq!(row[3], json!(890));
        assert_eq!(row[4], json!(90));
    }
}

#[test]
fn boolean_and_keepfilters_filters() {
    let mut s = shop();
    assert_eq!(card(&mut s, "[Online]"), json!(370));
    assert_eq!(card(&mut s, "[Big]"), json!(750));
    let v = add_visual(&mut s, "tableEx", &["Sales[Channel]", "[Online]"]);
    // KEEPFILTERS keeps the Store row's own filter: blank, so the row is hidden
    assert_eq!(rows(&data(&s, &v)), vec![json!(["App", 80]), json!(["Web", 290])]);
}

#[test]
fn iterators_context_transition_and_logic() {
    let mut s = shop();
    assert_eq!(card(&mut s, "[Twice]"), json!(1780));
    // VALUES includes the blank row: 90 + 750 + 50
    assert_eq!(card(&mut s, "[CatSum]"), json!(890));
    assert_eq!(card(&mut s, "[Pick]"), json!("many"));
    assert_eq!(card(&mut s, "[Level]"), json!(2));
    // DATESMTD of a column in a row context transitions: months of 31 days in 2023-2024
    assert_eq!(card(&mut s, "[LongMonths]"), json!(14));
    let v = add_visual(&mut s, "tableEx", &["Product[Category]", "[Pick]", "[Level]"]);
    assert_eq!(
        rows(&data(&s, &v)),
        vec![
            json!([null, null, 1]),
            json!(["Toys", "Toys", 1]),
            json!(["Wheels", "Wheels", 2])
        ]
    );
}

#[test]
fn userelationship_switches_the_active_relationship() {
    let mut s = shop();
    let v = add_visual(&mut s, "tableEx", &["Calendar[Year]", "[Total]", "[Shipped]"]);
    assert_eq!(
        rows(&data(&s, &v)),
        vec![json!([2023, 370, 320]), json!([2024, 520, 570])]
    );
}

#[test]
fn unsupported_functions_are_named() {
    let mut s = shop();
    let v = add_visual(&mut s, "card", &["[Hard]"]);
    let d = data(&s, &v);
    assert_eq!(d["status"], "unsupported", "{d}");
    assert!(d["reason"].as_str().unwrap().contains("RAND"), "{d}");
}

#[test]
fn deep_measure_recursion_is_refused() {
    let mut s = shop();
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Loop", "expression": "CALCULATE([Loop], ALL(Product))"}),
    );
    let v = add_visual(&mut s, "card", &["[Loop]"]);
    let d = data(&s, &v);
    assert_eq!(d["status"], "unsupported", "{d}");
}

// --- date rules (checked against Power BI Desktop) ---------------------------------------------

fn day(y: i64, m: i64, d: i64) -> i64 {
    serial(y, m, d)
}

fn days(lo: i64, hi: i64) -> Vec<i64> {
    (lo..=hi).collect()
}

#[test]
fn civil_dates_round_trip() {
    assert_eq!(day(2025, 1, 1), 45_658);
    for x in [0, 1, 59, 60, 61, 36_526, 45_351, 45_657, 45_658, 2_958_465] {
        let (y, m, d) = ymd(x);
        assert_eq!(serial(y, m, d), x);
    }
}

#[test]
fn dateadd_moves_runs_of_days() {
    let m = Interval::Month;
    // a whole month → the whole target month
    assert_eq!(
        shift_blocks(&days(day(2024, 2, 1), day(2024, 2, 29)), 1, m),
        vec![(day(2024, 3, 1), day(2024, 3, 31))]
    );
    // a single day keeps its day number (clipped)
    assert_eq!(
        shift_blocks(&[day(2024, 2, 29)], 1, m),
        vec![(day(2024, 3, 29), day(2024, 3, 29))]
    );
    assert_eq!(
        shift_blocks(&[day(2024, 3, 31)], -1, m),
        vec![(day(2024, 2, 29), day(2024, 2, 29))]
    );
    assert_eq!(
        shift_blocks(&[day(2024, 4, 30)], 1, m),
        vec![(day(2024, 5, 30), day(2024, 5, 30))]
    );
    // a run ending on a month end ends on the target month end
    assert_eq!(
        shift_blocks(&days(day(2024, 4, 15), day(2024, 4, 30)), 1, m),
        vec![(day(2024, 5, 15), day(2024, 5, 31))]
    );
    assert_eq!(
        shift_blocks(&days(day(2024, 4, 29), day(2024, 4, 30)), -1, m),
        vec![(day(2024, 3, 29), day(2024, 3, 31))]
    );
    assert_eq!(
        shift_blocks(&days(day(2024, 1, 30), day(2024, 1, 31)), 1, m),
        vec![(day(2024, 2, 29), day(2024, 2, 29))]
    );
    assert_eq!(
        shift_blocks(&days(day(2025, 2, 27), day(2025, 2, 28)), -1, Interval::Year),
        vec![(day(2024, 2, 27), day(2024, 2, 29))]
    );
    assert_eq!(
        shift_blocks(&[day(2025, 2, 28)], -1, Interval::Year),
        vec![(day(2024, 2, 28), day(2024, 2, 28))]
    );
    // separate runs move separately
    assert_eq!(
        shift_blocks(&[day(2024, 4, 15), day(2024, 4, 16), day(2024, 4, 30)], 1, m),
        vec![
            (day(2024, 5, 15), day(2024, 5, 16)),
            (day(2024, 5, 30), day(2024, 5, 30))
        ]
    );
    assert_eq!(
        shift_blocks(&days(day(2025, 12, 30), day(2025, 12, 31)), 1, Interval::Day),
        vec![(day(2025, 12, 31), day(2026, 1, 1))]
    );
}

#[test]
fn period_shifts_and_fiscal_years() {
    // DATESINPERIOD bounds: month ends map to month ends
    assert_eq!(shift_eom(day(2025, 2, 28), -1), day(2025, 1, 31));
    assert_eq!(shift_eom(day(2025, 2, 28), -12), day(2024, 2, 29));
    assert_eq!(shift_eom(day(2024, 1, 31), 1), day(2024, 2, 29));
    assert_eq!(shift_eom(day(2025, 3, 15), -1), day(2025, 2, 15));
    assert_eq!(
        fiscal_year(day(2025, 3, 15), (6, 30)),
        (day(2024, 7, 1), day(2025, 6, 30))
    );
    assert_eq!(
        fiscal_year(day(2024, 7, 3), (6, 30)),
        (day(2024, 7, 1), day(2025, 6, 30))
    );
    assert_eq!(
        fiscal_year(day(2024, 12, 31), (12, 31)),
        (day(2024, 1, 1), day(2024, 12, 31))
    );
}

// ---------------------------------------------------------------------------------------------
// Currency arithmetic and column variations
// ---------------------------------------------------------------------------------------------

/// Sales with a Currency (`decimal`) price, and an auto date/time local date table that
/// `Sales[OrderDate]` points to through a variation (as Desktop builds it).
fn till(measures: &[(&str, &str)]) -> Session {
    let mut s = Session::new();
    let sales = r#"#table({"OrderDate", "Channel", "Price", "Qty"}, {
        {#date(2024,1,10), "Web", 10.0001, 1},
        {#date(2024,2,15), "Store", 10.0002, 2},
        {#date(2024,3,20), "Web", 10.0002, 3},
        {#date(2025,1,5), "Store", 7.5, 4}})"#;
    let mut days = Vec::new();
    for y in 2024..=2025 {
        for m in 1..=12 {
            let n = match m {
                2 if y == 2024 => 29,
                2 => 28,
                4 | 6 | 9 | 11 => 30,
                _ => 31,
            };
            for d in 1..=n {
                days.push(format!("{{#date({y},{m},{d}), {y}, {m}}}"));
            }
        }
    }
    let ldt = format!(
        "#table({{\"Date\", \"Year\", \"MonthNo\"}}, {{{}}})",
        days.join(", ")
    );
    let mut steps = vec![
        ("model.new", json!({"name": "Till"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
        ),
        (
            "table.add",
            json!({"name": "LocalDateTable_1", "partition": {"source_type": "m", "expression": ldt}}),
        ),
    ];
    for (t, c, ty) in [
        ("Sales", "OrderDate", "dateTime"),
        ("Sales", "Channel", "string"),
        ("Sales", "Price", "decimal"),
        ("Sales", "Qty", "int64"),
        ("LocalDateTable_1", "Date", "dateTime"),
        ("LocalDateTable_1", "Year", "int64"),
        ("LocalDateTable_1", "MonthNo", "int64"),
    ] {
        steps.push(("column.add", json!({"table": t, "name": c, "data_type": ty})));
    }
    steps.push((
        "relationship.add",
        json!({"from": "Sales[OrderDate]", "to": "LocalDateTable_1[Date]"}),
    ));
    for (id, p) in steps {
        run(&mut s, id, p);
    }
    // The variation (no command edits variations; Desktop writes them).
    let m = s.model.as_mut().unwrap();
    let rel = m.relationships[0].name.clone();
    m.table_mut("Sales")
        .and_then(|t| t.column_mut("OrderDate"))
        .unwrap()
        .set_variations(&[bicraft_model::Variation {
            name: "Variation".into(),
            relationship: Some(rel),
            default_hierarchy: Some(("LocalDateTable_1".into(), "Date Hierarchy".into())),
            is_default: true,
            ..Default::default()
        }]);
    for (name, expr) in measures {
        run(
            &mut s,
            "measure.add",
            json!({"table": "Sales", "name": name, "expression": expr}),
        );
    }
    run(&mut s, "report.new", json!({}));
    run(&mut s, "page.add", json!({"display_name": "Main"}));
    s
}

/// Currency results are rounded half away from zero to 4 decimals, as the engine does
/// (checked on Power BI Desktop 2.158 with the same expressions). Prices sum to 37.5005 over 4
/// rows.
#[test]
fn currency_results_round_to_four_decimals() {
    let cases: &[(&str, &str, f64)] = &[
        // fast path
        ("Avg", "AVERAGE(Sales[Price])", 9.3751), // 9.375125
        ("PerSale", "DIVIDE(SUM(Sales[Price]), COUNTROWS(Sales))", 9.3751),
        ("PerSaleSlash", "SUM(Sales[Price]) / COUNTROWS(Sales)", 9.3751),
        ("Third", "SUM(Sales[Price]) / 3", 12.5002), // 12.500166…
        ("TimesReal", "SUM(Sales[Price]) * 1.1111", 41.6668), // 41.66680555
        ("PlusInt", "SUM(Sales[Price]) + 1", 38.5005),
        ("Sum", "SUM(Sales[Price])", 37.5005),
        // Currency with a double, or Currency / Currency: doubles, not rounded
        ("PlusReal", "SUM(Sales[Price]) + 0.00001", 37.50051),
        ("Ratio", "SUM(Sales[Price]) / (SUM(Sales[Price]) * 3)", 1.0 / 3.0),
        // general evaluator
        (
            "AvgWeb",
            "CALCULATE(AVERAGE(Sales[Price]), Sales[Channel] = \"Web\")",
            10.0002,
        ), // 10.00015
        ("AvgX", "AVERAGEX(Sales, Sales[Price] / 7)", 1.3393), // rows 1.4286 ×3, 1.0714
        ("SumX", "SUMX(Sales, Sales[Price] * 1.5)", 56.2508),  // 56.25075 unrounded
        ("Median", "CALCULATE(MEDIAN(Sales[Price]), ALL(Sales))", 10.0002),
        (
            "DivideX",
            "CALCULATE(DIVIDE(SUM(Sales[Price]), 3), ALL(Sales))",
            12.5002,
        ),
        (
            "RoundC",
            "CALCULATE(ROUND(SUM(Sales[Price]) / 3, 2), ALL(Sales))",
            12.5,
        ),
        (
            "MinusX",
            "CALCULATE(-SUM(Sales[Price]) / 3, ALL(Sales))",
            -12.5002,
        ),
        (
            "CurrencyFn",
            "CALCULATE(CURRENCY(0.00015) + CURRENCY(0.00025), ALL(Sales))",
            0.0004,
        ),
        (
            "RatioX",
            "CALCULATE(SUM(Sales[Price]) / SUM(Sales[Price]) / 3, ALL(Sales))",
            1.0 / 3.0,
        ),
    ];
    let mut s = till(&cases.iter().map(|(n, e, _)| (*n, *e)).collect::<Vec<_>>());
    for (name, _, want) in cases {
        let got = card(&mut s, &format!("[{name}]"));
        assert!(
            got.as_f64()
                .is_some_and(|g| g == *want || (g - want).abs() < 1e-12),
            "[{name}] = {got}, want {want}"
        );
    }
}

#[test]
fn currency_grouped_averages_are_rounded() {
    let mut s = till(&[("Avg", "AVERAGE(Sales[Price])")]);
    let v = add_visual(&mut s, "tableEx", &["Sales[Channel]", "[Avg]"]);
    let r = rows(&data(&s, &v));
    assert_eq!(row_of(&r, json!("Web")), json!(["Web", 10.0002]));
    assert_eq!(row_of(&r, json!("Store")), json!(["Store", 8.7501])); // 8.75010
}

/// `'Sales'[OrderDate].[Year]` reads the variation table's column; filters on it flow through
/// the relationship from the date column to the local date table.
#[test]
fn variation_references_evaluate_as_the_local_date_table() {
    let cases: &[(&str, &str, f64)] = &[
        (
            "Y2024",
            "CALCULATE(SUM(Sales[Qty]), Sales[OrderDate].[Year] = 2024)",
            6.0,
        ),
        ("Months", "SUMX(Sales, Sales[OrderDate].[MonthNo])", 7.0),
        // MAX of a variation column reads the whole local date table
        ("MaxYear", "MAX(Sales[OrderDate].[Year])", 2025.0),
        ("Years", "COUNTROWS(VALUES(Sales[OrderDate].[Year]))", 2.0),
        (
            "Jan",
            "CALCULATE(SUM(Sales[Qty]), Sales[OrderDate].[MonthNo] = 1, ALL(Sales[OrderDate].[Year]))",
            5.0,
        ),
        (
            "Filtered",
            "COUNTROWS(FILTER(ALL(Sales), Sales[OrderDate].[Year] = 2025))",
            1.0,
        ),
    ];
    let mut s = till(&cases.iter().map(|(n, e, _)| (*n, *e)).collect::<Vec<_>>());
    for (name, _, want) in cases {
        let got = card(&mut s, &format!("[{name}]"));
        assert!(close(&got, *want), "[{name}] = {got}, want {want}");
    }
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "YTD", "expression": "CALCULATE(SUM(Sales[Qty]), DATESYTD(Sales[OrderDate].[Date]))"}),
    );
    let v = add_visual(&mut s, "tableEx", &["LocalDateTable_1[Year]", "[YTD]"]);
    assert_eq!(rows(&data(&s, &v)), vec![json!([2024, 6]), json!([2025, 4])]);
}

// ---------------------------------------------------------------------------------------------
// Table functions, against the engine of Power BI Desktop
// ---------------------------------------------------------------------------------------------

/// A value as the recorded results hold it: numbers (dates as their serial), text, booleans,
/// null.
fn cell_text(v: &Value) -> String {
    match v {
        Value::Null => "∅".into(),
        Value::Number(n) => format!("{:.6}", n.as_f64().unwrap_or(f64::NAN)),
        Value::Object(o) => cell_text(o.get("date").unwrap_or(&Value::Null)),
        Value::Bool(b) => b.to_string(),
        Value::String(s) => format!("{s:?}"),
        other => other.to_string(),
    }
}

fn cell_of(c: &bicraft_vertipaq::CellValue) -> Value {
    use bicraft_vertipaq::CellValue as C;
    match c {
        C::Null => Value::Null,
        C::Int(n) => json!(*n),
        C::Real(x) | C::DateTime(x) => json!(*x),
        C::Decimal(n) => json!(*n as f64 / 10_000.0),
        C::Text(s) => json!(s),
        C::Bool(b) => json!(b),
    }
}

/// The type a calculated table column gets: the evaluator's, else the one its values show.
fn type_name(t: Option<bicraft_model::DataType>, values: &[bicraft_vertipaq::CellValue]) -> &'static str {
    use bicraft_model::DataType as D;
    use bicraft_vertipaq::CellValue as C;
    let t = t.or_else(|| {
        values.iter().find_map(|v| match v {
            C::Null => None,
            C::Int(_) => Some(D::Int64),
            C::Real(x) if x.fract() == 0.0 => Some(D::Int64),
            C::Real(_) => Some(D::Double),
            C::Decimal(_) => Some(D::Decimal),
            C::DateTime(_) => Some(D::DateTime),
            C::Text(_) => Some(D::String),
            C::Bool(_) => Some(D::Boolean),
        })
    });
    match t {
        Some(D::Int64) => "Int64",
        Some(D::Double) => "Double",
        Some(D::Decimal) => "Decimal",
        Some(D::DateTime) => "DateTime",
        Some(D::String) => "String",
        Some(D::Boolean) => "Boolean",
        _ => "?",
    }
}

/// Every table function of `preview_eval_tables.rs` on a synthetic model (calculated tables
/// only, `tests/table_functions/model.bim`), compared with what the Analysis Services engine of
/// Power BI Desktop 2.158 returned for `EVALUATE <expression>` on the same model
/// (`expected.json`, recorded with the development oracles, docs/dax-oracle.md): the same rows (in
/// any order), column names and data types. Entries the engine refused carry `"error": true`;
/// bicraft must refuse them too.
#[test]
fn table_functions_match_the_engine() {
    let model = bicraft_tmsl::read(include_str!("../tests/table_functions/model.bim")).unwrap();
    let expected: Value =
        serde_json::from_str(include_str!("../tests/table_functions/expected.json")).unwrap();
    let mut failures = Vec::new();
    for q in expected.as_array().unwrap() {
        let id = q["id"].as_str().unwrap();
        let expr = q["expression"].as_str().unwrap();
        let mut ev = crate::preview::StructEval::new(None, model.clone(), 100_000);
        let got = ev.table(expr);
        if q.get("error").is_some() {
            if got.is_ok() {
                failures.push(format!("{id}: the engine refuses it, bicraft evaluates it"));
            }
            continue;
        }
        let t = match got {
            Ok(t) => t,
            Err(e) => {
                failures.push(format!("{id}: {e}"));
                continue;
            }
        };
        let names: Vec<String> = t.columns.iter().map(|(n, _, _)| n.to_lowercase()).collect();
        // `Table[Column]` in the query result: `Column` in a calculated table, or `Table_Column`
        // for an unnamed SELECTCOLUMNS column (the engine's calculated-table naming, observed
        // with TOM on Desktop 2.158).
        let want_names: Vec<(String, Option<String>)> = q["columns"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| {
                let c = c.as_str().unwrap();
                match c.rsplit_once('[') {
                    Some((table, inner)) => {
                        let inner = inner.trim_end_matches(']').to_lowercase();
                        let qualified = (!table.is_empty() && expr.contains("SELECTCOLUMNS"))
                            .then(|| format!("{}_{inner}", table.to_lowercase()));
                        (inner, qualified)
                    }
                    None => (c.to_lowercase(), None),
                }
            })
            .collect();
        let same = names.len() == want_names.len()
            && names
                .iter()
                .zip(&want_names)
                .all(|(n, (inner, qualified))| n == inner || qualified.as_ref() == Some(n));
        if !same {
            failures.push(format!("{id}: columns {names:?}, engine {want_names:?}"));
            continue;
        }
        let types: Vec<&str> = t.columns.iter().map(|(_, ty, v)| type_name(*ty, v)).collect();
        let want_types: Vec<&str> = q["types"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap())
            .collect();
        if types != want_types {
            failures.push(format!("{id}: types {types:?}, engine {want_types:?}"));
        }
        let n = t.columns.first().map_or(0, |c| c.2.len());
        let mut rows: Vec<String> = (0..n)
            .map(|r| {
                t.columns
                    .iter()
                    .map(|c| cell_text(&cell_of(&c.2[r])))
                    .collect::<Vec<_>>()
                    .join(" | ")
            })
            .collect();
        let mut want: Vec<String> = q["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                r.as_array()
                    .unwrap()
                    .iter()
                    .map(cell_text)
                    .collect::<Vec<_>>()
                    .join(" | ")
            })
            .collect();
        rows.sort();
        want.sort();
        if rows != want {
            failures.push(format!("{id}:\n  bicraft {rows:?}\n  engine   {want:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Table functions in viewer measures: `CALENDARAUTO` over the data tables' dates (2023-2024 in
/// `shop()`), iterators over `SUMMARIZE`/`ADDCOLUMNS`/`TOPN` results, and a computed table as a
/// filter.
#[test]
fn table_functions_in_measures() {
    let cases: &[(&str, &str, f64)] = &[
        ("Days", "COUNTROWS(CALENDARAUTO())", 731.0),
        // fiscal years ending in June: 2022-07-01 … 2025-06-30
        ("FyDays", "COUNTROWS(CALENDARAUTO(6))", 1096.0),
        (
            "FyStart",
            "MINX(CALENDARAUTO(6), YEAR([Date]) * 100 + MONTH([Date]))",
            202_207.0,
        ),
        // the best product (Bike: 750) through ADDCOLUMNS + TOPN
        (
            "Best",
            "MAXX(TOPN(1, ADDCOLUMNS(VALUES(Sales[Product]), \"@T\", [Total]), [@T]), [@T])",
            750.0,
        ),
        // per-category totals summed back
        (
            "ByCat",
            "SUMX(SUMMARIZE(Sales, Product[Category], \"@T\", SUM(Sales[Amount])), [@T])",
            890.0,
        ),
        // a SUMMARIZE result filters by its lineage: the Web channel's products
        (
            "WebProducts",
            "CALCULATE([Total], SUMMARIZE(FILTER(Sales, Sales[Channel] = \"Web\"), Sales[Product]))",
            840.0,
        ),
        ("Series", "SUMX(GENERATESERIES(1, 4), [Value])", 10.0),
    ];
    let mut s = shop();
    for (name, expr, _) in cases {
        run(
            &mut s,
            "measure.add",
            json!({"table": "Sales", "name": name, "expression": expr}),
        );
    }
    for (name, _, want) in cases {
        let got = card(&mut s, &format!("[{name}]"));
        assert!(close(&got, *want), "[{name}] = {got}, want {want}");
    }
}

/// Relationship keys match as Analysis Services stores text (Desktop 2.158, en-US): `À` and `à`
/// are two "one" keys, `"A "` (imported, trimmed to `A`) matches `a`. Same totals as Desktop.
#[test]
fn relationship_text_keys_follow_stored_text() {
    let mut s = Session::new();
    let dim = r#"#table({"Key", "Label"}, {{"À", 1}, {"à", 2}, {"a", 3}, {"b", 4}})"#;
    let fact =
        r#"#table({"Key", "Amount"}, {{"à", 1}, {"À", 2}, {"A ", 4}, {"B", 8}, {"c", 64}, {"a", 128}})"#;
    for (id, p) in [
        ("model.new", json!({"name": "Keys"})),
        (
            "table.add",
            json!({"name": "Dim", "partition": {"source_type": "m", "expression": dim}}),
        ),
        (
            "table.add",
            json!({"name": "Fact", "partition": {"source_type": "m", "expression": fact}}),
        ),
        (
            "column.add",
            json!({"table": "Dim", "name": "Key", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Dim", "name": "Label", "data_type": "int64"}),
        ),
        (
            "column.add",
            json!({"table": "Fact", "name": "Key", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Fact", "name": "Amount", "data_type": "int64"}),
        ),
        (
            "measure.add",
            json!({"table": "Fact", "name": "Total", "expression": "SUM(Fact[Amount])"}),
        ),
        ("report.new", json!({})),
        ("page.add", json!({"display_name": "Main"})),
    ] {
        run(&mut s, id, p);
    }
    // Dim[Key] holds "À" and "à" (equal without regard to case): relationship.add refuses it
    // as a "one" side, but a file can hold such a relationship, so it goes in directly.
    s.model
        .as_mut()
        .unwrap()
        .relationships
        .push(bicraft_model::Relationship {
            name: "FactDim".into(),
            from_table: "Fact".into(),
            from_column: "Key".into(),
            to_table: "Dim".into(),
            to_column: "Key".into(),
            is_active: true,
            ..Default::default()
        });
    let v = add_visual(&mut s, "tableEx", &["Dim[Key]", "[Total]"]);
    let r = rows(&data(&s, &v));
    assert!(close(&row_of(&r, json!("À"))[1], 2.0), "{r:?}");
    assert!(close(&row_of(&r, json!("à"))[1], 1.0), "{r:?}");
    assert!(close(&row_of(&r, json!("a"))[1], 132.0), "{r:?}");
    assert!(close(&row_of(&r, json!("b"))[1], 8.0), "{r:?}");
    // RELATED goes through the same keys: 2*1 + 1*2 + 3*4 + 4*8 + 3*128.
    run(
        &mut s,
        "measure.add",
        json!({"table": "Fact", "name": "Linked", "expression": "SUMX(Fact, RELATED(Dim[Label]) * Fact[Amount])"}),
    );
    let linked = card(&mut s, "[Linked]");
    assert!(close(&linked, 432.0), "{linked}");
}

/// A visual's implicit "Standard deviation of" / "Variance of" are the population forms, as Power
/// BI Desktop 2.158 shows them (M10.2 oracle: 544,09 where STDEV.S gives 551,81 over 36 rows);
/// DAX `STDEV.P` / `VAR.P` evaluate the same, `STDEV.S` / `VAR.S` divide by n - 1. Qty 1..4.
#[test]
fn implicit_standard_deviation_and_variance_are_population_forms() {
    let mut s = till(&[
        ("SP", "STDEV.P(Sales[Qty])"),
        ("VP", "VAR.P(Sales[Qty])"),
        ("SS", "STDEV.S(Sales[Qty])"),
        ("VS", "VAR.S(Sales[Qty])"),
    ]);
    let close = |v: Value, want: f64| v.as_f64().is_some_and(|g| (g - want).abs() < 1e-12);
    assert!(close(
        card(&mut s, "StandardDeviation(Sales[Qty])"),
        1.25f64.sqrt()
    ));
    assert!(close(card(&mut s, "Variance(Sales[Qty])"), 1.25));
    assert!(close(card(&mut s, "[SP]"), 1.25f64.sqrt()));
    assert!(close(card(&mut s, "[VP]"), 1.25));
    assert!(close(card(&mut s, "[SS]"), (5.0f64 / 3.0).sqrt()));
    assert!(close(card(&mut s, "[VS]"), 5.0 / 3.0));
}

/// The allocation-free text order is the order of the lower-cased strings, for ASCII, accented
/// Latin, the letters whose lower case is two chars (`İ`), Greek with and without a capital
/// sigma (final or not), German sharp s, and invalid-free random mixes of them.
#[test]
fn cmp_text_is_the_order_of_the_lower_cased_strings() {
    let alphabet = [
        "a", "A", "b", "Z", "z", "0", " ", "-", "_", "é", "É", "e", "E", "İ", "i", "I", "ı", "ß", "ẞ", "Σ",
        "σ", "ς", "ΑΣ", "Ω", "ω", "ǅ", "ǆ", "Ǆ", "\u{0}", "\u{7f}", "\u{80}", "日", "😀", "K", "\u{212a}",
    ];
    let mut words: Vec<String> = alphabet.iter().map(|s| (*s).to_string()).collect();
    words.push(String::new());
    let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
    for _ in 0..600 {
        let mut w = String::new();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        for k in 0..(x % 6) {
            let i = ((x >> (8 * k)) as usize) % alphabet.len();
            w.push_str(alphabet[i]);
        }
        words.push(w);
    }
    for a in &words {
        for b in &words {
            assert_eq!(
                super::cmp_text(a, b),
                a.to_lowercase().cmp(&b.to_lowercase()),
                "{a:?} vs {b:?}"
            );
        }
    }
}

/// Calculated tables and columns read the clock as measures do (G30): `NOW()` is local time,
/// `UTCNOW()` that minus the offset (`utc_offset_minutes` when given, else the process's local
/// clock), `TODAY()` and `UTCTODAY()` their days. Documented, not oracle-checked.
#[test]
fn calculated_tables_read_the_local_clock() {
    use bicraft_vertipaq::CellValue as C;
    let model = bicraft_tmsl::read(include_str!("../tests/table_functions/model.bim")).unwrap();
    let mut ev = crate::preview::StructEval::new(None, model, 100);
    // 2026-03-10 23:30 UTC, local time 90 minutes ahead: 2026-03-11 01:00
    let utc_ms = (serial(2026, 3, 10) * 86_400 + 23 * 3600 + 30 * 60) * 1000;
    ev.set_clock(Some(utc_ms), Some(90));
    let t = ev
        .table(
            "ROW(\"d\", ROUND((NOW() - UTCNOW()) * 1440, 0), \"today\", TODAY(), \"utctoday\", UTCTODAY())",
        )
        .unwrap();
    let num = |v: &C| match v {
        C::Int(x) => *x as f64,
        C::Real(x) | C::DateTime(x) => *x,
        other => panic!("not a number: {other:?}"),
    };
    let first = |i: usize| t.columns[i].2[0].clone();
    assert_eq!(num(&first(0)), 90.0);
    assert_eq!(num(&first(1)), serial(2026, 3, 11) as f64);
    assert_eq!(num(&first(2)), serial(2026, 3, 10) as f64);
    // a calculated column too
    let c = ev
        .column("Product", "ROUND((NOW() - UTCNOW()) * 1440, 0)")
        .unwrap();
    assert!(!c.is_empty());
    assert!(c.iter().all(|v| num(v) == 90.0), "{c:?}");
    // without an offset: the process's local clock (UTC in this test binary)
    ev.set_clock(Some(utc_ms), None);
    let t = ev
        .table("ROW(\"d\", ROUND((NOW() - UTCNOW()) * 1440, 0), \"today\", TODAY())")
        .unwrap();
    assert_eq!(num(&t.columns[0].2[0]), 0.0);
    assert_eq!(num(&t.columns[1].2[0]), serial(2026, 3, 10) as f64);
}
