//! Tests of "Show value as" (`visual.show_value_as`) and "Show items with no data"
//! (`visual.show_items_no_data`): the commands, the values the preview computes, the PBIR / PBIP
//! round trip and hostile parameters.

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

/// Sales(Product, Region, Amount) and Product(Product, Category), many-to-one. Wheels sells
/// 400 in North and 50 in South, Toys 50 in South; Drone (Toys) and Doll (Games) sell nothing.
fn shop() -> Session {
    let mut s = Session::new();
    let sales = r#"#table({"Product", "Region", "Amount"}, {{"Bike", "North", 100}, {"Bike", "South", 50}, {"Car", "North", 300}, {"Ball", "South", 50}})"#;
    let product = r#"#table({"Product", "Category"}, {{"Bike", "Wheels"}, {"Car", "Wheels"}, {"Ball", "Toys"}, {"Drone", "Toys"}, {"Doll", "Games"}})"#;
    let steps = [
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
            "column.add",
            json!({"table": "Sales", "name": "Product", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Region", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
        ),
        (
            "column.add",
            json!({"table": "Product", "name": "Product", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Product", "name": "Category", "data_type": "string"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        (
            "relationship.add",
            json!({"from": "Sales[Product]", "to": "Product[Product]"}),
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
        json!({"page": "Main", "visual_type": visual_type, "x": 0, "y": 0, "width": 300, "height": 200,
               "fields": fields}),
    );
    r["visual"].as_str().unwrap_or_default().to_string()
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

/// The rows of `d` keyed by their first `n` cells, with the cell at `at`.
fn cells(rows: &Value, n: usize, at: usize) -> Vec<(Vec<Value>, Value)> {
    let mut out: Vec<(Vec<Value>, Value)> = rows
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| {
            let a = r.as_array().cloned().unwrap_or_default();
            (
                a.iter().take(n).cloned().collect(),
                a.get(at).cloned().unwrap_or(Value::Null),
            )
        })
        .collect();
    out.sort_by_key(|(k, _)| Value::Array(k.clone()).to_string());
    out
}

fn close(v: &Value, x: f64) -> bool {
    v.as_f64().is_some_and(|y| (y - x).abs() < 1e-12)
}

/// A matrix: Rows Category, Columns Region, Values Sum(Amount).
fn matrix(s: &mut Session) -> String {
    add_visual(
        s,
        "pivotTable",
        json!([{"role": "Rows", "field": "Product[Category]"},
               {"role": "Columns", "field": "Sales[Region]"},
               {"role": "Values", "field": "Sum(Sales[Amount])"}]),
    )
}

fn show_as(s: &mut Session, v: &str, kind: &str) -> Value {
    run(
        s,
        "visual.show_value_as",
        json!({"page": "Main", "visual": v, "role": "Values", "index": 0, "as": kind}),
    )
}

fn subtotal(d: &Value, rows: u64, columns: u64) -> Value {
    d["subtotals"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|l| l["rows"] == rows && l["columns"] == columns)
        .map(|l| l["data"].clone())
        .unwrap_or_else(|| panic!("no subtotal level {rows}/{columns} in {d}"))
}

#[test]
fn percent_of_grand_total_divides_every_cell_by_the_grand_total() {
    let mut s = shop();
    let m = matrix(&mut s);
    let out = show_as(&mut s, &m, "percent_of_grand_total");
    assert_eq!(out["as"], "percent_of_grand_total");
    assert_eq!(out["fields"][2]["show_as"], "percent_of_grand_total");
    let d = data(&s, &m);
    let c = cells(&d["rows"], 2, 2);
    let want = [
        (["Toys", "South"], 0.1),
        (["Wheels", "North"], 0.8),
        (["Wheels", "South"], 0.1),
    ];
    assert_eq!(c.len(), want.len(), "{d}");
    for ((k, v), (wk, wv)) in c.iter().zip(want) {
        assert_eq!(k, &vec![json!(wk[0]), json!(wk[1])]);
        assert!(close(v, wv), "{k:?}: {v} (want {wv})");
    }
    assert_eq!(d["columns"][2]["name"], "%GT Sum of Amount");
    assert_eq!(d["columns"][2]["format_string"], "0.00%");
    // the totals: the grand total is 100 %, the column totals 80 % and 20 %
    assert!(close(&subtotal(&d, 0, 0)[0][2], 1.0));
    let cols = cells(&subtotal(&d, 0, 1), 2, 2);
    assert!(close(&cols[0].1, 0.8) && close(&cols[1].1, 0.2), "{cols:?}");
    // the snapshot's field says so; back to the value
    assert_eq!(visual(&s, &m)["fields"][2]["show_as"], "percent_of_grand_total");
    show_as(&mut s, &m, "none");
    let d = data(&s, &m);
    assert_eq!(d["columns"][2]["name"], "Sum of Amount");
    assert!(cells(&d["rows"], 2, 2).iter().any(|(_, v)| close(v, 400.0)));
}

#[test]
fn percent_of_column_and_row_totals_keep_the_matrix_columns_or_rows() {
    let mut s = shop();
    let m = matrix(&mut s);
    show_as(&mut s, &m, "percent_of_column_total");
    let d = data(&s, &m);
    let c = cells(&d["rows"], 2, 2);
    // North: Wheels 400 / 400; South: Wheels 50 / 100, Toys 50 / 100
    assert!(
        close(&c[0].1, 0.5) && close(&c[1].1, 1.0) && close(&c[2].1, 0.5),
        "{c:?}"
    );
    assert_eq!(d["columns"][2]["name"], "%CT Sum of Amount");
    // the row totals (no column kept): their share of the grand total
    let rt = cells(&subtotal(&d, 1, 0), 1, 2);
    assert!(close(&rt[0].1, 0.1) && close(&rt[1].1, 0.9), "{rt:?}");
    show_as(&mut s, &m, "percent_of_row_total");
    let d = data(&s, &m);
    let c = cells(&d["rows"], 2, 2);
    // Toys: 50 / 50; Wheels: 400 / 450 and 50 / 450
    assert!(
        close(&c[0].1, 1.0) && close(&c[1].1, 400.0 / 450.0) && close(&c[2].1, 50.0 / 450.0),
        "{c:?}"
    );
    assert_eq!(d["columns"][2]["name"], "%RT Sum of Amount");
    let rt = cells(&subtotal(&d, 1, 0), 1, 2);
    assert!(close(&rt[0].1, 1.0) && close(&rt[1].1, 1.0), "{rt:?}");
    // a renamed field keeps its name
    run(
        &mut s,
        "visual.rename_field",
        json!({"page": "Main", "visual": m, "role": "Values", "index": 0, "name": "Share"}),
    );
    assert_eq!(data(&s, &m)["columns"][2]["name"], "Share");
}

#[test]
fn a_ratio_measure_is_divided_by_its_evaluated_total() {
    let mut s = shop();
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Avg", "expression": "AVERAGE(Sales[Amount])"}),
    );
    let t = add_visual(
        &mut s,
        "tableEx",
        json!([{"role": "Values", "field": "Sales[Region]"}, {"role": "Values", "field": "[Avg]"}]),
    );
    run(
        &mut s,
        "visual.show_value_as",
        json!({"page": "Main", "visual": t, "role": "Values", "field": "[Avg]", "as": "percent_of_grand_total"}),
    );
    let d = data(&s, &t);
    // North avg 200, South avg 50, overall avg 125 (not the sum of the rows)
    let c = cells(&d["rows"], 1, 1);
    assert!(
        close(&c[0].1, 200.0 / 125.0) && close(&c[1].1, 50.0 / 125.0),
        "{c:?}"
    );
    assert!(close(&d["totals"][1], 1.0), "{d}");
}

#[test]
fn show_items_with_no_data_lists_the_empty_categories() {
    let mut s = shop();
    let c = add_visual(
        &mut s,
        "clusteredColumnChart",
        json!([{"role": "Category", "field": "Product[Category]"}, {"role": "Y", "field": "[Total]"}]),
    );
    let before = cells(&data(&s, &c)["rows"], 1, 1);
    assert_eq!(before.len(), 2, "{before:?}");
    let out = run(
        &mut s,
        "visual.show_items_no_data",
        json!({"page": "Main", "visual": c, "role": "Category", "show": true}),
    );
    assert_eq!(out["show"], true);
    assert_eq!(visual(&s, &c)["fields"][0]["show_all"], true);
    let after = cells(&data(&s, &c)["rows"], 1, 1);
    assert_eq!(
        after,
        vec![
            (vec![json!("Games")], Value::Null),
            (vec![json!("Toys")], json!(50)),
            (vec![json!("Wheels")], json!(450)),
        ]
    );
    // a filter on the category's table keeps its rows out
    run(
        &mut s,
        "visual.filter",
        json!({"page": "Main", "visual": c, "field": "Product[Category]", "in": ["Wheels", "Games"]}),
    );
    let filtered = cells(&data(&s, &c)["rows"], 1, 1);
    assert_eq!(
        filtered,
        vec![
            (vec![json!("Games")], Value::Null),
            (vec![json!("Wheels")], json!(450))
        ]
    );
    // two fields of one table: the combinations its rows hold
    let t = add_visual(
        &mut s,
        "tableEx",
        json!([{"role": "Values", "field": "Product[Category]"}, {"role": "Values", "field": "Product[Product]"},
               {"role": "Values", "field": "[Total]"}]),
    );
    run(
        &mut s,
        "visual.show_items_no_data",
        json!({"page": "Main", "visual": t, "role": "Values", "show": true}),
    );
    let rows = cells(&data(&s, &t)["rows"], 2, 2);
    assert_eq!(rows.len(), 5, "{rows:?}");
    assert!(rows.contains(&(vec![json!("Toys"), json!("Drone")], Value::Null)));
    assert!(rows.contains(&(vec![json!("Wheels"), json!("Car")], json!(300))));
    // off again
    run(
        &mut s,
        "visual.show_items_no_data",
        json!({"page": "Main", "visual": c, "role": "Category", "show": false}),
    );
    assert_eq!(cells(&data(&s, &c)["rows"], 1, 1).len(), 1);
    assert!(visual(&s, &c)["fields"][0].get("show_all").is_none());
}

/// Power BI Desktop 2.159.1231.0 (2026-11-23) draws a chart with a percentage of the grand total
/// without its empty categories even when Show items with no data is on, and flags the two as
/// not compatible: the preview does the same and both commands warn.
#[test]
fn show_items_with_no_data_is_off_beside_a_percentage_of_the_grand_total() {
    let mut s = shop();
    let c = add_visual(
        &mut s,
        "clusteredColumnChart",
        json!([{"role": "Category", "field": "Product[Category]"}, {"role": "Y", "field": "[Total]"}]),
    );
    let out = run(
        &mut s,
        "visual.show_items_no_data",
        json!({"page": "Main", "visual": c, "role": "Category", "show": true}),
    );
    assert!(out.get("warnings").is_none(), "{out}");
    assert_eq!(cells(&data(&s, &c)["rows"], 1, 1).len(), 3);
    let out = run(
        &mut s,
        "visual.show_value_as",
        json!({"page": "Main", "visual": c, "role": "Y", "index": 0, "as": "percent_of_grand_total"}),
    );
    assert!(
        out["warnings"][0].as_str().unwrap().contains("not compatible"),
        "{out}"
    );
    let d = data(&s, &c);
    let rows = cells(&d["rows"], 1, 1);
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert!(!rows.iter().any(|(k, _)| k[0] == "Games"), "{rows:?}");
    assert!(d.to_string().contains("not compatible"), "{d}");
    // asked in the other order, the same warning
    run(
        &mut s,
        "visual.show_items_no_data",
        json!({"page": "Main", "visual": c, "role": "Category", "show": false}),
    );
    let out = run(
        &mut s,
        "visual.show_items_no_data",
        json!({"page": "Main", "visual": c, "role": "Category", "show": true}),
    );
    assert!(
        out["warnings"][0].as_str().unwrap().contains("not compatible"),
        "{out}"
    );
}

#[test]
fn both_options_survive_a_pbip_save_and_reopen() {
    let mut s = shop();
    let m = matrix(&mut s);
    show_as(&mut s, &m, "percent_of_row_total");
    run(
        &mut s,
        "visual.show_items_no_data",
        json!({"page": "Main", "visual": m, "role": "Rows", "show": true}),
    );
    let dir = crate::tempdir::TempDir::new("engine-show-as");
    let path = dir.path().join("Shop.pbip");
    run(&mut s, "file.save_as", json!({"path": path.to_string_lossy()}));
    // the PBIR query as written
    let visual_json = walk(dir.path())
        .into_iter()
        .find(|p| {
            p.file_name().is_some_and(|n| n == "visual.json")
                && std::fs::read_to_string(p).is_ok_and(|t| t.contains("pivotTable"))
        })
        .unwrap_or_else(|| panic!("no matrix visual.json under {}", dir.path().display()));
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(&visual_json).unwrap_or_default())
        .unwrap_or_else(|e| panic!("{e}"));
    let qs = &doc["visual"]["query"]["queryState"];
    assert_eq!(qs["Rows"]["showAll"], true, "{qs}");
    let f = &qs["Values"]["projections"][0]["field"]["Arithmetic"];
    assert_eq!(f["Operator"], 3, "{qs}");
    assert_eq!(
        f["Right"]["ScopedEval"]["Scope"],
        json!([{"Column": {"Expression": {"SourceRef": {"Entity": "Product"}}, "Property": "Category"}}])
    );
    let orig = s
        .report
        .as_ref()
        .unwrap_or_else(|| panic!("no report"))
        .pages
        .iter()
        .flat_map(|p| &p.visuals)
        .find(|v| v.name == m)
        .map(|v| v.fields.clone());
    // PBIP, and PBIX (the legacy layout: the expression in prototypeQuery, showAllRoles)
    let pbix = dir.path().join("Shop.pbix");
    run(&mut s, "file.save_as", json!({"path": pbix.to_string_lossy()}));
    for file in [path, pbix] {
        let mut back = Session::new();
        run(&mut back, "file.open", json!({"path": file.to_string_lossy()}));
        let v = back
            .report
            .as_ref()
            .unwrap_or_else(|| panic!("no report"))
            .pages
            .iter()
            .flat_map(|p| &p.visuals)
            .find(|v| v.name == m)
            .cloned()
            .unwrap_or_else(|| panic!("no visual {m} in {}", file.display()));
        assert_eq!(
            v.fields[2].show_as,
            Some(bicraft_report::ShowAs::RowTotal),
            "{}",
            file.display()
        );
        assert!(bicraft_report::query::show_all(
            v.visual_extra.get("query"),
            "Rows"
        ));
        assert_eq!(Some(v.fields), orig, "{}", file.display());
        // and the reopened file computes the same percentages
        let d = data(&back, &m);
        assert!(cells(&d["rows"], 2, 2).iter().any(|(_, x)| close(x, 1.0)), "{d}");
    }
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p);
            }
        }
    }
    out
}

#[test]
fn the_commands_refuse_what_they_cannot_do() {
    let mut s = shop();
    let m = matrix(&mut s);
    let c = add_visual(
        &mut s,
        "clusteredColumnChart",
        json!([{"role": "Category", "field": "Product[Category]"}, {"role": "Y", "field": "[Total]"}]),
    );
    let sva = "visual.show_value_as";
    let p = |v: &str, role: &str, extra: Value| {
        let mut x = json!({"page": "Main", "visual": v, "role": role});
        if let (Some(o), Some(e)) = (x.as_object_mut(), extra.as_object()) {
            o.extend(e.clone());
        }
        x
    };
    // a category is no value; column totals are a matrix's; a value already shown that way
    assert!(
        fails(
            &mut s,
            sva,
            p(&m, "Rows", json!({"index": 0, "as": "percent_of_grand_total"}))
        )
        .contains("not a value")
    );
    assert!(
        fails(
            &mut s,
            sva,
            p(&c, "Y", json!({"index": 0, "as": "percent_of_column_total"}))
        )
        .contains("matrix")
    );
    assert!(fails(&mut s, sva, p(&m, "Values", json!({"index": 0, "as": "none"}))).contains("already"));
    for bad in [
        json!({"index": 0, "as": "half"}),
        json!({"index": 0, "as": 3}),
        json!({"index": 0}),
        json!({"index": 9, "as": "none"}),
        json!({"as": "none"}),
        json!({"index": 0, "as": "none", "x": 1}),
    ] {
        fails(&mut s, sva, p(&m, "Values", bad));
    }
    fails(
        &mut s,
        sva,
        p("nope", "Values", json!({"index": 0, "as": "none"})),
    );
    // a matrix without a column in Columns has no column total to show
    let lone = add_visual(
        &mut s,
        "pivotTable",
        json!([{"role": "Rows", "field": "Product[Category]"}, {"role": "Values", "field": "[Total]"}]),
    );
    assert!(
        fails(
            &mut s,
            sva,
            p(
                &lone,
                "Values",
                json!({"index": 0, "as": "percent_of_column_total"})
            )
        )
        .contains("Columns")
    );
    let sid = "visual.show_items_no_data";
    assert!(fails(&mut s, sid, p(&c, "Y", json!({"show": true}))).contains("no category"));
    assert!(fails(&mut s, sid, p(&c, "Category", json!({"show": false}))).contains("does not show"));
    for bad in [
        json!({}),
        json!({"show": "yes"}),
        json!({"show": true, "index": 0}),
    ] {
        fails(&mut s, sid, p(&c, "Category", bad));
    }
    // each change is one undo step
    run(&mut s, sid, p(&c, "Category", json!({"show": true})));
    run(&mut s, "edit.undo", json!({}));
    assert!(visual(&s, &c)["fields"][0].get("show_all").is_none());
}

/// The members of a subtotal level of `d` (its first cell) with their value at `at`.
fn level_members(d: &Value, rows: u64, at: usize) -> Vec<(Value, Value)> {
    cells(&subtotal(d, rows, 0), 1, at)
        .into_iter()
        .map(|(k, v)| (k.first().cloned().unwrap_or(Value::Null), v))
        .collect()
}

#[test]
fn a_matrix_subtotal_level_lists_its_items_with_no_data() {
    // Rows Category › Product: Games (Doll) sells nothing, so it has a leaf row with no value
    // once Rows shows its items with no data, and its Category subtotal must be listed too
    let mut s = shop();
    let m = add_visual(
        &mut s,
        "pivotTable",
        json!([{"role": "Rows", "field": "Product[Category]"}, {"role": "Rows", "field": "Product[Product]"},
               {"role": "Values", "field": "[Total]"}]),
    );
    let before = level_members(&data(&s, &m), 1, 2);
    assert!(!before.iter().any(|(k, _)| k == "Games"), "{before:?}");
    run(
        &mut s,
        "visual.show_items_no_data",
        json!({"page": "Main", "visual": m, "role": "Rows", "show": true}),
    );
    let d = data(&s, &m);
    let leaves = cells(&d["rows"], 2, 2);
    assert!(
        leaves.contains(&(vec![json!("Games"), json!("Doll")], Value::Null)),
        "{leaves:?}"
    );
    let after = level_members(&d, 1, 2);
    // the empty category joins the level; the others keep their subtotals
    assert!(after.contains(&(json!("Games"), Value::Null)), "{after:?}");
    for (k, v) in &before {
        assert!(after.contains(&(k.clone(), v.clone())), "{k}: {after:?}");
    }
    assert!(after.len() > before.len());
}

#[test]
fn a_percentage_over_hierarchy_level_rows_writes_the_levels_in_its_scope() {
    let mut s = shop();
    run(
        &mut s,
        "hierarchy.add",
        json!({"table": "Product", "name": "Range", "levels": ["Category", "Product"]}),
    );
    let m = add_visual(
        &mut s,
        "pivotTable",
        json!([{"role": "Columns", "field": "Sales[Region]"}, {"role": "Values", "field": "Sum(Sales[Amount])"}]),
    );
    run(
        &mut s,
        "visual.bind_hierarchy",
        json!({"page": "Main", "visual": m, "role": "Rows", "hierarchy": "Range"}),
    );
    let at = visual(&s, &m)["fields"]
        .as_array()
        .and_then(|f| f.iter().position(|x| x["role"] == "Values"))
        .unwrap_or_else(|| panic!("no value field"));
    show_as(&mut s, &m, "percent_of_row_total");
    // the top level shows: Wheels in North is 400 of its row's 450
    let d = data(&s, &m);
    assert!(
        d["rows"].as_array().into_iter().flatten().any(|r| r
            .as_array()
            .is_some_and(|a| a.contains(&json!("Wheels")) && a.contains(&json!("North")))
            && close(&r[at], 400.0 / 450.0)),
        "{d}"
    );
    let dir = crate::tempdir::TempDir::new("engine-show-as-levels");
    let path = dir.path().join("Shop.pbip");
    run(&mut s, "file.save_as", json!({"path": path.to_string_lossy()}));
    let visual_json = walk(dir.path())
        .into_iter()
        .find(|p| {
            p.file_name().is_some_and(|n| n == "visual.json")
                && std::fs::read_to_string(p).is_ok_and(|t| t.contains("pivotTable"))
        })
        .unwrap_or_else(|| panic!("no matrix visual.json under {}", dir.path().display()));
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(&visual_json).unwrap_or_default())
        .unwrap_or_else(|e| panic!("{e}"));
    let qs = &doc["visual"]["query"]["queryState"];
    let scope = qs["Values"]["projections"][0]["field"]["Arithmetic"]["Right"]["ScopedEval"]["Scope"].clone();
    let rows: Vec<Value> = qs["Rows"]["projections"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| p["field"].clone())
        .collect();
    // the scope is the row levels, as their projections name them
    assert_eq!(scope, Value::Array(rows.clone()), "{qs}");
    assert!(
        rows.len() == 2 && rows.iter().all(|f| f.get("HierarchyLevel").is_some()),
        "{qs}"
    );
    let orig = s
        .report
        .as_ref()
        .and_then(|r| r.pages.iter().flat_map(|p| &p.visuals).find(|v| v.name == m))
        .map(|v| v.fields.clone());
    let pbix = dir.path().join("Shop.pbix");
    run(&mut s, "file.save_as", json!({"path": pbix.to_string_lossy()}));
    for file in [path, pbix] {
        let mut back = Session::new();
        run(&mut back, "file.open", json!({"path": file.to_string_lossy()}));
        let fields = back
            .report
            .as_ref()
            .and_then(|r| r.pages.iter().flat_map(|p| &p.visuals).find(|v| v.name == m))
            .map(|v| v.fields.clone());
        assert_eq!(fields, orig, "{}", file.display());
    }
}
