//! Reading a workbook Excel saved (`tests/fixtures/excel/excel-model.xlsx`: Excel 16.0 build
//! 20625 built the model from our synthetic worksheet tables through its COM object model, see
//! that folder's `SOURCE.txt`). The expected values are what we put in; the tests check that
//! the reader gives them back, with the measures' properties, and that the backup's CRC markers
//! are the ones `synthetic::crc_marker` computes.

use bicraft_model::DataType;
use bicraft_powerpivot::{ITEM_DATA, Limits, read_workbook, synthetic};
use bicraft_vertipaq::CellValue;

fn fixture() -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/excel/excel-model.xlsx");
    std::fs::read(path).unwrap()
}

fn ints(v: &[i64]) -> Vec<CellValue> {
    v.iter().map(|&i| CellValue::Int(i)).collect()
}

fn texts(v: &[&str]) -> Vec<CellValue> {
    v.iter().map(|s| CellValue::Text((*s).into())).collect()
}

#[test]
fn the_excel_saved_workbook_gives_back_its_model() {
    let wb = read_workbook(&fixture(), &Limits::default()).unwrap();
    assert!(wb.warnings.is_empty(), "{:?}", wb.warnings);
    assert_eq!(wb.compatibility_level, Some(1103));
    let m = &wb.model;
    let names: Vec<&str> = m.tables.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["Product", "Customer", "Sales"]);
    let cols = |t: &str| -> Vec<(String, DataType)> {
        m.table(t)
            .unwrap()
            .columns
            .iter()
            .map(|c| (c.name.clone(), c.data_type))
            .collect()
    };
    let owned = |v: &[(&str, DataType)]| -> Vec<(String, DataType)> {
        v.iter().map(|(n, t)| ((*n).to_owned(), *t)).collect()
    };
    assert_eq!(
        cols("Product"),
        owned(&[
            ("ProductKey", DataType::Int64),
            ("Product", DataType::String),
            ("Category", DataType::String),
            ("UnitPrice", DataType::Double)
        ])
    );
    assert_eq!(
        cols("Customer"),
        owned(&[
            ("CustomerKey", DataType::Int64),
            ("Customer", DataType::String),
            ("Country", DataType::String)
        ])
    );
    assert_eq!(
        cols("Sales"),
        owned(&[
            ("SaleID", DataType::Int64),
            ("ProductKey", DataType::Int64),
            ("CustomerKey", DataType::Int64),
            ("OrderDate", DataType::DateTime),
            ("Quantity", DataType::Int64)
        ])
    );
    assert!(
        m.tables
            .iter()
            .all(|t| !t.is_hidden && t.columns.iter().all(|c| !c.is_hidden))
    );

    let rels: Vec<(&str, &str, &str, &str, bool)> = m
        .relationships
        .iter()
        .map(|r| {
            (
                r.from_table.as_str(),
                r.from_column.as_str(),
                r.to_table.as_str(),
                r.to_column.as_str(),
                r.is_active,
            )
        })
        .collect();
    assert_eq!(
        rels,
        [
            ("Sales", "ProductKey", "Product", "ProductKey", true),
            ("Sales", "CustomerKey", "Customer", "CustomerKey", true)
        ]
    );

    let measure = |t: &str, n: &str| {
        m.table(t)
            .unwrap()
            .measures
            .iter()
            .find(|x| x.name == n)
            .cloned()
            .unwrap()
    };
    let q = measure("Sales", "Total quantity");
    assert_eq!(q.expression, "SUM(Sales[Quantity])");
    assert_eq!(q.format_string.as_deref(), Some("#,0"));
    assert_eq!(q.description.as_deref(), Some("Units sold"));
    let r = measure("Sales", "Revenue");
    assert_eq!(
        r.expression,
        "SUMX(Sales, Sales[Quantity] * RELATED(Product[UnitPrice]))"
    );
    assert_eq!(r.format_string.as_deref(), Some("#,0.00"));
    assert_eq!(r.description, None);
    let c = measure("Customer", "Customers");
    assert_eq!(c.expression, "COUNTROWS(Customer)");
    assert_eq!(c.format_string, None, "General has no format string");
    assert!(!q.is_hidden && !r.is_hidden && !c.is_hidden);
    assert_eq!(m.tables.iter().map(|t| t.measures.len()).sum::<usize>(), 3);
}

#[test]
fn the_excel_saved_workbook_gives_back_its_values() {
    let wb = read_workbook(&fixture(), &Limits::default()).unwrap();
    let table = |n: &str| wb.tables.iter().find(|t| t.name == n).unwrap();
    let col = |t: &str, c: &str| table(t).columns.iter().find(|(n, _)| n == c).unwrap().1.clone();
    assert_eq!(col("Product", "ProductKey"), ints(&[1, 2, 3, 4, 5]));
    assert_eq!(
        col("Product", "Product"),
        texts(&["Anvil", "Bolt", "Kite", "Yo-yo", "Lamp"])
    );
    assert_eq!(
        col("Product", "Category"),
        texts(&["Tools", "Tools", "Toys", "Toys", "Home"])
    );
    assert_eq!(
        col("Product", "UnitPrice"),
        [12.5, 0.25, 7.75, 3.0, 19.99].map(CellValue::Real).to_vec()
    );
    assert_eq!(col("Customer", "CustomerKey"), ints(&[10, 20, 30]));
    assert_eq!(
        col("Customer", "Customer"),
        texts(&["Alder Ltd", "Birch & Co", "Cedar SA"])
    );
    assert_eq!(
        col("Customer", "Country"),
        texts(&["France", "Belgium", "France"])
    );
    assert_eq!(col("Sales", "SaleID"), ints(&(101..=112).collect::<Vec<_>>()));
    assert_eq!(
        col("Sales", "ProductKey"),
        ints(&[1, 2, 3, 4, 5, 1, 2, 3, 2, 5, 4, 2])
    );
    assert_eq!(
        col("Sales", "CustomerKey"),
        ints(&[10, 20, 30, 10, 20, 30, 10, 10, 20, 30, 30, 20])
    );
    // 2024-01-01 (OLE date 45292) then every 7 days.
    assert_eq!(
        col("Sales", "OrderDate"),
        (0..12)
            .map(|i| CellValue::DateTime(45292.0 + 7.0 * f64::from(i)))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        col("Sales", "Quantity"),
        ints(&[3, 10, 1, 4, 2, 7, 5, 1, 6, 2, 8, 9])
    );
}

#[test]
fn excel_crc_markers_are_the_ones_bicraft_computes() {
    let package = bicraft_opc::Package::read_bytes(&fixture()).unwrap();
    let item = package.get(ITEM_DATA).unwrap().bytes().unwrap();
    let abf = bicraft_abf::Abf::parse(&item).unwrap();
    assert!(abf.header().crc_markers && abf.header().apply_compression);
    assert!(abf.files().len() > 50);
    for f in abf.files() {
        let stored = abf.stored(f).unwrap();
        let (body, marker) = stored.split_at(stored.len() - 4);
        assert_eq!(synthetic::crc_marker(body), marker, "{}", f.name());
    }
}
