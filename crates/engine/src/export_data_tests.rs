//! `visual.export_data` and `report.settings {export_data}`: the CSV and XLSX files hold the visual's
//! query result, the row limits hold, the report's export setting is honoured.

use serde_json::{Value, json};

use super::*;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("bicraft-export-data-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&d);
    d.join(name)
}

/// Sales(Product, Amount, Qty) related to Product(Product, Category), a Total measure, and a
/// Big(N) table of 40,000 rows; one page `Main`.
fn shop() -> Session {
    let mut s = Session::new();
    let sales = r#"#table({"Product", "Amount", "Qty"}, {{"Bike", 100, 1}, {"Bike", 50.5, 2}, {"Car", 1000, 1}, {"=Ball", 5, 10}, {"Kite, red", 7, 1}})"#;
    let product =
        r#"#table({"Product", "Category"}, {{"Bike", "Wheels"}, {"Car", "Wheels"}, {"=Ball", "Toys"}})"#;
    let big = r#"Table.FromColumns({List.Numbers(1, 40000)}, {"N"})"#;
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
            "table.add",
            json!({"name": "Big", "partition": {"source_type": "m", "expression": big}}),
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
            json!({"table": "Product", "name": "Product", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Product", "name": "Category", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Big", "name": "N", "data_type": "int64"}),
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
        json!({"page": "Main", "visual_type": visual_type, "x": 0, "y": 0, "width": 300, "height": 200, "fields": fields}),
    );
    r["visual"].as_str().unwrap().to_string()
}

/// The visual's data as the snapshot shows it (the viewer's query result).
fn shown(s: &Session, visual: &str) -> Value {
    let snap = s.snapshot();
    for p in snap["report"]["pages"].as_array().unwrap() {
        for v in p["visuals"].as_array().unwrap() {
            if v["name"] == visual {
                return v["data"].clone();
            }
        }
    }
    panic!("no visual {visual}")
}

/// A CSV file's rows (RFC 4180 quoting), without the byte order mark.
fn read_csv(path: &Path) -> Vec<Vec<String>> {
    let text = std::fs::read_to_string(path).unwrap();
    let text = text.strip_prefix('\u{feff}').expect("a byte order mark");
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, quoted) {
            ('"', true) if chars.peek() == Some(&'"') => {
                chars.next();
                cell.push('"');
            }
            ('"', _) => quoted = !quoted,
            (',', false) => row.push(std::mem::take(&mut cell)),
            ('\r', false) => {}
            ('\n', false) => {
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
            }
            (c, _) => cell.push(c),
        }
    }
    rows
}

/// An XLSX file's cells, row by row (inline strings and numbers), read back through OPC.
fn read_xlsx(path: &Path) -> Vec<Vec<String>> {
    let pkg = bicraft_opc::Package::open(path).unwrap();
    for part in [
        "[Content_Types].xml",
        "_rels/.rels",
        "xl/workbook.xml",
        "xl/_rels/workbook.xml.rels",
        "xl/styles.xml",
    ] {
        assert!(pkg.get(part).is_some(), "{part}");
    }
    let sheet = String::from_utf8(
        pkg.get("xl/worksheets/sheet1.xml")
            .unwrap()
            .bytes()
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    let mut rows = Vec::new();
    for r in sheet.split("<row ").skip(1) {
        let mut cells = Vec::new();
        for c in r.split("<c ").skip(1) {
            let v = if let Some(t) = c.split("<t xml:space=\"preserve\">").nth(1) {
                t.split("</t>").next().unwrap_or("").to_string()
            } else if let Some(t) = c.split("<v>").nth(1) {
                t.split("</v>").next().unwrap_or("").to_string()
            } else {
                String::new()
            };
            cells.push(
                v.replace("&amp;", "&")
                    .replace("&lt;", "<")
                    .replace("&gt;", ">")
                    .replace("&quot;", "\""),
            );
        }
        rows.push(cells);
    }
    rows
}

fn cell_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

#[test]
fn summarized_csv_and_xlsx_equal_the_visual_query() {
    let mut s = shop();
    let v = add_visual(
        &mut s,
        "clusteredColumnChart",
        json!([{"role": "Category", "field": "Sales[Product]"}, {"role": "Y", "field": "[Total]"}]),
    );
    let d = shown(&s, &v);
    assert_eq!(d["status"], "ok", "{d}");
    let want: Vec<Vec<String>> = d["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_array().unwrap().iter().map(cell_text).collect())
        .collect();
    // CSV
    let out = tmp("chart.csv");
    let r = run(
        &mut s,
        "visual.export_data",
        json!({"visual": v, "out": out.display().to_string()}),
    );
    assert_eq!(
        (r["format"].as_str(), r["kind"].as_str(), r["truncated"].as_bool()),
        (Some("csv"), Some("summarized"), Some(false))
    );
    assert_eq!(r["limit"], 30_000);
    let csv = read_csv(&out);
    assert_eq!(
        csv[0],
        vec!["Product".to_string(), "Total".to_string()],
        "{csv:?}"
    );
    let mut body: Vec<Vec<String>> = csv[1..].to_vec();
    // a text starting with = is written with a leading quote (no formula in a spreadsheet)
    for row in &mut body {
        if row[0] == "'=Ball" {
            row[0] = "=Ball".into();
        }
    }
    let mut want_sorted = want.clone();
    want_sorted.sort();
    body.sort();
    assert_eq!(body, want_sorted);
    assert!(std::fs::read_to_string(&out).unwrap().contains("\"Kite, red\""));
    // XLSX: the same cells, a header row
    let out = tmp("chart.xlsx");
    let r = run(
        &mut s,
        "visual.export_data",
        json!({"visual": v, "out": out.display().to_string()}),
    );
    assert_eq!(
        (r["format"].as_str(), r["limit"].as_u64()),
        (Some("xlsx"), Some(150_000))
    );
    let x = read_xlsx(&out);
    assert_eq!(x[0], vec!["Product".to_string(), "Total".to_string()]);
    let mut body = x[1..].to_vec();
    body.sort();
    assert_eq!(body, want_sorted);
}

#[test]
fn underlying_groups_aggregated_columns_and_follows_the_setting() {
    let mut s = shop();
    let v = add_visual(
        &mut s,
        "clusteredColumnChart",
        json!([{"role": "Category", "field": "Product[Category]"}, {"role": "Y", "field": "Sum(Sales[Amount])"}]),
    );
    let out = tmp("under.csv");
    let p = json!({"visual": v, "kind": "underlying", "out": out.display().to_string()});
    // a report without the setting allows summarized data only
    let e = s.execute("visual.export_data", &p).unwrap_err();
    assert!(e.contains("summarized data only"), "{e}");
    assert_eq!(s.snapshot()["report"]["export_data_mode"], "AllowSummarized");
    let r = run(
        &mut s,
        "report.settings",
        json!({"export_data": "AllowSummarizedAndUnderlying"}),
    );
    assert_eq!(
        (r["changed"].as_bool(), r["export_data"].as_str()),
        (Some(true), Some("AllowSummarizedAndUnderlying"))
    );
    assert_eq!(
        s.snapshot()["report"]["export_data_mode"],
        "AllowSummarizedAndUnderlying"
    );
    let r = run(&mut s, "visual.export_data", p.clone());
    assert_eq!(r["kind"], "underlying");
    let csv = read_csv(&out);
    // summarized: a row per category; underlying: a row per (category, amount)
    let summarized = tmp("summ.csv");
    run(
        &mut s,
        "visual.export_data",
        json!({"visual": v, "out": summarized.display().to_string()}),
    );
    let sum = read_csv(&summarized);
    assert_eq!(sum.len() - 1, 3, "{sum:?}"); // Wheels, Toys and the blank category of Kite
    assert!(csv.len() > 5, "{csv:?}");
    assert!(
        csv.iter().any(|r| r.get(1).map(String::as_str) == Some("50.5")),
        "{csv:?}"
    );
    // None forbids every export, and is undone like any report edit
    run(&mut s, "report.settings", json!({"export_data": "None"}));
    let e = s
        .execute(
            "visual.export_data",
            &json!({"visual": v, "out": summarized.display().to_string()}),
        )
        .unwrap_err();
    assert!(e.contains("does not allow exporting"), "{e}");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(
        s.snapshot()["report"]["export_data_mode"],
        "AllowSummarizedAndUnderlying"
    );
    // the setting is kept in report.json
    let r = s.report.as_ref().unwrap();
    assert_eq!(
        r.extra["settings"]["exportDataMode"],
        "AllowSummarizedAndUnderlying"
    );
}

#[test]
fn row_limits_are_enforced() {
    let mut s = shop();
    let v = add_visual(&mut s, "tableEx", json!([{"role": "Values", "field": "Big[N]"}]));
    let out = tmp("big.csv");
    let r = run(
        &mut s,
        "visual.export_data",
        json!({"visual": v, "out": out.display().to_string()}),
    );
    assert_eq!(
        (r["rows"].as_u64(), r["truncated"].as_bool()),
        (Some(30_000), Some(true)),
        "{r}"
    );
    assert_eq!(r["total_rows"], 40_000);
    assert!(r["warnings"][0].as_str().unwrap().contains("30000"));
    assert_eq!(read_csv(&out).len(), 30_001);
    // XLSX takes them all (under 150,000)
    let out = tmp("big.xlsx");
    let r = run(
        &mut s,
        "visual.export_data",
        json!({"visual": v, "out": out.display().to_string()}),
    );
    assert_eq!(
        (r["rows"].as_u64(), r["truncated"].as_bool()),
        (Some(40_000), Some(false)),
        "{r}"
    );
    assert_eq!(read_xlsx(&out).len(), 40_001);
}

#[test]
fn hostile_and_missing_params_are_errors() {
    let mut s = shop();
    let v = add_visual(&mut s, "card", json!([{"role": "Values", "field": "[Total]"}]));
    let out = tmp("card.csv").display().to_string();
    for p in [
        json!({"visual": v}),
        json!({"visual": v, "out": ""}),
        json!({"visual": v, "out": "  "}),
        json!({"visual": v, "out": "x.txt"}),
        json!({"visual": v, "out": out, "format": "pdf"}),
        json!({"visual": v, "out": out, "kind": "all"}),
        json!({"visual": "nope", "out": out}),
        json!({"visual": v, "page": "Nope", "out": out}),
        json!({"visual": 3, "out": out}),
        json!({"visual": v, "out": out, "extra": 1}),
    ] {
        assert!(s.execute("visual.export_data", &p).is_err(), "{p}");
    }
    for p in [
        json!({"export_data": "maybe"}),
        json!({"export_data": 1}),
        json!({"mode": "None"}),
    ] {
        assert!(s.execute("report.settings", &p).is_err(), "{p}");
    }
    // a card exports its one value; `format` wins over the extension
    let r = run(
        &mut s,
        "visual.export_data",
        json!({"visual": v, "out": tmp("card.dat").display().to_string(), "format": "csv"}),
    );
    assert_eq!(r["rows"], 1);
    // no report
    let mut empty = Session::new();
    assert!(
        empty
            .execute("visual.export_data", &json!({"visual": "v", "out": out}))
            .is_err()
    );
    assert!(
        empty
            .execute("report.settings", &json!({"export_data": "None"}))
            .is_err()
    );
}

#[test]
fn cells_are_quoted_dated_and_lettered() {
    assert_eq!(col_letters(0), "A");
    assert_eq!(col_letters(25), "Z");
    assert_eq!(col_letters(26), "AA");
    assert_eq!(col_letters(701), "ZZ");
    assert_eq!(col_letters(702), "AAA");
    assert_eq!(serial("1899-12-30"), Some((0.0, false)));
    assert_eq!(serial("2025-01-01"), Some((45658.0, false)));
    assert_eq!(serial("2025-01-01T12:00:00"), Some((45658.5, true)));
    assert_eq!(serial("2025-13-01"), None);
    assert_eq!(serial("nope"), None);
    let text = Col {
        name: "a".into(),
        text: true,
        date: false,
    };
    assert_eq!(csv_text(&json!("+1"), &text), "'+1");
    assert_eq!(csv_text(&json!("@x"), &text), "'@x");
    assert_eq!(csv_text(&json!(true), &text), "True");
    assert_eq!(xml_escape("a<b>&\"\u{1}"), "a&lt;b&gt;&amp;&quot;");
    let mut o = String::new();
    csv_field(&mut o, "a\"b");
    assert_eq!(o, "\"a\"\"b\"");
}
