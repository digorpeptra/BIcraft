//! Tests of visual calculations and hidden fields: the commands (`visual.calculation.add`,
//! `.set`, `.remove`), the values the preview computes over the visual matrix, the PBIR / PBIX
//! round trips and hostile parameters.
//!
//! The expected values are computed by hand from the semantics Microsoft Learn documents
//! ("Visual calculations overview" and the function pages); they are not checked against the
//! engine (docs/gaps.md).

use serde_json::{Value, json};

use crate::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn fails(s: &mut Session, id: &str, params: Value) -> String {
    let before = s.report.clone();
    let err = s
        .execute(id, &params)
        .expect_err(&format!("{id} {params} should fail"));
    assert_eq!(s.report, before, "{id} changed the report although it failed");
    err
}

/// Sales(Product, Region, Amount) and Product(Product, Category), many-to-one: Wheels sells
/// Bike 100 North and 50 South, Car 300 North; Toys sells Ball 50 South.
fn shop() -> Session {
    let mut s = Session::new();
    let sales = r#"#table({"Product", "Region", "Amount"}, {{"Bike", "North", 100}, {"Bike", "South", 50}, {"Car", "North", 300}, {"Ball", "South", 50}})"#;
    let product =
        r#"#table({"Product", "Category"}, {{"Bike", "Wheels"}, {"Car", "Wheels"}, {"Ball", "Toys"}})"#;
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

fn calc(s: &mut Session, v: &str, name: &str, expression: &str) -> Value {
    run(
        s,
        "visual.calculation.add",
        json!({"page": "Main", "visual": v, "name": name, "expression": expression}),
    )
}

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

/// The column of `d` named `name`.
fn col(d: &Value, name: &str) -> usize {
    d["columns"]
        .as_array()
        .and_then(|c| c.iter().position(|x| x["name"] == name))
        .unwrap_or_else(|| panic!("no column {name} in {}", d["columns"]))
}

/// `(first n cells joined, cell at)` of every row, sorted.
fn cells(rows: &Value, n: usize, at: usize) -> Vec<(String, Value)> {
    let mut out: Vec<(String, Value)> = rows
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| {
            let a = r.as_array().cloned().unwrap_or_default();
            let k: Vec<String> = a
                .iter()
                .take(n)
                .map(|x| x.as_str().map_or_else(|| x.to_string(), str::to_string))
                .collect();
            (k.join("/"), a.get(at).cloned().unwrap_or(Value::Null))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn approx(v: &Value, x: f64) -> bool {
    v.as_f64().is_some_and(|y| (y - x).abs() < 1e-9)
}

fn assert_cells(got: &[(String, Value)], want: &[(&str, Option<f64>)], what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: {got:?}");
    for ((k, v), (wk, wv)) in got.iter().zip(want) {
        assert_eq!(k, wk, "{what}: {got:?}");
        match wv {
            Some(x) => assert!(approx(v, *x), "{what} {k}: {v} instead of {x} ({got:?})"),
            None => assert!(v.is_null(), "{what} {k}: {v} instead of blank"),
        }
    }
}

/// A table by product: Ball 50, Bike 150, Car 300 (total 500).
fn table(s: &mut Session) -> String {
    add_visual(
        s,
        "tableEx",
        json!([{"role": "Values", "field": "Sales[Product]"},
               {"role": "Values", "field": "Sum(Sales[Amount])"}]),
    )
}

#[test]
fn window_shortcuts_walk_the_rows_of_a_table() {
    let mut s = shop();
    let t = table(&mut s);
    calc(&mut s, &t, "Running", "RUNNINGSUM([Sum of Amount])");
    calc(
        &mut s,
        &t,
        "Versus previous",
        "[Sum of Amount] - PREVIOUS([Sum of Amount])",
    );
    calc(&mut s, &t, "Next", "NEXT([Sum of Amount])");
    calc(&mut s, &t, "First", "FIRST([Sum of Amount])");
    calc(&mut s, &t, "Last", "LAST([Sum of Amount])");
    calc(&mut s, &t, "Moving", "MOVINGAVERAGE([Sum of Amount], 2)");
    calc(
        &mut s,
        &t,
        "Moving before",
        "MOVINGAVERAGE([Sum of Amount], 2, FALSE)",
    );
    calc(&mut s, &t, "Range", "AVERAGEX(RANGE(-1), [Sum of Amount])");
    calc(
        &mut s,
        &t,
        "Range sum",
        "CALCULATE(SUM([Sum of Amount]), RANGE(1, FALSE, ROWS))",
    );
    calc(
        &mut s,
        &t,
        "Share",
        "DIVIDE([Sum of Amount], COLLAPSEALL([Sum of Amount], ROWS))",
    );
    calc(&mut s, &t, "Level", "IF(ISATLEVEL([Product]), 1, 0)");
    // a calculation over another one
    calc(&mut s, &t, "Double running", "[Running] * 2");
    let d = data(&s, &t);
    let at = |n: &str| col(&d, n);
    let want = |name: &str, w: [Option<f64>; 3]| {
        let got = cells(&d["rows"], 1, at(name));
        assert_cells(&got, &[("Ball", w[0]), ("Bike", w[1]), ("Car", w[2])], name);
    };
    want("Running", [Some(50.0), Some(200.0), Some(500.0)]);
    want("Versus previous", [Some(50.0), Some(100.0), Some(150.0)]);
    want("Next", [Some(150.0), Some(300.0), None]);
    want("First", [Some(50.0); 3]);
    want("Last", [Some(300.0); 3]);
    want("Moving", [Some(50.0), Some(100.0), Some(225.0)]);
    want("Moving before", [None, Some(50.0), Some(100.0)]);
    want("Range", [Some(50.0), Some(100.0), Some(225.0)]);
    want("Range sum", [Some(150.0), Some(300.0), None]);
    want("Share", [Some(0.1), Some(0.3), Some(0.6)]);
    want("Level", [Some(1.0); 3]);
    want("Double running", [Some(100.0), Some(400.0), Some(1000.0)]);
    // the total row is the top level: a partition of one cell, not on the Product level
    let totals = d["totals"].as_array().cloned().unwrap_or_default();
    assert!(approx(&totals[at("Running")], 500.0), "{totals:?}");
    assert!(approx(&totals[at("Share")], 1.0), "{totals:?}");
    assert!(approx(&totals[at("Level")], 0.0), "{totals:?}");
    assert_eq!(d["columns"][at("Running")]["kind"], "visual_calculation");
    assert!(
        d["visual_calculations"]
            .as_array()
            .is_some_and(|a| a.iter().all(|x| x.get("reason").is_none())),
        "{d}"
    );
}

/// A matrix: Rows Category then Product, Columns Region, Values Sum(Amount).
fn matrix(s: &mut Session) -> String {
    add_visual(
        s,
        "pivotTable",
        json!([{"role": "Rows", "field": "Product[Category]"},
               {"role": "Rows", "field": "Sales[Product]"},
               {"role": "Columns", "field": "Sales[Region]"},
               {"role": "Values", "field": "Sum(Sales[Amount])"}]),
    )
}

#[test]
fn axes_resets_and_navigation_follow_the_matrix() {
    let mut s = shop();
    let m = matrix(&mut s);
    calc(
        &mut s,
        &m,
        "Parent",
        "DIVIDE([Sum of Amount], COLLAPSE([Sum of Amount], ROWS))",
    );
    calc(&mut s, &m, "Down", "RUNNINGSUM([Sum of Amount])");
    calc(&mut s, &m, "Across", "RUNNINGSUM([Sum of Amount], COLUMNS)");
    calc(
        &mut s,
        &m,
        "By category",
        "RUNNINGSUM([Sum of Amount], HIGHESTPARENT)",
    );
    calc(&mut s, &m, "By field", "RUNNINGSUM([Sum of Amount], [Category])");
    calc(&mut s, &m, "Children", "EXPAND(AVERAGE([Sum of Amount]), ROWS)");
    calc(&mut s, &m, "Both", "RUNNINGSUM([Sum of Amount], ROWS COLUMNS)");
    let d = data(&s, &m);
    let at = |n: &str| col(&d, n);
    let leaf = |name: &str| cells(&d["rows"], 3, at(name));
    // leaf cells: Toys/Ball/South 50, Wheels/Bike/North 100, Wheels/Bike/South 50, Wheels/Car/North 300
    let keys = [
        "Toys/Ball/South",
        "Wheels/Bike/North",
        "Wheels/Bike/South",
        "Wheels/Car/North",
    ];
    let want = |name: &str, w: [f64; 4]| {
        let pairs: Vec<(&str, Option<f64>)> = keys.iter().copied().zip(w.iter().map(|x| Some(*x))).collect();
        assert_cells(&leaf(name), &pairs, name);
    };
    // parent: the category's cell in the same column (Wheels North 400, Wheels South 50, Toys South 50)
    want("Parent", [1.0, 0.25, 1.0, 0.75]);
    // down each column, ordered by category then product (North: Wheels Bike, Wheels Car;
    // South: Toys Ball, Wheels Bike)
    want("Down", [50.0, 100.0, 100.0, 400.0]);
    // across each row: North then South
    want("Across", [50.0, 100.0, 150.0, 300.0]);
    // reset by category (HIGHESTPARENT or the field itself)
    want("By category", [50.0, 100.0, 50.0, 400.0]);
    want("By field", [50.0, 100.0, 50.0, 400.0]);
    // rows then columns: North (Bike 100, Car 300) then South (Ball 50, Bike 50)
    want("Both", [450.0, 100.0, 500.0, 400.0]);
    // the category level (a subtotal): its children's average, its share of the column total
    let sub = d["subtotals"]
        .as_array()
        .and_then(|l| l.iter().find(|x| x["rows"] == 1 && x["columns"] == 1))
        .map(|x| x["data"].clone())
        .unwrap_or_else(|| panic!("no Category × Region subtotal in {d}"));
    let pick = |name: &str| -> Vec<(String, Value)> {
        cells(&sub, 3, at(name))
            .into_iter()
            .map(|(k, v)| (k.replace("/null", ""), v))
            .collect()
    };
    assert_cells(
        &pick("Children"),
        &[
            ("Toys/South", Some(50.0)),
            ("Wheels/North", Some(200.0)),
            ("Wheels/South", Some(50.0)),
        ],
        "Children",
    );
    assert_cells(
        &pick("Parent"),
        &[
            ("Toys/South", Some(0.5)),
            ("Wheels/North", Some(1.0)),
            ("Wheels/South", Some(0.5)),
        ],
        "Parent at the category level",
    );
}

#[test]
fn hidden_fields_stay_in_the_matrix() {
    let mut s = shop();
    let t = table(&mut s);
    calc(&mut s, &t, "Running", "RUNNINGSUM([Sum of Amount])");
    let r = run(
        &mut s,
        "visual.calculation.set",
        json!({"page": "Main", "visual": t, "name": "Sum of Amount", "hidden": true}),
    );
    assert_eq!(r["hidden"], true);
    let v = visual(&s, &t);
    assert_eq!(v["fields"][1]["hidden"], true, "{}", v["fields"]);
    assert_eq!(v["fields"][2]["kind"], "visual_calculation");
    let d = data(&s, &t);
    assert_eq!(d["columns"][1]["hidden"], true, "{}", d["columns"]);
    // the running sum still reads the hidden field
    let got = cells(&d["rows"], 1, col(&d, "Running"));
    assert_cells(
        &got,
        &[("Ball", Some(50.0)), ("Bike", Some(200.0)), ("Car", Some(500.0))],
        "Running",
    );
    run(
        &mut s,
        "visual.calculation.set",
        json!({"page": "Main", "visual": t, "name": "Sum of Amount", "hidden": false}),
    );
    assert!(data(&s, &t)["columns"][1].get("hidden").is_none());
}

#[test]
fn the_commands_edit_and_refuse() {
    let mut s = shop();
    let t = table(&mut s);
    // default names, the value role
    let a = run(
        &mut s,
        "visual.calculation.add",
        json!({"page": "Main", "visual": t, "expression": "[Sum of Amount] * 2"}),
    );
    assert_eq!(a["name"], "Calculation");
    assert_eq!(a["role"], "Values");
    let b = run(
        &mut s,
        "visual.calculation.add",
        json!({"page": "Main", "visual": t, "expression": "[Calculation] + 1"}),
    );
    assert_eq!(b["name"], "Calculation 2");
    // what is refused, the report unchanged
    let base = json!({"page": "Main", "visual": t});
    let with = |extra: Value| {
        let mut p = base.clone();
        if let (Some(o), Some(e)) = (p.as_object_mut(), extra.as_object()) {
            o.extend(e.clone());
        }
        p
    };
    for (id, p, says) in [
        (
            "visual.calculation.add",
            with(json!({"expression": "Sales[Amount]"})),
            "visual's fields",
        ),
        (
            "visual.calculation.add",
            with(json!({"expression": "[Nope]"})),
            "no field named",
        ),
        (
            "visual.calculation.add",
            with(json!({"expression": "1", "name": "Calculation"})),
            "already has",
        ),
        (
            "visual.calculation.add",
            with(json!({"expression": "  "})),
            "empty",
        ),
        (
            "visual.calculation.add",
            with(json!({"expression": "1", "name": "a]b"})),
            "brackets",
        ),
        (
            "visual.calculation.set",
            with(json!({"name": "Calculation"})),
            "nothing to change",
        ),
        (
            "visual.calculation.set",
            with(json!({"name": "Calculation", "expression": "[Calculation 2]"})),
            "itself",
        ),
        (
            "visual.calculation.set",
            with(json!({"name": "Calculation", "new_name": "X"})),
            "used by",
        ),
        (
            "visual.calculation.set",
            with(json!({"name": "Product", "expression": "1"})),
            "not a visual calculation",
        ),
        (
            "visual.calculation.set",
            with(json!({"name": "Ghost", "hidden": true})),
            "no field",
        ),
        (
            "visual.calculation.remove",
            with(json!({"name": "Calculation"})),
            "used by",
        ),
        (
            "visual.calculation.remove",
            with(json!({"name": "Product"})),
            "not a visual calculation",
        ),
    ] {
        let e = fails(&mut s, id, p.clone());
        assert!(e.contains(says), "{id} {p}: {e}");
    }
    // rename and change
    let r = run(
        &mut s,
        "visual.calculation.set",
        json!({"page": "Main", "visual": t, "name": "Calculation 2", "new_name": "Plus one",
               "expression": "[Calculation] + 10"}),
    );
    assert_eq!(r["name"], "Plus one");
    assert_eq!(r["expression"], "[Calculation] + 10");
    let d = data(&s, &t);
    let got = cells(&d["rows"], 1, col(&d, "Plus one"));
    assert_cells(
        &got,
        &[("Ball", Some(110.0)), ("Bike", Some(310.0)), ("Car", Some(610.0))],
        "Plus one",
    );
    // force keeps an invalid expression, its diagnostics in warnings; the preview says why
    let w = run(
        &mut s,
        "visual.calculation.add",
        json!({"page": "Main", "visual": t, "name": "Broken", "expression": "[Nope]", "force": true}),
    );
    assert!(!w["warnings"].as_array().is_none_or(Vec::is_empty), "{w}");
    let d = data(&s, &t);
    assert!(
        d["columns"][col(&d, "Broken")]["reason"]
            .as_str()
            .is_some_and(|r| r.contains("Nope")),
        "{}",
        d["columns"]
    );
    // a construct BIcraft does not evaluate: a blank column with its reason
    calc(
        &mut s,
        &t,
        "Lookup",
        "LOOKUP([Sum of Amount], [Sum of Amount], 50)",
    );
    let d = data(&s, &t);
    assert!(
        d["columns"][col(&d, "Lookup")]["reason"]
            .as_str()
            .is_some_and(|r| r.contains("no axis")),
        "{}",
        d["columns"]
    );
    // remove: forced over a referrer, then the rest
    run(
        &mut s,
        "visual.calculation.remove",
        json!({"page": "Main", "visual": t, "name": "Calculation", "force": true}),
    );
    for n in ["Plus one", "Broken", "Lookup"] {
        run(
            &mut s,
            "visual.calculation.remove",
            json!({"page": "Main", "visual": t, "name": n}),
        );
    }
    assert_eq!(visual(&s, &t)["fields"].as_array().map(Vec::len), Some(2));
    // slicers have none
    let sl = add_visual(
        &mut s,
        "slicer",
        json!([{"role": "Values", "field": "Sales[Region]"}]),
    );
    let e = fails(
        &mut s,
        "visual.calculation.add",
        json!({"page": "Main", "visual": sl, "expression": "1"}),
    );
    assert!(e.contains("cannot have visual calculations"), "{e}");
    // undo restores the calculation
    calc(&mut s, &t, "Again", "1");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(visual(&s, &t)["fields"].as_array().map(Vec::len), Some(2));
}

#[test]
fn calculations_and_hidden_fields_survive_pbip_and_pbix() {
    let mut s = shop();
    let t = table(&mut s);
    calc(&mut s, &t, "Running", "RUNNINGSUM([Sum of Amount])");
    run(
        &mut s,
        "visual.calculation.set",
        json!({"page": "Main", "visual": t, "name": "Sum of Amount", "hidden": true}),
    );
    let dir = crate::tempdir::TempDir::new("engine-viscalc");
    let pbip = dir.path().join("Shop.pbip");
    run(&mut s, "file.save_as", json!({"path": pbip.to_string_lossy()}));
    // the PBIR projection as written: NativeVisualCalculation, queryRef and nativeQueryRef its name
    let text = walk(dir.path())
        .into_iter()
        .filter(|p| p.file_name().is_some_and(|n| n == "visual.json"))
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .find(|t| t.contains("NativeVisualCalculation"))
        .unwrap_or_else(|| panic!("no visual.json with the calculation"));
    let doc: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}"));
    let ps = &doc["visual"]["query"]["queryState"]["Values"]["projections"];
    assert_eq!(ps[1]["hidden"], true, "{ps}");
    assert_eq!(
        ps[2],
        json!({"field": {"NativeVisualCalculation": {"Language": "dax", "Expression": "RUNNINGSUM([Sum of Amount])",
                                                       "Name": "Running"}},
               "queryRef": "Running", "nativeQueryRef": "Running"}),
        "{ps}"
    );
    let pbix = dir.path().join("Shop.pbix");
    run(&mut s, "file.save_as", json!({"path": pbix.to_string_lossy()}));
    let orig = s
        .report
        .as_ref()
        .and_then(|r| r.pages.iter().flat_map(|p| &p.visuals).find(|v| v.name == t))
        .map(|v| v.fields.clone());
    for file in [pbip, pbix] {
        let mut back = Session::new();
        run(&mut back, "file.open", json!({"path": file.to_string_lossy()}));
        let v = back
            .report
            .as_ref()
            .and_then(|r| r.pages.iter().flat_map(|p| &p.visuals).find(|v| v.name == t))
            .cloned()
            .unwrap_or_else(|| panic!("no visual {t} in {}", file.display()));
        assert_eq!(Some(v.fields.clone()), orig, "{}", file.display());
        assert!(crate::visual_calcs::hidden(&v, 1), "{}", file.display());
        let d = data(&back, &t);
        assert_eq!(d["columns"][1]["hidden"], true, "{}", file.display());
        let got = cells(&d["rows"], 1, col(&d, "Running"));
        assert_cells(
            &got,
            &[("Ball", Some(50.0)), ("Bike", Some(200.0)), ("Car", Some(500.0))],
            &file.display().to_string(),
        );
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

/// The rows of a table (by their first cell) and its total row, for calculation `name`.
fn table_values(d: &Value, name: &str) -> (Vec<(String, Value)>, Value) {
    let at = col(d, name);
    let total = d["totals"]
        .as_array()
        .and_then(|t| t.get(at))
        .cloned()
        .unwrap_or(Value::Null);
    (cells(&d["rows"], 1, at), total)
}

#[test]
fn lookup_and_window_functions_walk_a_table() {
    let mut s = shop();
    let t = table(&mut s);
    // Ball 50, Bike 150, Car 300 along the rows (ordered by product); total 500
    for (name, expr) in [
        ("Offset", "CALCULATE(SUM([Sum of Amount]), OFFSET(-1, ROWS))"),
        ("Last", "CALCULATE(SUM([Sum of Amount]), INDEX(-1))"),
        ("Second", "SUMX(INDEX(2, ROWS), [Sum of Amount])"),
        ("Zero", "CALCULATE(SUM([Sum of Amount]), INDEX(0))"),
        ("Running", "SUMX(WINDOW(1, ABS, 0, REL), [Sum of Amount])"),
        ("Around", "SUMX(WINDOW(-1, 1), [Sum of Amount])"),
        ("Tail", "SUMX(WINDOW(-2, ABS, -1, ABS, ROWS), [Sum of Amount])"),
        ("Rank", "RANK(DENSE, ORDERBY([Sum of Amount], DESC))"),
        (
            "Rank big skip",
            "RANK(SKIP, ORDERBY([Sum of Amount] > 100, DESC))",
        ),
        (
            "Rank big dense",
            "RANK(DENSE, ORDERBY([Sum of Amount] > 100, DESC))",
        ),
        ("Row", "ROWNUMBER()"),
        ("Row desc", "ROWNUMBER(ROWS, ORDERBY([Product], DESC))"),
        ("Bike", "LOOKUP([Sum of Amount], [Product], \"Bike\")"),
        (
            "Bike explicit",
            "LOOKUP([Sum of Amount], [Product], \"bike\", EXPLICIT)",
        ),
        (
            "Car total",
            "LOOKUPWITHTOTALS([Sum of Amount], [Product], \"Car\")",
        ),
        ("Grand total", "LOOKUPWITHTOTALS([Sum of Amount])"),
        ("Missing", "LOOKUP([Sum of Amount], [Product], \"Kite\")"),
    ] {
        calc(&mut s, &t, name, expr);
    }
    let d = data(&s, &t);
    let want = |name: &str, w: [Option<f64>; 3], total: Option<f64>| {
        let (got, tot) = table_values(&d, name);
        assert_cells(&got, &[("Ball", w[0]), ("Bike", w[1]), ("Car", w[2])], name);
        match total {
            Some(x) => assert!(approx(&tot, x), "{name} total: {tot}"),
            None => assert!(tot.is_null(), "{name} total: {tot}"),
        }
    };
    want("Offset", [None, Some(50.0), Some(150.0)], None);
    // the total row is a partition of one cell: INDEX(-1) is itself, INDEX(2) is outside
    want("Last", [Some(300.0); 3], Some(500.0));
    want("Second", [Some(150.0); 3], None);
    want("Zero", [None; 3], None);
    want("Running", [Some(50.0), Some(200.0), Some(500.0)], Some(500.0));
    want("Around", [Some(200.0), Some(500.0), Some(450.0)], Some(500.0));
    // the second last and last rows; on the total row the window is cut to its one cell
    want("Tail", [Some(450.0); 3], Some(500.0));
    want("Rank", [Some(3.0), Some(2.0), Some(1.0)], None);
    // Bike and Car tie (above 100): SKIP leaves rank 2 out, DENSE does not
    want("Rank big skip", [Some(3.0), Some(1.0), Some(1.0)], None);
    want("Rank big dense", [Some(2.0), Some(1.0), Some(1.0)], None);
    want("Row", [Some(1.0), Some(2.0), Some(3.0)], None);
    want("Row desc", [Some(3.0), Some(2.0), Some(1.0)], None);
    // the total row takes LOOKUP's filter too (it keeps no other field)
    want("Bike", [Some(150.0); 3], Some(150.0));
    want("Bike explicit", [Some(150.0); 3], Some(150.0));
    want("Car total", [Some(300.0); 3], Some(300.0));
    want("Grand total", [Some(500.0); 3], Some(500.0));
    // no cell matches: an error in each cell, so blank; the calculation itself has no reason
    want("Missing", [None; 3], None);
    assert!(
        d["visual_calculations"]
            .as_array()
            .is_some_and(|a| a.iter().all(|x| x.get("reason").is_none())),
        "{}",
        d["visual_calculations"]
    );
}

#[test]
fn lookup_and_window_functions_follow_the_matrix() {
    let mut s = shop();
    let m = matrix(&mut s);
    for (name, expr) in [
        ("North", "LOOKUP([Sum of Amount], [Region], \"North\")"),
        ("Wheels", "LOOKUP([Sum of Amount], [Category], \"Wheels\")"),
        (
            "Wheels total",
            "LOOKUPWITHTOTALS([Sum of Amount], [Category], \"Wheels\")",
        ),
        (
            "Wheels South",
            "LOOKUPWITHTOTALS([Sum of Amount], [Category], \"Wheels\", [Region], \"South\")",
        ),
        (
            "Rank in category",
            "RANK(DENSE, ORDERBY([Sum of Amount], DESC), PARTITIONBY([Category]))",
        ),
        (
            "First in category",
            "CALCULATE(SUM([Sum of Amount]), INDEX(1, ROWS, HIGHESTPARENT))",
        ),
        (
            "Next column",
            "CALCULATE(SUM([Sum of Amount]), OFFSET(1, COLUMNS))",
        ),
        ("Row in column", "ROWNUMBER(ROWS)"),
    ] {
        calc(&mut s, &m, name, expr);
    }
    let d = data(&s, &m);
    let keys = [
        "Toys/Ball/South",
        "Wheels/Bike/North",
        "Wheels/Bike/South",
        "Wheels/Car/North",
    ];
    let want = |name: &str, w: [Option<f64>; 4]| {
        let pairs: Vec<(&str, Option<f64>)> = keys.iter().copied().zip(w).collect();
        assert_cells(&cells(&d["rows"], 3, col(&d, name)), &pairs, name);
    };
    // the same product and category in the North column (Ball has none there)
    want("North", [None, Some(100.0), Some(100.0), Some(300.0)]);
    // the same product and region under Wheels (no Ball under Wheels)
    want("Wheels", [None, Some(100.0), Some(50.0), Some(300.0)]);
    // the Wheels row total (400 North + 50 South), every other field at its total
    want("Wheels total", [Some(450.0); 4]);
    want("Wheels South", [Some(50.0); 4]);
    // within each category and column: North Wheels Car 300 before Bike 100
    want("Rank in category", [Some(1.0), Some(2.0), Some(1.0), Some(1.0)]);
    // the first product of each category in each column
    want(
        "First in category",
        [Some(50.0), Some(100.0), Some(50.0), Some(100.0)],
    );
    // the next region of the same row (Bike North, then Bike South)
    want("Next column", [None, Some(50.0), None, None]);
    // down each column: North Bike 1, Car 2; South Ball 1, Bike 2
    want("Row in column", [Some(1.0), Some(1.0), Some(2.0), Some(2.0)]);
}

#[test]
fn rowpages_walks_the_small_multiples() {
    let mut s = shop();
    let c = add_visual(
        &mut s,
        "clusteredColumnChart",
        json!([{"role": "Category", "field": "Sales[Product]"},
               {"role": "Rows", "field": "Sales[Region]"},
               {"role": "Y", "field": "Sum(Sales[Amount])"}]),
    );
    calc(&mut s, &c, "Rows", "RUNNINGSUM([Sum of Amount])");
    calc(&mut s, &c, "Pages", "RUNNINGSUM([Sum of Amount], ROWPAGES)");
    calc(&mut s, &c, "Both", "RUNNINGSUM([Sum of Amount], ROWS ROWPAGES)");
    calc(&mut s, &c, "Collapse", "COLLAPSE([Sum of Amount], ROWPAGES)");
    let d = data(&s, &c);
    let product = col(&d, "Product");
    let region = col(&d, "Region");
    let got = |name: &str| -> Vec<(String, Value)> {
        let at = col(&d, name);
        let mut out: Vec<(String, Value)> = d["rows"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|r| {
                let k = format!(
                    "{}/{}",
                    r[region].as_str().unwrap_or_default(),
                    r[product].as_str().unwrap_or_default()
                );
                (k, r[at].clone())
            })
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    };
    let keys = ["North/Bike", "North/Car", "South/Ball", "South/Bike"];
    let want = |name: &str, w: [f64; 4]| {
        let pairs: Vec<(&str, Option<f64>)> = keys.iter().copied().zip(w.map(Some)).collect();
        assert_cells(&got(name), &pairs, name);
    };
    // each small multiple apart
    want("Rows", [100.0, 400.0, 50.0, 100.0]);
    // across the small multiples, per product: Bike North 100, then South 150
    want("Pages", [100.0, 300.0, 50.0, 150.0]);
    // the rows, then the small multiples: North Bike, North Car, South Ball, South Bike
    want("Both", [100.0, 400.0, 450.0, 500.0]);
    // the product across the small multiples
    want("Collapse", [100.0 + 50.0, 300.0, 50.0, 150.0]);
}

/// A column chart by product (Category) with a small multiple per region: North (Bike 100,
/// Car 300) and South (Ball 50, Bike 50).
fn multiples(s: &mut Session) -> String {
    add_visual(
        s,
        "clusteredColumnChart",
        json!([{"role": "Category", "field": "Sales[Product]"},
               {"role": "Rows", "field": "Sales[Region]"},
               {"role": "Y", "field": "Sum(Sales[Amount])"}]),
    )
}

/// `(region/product, cell)` of every row of `d` for calculation `name`, sorted.
fn by_page(d: &Value, name: &str) -> Vec<(String, Value)> {
    let (product, region, at) = (col(d, "Product"), col(d, "Region"), col(d, name));
    let mut out: Vec<(String, Value)> = d["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| {
            let k = format!(
                "{}/{}",
                r[region].as_str().unwrap_or_default(),
                r[product].as_str().unwrap_or_default()
            );
            (k, r[at].clone())
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[test]
fn rowpages_navigates_across_the_small_multiples() {
    let mut s = shop();
    let c = multiples(&mut s);
    for (name, expr) in [
        ("Running", "RUNNINGSUM([Sum of Amount])"),
        ("Previous", "PREVIOUS([Sum of Amount])"),
        ("Across", "COLLAPSE([Sum of Amount], ROWPAGES)"),
        (
            "Share",
            "DIVIDE([Sum of Amount], COLLAPSEALL([Sum of Amount], ROWPAGES))",
        ),
        ("To field", "COLLAPSE([Sum of Amount], [Region])"),
        (
            "Children",
            "COLLAPSE(EXPAND(SUM([Sum of Amount]), ROWPAGES), ROWPAGES)",
        ),
        ("Grand", "COLLAPSEALL([Sum of Amount], ROWS ROWPAGES)"),
        (
            "Bike total",
            "LOOKUPWITHTOTALS([Sum of Amount], [Product], \"Bike\")",
        ),
        (
            "South total",
            "LOOKUPWITHTOTALS([Sum of Amount], [Region], \"South\")",
        ),
        ("At page", "IF(ISATLEVEL([Region]), 1, 0)"),
    ] {
        calc(&mut s, &c, name, expr);
    }
    let d = data(&s, &c);
    let keys = ["North/Bike", "North/Car", "South/Ball", "South/Bike"];
    let want = |name: &str, w: [Option<f64>; 4]| {
        let pairs: Vec<(&str, Option<f64>)> = keys.iter().copied().zip(w).collect();
        assert_cells(&by_page(&d, name), &pairs, name);
    };
    // each small multiple is a page of its own: the running sum restarts, PREVIOUS is blank on
    // a page's first row
    want("Running", [Some(100.0), Some(400.0), Some(50.0), Some(100.0)]);
    want("Previous", [None, Some(100.0), None, Some(50.0)]);
    // up the small multiples: the product over every region
    want("Across", [Some(150.0), Some(300.0), Some(50.0), Some(150.0)]);
    want("To field", [Some(150.0), Some(300.0), Some(50.0), Some(150.0)]);
    want(
        "Share",
        [Some(100.0 / 150.0), Some(1.0), Some(1.0), Some(50.0 / 150.0)],
    );
    // down again from there: every region's cell of the product
    want("Children", [Some(150.0), Some(300.0), Some(50.0), Some(150.0)]);
    want("Grand", [Some(500.0); 4]);
    // the fields not given are at their total, the small multiples included
    want("Bike total", [Some(150.0); 4]);
    want("South total", [Some(100.0); 4]);
    want("At page", [Some(1.0); 4]);
    assert!(
        d["visual_calculations"]
            .as_array()
            .is_some_and(|a| a.iter().all(|x| x.get("reason").is_none())),
        "{d}"
    );
}

#[test]
fn the_axis_follows_the_sort_by_column() {
    let mut s = shop();
    // Item[Item] sorted by Item[Order]: Car 1, Ball 2, Bike 3
    let items =
        r#"#table({"Item", "Order", "Amount"}, {{"Bike", 3, 150}, {"Car", 1, 300}, {"Ball", 2, 50}})"#;
    run(
        &mut s,
        "table.add",
        json!({"name": "Item", "partition": {"source_type": "m", "expression": items}}),
    );
    for (name, ty) in [("Item", "string"), ("Order", "int64"), ("Amount", "double")] {
        run(
            &mut s,
            "column.add",
            json!({"table": "Item", "name": name, "data_type": ty}),
        );
    }
    run(
        &mut s,
        "column.set",
        json!({"table": "Item", "name": "Item", "sort_by_column": "Order"}),
    );
    let t = add_visual(
        &mut s,
        "tableEx",
        json!([{"role": "Values", "field": "Item[Item]"},
               {"role": "Values", "field": "Sum(Item[Amount])"}]),
    );
    calc(&mut s, &t, "Running", "RUNNINGSUM([Sum of Amount])");
    calc(&mut s, &t, "Previous", "PREVIOUS([Item])");
    calc(&mut s, &t, "Row", "ROWNUMBER()");
    let d = data(&s, &t);
    let (running, _) = table_values(&d, "Running");
    assert_cells(
        &running,
        &[("Ball", Some(350.0)), ("Bike", Some(500.0)), ("Car", Some(300.0))],
        "Running",
    );
    let (rows, _) = table_values(&d, "Row");
    assert_cells(
        &rows,
        &[("Ball", Some(2.0)), ("Bike", Some(3.0)), ("Car", Some(1.0))],
        "Row",
    );
    let (prev, _) = table_values(&d, "Previous");
    assert_eq!(
        prev,
        vec![
            ("Ball".to_string(), json!("Car")),
            ("Bike".to_string(), json!("Ball")),
            ("Car".to_string(), Value::Null),
        ]
    );
}

#[test]
fn window_arguments_out_of_reach_give_reasons() {
    let mut s = shop();
    let t = table(&mut s);
    for (name, expr, says) in [
        ("Scalar offset", "OFFSET(1, ROWS)", "returns cells"),
        (
            "Value filter",
            "LOOKUP([Sum of Amount], [Sum of Amount], 50)",
            "no axis",
        ),
        (
            "Match",
            "RANK(DENSE, ROWS, ORDERBY([Sum of Amount]), DEFAULT, MATCHBY([Sum of Amount] + 1))",
            "MATCHBY",
        ),
    ] {
        calc(&mut s, &t, name, expr);
        let d = data(&s, &t);
        let reason = d["columns"][col(&d, name)]["reason"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        assert!(reason.contains(says), "{name}: {reason}");
        run(
            &mut s,
            "visual.calculation.remove",
            json!({"page": "Main", "visual": t, "name": name}),
        );
    }
    // huge positions are capped, never an overflow
    calc(
        &mut s,
        &t,
        "Far",
        "SUMX(WINDOW(-1000000000000000, ABS, 1000000000000000, REL), [Sum of Amount])",
    );
    let (far, total) = table_values(&data(&s, &t), "Far");
    assert_cells(
        &far,
        &[("Ball", Some(500.0)), ("Bike", Some(500.0)), ("Car", Some(500.0))],
        "Far",
    );
    assert!(approx(&total, 500.0), "{total}");
}

#[test]
fn matchby_and_lookup_inferred_follow_the_matrix() {
    let mut s = shop();
    let m = matrix(&mut s);
    for (name, expr) in [
        // the product singles out each cell of a column: the same as without MATCHBY
        (
            "Row by product",
            "ROWNUMBER(ROWS, ORDERBY([Product]), MATCHBY([Product]))",
        ),
        // the category does not: Wheels holds Bike and Car in the North column
        (
            "Row by category",
            "ROWNUMBER(ROWS, ORDERBY([Product]), MATCHBY([Category]))",
        ),
        (
            "Rank matched",
            "RANK(DENSE, ROWS, ORDERBY([Sum of Amount], DESC), DEFAULT, PARTITIONBY([Category]), MATCHBY([Product]))",
        ),
        // EXPLICIT: the category comes from the current row
        ("Bike explicit", "LOOKUP([Sum of Amount], [Product], \"Bike\")"),
        // INFERRED: Bike determines its category (Wheels) on the row axis; the region, on the
        // other axis, still comes from the current cell
        (
            "Bike inferred",
            "LOOKUP([Sum of Amount], [Product], \"Bike\", INFERRED)",
        ),
        // a supplied value that conflicts with the inferred one matches nothing: blank
        (
            "Bike in Toys",
            "LOOKUP([Sum of Amount], [Product], \"Bike\", [Category], \"Toys\", INFERRED)",
        ),
    ] {
        calc(&mut s, &m, name, expr);
    }
    let d = data(&s, &m);
    let keys = [
        "Toys/Ball/South",
        "Wheels/Bike/North",
        "Wheels/Bike/South",
        "Wheels/Car/North",
    ];
    let want = |name: &str, w: [Option<f64>; 4]| {
        let pairs: Vec<(&str, Option<f64>)> = keys.iter().copied().zip(w).collect();
        assert_cells(&cells(&d["rows"], 3, col(&d, name)), &pairs, name);
    };
    want("Row by product", [Some(1.0), Some(1.0), Some(2.0), Some(2.0)]);
    want("Row by category", [Some(1.0), None, Some(2.0), None]);
    want("Rank matched", [Some(1.0), Some(2.0), Some(1.0), Some(1.0)]);
    want("Bike explicit", [None, Some(100.0), Some(50.0), Some(100.0)]);
    want(
        "Bike inferred",
        [Some(50.0), Some(100.0), Some(50.0), Some(100.0)],
    );
    want("Bike in Toys", [None; 4]);
    for name in ["Row by category", "Bike inferred"] {
        assert!(
            d["columns"][col(&d, name)].get("reason").is_none(),
            "{name}: {}",
            d["columns"]
        );
    }
}

#[test]
fn a_preview_evaluates_the_calculation_without_changing_the_report() {
    let mut s = shop();
    let t = table(&mut s);
    let before = s.report.clone();
    let revision = s.revision;
    let r = run(
        &mut s,
        "visual.calculation.add",
        json!({"page": "Main", "visual": t, "name": "Running",
               "expression": "RUNNINGSUM([Sum of Amount])", "preview": true}),
    );
    assert_eq!(s.report, before, "a preview changed the report");
    assert_eq!(s.revision, revision, "a preview is a new revision");
    let d = &r["data"];
    assert_eq!(d["status"], "ok", "{d}");
    assert_cells(
        &cells(&d["rows"], 1, col(d, "Running")),
        &[("Ball", Some(50.0)), ("Bike", Some(200.0)), ("Car", Some(500.0))],
        "previewed running sum",
    );
    // an expression that does not bind is refused, still without a change
    let err = fails(
        &mut s,
        "visual.calculation.add",
        json!({"page": "Main", "visual": t, "expression": "RUNNINGSUM([Nope])", "preview": true}),
    );
    assert!(err.contains("Nope"), "{err}");
    // an edit's preview: the new expression's values, the stored one still on the visual
    calc(&mut s, &t, "Running", "RUNNINGSUM([Sum of Amount])");
    let before = s.report.clone();
    let r = run(
        &mut s,
        "visual.calculation.set",
        json!({"page": "Main", "visual": t, "name": "Running",
               "expression": "[Sum of Amount] * 2", "preview": true}),
    );
    assert_eq!(s.report, before, "a preview changed the report");
    let d = &r["data"];
    assert_cells(
        &cells(&d["rows"], 1, col(d, "Running")),
        &[("Ball", Some(100.0)), ("Bike", Some(300.0)), ("Car", Some(600.0))],
        "previewed edit",
    );
    let stored = data(&s, &t);
    assert_cells(
        &cells(&stored["rows"], 1, col(&stored, "Running")),
        &[("Ball", Some(50.0)), ("Bike", Some(200.0)), ("Car", Some(500.0))],
        "the visual after a preview",
    );
}
