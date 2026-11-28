//! `column.group` (New group: list groups and bins): the calculated column and its
//! `GroupingMetadata`, the evaluator computing it (queries and visuals), one undo step, refusals
//! that change nothing, the definition read back for Edit groups, and round trips through PBIP
//! (TMDL) and PBIX.

use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Sales(Date, Product, Amount, Qty), seven rows, one without a product, and [Total].
fn shop() -> Session {
    let mut s = Session::new();
    let sales = r#"#table(type table [Date = date, Product = text, Amount = number, Qty = Int64.Type], {
        {#date(2024, 1, 5), "Bike", 100, 1}, {#date(2024, 2, 5), "Bike", 50, 2},
        {#date(2024, 4, 10), "Car", 1000, 3}, {#date(2025, 1, 7), "Ball", 5, 4},
        {#date(2025, 3, 7), null, 7, 5}, {#date(2025, 3, 9), "Kite", 42, 6},
        {#date(2025, 8, 1), "Doll", 18, 7}})"#;
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
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
    ] {
        run(&mut s, id, p);
    }
    s
}

/// `[Total]` by the values of `column`: (value, total) sorted by value.
fn totals(s: &mut Session, column: &str) -> Vec<(Value, Value)> {
    let q =
        format!("EVALUATE SUMMARIZECOLUMNS('Sales'[{column}], \"t\", [Total]) ORDER BY 'Sales'[{column}]");
    let v = run(s, "dax.evaluate", json!({ "query": q }));
    v["results"][0]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r[format!("Sales[{column}]")].clone(), r["[t]"].clone()))
        .collect()
}

fn pairs(list: &[(&str, f64)]) -> Vec<(Value, Value)> {
    list.iter().map(|(k, v)| (json!(k), json!(v))).collect()
}

fn ext(s: &Session, column: &str) -> Value {
    let c = s
        .model
        .as_ref()
        .unwrap()
        .table("Sales")
        .unwrap()
        .column(column)
        .unwrap()
        .clone();
    c.extra["extendedProperties"][0].clone()
}

fn snapshot_column(s: &Session, column: &str) -> Value {
    let snap = s.snapshot();
    let t = snap["model"]["tables"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "Sales")
        .unwrap()
        .clone();
    t["columns"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == column)
        .unwrap()
        .clone()
}

fn list_group(s: &mut Session) -> Value {
    run(
        s,
        "column.group",
        json!({"table": "Sales", "column": "Product", "groups": [
            {"name": "Wheels", "values": ["Bike", "Car"]},
            {"name": "Toys", "values": ["Ball", "Kite"]},
        ]}),
    )
}

#[test]
fn a_list_group_is_a_switch_column_with_other_and_blank_in_one_undo_step() {
    let mut s = shop();
    let before = s.model.clone();
    let r = list_group(&mut s);
    assert_eq!(r["column"], "Product (groups)");
    assert_eq!(r["kind"], "list");
    assert_eq!(r["data_type"], "string");
    assert_eq!(
        r["expression"],
        "SWITCH(\n    TRUE,\n    ISBLANK('Sales'[Product]), \"(Blank)\",\n    \
         'Sales'[Product] IN {\"Bike\", \"Car\"}, \"Wheels\",\n    \
         'Sales'[Product] IN {\"Ball\", \"Kite\"}, \"Toys\",\n    \"Other\"\n)"
    );
    assert_eq!(
        ext(&s, "Product (groups)"),
        json!({"type": "json", "name": "GroupingMetadata", "value": {"version": 0, "groupedColumns": [
            {"Column": {"Expression": {"SourceRef": {"Entity": "Sales"}}, "Property": "Product"}}]}})
    );
    let c = s
        .model
        .as_ref()
        .unwrap()
        .table("Sales")
        .unwrap()
        .column("Product (groups)")
        .unwrap()
        .clone();
    assert_eq!(c.summarize_by.as_deref(), Some("none"));
    // the evaluator computes the column: Doll goes to Other, the blank product to (Blank)
    assert_eq!(
        totals(&mut s, "Product (groups)"),
        pairs(&[
            ("(Blank)", 7.0),
            ("Other", 18.0),
            ("Toys", 47.0),
            ("Wheels", 1150.0)
        ])
    );
    // one undo step takes it away
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.model, before);
}

fn design_state(s: &Session, column: &str) -> Option<Value> {
    let c = s.model.as_ref()?.table("Sales")?.column(column)?;
    let a = c.annotations.iter().find(|a| a.name == "GroupingDesignState")?;
    serde_json::from_str(&a.value).ok()
}

/// Power BI Desktop 2.159.1231.0 (2026-11-23) opened Edit groups on a list group BIcraft wrote
/// with no group listed: it reads them from the `GroupingDesignState` annotation, which it
/// writes in this shape for its own groups.
#[test]
fn a_list_group_carries_desktop_s_grouping_design_state() {
    let mut s = shop();
    list_group(&mut s);
    let col = json!({"Column": {"Expression": {"SourceRef": {"Source": "s"}}, "Property": "Product"}});
    let lit = |v: &str| json!([{"Literal": {"Value": v}}]);
    assert_eq!(
        design_state(&s, "Product (groups)"),
        Some(
            json!({"Version": 0, "Sources": [{"Name": "s", "Entity": "Sales"}], "GroupedColumns": [col],
            "GroupItems": [
                {"DisplayName": "(Blank)", "BlankDefaultPlaceholder": true},
                {"DisplayName": "Wheels", "Expression": {"In": {"Expressions": [col], "Values": [lit("'Bike'"), lit("'Car'")]}}},
                {"DisplayName": "Toys", "Expression": {"In": {"Expressions": [col], "Values": [lit("'Ball'"), lit("'Kite'")]}}},
                {"DisplayName": "Other"}]})
        )
    );
    // numbers are typed literals; without Other there is no Other item
    run(
        &mut s,
        "column.group",
        json!({"table": "Sales", "column": "Qty", "kind": "list", "other": false,
               "groups": [{"name": "Few", "values": [1, 2]}]}),
    );
    let d = design_state(&s, "Qty (groups)").unwrap();
    assert_eq!(d["GroupItems"].as_array().unwrap().len(), 2, "{d}");
    assert_eq!(d["GroupItems"][1]["Expression"]["In"]["Values"][1], lit("2L"));
    // a group listing a blank: the blank is a `null` member, no `(Blank)` placeholder (G75)
    run(
        &mut s,
        "column.group",
        json!({"table": "Sales", "column": "Product", "replace": true, "groups": [
            {"name": "Wheels", "values": ["Bike", null]}]}),
    );
    let d = design_state(&s, "Product (groups)").unwrap();
    assert_eq!(d["GroupItems"][0]["DisplayName"], "Wheels", "{d}");
    assert_eq!(d["GroupItems"][0]["Expression"]["In"]["Values"][1], lit("null"));
    // it is saved as a TMDL annotation and read back
    let dir = TempDir::new("bicraft-groups-design");
    let mut s = shop();
    list_group(&mut s);
    let t = reopened(&mut s, &dir, "Shop.pbip");
    let tmdl =
        std::fs::read_to_string(dir.path().join("Shop.SemanticModel/definition/tables/Sales.tmdl")).unwrap();
    assert!(tmdl.contains("annotation GroupingDesignState = {"), "{tmdl}");
    assert_eq!(
        design_state(&t, "Product (groups)"),
        design_state(&s, "Product (groups)")
    );
}

#[test]
fn without_other_the_ungrouped_values_keep_their_own_and_a_blank_can_be_grouped() {
    let mut s = shop();
    let r = run(
        &mut s,
        "column.group",
        json!({"table": "Sales", "column": "Product", "name": "Kind", "other": false, "groups": [
            {"name": "Wheels", "values": ["bike", "Car", null]},
        ]}),
    );
    assert_eq!(
        r["expression"],
        "SWITCH(\n    TRUE,\n    ISBLANK('Sales'[Product]) || 'Sales'[Product] IN {\"bike\", \"Car\"}, \"Wheels\",\n    \
         'Sales'[Product]\n)"
    );
    assert_eq!(
        totals(&mut s, "Kind"),
        pairs(&[("Ball", 5.0), ("Doll", 18.0), ("Kite", 42.0), ("Wheels", 1157.0)])
    );
}

#[test]
fn numeric_bins_by_size_and_by_count() {
    let mut s = shop();
    let r = run(
        &mut s,
        "column.group",
        json!({"table": "Sales", "column": "Amount", "bin_size": 50}),
    );
    assert_eq!(r["column"], "Amount (bins)");
    assert_eq!(r["kind"], "bin");
    assert_eq!(
        r["expression"],
        "IF(\n    ISBLANK('Sales'[Amount]),\n    BLANK(),\n    IF(\n        'Sales'[Amount] >= 0,\n        \
         ROUNDDOWN('Sales'[Amount] / 50, 0) * 50,\n        ROUNDUP('Sales'[Amount] / 50, 0) * 50\n    )\n)"
    );
    assert_eq!(
        ext(&s, "Amount (bins)")["value"]["binningMetadata"],
        json!({"binSize": {"value": 50.0, "unit": 0}})
    );
    let t: Vec<(Value, Value)> = totals(&mut s, "Amount (bins)");
    assert_eq!(
        t,
        vec![
            (json!(0.0), json!(72.0)),
            (json!(50.0), json!(50.0)),
            (json!(100.0), json!(100.0)),
            (json!(1000.0), json!(1000.0)),
        ]
    );
    // a whole bin size on a whole number column stays whole
    let r = run(
        &mut s,
        "column.group",
        json!({"table": "Sales", "column": "Qty", "bin_size": 3}),
    );
    assert_eq!(r["data_type"], "int64");
    // four equal bins between the minimum (5) and the maximum (1000): the maximum in the last
    let r = run(
        &mut s,
        "column.group",
        json!({"table": "Sales", "column": "Amount", "name": "Amount bands", "bin_count": 4}),
    );
    assert_eq!(r["bin_count"], json!(4));
    let t = totals(&mut s, "Amount bands");
    assert_eq!(
        t,
        vec![(json!(5.0), json!(222.0)), (json!(751.25), json!(1000.0))]
    );
    // read back for Edit groups
    assert_eq!(
        snapshot_column(&s, "Amount bands")["group"],
        json!({"column": "Amount", "kind": "bin", "bin_count": 4, "unit": "number"})
    );
}

#[test]
fn date_bins_by_year_month_and_day() {
    let mut s = shop();
    let r = run(
        &mut s,
        "column.group",
        json!({"table": "Sales", "column": "Date"}),
    );
    assert_eq!(r["unit"], "year");
    assert_eq!(r["data_type"], "dateTime");
    assert_eq!(
        totals(&mut s, "Date (bins)"),
        vec![
            (json!("2024-01-01T00:00:00"), json!(1150.0)),
            (json!("2025-01-01T00:00:00"), json!(72.0)),
        ]
    );
    run(
        &mut s,
        "column.group",
        json!({"table": "Sales", "column": "Date", "name": "Half", "unit": "month", "bin_size": 6}),
    );
    assert_eq!(
        totals(&mut s, "Half"),
        vec![
            (json!("2024-01-01T00:00:00"), json!(1150.0)),
            (json!("2025-01-01T00:00:00"), json!(54.0)),
            (json!("2025-07-01T00:00:00"), json!(18.0)),
        ]
    );
    // ten-day bins as Desktop computes them: 1 + (day serial number - 1) floored to a multiple of
    // 10 (2025-03-07 and -09 in the bin of 03-05)
    run(
        &mut s,
        "column.group",
        json!({"table": "Sales", "column": "Date", "name": "Tens", "unit": "day", "bin_size": 10}),
    );
    let t = totals(&mut s, "Tens");
    assert_eq!(t.len(), 6, "{t:?}");
    assert!(t.contains(&(json!("2025-03-05T00:00:00"), json!(49.0))), "{t:?}");
}

/// Power BI Desktop 2.159.1231.0 (2026-11-23) opened Edit groups on BIcraft's bins with an empty
/// bin type (numbers) or an empty dialog and a rendering error (dates): it reads a bin from the
/// `GroupingDesignState` annotation (`BinItem`) and its own `binningMetadata` unit codes, which it
/// wrote in these shapes for the bins made in its Groups dialog (docs/formats/groups.md).
#[test]
fn bins_carry_desktop_s_grouping_design_state_and_unit_codes() {
    let mut s = shop();
    let col = |c: &str| json!({"Column": {"Expression": {"SourceRef": {"Source": "s"}}, "Property": c}});
    let state = |c: &str, item: Value| {
        json!({"Version": 0, "Sources": [{"Name": "s", "Entity": "Sales"}], "GroupedColumns": [col(c)],
               "BinItem": {"Expression": item}})
    };
    run(
        &mut s,
        "column.group",
        json!({"table": "Sales", "column": "Amount", "bin_size": 100}),
    );
    assert_eq!(
        design_state(&s, "Amount (bins)"),
        Some(state(
            "Amount",
            json!({"Floor": {"Expression": col("Amount"), "Size": 100.0}})
        ))
    );
    // a number of bins: no binningMetadata, a Discretize member, bins that follow the data
    let r = run(
        &mut s,
        "column.group",
        json!({"table": "Sales", "column": "Qty", "bin_count": 4}),
    );
    assert_eq!(ext(&s, "Qty (bins)")["value"].get("binningMetadata"), None);
    assert_eq!(
        design_state(&s, "Qty (bins)"),
        Some(state(
            "Qty",
            json!({"Member": {"Expression": {"Discretize": {"Expression": col("Qty"), "Count": 4}}, "Member": "Min"}})
        ))
    );
    assert!(
        r["expression"]
            .as_str()
            .unwrap()
            .contains("VAR __Min = MIN('Sales'[Qty])"),
        "{r}"
    );
    assert_eq!(
        snapshot_column(&s, "Qty (bins)")["group"],
        json!({"column": "Qty", "kind": "bin", "bin_count": 4, "unit": "number"})
    );
    // dates: Desktop's unit codes, the query TimeUnit, a format string per unit
    for (unit, size, code, time_unit, format) in [
        ("year", 1, 4, 3, "yyyy"),
        ("month", 2, 6, 2, "mmmm yyyy"),
        ("day", 10, 8, 0, "Short Date"),
        ("hour", 12, 9, 7, "General Date"),
        ("minute", 15, 10, 6, "General Date"),
        ("second", 30, 11, 5, "General Date"),
    ] {
        let name = format!("By {unit}");
        run(
            &mut s,
            "column.group",
            json!({"table": "Sales", "column": "Date", "name": name, "unit": unit, "bin_size": size}),
        );
        assert_eq!(
            ext(&s, &name)["value"]["binningMetadata"],
            json!({"binSize": {"value": f64::from(size), "unit": code}}),
            "{unit}"
        );
        assert_eq!(
            design_state(&s, &name),
            Some(state(
                "Date",
                json!({"Floor": {"Expression": col("Date"), "Size": f64::from(size), "TimeUnit": time_unit}})
            )),
            "{unit}"
        );
        let c = s
            .model
            .as_ref()
            .unwrap()
            .table("Sales")
            .unwrap()
            .column(&name)
            .unwrap()
            .clone();
        assert_eq!(c.format_string.as_deref(), Some(format), "{unit}");
        assert_eq!(
            snapshot_column(&s, &name)["group"],
            json!({"column": "Date", "kind": "bin", "bin_size": size, "unit": unit}),
            "{unit}"
        );
    }
    // twelve-hour bins of a date: its midnight
    assert_eq!(
        totals(&mut s, "By hour")[0],
        (json!("2024-01-05T00:00:00"), json!(100.0))
    );
}

#[test]
fn refusals_change_nothing() {
    let mut s = shop();
    let before = s.model.clone();
    for (p, why) in [
        (
            json!({"table": "Sales", "column": "Nope", "groups": []}),
            "has no column",
        ),
        (
            json!({"table": "Sales", "column": "Product", "kind": "bin", "bin_size": 1}),
            "not a number or a date",
        ),
        (json!({"table": "Sales", "column": "Product"}), "needs `groups`"),
        (
            json!({"table": "Sales", "column": "Product", "groups": [
                {"name": "A", "values": ["Bike"]}, {"name": "B", "values": ["bike"]}]}),
            "more than one group",
        ),
        (
            json!({"table": "Sales", "column": "Product", "groups": [{"name": "A", "values": [3]}]}),
            "does not fit",
        ),
        (
            json!({"table": "Sales", "column": "Amount", "bin_size": 0}),
            "greater than 0",
        ),
        (
            json!({"table": "Sales", "column": "Amount", "bin_size": 1, "bin_count": 2}),
            "not both",
        ),
        (
            json!({"table": "Sales", "column": "Date", "unit": "month", "bin_size": 1.5}),
            "whole number",
        ),
        (
            json!({"table": "Sales", "column": "Amount", "name": "Amount", "bin_size": 1}),
            "already has",
        ),
    ] {
        let e = s.execute("column.group", &p).unwrap_err();
        assert!(e.contains(why), "{p}: {e}");
        assert_eq!(s.model, before, "{p}");
    }
    // a column that is not a group is never replaced
    let e = s
        .execute(
            "column.group",
            &json!({"table": "Sales", "column": "Amount", "name": "Qty", "bin_size": 1, "replace": true}),
        )
        .unwrap_err();
    assert!(e.contains("not a group column"), "{e}");
}

#[test]
fn edit_groups_reads_the_definition_back_and_replaces_the_column() {
    let mut s = shop();
    list_group(&mut s);
    let g = snapshot_column(&s, "Product (groups)")["group"].clone();
    assert_eq!(
        g,
        json!({"column": "Product", "kind": "list", "other": true, "groups": [
            {"name": "Wheels", "values": ["Bike", "Car"]},
            {"name": "Toys", "values": ["Ball", "Kite"]},
        ]})
    );
    // Edit groups: Doll joins Toys
    run(
        &mut s,
        "column.group",
        json!({"table": "Sales", "column": "Product", "replace": true, "groups": [
            {"name": "Wheels", "values": ["Bike", "Car"]},
            {"name": "Toys", "values": ["Ball", "Kite", "Doll"]},
        ]}),
    );
    let cols = &s.model.as_ref().unwrap().table("Sales").unwrap().columns;
    assert_eq!(cols.iter().filter(|c| c.name == "Product (groups)").count(), 1);
    assert_eq!(
        totals(&mut s, "Product (groups)"),
        pairs(&[("(Blank)", 7.0), ("Toys", 65.0), ("Wheels", 1150.0)])
    );
}

#[test]
fn a_visual_on_a_group_column_shows_its_groups() {
    let mut s = shop();
    run(&mut s, "report.new", json!({}));
    list_group(&mut s);
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    let r = run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "tableEx", "x": 0, "y": 0, "width": 300, "height": 200,
               "fields": [{"role": "Values", "field": "Sales[Product (groups)]"}, {"role": "Values", "field": "[Total]"}]}),
    );
    let name = r["visual"].as_str().unwrap().to_string();
    let snap = s.snapshot();
    let v = snap["report"]["pages"][0]["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == name.as_str())
        .unwrap()
        .clone();
    assert_eq!(v["data"]["status"], "ok", "{}", v["data"]);
    let rows = v["data"]["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 4, "{rows:?}");
}

/// Saves `s` as `file` in a new folder and opens it again.
fn reopened(s: &mut Session, dir: &TempDir, file: &str) -> Session {
    let out = dir.path().join(file);
    run(s, "file.save_as", json!({"path": out.to_string_lossy()}));
    let mut t = Session::new();
    run(&mut t, "file.open", json!({"path": out.to_string_lossy()}));
    t
}

fn groups(s: &mut Session) {
    list_group(s);
    run(
        s,
        "column.group",
        json!({"table": "Sales", "column": "Amount", "bin_size": 50}),
    );
}

#[test]
fn groups_survive_a_pbip_save_and_reopen() {
    let dir = TempDir::new("bicraft-groups-pbip");
    let mut s = shop();
    groups(&mut s);
    let saved = s.model.as_ref().unwrap().table("Sales").unwrap().clone();
    let mut t = reopened(&mut s, &dir, "Shop.pbip");
    // the TMDL file holds the expression and the extended property
    let tmdl =
        std::fs::read_to_string(dir.path().join("Shop.SemanticModel/definition/tables/Sales.tmdl")).unwrap();
    assert!(tmdl.contains("column 'Product (groups)' ="), "{tmdl}");
    assert!(tmdl.contains("extendedProperty GroupingMetadata ="), "{tmdl}");
    let back = t.model.as_ref().unwrap().table("Sales").unwrap().clone();
    for name in ["Product (groups)", "Amount (bins)"] {
        let (a, b) = (saved.column(name).unwrap(), back.column(name).unwrap());
        assert_eq!(a.expression, b.expression, "{name}");
        assert_eq!(
            a.extra["extendedProperties"], b.extra["extendedProperties"],
            "{name}"
        );
        assert_eq!(a.data_type, b.data_type, "{name}");
    }
    assert_eq!(
        snapshot_column(&t, "Product (groups)")["group"]["groups"][1]["name"],
        "Toys"
    );
    assert_eq!(
        totals(&mut t, "Product (groups)"),
        pairs(&[
            ("(Blank)", 7.0),
            ("Other", 18.0),
            ("Toys", 47.0),
            ("Wheels", 1150.0)
        ])
    );
}

#[test]
fn groups_survive_a_pbix_save_and_reopen_with_their_values() {
    let dir = TempDir::new("bicraft-groups-pbix");
    let mut s = shop();
    groups(&mut s);
    let saved = s.model.as_ref().unwrap().table("Sales").unwrap().clone();
    let mut t = reopened(&mut s, &dir, "Shop.pbix");
    let back = t.model.as_ref().unwrap().table("Sales").unwrap().clone();
    for name in ["Product (groups)", "Amount (bins)"] {
        let (a, b) = (saved.column(name).unwrap(), back.column(name).unwrap());
        assert_eq!(a.expression, b.expression, "{name}");
        assert_eq!(
            a.extra["extendedProperties"], b.extra["extendedProperties"],
            "{name}"
        );
    }
    assert_eq!(
        snapshot_column(&t, "Amount (bins)")["group"]["bin_size"],
        json!(50)
    );
    assert_eq!(
        totals(&mut t, "Product (groups)"),
        pairs(&[
            ("(Blank)", 7.0),
            ("Other", 18.0),
            ("Toys", 47.0),
            ("Wheels", 1150.0)
        ])
    );
    // a group added after the data was stored is computed too
    run(
        &mut t,
        "column.group",
        json!({"table": "Sales", "column": "Product", "name": "Kind", "groups": [{"name": "Car", "values": ["Car"]}]}),
    );
    assert_eq!(
        totals(&mut t, "Kind"),
        pairs(&[("(Blank)", 7.0), ("Car", 1000.0), ("Other", 215.0)])
    );
}

/// The bin of each date of `column`: (date, bin) for every row, sorted by date.
fn date_bins(s: &mut Session, column: &str) -> Vec<(String, String)> {
    let q = format!("EVALUATE SUMMARIZECOLUMNS('Sales'[Date], 'Sales'[{column}]) ORDER BY 'Sales'[Date]");
    let v = run(s, "dax.evaluate", json!({ "query": q }));
    v["results"][0]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["Sales[Date]"].as_str().unwrap()[..10].to_string(),
                r[format!("Sales[{column}]")].as_str().unwrap()[..10].to_string(),
            )
        })
        .collect()
}

/// A number of bins on dates (G75): the size in units comes from the data's first and last date,
/// written as a bin size (Desktop's date form); not seen in a file Desktop saved.
#[test]
fn a_number_of_bins_on_dates_sizes_them_from_the_first_and_last_date() {
    let mut s = shop();
    // 2024-01-05 to 2025-08-01: two years, so four bins are counted in months, 20 months / 4 = 5
    let r = run(
        &mut s,
        "column.group",
        json!({"table": "Sales", "column": "Date", "name": "Four", "bin_count": 4}),
    );
    assert_eq!(r["unit"], "month", "{r}");
    assert_eq!(r["bin_size"], json!(5), "{r}");
    assert_eq!(r["bin_count"], json!(4), "{r}");
    // each date falls in the five-month bin that holds it (months restart each year)
    for (date, bin) in date_bins(&mut s, "Four") {
        let (y, m): (i32, i32) = (date[..4].parse().unwrap(), date[5..7].parse().unwrap());
        let start = 1 + (m - 1) / 5 * 5;
        assert_eq!(bin, format!("{y:04}-{start:02}-01"), "{date}");
    }
    // stored as a bin size of months, as Edit groups reads it back
    assert_eq!(
        ext(&s, "Four")["value"]["binningMetadata"],
        json!({"binSize": {"value": 5.0, "unit": 6}})
    );
    assert_eq!(
        snapshot_column(&s, "Four")["group"],
        json!({"column": "Date", "kind": "bin", "bin_size": 5, "unit": "month"})
    );
    // a unit given is kept: 2 bins of years over 2024-2025 are one year each
    let r = run(
        &mut s,
        "column.group",
        json!({"table": "Sales", "column": "Date", "name": "Years", "unit": "year", "bin_count": 2}),
    );
    assert_eq!(
        (r["unit"].clone(), r["bin_size"].clone()),
        (json!("year"), json!(1))
    );
    let years: Vec<String> = date_bins(&mut s, "Years").into_iter().map(|(_, b)| b).collect();
    assert!(
        years.iter().all(|b| b == "2024-01-01" || b == "2025-01-01"),
        "{years:?}"
    );
    // a count that a unit cannot hold in 10000 units is refused, and changes nothing
    let before = s.model.clone();
    let e = s
        .execute(
            "column.group",
            &json!({"table": "Sales", "column": "Date", "name": "X", "unit": "second", "bin_count": 1}),
        )
        .unwrap_err();
    assert!(e.contains("larger unit"), "{e}");
    assert_eq!(s.model, before);
}

/// A list group listing a blank (G75): the blank is an explicit member of its group, the `null`
/// literal of the design state, with no `(Blank)` placeholder; it round-trips through a PBIP
/// save. BIcraft's own form: Desktop's dialog never lists a blank, so it is not compared with
/// Desktop.
#[test]
fn a_blank_listed_in_a_group_is_a_member_of_its_design_state_and_round_trips() {
    let dir = TempDir::new("bicraft-groups-blank");
    let mut s = shop();
    run(
        &mut s,
        "column.group",
        json!({"table": "Sales", "column": "Product", "groups": [
            {"name": "Wheels", "values": ["Bike", null]}, {"name": "Toys", "values": ["Ball"]}]}),
    );
    let col = json!({"Column": {"Expression": {"SourceRef": {"Source": "s"}}, "Property": "Product"}});
    let lit = |v: &str| json!([{"Literal": {"Value": v}}]);
    let d = design_state(&s, "Product (groups)").expect("a design state");
    assert_eq!(
        d["GroupItems"],
        json!([
            {"DisplayName": "Wheels", "Expression": {"In": {"Expressions": [col], "Values": [lit("'Bike'"), lit("null")]}}},
            {"DisplayName": "Toys", "Expression": {"In": {"Expressions": [col], "Values": [lit("'Ball'")]}}},
            {"DisplayName": "Other"}])
    );
    // the blank row is counted in Wheels
    assert_eq!(
        totals(&mut s, "Product (groups)"),
        pairs(&[("Other", 1060.0), ("Toys", 5.0), ("Wheels", 157.0)])
    );
    let t = reopened(&mut s, &dir, "Shop.pbip");
    assert_eq!(design_state(&t, "Product (groups)"), Some(d));
    assert_eq!(
        snapshot_column(&t, "Product (groups)")["group"],
        json!({"column": "Product", "kind": "list", "other": true, "groups": [
            {"name": "Wheels", "values": [null, "Bike"]}, {"name": "Toys", "values": ["Ball"]}]})
    );
}

/// Sales by Product in a column chart on the first page: (page, visual).
fn product_chart(s: &mut Session) -> (String, String) {
    run(s, "report.new", json!({}));
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    let r = run(
        s,
        "visual.add",
        json!({"page": page, "visual_type": "clusteredColumnChart", "x": 0, "y": 0, "width": 400, "height": 300,
               "fields": [{"role": "Category", "field": "Sales[Product]"}, {"role": "Y", "field": "[Total]"}]}),
    );
    (page, r["visual"].as_str().unwrap().to_string())
}

/// Group data on points selected in a visual (G75): Ctrl+click two bars, then the group holds
/// exactly their values; a second selection joins the same group column.
#[test]
fn group_data_groups_exactly_the_selected_points() {
    let mut s = shop();
    let (page, chart) = product_chart(&mut s);
    // nothing selected: refused, nothing changes
    let before = s.model.clone();
    let e = s
        .execute("visual.group_points", &json!({"page": page, "visual": chart}))
        .unwrap_err();
    assert!(e.contains("selected"), "{e}");
    assert_eq!(s.model, before);
    for v in ["Bike", "Kite"] {
        run(
            &mut s,
            "view.interact",
            json!({"action": "select", "page": page, "visual": chart, "ctrl": true, "point": [{"c": 0, "v": v}]}),
        );
    }
    let r = run(
        &mut s,
        "visual.group_points",
        json!({"page": page, "visual": chart}),
    );
    assert_eq!(r["column"], "Product (groups)", "{r}");
    assert_eq!(r["group"], "Bike & Kite", "{r}");
    assert_eq!(
        snapshot_column(&s, "Product (groups)")["group"]["groups"],
        json!([{"name": "Bike & Kite", "values": ["Bike", "Kite"]}])
    );
    assert_eq!(
        totals(&mut s, "Product (groups)"),
        pairs(&[("(Blank)", 7.0), ("Bike & Kite", 192.0), ("Other", 1023.0)])
    );
    // one undo step
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.model, before);
    run(&mut s, "edit.redo", json!({}));
    // given points: Kite moves to a new group, Bike stays alone in the first
    let r = run(
        &mut s,
        "visual.group_points",
        json!({"page": page, "visual": chart, "group": "Air",
               "points": [[{"c": 0, "v": "Kite"}], [{"c": 0, "v": "Doll"}]]}),
    );
    assert_eq!(r["values"], json!(["Kite", "Doll"]));
    assert_eq!(
        snapshot_column(&s, "Product (groups)")["group"]["groups"],
        json!([{"name": "Bike & Kite", "values": ["Bike"]}, {"name": "Air", "values": ["Kite", "Doll"]}])
    );
}
