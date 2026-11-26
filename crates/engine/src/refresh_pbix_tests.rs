//! `model.refresh` on a PBIX (refresh_pbix.rs): a PBIX built by bicraft from CSV and Excel
//! sources, the sources changed, the file refreshed, then reopened with bicraft's reader (new
//! rows, refresh state, the other tables' storage byte for byte); partial refreshes and what
//! depends on them; a failing source and every refusal leave the file as it was.

use std::path::Path;

use bicraft_pbix::structural::DataModelEditor;
use serde_json::{Value, json};

use super::*;
use crate::tempdir::TempDir;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

#[test]
fn filetime_of_the_refresh_clock() {
    // 1970-01-01T00:00Z: 25 569 days after 1899-12-30, 116 444 736 000 000 000 FILETIME ticks
    assert_eq!(filetime(25_569 * 86_400_000), 116_444_736_000_000_000);
}

/// A minimal `.xlsx` with one sheet `Sales` of inline strings and numbers (first row: headers).
pub(crate) fn xlsx(rows: &[Vec<&str>]) -> Vec<u8> {
    let mut sheet = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheetData>",
    );
    for (r, row) in rows.iter().enumerate() {
        sheet.push_str(&format!("<row r=\"{}\">", r + 1));
        for (c, v) in row.iter().enumerate() {
            let col = char::from(b'A' + u8::try_from(c).unwrap());
            let at = format!("{col}{}", r + 1);
            if r > 0 && v.parse::<f64>().is_ok() {
                sheet.push_str(&format!("<c r=\"{at}\"><v>{v}</v></c>"));
            } else if !v.is_empty() {
                sheet.push_str(&format!("<c r=\"{at}\" t=\"inlineStr\"><is><t>{v}</t></is></c>"));
            }
        }
        sheet.push_str("</row>");
    }
    sheet.push_str("</sheetData></worksheet>");
    let mut p = bicraft_opc::Package::default();
    p.put(
        "[Content_Types].xml",
        b"<?xml version=\"1.0\" encoding=\"UTF-8\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
          <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
          <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
          <Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>\
          <Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/></Types>"
            .to_vec(),
        false,
    );
    p.put(
        "_rels/.rels",
        b"<?xml version=\"1.0\" encoding=\"UTF-8\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
          <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>"
            .to_vec(),
        false,
    );
    p.put(
        "xl/workbook.xml",
        b"<?xml version=\"1.0\" encoding=\"UTF-8\"?><workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" \
          xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><sheets>\
          <sheet name=\"Sales\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>"
            .to_vec(),
        false,
    );
    p.put(
        "xl/_rels/workbook.xml.rels",
        b"<?xml version=\"1.0\" encoding=\"UTF-8\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
          <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/></Relationships>"
            .to_vec(),
        false,
    );
    p.put("xl/worksheets/sheet1.xml", sheet.into_bytes(), false);
    p.to_bytes().unwrap()
}

fn escape(p: &Path) -> String {
    p.display().to_string().replace('\\', "\\\\")
}

/// Product from a CSV file, Sales from an Excel sheet (a calculated column reading Product's
/// price through the relationship), Notes an unrelated `#table` literal, Big Sales a calculated
/// table over Sales, a measure and an RLS role.
fn bim(data: &Path) -> String {
    let csv = escape(&data.join("products.csv"));
    let xl = escape(&data.join("sales.xlsx"));
    let m_product = format!(
        "let Source = Csv.Document(File.Contents(\"{csv}\"), [Delimiter=\",\", Encoding=65001, QuoteStyle=QuoteStyle.Csv]), \
         Promoted = Table.PromoteHeaders(Source, [PromoteAllScalars=true]), \
         Typed = Table.TransformColumnTypes(Promoted, {{{{\"Key\", Int64.Type}}, {{\"Name\", type text}}, {{\"Price\", type number}}}}) in Typed"
    );
    let m_sales = format!(
        "let Source = Excel.Workbook(File.Contents(\"{xl}\"), null, true), \
         Sheet = Source{{[Item=\"Sales\",Kind=\"Sheet\"]}}[Data], \
         Promoted = Table.PromoteHeaders(Sheet, [PromoteAllScalars=true]), \
         Typed = Table.TransformColumnTypes(Promoted, {{{{\"Order\", Int64.Type}}, {{\"Key\", Int64.Type}}, {{\"Qty\", Int64.Type}}}}) in Typed"
    );
    serde_json::to_string_pretty(&json!({
        "name": "Shop",
        "compatibilityLevel": 1606,
        "model": {
            "culture": "en-US",
            "defaultPowerBIDataSourceVersion": "powerBI_V3",
            "tables": [
                {
                    "name": "Product",
                    "columns": [
                        {"name": "Key", "dataType": "int64", "sourceColumn": "Key", "summarizeBy": "none"},
                        {"name": "Name", "dataType": "string", "sourceColumn": "Name"},
                        {"name": "Price", "dataType": "double", "sourceColumn": "Price"}
                    ],
                    "partitions": [{"name": "Product", "mode": "import", "source": {"type": "m", "expression": m_product}}],
                    "measures": [{"name": "Products", "expression": "COUNTROWS(Product)"}]
                },
                {
                    "name": "Sales",
                    "columns": [
                        {"name": "Order", "dataType": "int64", "sourceColumn": "Order", "summarizeBy": "none"},
                        {"name": "Key", "dataType": "int64", "sourceColumn": "Key", "summarizeBy": "none"},
                        {"name": "Qty", "dataType": "int64", "sourceColumn": "Qty"},
                        {"name": "Amount", "dataType": "double", "type": "calculated",
                         "expression": "Sales[Qty] * RELATED(Product[Price])"}
                    ],
                    "partitions": [{"name": "Sales", "mode": "import", "source": {"type": "m", "expression": m_sales}}],
                    "measures": [{"name": "Total", "expression": "SUM(Sales[Amount])"}]
                },
                {
                    "name": "Notes",
                    "columns": [
                        {"name": "Id", "dataType": "int64", "sourceColumn": "Id"},
                        {"name": "Text", "dataType": "string", "sourceColumn": "Text"}
                    ],
                    "partitions": [{"name": "Notes", "mode": "import", "source": {"type": "m",
                        "expression": "let Source = #table(type table [Id = Int64.Type, Text = text], {{1, \"keep\"}, {2, \"me\"}}) in Source"}}]
                },
                {
                    "name": "Big Sales",
                    "partitions": [{"name": "Big Sales", "source": {"type": "calculated",
                        "expression": "FILTER(Sales, Sales[Qty] >= 3)"}}]
                }
            ],
            "relationships": [
                {"name": "0b7c1d52-2b55-4b1c-a3c9-8f3a5d7e9b01", "fromTable": "Sales", "fromColumn": "Key", "toTable": "Product", "toColumn": "Key"}
            ],
            "roles": [
                {"name": "Cheap", "modelPermission": "read",
                 "tablePermissions": [{"name": "Product", "filterExpression": "Product[Price] < 2"}]}
            ]
        }
    }))
    .unwrap()
}

const PRODUCTS: &str = "Key,Name,Price\n1,Apple,1.5\n2,Banana,0.25\n3,Leek,2\n";

fn sales_rows() -> Vec<Vec<&'static str>> {
    vec![
        vec!["Order", "Key", "Qty"],
        vec!["10", "1", "2"],
        vec!["11", "2", "5"],
        vec!["12", "1", "3"],
    ]
}

/// A PBIX built by bicraft in `dir` from the sources in `dir/data`; the session has it open.
fn fixture(dir: &TempDir) -> (Session, std::path::PathBuf, std::path::PathBuf) {
    let data = dir.join("data");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("products.csv"), PRODUCTS).unwrap();
    std::fs::write(data.join("sales.xlsx"), xlsx(&sales_rows())).unwrap();
    let bim_path = dir.join("model.bim");
    std::fs::write(&bim_path, bim(&data)).unwrap();
    let path = dir.join("shop.pbix");
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": bim_path, "data_root": data}));
    let out = run(&mut s, "file.save_as", json!({"path": path}));
    assert_eq!(out["built_from_scratch"], true, "{out}");
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": path, "data_root": data}));
    (s, path, data)
}

fn editor(path: &Path) -> DataModelEditor {
    let pkg = bicraft_opc::Package::open(path).unwrap();
    let part = pkg.get(bicraft_pbix::DATA_MODEL).unwrap();
    DataModelEditor::open(&part.data, &bicraft_pbix::DataModelLimits::default()).unwrap()
}

fn values(e: &DataModelEditor, table: &str, column: &str) -> Vec<CellValue> {
    e.column_values(table, column)
        .unwrap_or_else(|err| panic!("{table}[{column}]: {err}"))
}

fn ints(v: &[i64]) -> Vec<CellValue> {
    v.iter().map(|i| CellValue::Int(*i)).collect()
}

fn reals(v: &[f64]) -> Vec<CellValue> {
    v.iter().map(|x| CellValue::Real(*x)).collect()
}

/// The backup's storage files under `table`'s folder, with their bytes.
fn table_files(path: &Path, table: &str) -> Vec<(String, Vec<u8>)> {
    let pkg = bicraft_opc::Package::open(path).unwrap();
    let part = pkg.get(bicraft_pbix::DATA_MODEL).unwrap();
    let (_, abf) =
        bicraft_pbix::read_data_model_keeping(&part.data, &bicraft_pbix::DataModelLimits::default()).unwrap();
    let abf = abf.unwrap();
    let parsed = bicraft_abf::Abf::parse(&abf).unwrap();
    let prefix = format!("{table} (");
    let mut out: Vec<(String, Vec<u8>)> = parsed
        .files()
        .iter()
        .filter(|f| f.path.split('/').any(|seg| seg.starts_with(&prefix)))
        .map(|f| (f.path.clone(), parsed.data(f).unwrap().to_vec()))
        .collect();
    out.sort();
    out
}

fn refresh_state(s: &mut Session, table: &str) -> Value {
    let t = run(s, "table.get", json!({"name": table}));
    t["partitions"][0]["refresh"].clone()
}

fn origins(out: &Value) -> Vec<(String, String, u64)> {
    out["tables"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (
                t["table"].as_str().unwrap().to_string(),
                t["origin"].as_str().unwrap().to_string(),
                t["rows"].as_u64().unwrap(),
            )
        })
        .collect()
}

fn has(list: &[(String, String, u64)], table: &str, origin: &str, rows: u64) -> bool {
    list.iter()
        .any(|(t, o, r)| t == table && o == origin && *r == rows)
}

#[test]
fn refresh_reads_the_new_rows_into_the_pbix() {
    let dir = TempDir::new("pbix-refresh-all");
    let (mut s, path, data) = fixture(&dir);
    assert!(!table_files(&path, "Notes").is_empty(), "Notes has storage files");
    let before = refresh_state(&mut s, "Product");
    assert_eq!(before["state"], "Ready", "{before}");

    // the sources change: a new product, a new price, two more sales lines
    std::fs::write(
        data.join("products.csv"),
        "Key,Name,Price\n1,Apple,2\n2,Banana,0.25\n3,Leek,2\n4,Kale,4\n",
    )
    .unwrap();
    let mut rows = sales_rows();
    rows.push(vec!["13", "4", "1"]);
    rows.push(vec!["14", "3", "4"]);
    std::fs::write(data.join("sales.xlsx"), xlsx(&rows)).unwrap();

    let out = run(&mut s, "model.refresh", json!({}));
    assert_eq!(out["format"], "pbix", "{out}");
    assert_eq!(out["verified"], true);
    let list = origins(&out);
    assert!(has(&list, "Product", "m", 4), "{out}");
    assert!(has(&list, "Sales", "m", 5), "{out}");
    assert!(has(&list, "Notes", "m", 2), "{out}");
    assert!(has(&list, "Big Sales", "dax", 3), "{out}");
    assert_eq!(out["kept"], 0);
    assert!(!dir.join("shop.pbix.bicraft-tmp").exists());
    assert!(!dir.join("shop.pbix.bak").exists(), "no backup unless asked");

    // bicraft's reader sees the new rows
    let e = editor(&path);
    assert_eq!(e.row_count("Product").unwrap(), Some(4));
    assert_eq!(values(&e, "Product", "Price"), reals(&[2.0, 0.25, 2.0, 4.0]));
    assert_eq!(values(&e, "Sales", "Order"), ints(&[10, 11, 12, 13, 14]));
    // the calculated column follows the new prices
    assert_eq!(values(&e, "Sales", "Amount"), reals(&[4.0, 1.25, 6.0, 4.0, 8.0]));
    assert_eq!(e.row_count("Big Sales").unwrap(), Some(3));
    // the structure is the one of the file before
    let m = e.base();
    let names: Vec<&str> = m.tables.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["Product", "Sales", "Notes", "Big Sales"]);
    assert_eq!(m.relationships.len(), 1);
    assert_eq!(m.roles.len(), 1);
    // the session reads the new data, the measures follow
    let q = run(
        &mut s,
        "dax.evaluate",
        json!({"query": "EVALUATE ROW(\"p\", [Products], \"t\", [Total])"}),
    );
    assert_eq!(q["results"][0]["rows"][0]["[p]"], 4, "{q}");
    assert_eq!(q["results"][0]["rows"][0]["[t]"], 23.25, "{q}");
    // the refresh state: Ready, refreshed at the refresh time, as Desktop writes it
    let after = refresh_state(&mut s, "Product");
    assert_eq!(after["state"], "Ready", "{after}");
    let at = after["refreshed_time"].as_str().unwrap();
    let want = out["refreshed_at"].as_str().unwrap();
    assert_eq!(at.get(..16), want.get(..16), "{after} vs {out}");
    assert_ne!(after["refreshed_time"], before["refreshed_time"]);
    assert!(!s.dirty, "a refresh writes the file: nothing left to save");
    // a fresh session opens the file the same way
    let mut again = Session::new();
    run(&mut again, "file.open", json!({"path": path}));
    let q = run(
        &mut again,
        "dax.evaluate",
        json!({"query": "EVALUATE ROW(\"t\", [Total])"}),
    );
    assert_eq!(q["results"][0]["rows"][0]["[t]"], 23.25, "{q}");
}

#[test]
fn refreshing_one_table_keeps_the_others_byte_for_byte() {
    let dir = TempDir::new("pbix-refresh-one");
    let (mut s, path, data) = fixture(&dir);
    let notes_before = table_files(&path, "Notes");
    let sales_order_before = values(&editor(&path), "Sales", "Order");
    let big_before = table_files(&path, "Big Sales");
    // both sources change; only Product is refreshed
    std::fs::write(
        data.join("products.csv"),
        "Key,Name,Price\n1,Apple,10\n2,Banana,0.25\n3,Leek,2\n",
    )
    .unwrap();
    let mut rows = sales_rows();
    rows.push(vec!["99", "1", "7"]);
    std::fs::write(data.join("sales.xlsx"), xlsx(&rows)).unwrap();

    let out = run(&mut s, "model.refresh", json!({"tables": ["Product"]}));
    let list = origins(&out);
    assert!(has(&list, "Product", "m", 3), "{out}");
    // Sales[Amount] reads Product[Price]: recomputed over Sales' stored rows
    assert!(has(&list, "Sales", "stored", 3), "{out}");
    // Big Sales reads Sales as a whole: evaluated again
    assert!(has(&list, "Big Sales", "dax", 2), "{out}");
    assert!(!list.iter().any(|(t, _, _)| t == "Notes"), "{out}");
    assert_eq!(out["kept"], 1);

    let e = editor(&path);
    assert_eq!(values(&e, "Product", "Price"), reals(&[10.0, 0.25, 2.0]));
    // Sales keeps its stored rows (the new Excel line is not read), its column is recomputed
    assert_eq!(values(&e, "Sales", "Order"), sales_order_before);
    assert_eq!(values(&e, "Sales", "Amount"), reals(&[20.0, 1.25, 30.0]));
    // Notes: every storage file byte for byte
    assert_eq!(table_files(&path, "Notes"), notes_before);
    assert_ne!(table_files(&path, "Big Sales"), big_before);
    // its partition keeps its refresh time, Product's moves
    let notes = refresh_state(&mut s, "Notes");
    let product = refresh_state(&mut s, "Product");
    assert_ne!(
        notes["refreshed_time"], product["refreshed_time"],
        "{notes} {product}"
    );
}

#[test]
fn a_failing_source_leaves_the_file_untouched() {
    let dir = TempDir::new("pbix-refresh-fail");
    let (mut s, path, data) = fixture(&dir);
    let bytes = std::fs::read(&path).unwrap();
    std::fs::remove_file(data.join("sales.xlsx")).unwrap();
    let e = s.execute("model.refresh", &json!({})).unwrap_err();
    assert!(e.contains("nothing was refreshed"), "{e}");
    assert!(e.contains("Sales"), "{e}");
    assert_eq!(std::fs::read(&path).unwrap(), bytes, "the file is untouched");
    assert!(!dir.join("shop.pbix.bicraft-tmp").exists());
    // new rows that break a relationship (its "one" side not unique) are refused too
    std::fs::write(data.join("sales.xlsx"), xlsx(&sales_rows())).unwrap();
    std::fs::write(
        data.join("products.csv"),
        "Key,Name,Price\n1,Apple,1\n1,Again,2\n",
    )
    .unwrap();
    let e = s
        .execute("model.refresh", &json!({"tables": ["Product"]}))
        .unwrap_err();
    assert!(e.contains("nothing was refreshed"), "{e}");
    assert_eq!(std::fs::read(&path).unwrap(), bytes, "the file is untouched");
    // the session goes on with the file as it was
    let q = run(
        &mut s,
        "dax.evaluate",
        json!({"query": "EVALUATE ROW(\"p\", [Products])"}),
    );
    assert_eq!(q["results"][0]["rows"][0]["[p]"], 3, "{q}");
}

#[test]
fn refusals_and_force() {
    let dir = TempDir::new("pbix-refresh-refusals");
    let (mut s, path, _data) = fixture(&dir);
    let bytes = std::fs::read(&path).unwrap();
    let e = s
        .execute("model.refresh", &json!({"tables": ["Nope"]}))
        .unwrap_err();
    assert!(e.contains("not in the model"), "{e}");
    let e = s
        .execute("model.refresh", &json!({"allow_connectors": ["ftp"]}))
        .unwrap_err();
    assert!(e.contains("unknown connector"), "{e}");
    // unsaved edits: refused without force
    run(
        &mut s,
        "measure.add",
        json!({"table": "Product", "name": "Top", "expression": "MAX(Product[Price])"}),
    );
    let e = s.execute("model.refresh", &json!({})).unwrap_err();
    assert!(e.contains("unsaved edits") && e.contains("force"), "{e}");
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    // with force: the saved model is refreshed, the edit stays unsaved and is saved afterwards
    let out = run(&mut s, "model.refresh", json!({"force": true, "backup": true}));
    assert!(s.dirty);
    assert!(out["warnings"][0].as_str().unwrap().contains("unsaved"), "{out}");
    assert_eq!(std::fs::read(dir.join("shop.pbix.bak")).unwrap(), bytes, "{out}");
    run(&mut s, "file.save", json!({"backup": false}));
    let back = editor(&path);
    let product = back.base().tables.iter().find(|t| t.name == "Product").unwrap();
    assert!(
        product.measures.iter().any(|m| m.name == "Top"),
        "the edit is saved over the refreshed file"
    );
    assert_eq!(back.row_count("Product").unwrap(), Some(3));
}

#[test]
fn a_pbit_holds_no_data() {
    let dir = TempDir::new("pbix-refresh-pbit");
    let (mut s, path, data) = fixture(&dir);
    let pbit = dir.join("shop.pbit");
    run(&mut s, "file.convert", json!({"from": path, "to": pbit}));
    let mut t = Session::new();
    run(&mut t, "file.open", json!({"path": pbit, "data_root": data}));
    let e = t.execute("model.refresh", &json!({})).unwrap_err();
    assert!(e.contains("PBIT holds no data"), "{e}");
    // the button: enabled on the PBIX, greyed on the PBIT
    use crate::command_ui::Caller;
    assert!(s.command_enabled("model.refresh", Caller::Editor).is_ok());
    assert!(t.command_enabled("model.refresh", Caller::Editor).is_err());
}

#[test]
fn over_the_memory_limit_nothing_is_written() {
    let dir = TempDir::new("pbix-refresh-limit");
    let (mut s, path, data) = fixture(&dir);
    let bytes = std::fs::read(&path).unwrap();
    std::fs::write(data.join("products.csv"), "Key,Name,Price\n1,Apple,2\n").unwrap();
    // a pool of its own, so that the limit does not reach the other tests of the process: room
    // for the part and its backup, not for the rows
    let pool: &'static bicraft_mem::Pool = Box::leak(Box::new(bicraft_mem::Pool::new()));
    let part = bicraft_opc::Package::open(&path)
        .unwrap()
        .get(bicraft_pbix::DATA_MODEL)
        .unwrap()
        .data
        .len();
    let held = editor(&path).backup_len() + part;
    pool.set_limit(Some(held + 64));
    s.preview.set_memory_pool(pool);
    let e = s.execute("model.refresh", &json!({})).unwrap_err();
    assert!(e.contains("memory") && e.contains("nothing was refreshed"), "{e}");
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(pool.stats().used, 0, "every reservation is given back");
    // with room, the same refresh goes through
    pool.set_limit(None);
    run(&mut s, "model.refresh", json!({}));
    assert_eq!(editor(&path).row_count("Product").unwrap(), Some(1));
}

/// A larger table (200,000 rows from a CSV file) refreshed under a memory limit: what the
/// refresh holds is counted against the limit, stays under it, and is given back.
#[test]
fn a_large_table_refreshes_within_the_memory_limit() {
    let dir = TempDir::new("pbix-refresh-large");
    let data = dir.join("data");
    std::fs::create_dir_all(&data).unwrap();
    let csv = data.join("lines.csv");
    let lines = |n: usize, k: i64| {
        let mut s = String::from("Id,Shop,Amount\n");
        for i in 0..n {
            s.push_str(&format!("{i},S{},{}\n", i % 97, (i as i64 * k) % 1000));
        }
        s
    };
    std::fs::write(&csv, lines(200_000, 7)).unwrap();
    let m = format!(
        "let Source = Csv.Document(File.Contents(\"{}\"), [Delimiter=\",\", Encoding=65001]), \
         Promoted = Table.PromoteHeaders(Source, [PromoteAllScalars=true]), \
         Typed = Table.TransformColumnTypes(Promoted, {{{{\"Id\", Int64.Type}}, {{\"Shop\", type text}}, {{\"Amount\", Int64.Type}}}}) in Typed",
        escape(&csv)
    );
    let path = dir.join("large.pbix");
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Large"}));
    run(
        &mut s,
        "table.import",
        json!({"name": "Lines", "m": m, "data_root": data}),
    );
    run(&mut s, "file.save_as", json!({"path": path}));
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": path, "data_root": data}));
    std::fs::write(&csv, lines(250_000, 11)).unwrap();
    let pool: &'static bicraft_mem::Pool = Box::leak(Box::new(bicraft_mem::Pool::new()));
    let limit = 256usize << 20;
    pool.set_limit(Some(limit));
    s.preview.set_memory_pool(pool);
    let out = run(&mut s, "model.refresh", json!({}));
    assert_eq!(out["tables"][0]["rows"], 250_000, "{out}");
    let st = pool.stats();
    // the rows were counted (3 columns of 250,000 values), under the limit, then given back
    assert!(st.peak > 250_000 * 3 * 40, "{st:?}");
    assert!(st.peak <= limit, "{st:?}");
    assert_eq!(st.used, 0, "{st:?}");
    assert_eq!(editor(&path).row_count("Lines").unwrap(), Some(250_000));
}
