//! `read_sqlitedb` over a synthetic database written by Python's `sqlite3` (see gen_fixture.py).

use bicraft_metadata::{Error, read_sqlitedb, tables_from_sqlitedb};
use bicraft_model::DataType;

const FIXTURE: &[u8] = include_bytes!("fixtures/synthetic_metadata.sqlitedb");

#[test]
fn reads_the_synthetic_database() {
    let m = read_sqlitedb(FIXTURE).unwrap();
    assert_eq!(m.culture.as_deref(), Some("en-US"));
    assert_eq!(m.extra.get("name"), Some(&serde_json::json!("Model")));
    assert_eq!(
        m.extra.get("defaultPowerBIDataSourceVersion"),
        Some(&serde_json::json!("powerBI_V3"))
    );
    assert_eq!(m.annotations.len(), 1);

    let names: Vec<_> = m.tables.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["Calendar", "Sales"], "ID order, system table dropped");

    let calendar = &m.tables[0];
    assert!(calendar.is_hidden);
    assert_eq!(calendar.description.as_deref(), Some("Days"));
    let date = &calendar.columns[0];
    assert_eq!(date.name, "Date");
    assert_eq!(date.data_type, DataType::DateTime);
    assert_eq!(
        date.extra.get("type"),
        Some(&serde_json::json!("calculatedTableColumn"))
    );
    assert_eq!(date.annotations.len(), 1);
    let month = &calendar.columns[1];
    assert_eq!(month.expression.as_deref(), Some("FORMAT([Date], \"MMM\")"));
    assert_eq!(month.sort_by_column.as_deref(), Some("Date"));
    assert_eq!(calendar.partitions[0].source_type, "calculated");

    let sales = &m.tables[1];
    let cols: Vec<_> = sales.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(cols, ["Amount", "RowNumber-0", "Day"]);
    assert!(sales.columns[1].is_row_number());
    assert!(
        sales.column("RowNumber-0").is_none(),
        "row-number columns are not user columns"
    );
    assert_eq!(sales.columns[0].data_type, DataType::Double);
    assert_eq!(sales.columns[0].summarize_by.as_deref(), Some("sum"));
    let measures: Vec<_> = sales.measures.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(measures, ["Rows", "Total"]);
    assert_eq!(sales.measures[1].display_folder.as_deref(), Some("KPIs"));
    let p = &sales.partitions[0];
    assert_eq!((p.source_type.as_str(), p.mode.as_deref()), ("m", Some("import")));
    assert!(p.expression.as_deref().unwrap().starts_with("let\n"));

    let r = &m.relationships[0];
    assert_eq!(
        (
            r.from_table.as_str(),
            r.from_column.as_str(),
            r.to_table.as_str(),
            r.to_column.as_str()
        ),
        ("Sales", "Day", "Calendar", "Date")
    );
    assert_eq!(m.expressions[0].kind, "m");
}

#[test]
fn only_known_tables_are_loaded() {
    let t = tables_from_sqlitedb(FIXTURE).unwrap();
    assert!(t.has_table("Column"));
    assert!(!t.has_table("StorageFile"));
}

#[test]
fn truncated_and_corrupted_files_do_not_panic() {
    for len in [
        0,
        1,
        99,
        100,
        512,
        4096,
        4097,
        FIXTURE.len() / 2,
        FIXTURE.len() - 1,
    ] {
        let _ = read_sqlitedb(FIXTURE.get(..len).unwrap_or_default());
    }
    // Flip bytes across the file, one position at a time (strided to keep the test fast).
    let mut bytes = FIXTURE.to_vec();
    for pos in (0..bytes.len()).step_by(97) {
        let old = bytes[pos];
        bytes[pos] = old ^ 0xA5;
        let _ = read_sqlitedb(&bytes);
        bytes[pos] = old;
    }
    assert!(matches!(
        read_sqlitedb(b"SQLite format 3\0"),
        Err(Error::Sqlite(_))
    ));
}
