//! `file.open` of a Power Pivot workbook (M11.33), on the synthetic workbook of
//! `bicraft_powerpivot::synthetic`.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::Session;
use crate::tempdir::TempDir;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn workbook(dir: &Path) -> PathBuf {
    let path = dir.join("Sales book.xlsx");
    std::fs::write(&path, bicraft_powerpivot::synthetic::sample().unwrap()).unwrap();
    path
}

/// Units, total amount and rows of Sales, as the DAX evaluator computes them.
fn totals(s: &mut Session) -> Value {
    let r = run(
        s,
        "dax.evaluate",
        json!({"query": "EVALUATE ROW(\"u\", [Units], \"a\", [Total amount], \"n\", COUNTROWS(Sales), \
                         \"c\", CALCULATE([Units], Product[Category] = \"Seating\"))"}),
    );
    r["results"][0]["rows"][0].clone()
}

fn expected_units() -> i64 {
    (0..240usize).map(|i| ((i * 13) % 9 + 1) as i64).sum()
}

#[test]
fn opening_a_power_pivot_workbook_gives_its_model_and_data() {
    let dir = TempDir::new("engine-workbook-open");
    let path = workbook(dir.path());
    let mut s = Session::new();
    let r = run(&mut s, "file.open", json!({"path": path}));
    assert_eq!(s.model_error, None);
    assert_eq!(r["imported"]["from"], "excel-power-pivot", "{r}");
    assert_eq!(r["imported"]["compatibility_level"], 1103);
    assert_eq!(r["imported"]["tables"], 2);
    assert_eq!(r["imported"]["rows"], 245);
    let model = s.model.as_ref().unwrap();
    let names: Vec<&str> = model.tables.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["Product", "Sales"]);
    assert_eq!(model.relationships.len(), 1);
    let snap = s.snapshot();
    assert_eq!(snap["source"]["format"], "xlsx", "{}", snap["source"]);
    let t = totals(&mut s);
    assert_eq!(t["[u]"], expected_units(), "{t}");
    assert_eq!(t["[n]"], 240, "{t}");
    assert!(t["[a]"].as_f64().unwrap() > 0.0, "{t}");
}

#[test]
fn an_imported_workbook_saves_as_pbix_and_pbip_never_over_itself() {
    let dir = TempDir::new("engine-workbook-save");
    let path = workbook(dir.path());
    let original = std::fs::read(&path).unwrap();
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": path}));
    let before = totals(&mut s);
    let err = s.execute("file.save", &json!({})).unwrap_err();
    assert!(err.contains("file.save_as"), "{err}");
    assert_eq!(
        std::fs::read(&path).unwrap(),
        original,
        "the workbook is never written"
    );

    let pbix = dir.path().join("Sales.pbix");
    run(&mut s, "file.save_as", json!({"path": pbix}));
    let mut again = Session::new();
    run(&mut again, "file.open", json!({"path": pbix}));
    assert_eq!(again.model_error, None);
    assert_eq!(totals(&mut again), before);

    let pbip = dir.path().join("Sales.pbip");
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": path}));
    run(&mut s, "file.save_as", json!({"path": pbip}));
    let mut again = Session::new();
    run(&mut again, "file.open", json!({"path": pbip}));
    assert_eq!(again.model_error, None);
    assert_eq!(totals(&mut again), before);
}

#[test]
fn file_convert_turns_a_workbook_into_a_pbix() {
    let dir = TempDir::new("engine-workbook-convert");
    let path = workbook(dir.path());
    let out = dir.path().join("out.pbix");
    let mut s = Session::new();
    run(&mut s, "file.convert", json!({"from": path, "to": out}));
    let mut again = Session::new();
    run(&mut again, "file.open", json!({"path": out}));
    assert_eq!(totals(&mut again)["[u]"], expected_units());
}

/// The workbook Excel 16.0 build 20625 saved from our synthetic tables
/// (`crates/powerpivot/tests/fixtures/excel/SOURCE.txt`), copied into `dir`.
fn excel_saved(dir: &Path) -> PathBuf {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../powerpivot/tests/fixtures/excel/excel-model.xlsx");
    let path = dir.join("excel-model.xlsx");
    std::fs::copy(fixture, &path).unwrap();
    path
}

/// The measures by country and category: the values Excel's own engine returned for the same
/// queries on this workbook (recorded when the fixture was made, SOURCE.txt).
fn excel_results(s: &mut Session) -> Vec<Value> {
    [
        "EVALUATE ROW(\"q\", [Total quantity], \"r\", [Revenue], \"c\", [Customers])",
        "EVALUATE ADDCOLUMNS(VALUES(Customer[Country]), \"r\", [Revenue], \"q\", [Total quantity])",
        "EVALUATE ADDCOLUMNS(VALUES(Product[Category]), \"r\", [Revenue], \"q\", [Total quantity])",
    ]
    .iter()
    .map(|q| run(s, "dax.evaluate", json!({ "query": q }))["results"][0]["rows"].clone())
    .collect()
}

fn assert_excel_results(got: &[Value]) {
    let close = |v: &Value, want: f64| (v.as_f64().unwrap() - want).abs() < 1e-9;
    let row = &got[0][0];
    assert_eq!(row["[q]"], 58, "{row}");
    assert!(close(&row["[r]"], 263.96), "{row}");
    assert_eq!(row["[c]"], 3, "{row}");
    let by = |rows: &Value, key: &str, name: &str| -> (f64, i64) {
        let r = rows
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r[key] == name)
            .unwrap_or_else(|| panic!("{name} in {rows}"));
        (r["[r]"].as_f64().unwrap(), r["[q]"].as_i64().unwrap())
    };
    for (name, r, q) in [("France", 217.73, 31), ("Belgium", 46.23, 27)] {
        let (gr, gq) = by(&got[1], "Customer[Country]", name);
        assert!((gr - r).abs() < 1e-9 && gq == q, "{name}: {gr} {gq}");
    }
    for (name, r, q) in [("Tools", 132.5, 40), ("Toys", 51.5, 14), ("Home", 79.96, 4)] {
        let (gr, gq) = by(&got[2], "Product[Category]", name);
        assert!((gr - r).abs() < 1e-9 && gq == q, "{name}: {gr} {gq}");
    }
}

#[test]
fn a_workbook_excel_saved_opens_with_excels_results_and_saves_as_pbix() {
    let dir = TempDir::new("engine-workbook-excel");
    let path = excel_saved(dir.path());
    let mut s = Session::new();
    let r = run(&mut s, "file.open", json!({"path": path}));
    assert_eq!(s.model_error, None);
    assert_eq!(r["imported"]["compatibility_level"], 1103, "{r}");
    assert_eq!(r["imported"]["tables"], 3);
    assert_eq!(r["imported"]["rows"], 20);
    assert_excel_results(&excel_results(&mut s));

    let pbix = dir.path().join("excel-model.pbix");
    run(&mut s, "file.save_as", json!({"path": pbix}));
    let mut again = Session::new();
    run(&mut again, "file.open", json!({"path": pbix}));
    assert_eq!(again.model_error, None);
    assert_excel_results(&excel_results(&mut again));
    let model = again.model.as_ref().unwrap();
    let revenue = model
        .table("Sales")
        .and_then(|t| t.measures.iter().find(|m| m.name == "Revenue"))
        .unwrap();
    assert_eq!(revenue.format_string.as_deref(), Some("#,0.00"));
}

#[test]
fn a_workbook_without_a_model_is_a_clear_error() {
    let dir = TempDir::new("engine-workbook-none");
    let path = dir.path().join("plain.xlsx");
    std::fs::write(&path, bicraft_powerpivot::synthetic::without_model().unwrap()).unwrap();
    let mut s = Session::new();
    let err = s.execute("file.open", &json!({"path": path})).unwrap_err();
    assert!(err.contains("no Power Pivot data model"), "{err}");
    let err = Session::new()
        .execute("file.open", &json!({"path": dir.path().join("missing.xlsm")}))
        .unwrap_err();
    assert!(!err.is_empty());
}
