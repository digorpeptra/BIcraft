//! The Navigator's items and queries (`navigate.rs`) and entered tables (`entered.rs`), over the
//! synthetic samples of `tests_desktop.rs`.

use super::entered::{self, Cell, EnteredType};
use super::navigate::{NavKind, NavOptions, navigate};
use super::tests_desktop::data_root;
use crate::{CellValue, Culture, DataType, SourceOptions};

fn opts(root: &std::path::Path, culture: &str) -> SourceOptions {
    SourceOptions {
        file_roots: vec![root.to_path_buf()],
        culture: Some(culture.into()),
        ..SourceOptions::default()
    }
}

fn types(m: &str, o: &SourceOptions) -> Vec<(String, DataType)> {
    let d = super::load_m_with(m, o).unwrap_or_else(|e| panic!("{e}\n{m}"));
    d.columns()
        .iter()
        .map(|c| (c.name.clone(), c.data_type))
        .collect()
}

fn path(root: &std::path::Path, f: &str) -> String {
    root.join(f).to_str().unwrap().to_string()
}

#[test]
fn a_workbook_lists_its_sheets_and_tables_with_typed_queries() {
    let root = data_root();
    let o = opts(&root, "en-US");
    let p = path(&root, "book.xlsx");
    let items = navigate(NavKind::Excel, &p, &o, &NavOptions::default()).unwrap();
    let names: Vec<(&str, &str)> = items.iter().map(|i| (i.name.as_str(), i.kind.as_str())).collect();
    assert_eq!(names, [("Data", "Sheet"), ("Other", "Sheet")]);
    let data = &items[0];
    assert!(data.error.is_none(), "{:?}", data.error);
    // the query our Desktop golden test loads (tests_desktop.rs, desktop_t16_excel), but for
    // `When`: one of its cells is at noon, so detection says date/time where that test says date
    let want = format!(
        "let\n    Source = Excel.Workbook(File.Contents(\"{p}\"), null, true),\n    Data_Sheet = \
         Source{{[Item=\"Data\",Kind=\"Sheet\"]}}[Data],\n    #\"Promoted Headers\" = \
         Table.PromoteHeaders(Data_Sheet, [PromoteAllScalars=true]),\n    #\"Changed Type\" = \
         Table.TransformColumnTypes(#\"Promoted Headers\",{{{{\"Name\", type text}}, {{\"Qty\", \
         Int64.Type}}, {{\"When\", type datetime}}, {{\"Ok\", type logical}}, {{\"Price\", type \
         number}}}})\nin\n    #\"Changed Type\""
    );
    assert_eq!(data.m, want);
    // a sheet whose first row is not text keeps its rows
    assert!(!items[1].m.contains("PromoteHeaders"), "{}", items[1].m);
    // a workbook's table: no header step, its own column names
    let p2 = path(&root, "book2.xlsx");
    let items = navigate(NavKind::Excel, &p2, &o, &NavOptions::default()).unwrap();
    let sales = items.iter().find(|i| i.kind == "Table").expect("the Sales table");
    assert_eq!(sales.name, "Sales");
    assert!(!sales.m.contains("PromoteHeaders"));
    assert_eq!(
        types(&sales.m, &o),
        [
            ("Item".into(), DataType::String),
            ("Units".into(), DataType::Int64)
        ]
    );
}

#[test]
fn a_text_file_is_one_item_with_its_delimiter_headers_and_types() {
    let root = data_root();
    let o = opts(&root, "en-US");
    let p = path(&root, "sales.csv");
    let items = navigate(NavKind::Csv, &p, &o, &NavOptions::default()).unwrap();
    assert_eq!(items.len(), 1);
    let it = &items[0];
    assert_eq!((it.name.as_str(), it.kind.as_str()), ("sales", "File"));
    assert!(
        it.m.contains("[Delimiter=\",\", Encoding=65001, QuoteStyle=QuoteStyle.Csv]"),
        "{}",
        it.m
    );
    assert!(it.m.contains("Table.PromoteHeaders(Source, [PromoteAllScalars=true])"));
    assert_eq!(
        types(&it.m, &o),
        [
            ("Id".into(), DataType::Int64),
            ("Name".into(), DataType::String),
            ("Amount".into(), DataType::Double),
            ("Date".into(), DataType::DateTime),
            ("Note".into(), DataType::String),
        ]
    );
    // semicolons, a Windows-1252 file
    let it = &navigate(
        NavKind::Csv,
        &path(&root, "cp1252.csv"),
        &o,
        &NavOptions::default(),
    )
    .unwrap()[0];
    assert!(it.m.contains("Delimiter=\";\", Encoding=1252"), "{}", it.m);
    // the delimiter given
    let nav = NavOptions {
        delimiter: Some('\t'),
        no_types: true,
    };
    let it = &navigate(NavKind::Csv, &p, &o, &nav).unwrap()[0];
    assert!(it.m.contains("Delimiter=\"#(tab)\""), "{}", it.m);
    assert!(!it.m.contains("TransformColumnTypes"));
}

#[test]
fn a_json_list_of_records_is_a_table_and_nested_values_say_why_they_do_not_load() {
    let root = data_root();
    let o = opts(&root, "en-US");
    let items = navigate(
        NavKind::Json,
        &path(&root, "people.json"),
        &o,
        &NavOptions::default(),
    )
    .unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].kind, "List");
    assert!(items[0].m.contains("Table.FromRecords(Source)"), "{}", items[0].m);
    // its `tags` cells are lists
    assert!(items[0].error.is_some(), "{:?}", items[0]);
    // a record of lists: one item per list
    std::fs::write(
        root.join("shop.json"),
        r#"{"name": "Shop", "orders": [{"id": 1, "total": 2.5}, {"id": 2, "total": 4}], "tags": ["a", "b"]}"#,
    )
    .unwrap();
    let items = navigate(
        NavKind::Json,
        &path(&root, "shop.json"),
        &o,
        &NavOptions::default(),
    )
    .unwrap();
    let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, ["shop", "orders", "tags"]);
    let orders = &items[1];
    assert!(orders.error.is_none(), "{:?}", orders.error);
    assert_eq!(
        types(&orders.m, &o),
        [("id".into(), DataType::Int64), ("total".into(), DataType::Double)]
    );
    assert_eq!(types(&items[2].m, &o), [("Value".into(), DataType::String)]);
}

#[test]
fn a_folder_lists_its_files_and_appends_its_text_files() {
    let root = data_root();
    let sub = root.join("monthly");
    std::fs::create_dir(&sub).unwrap();
    std::fs::write(sub.join("jan.csv"), "Region,Units\nNorth,3\n").unwrap();
    std::fs::write(sub.join("feb.csv"), "Region,Units\nSouth,5\nEast,1\n").unwrap();
    std::fs::write(sub.join("notes.txt"), "not data").unwrap();
    let o = opts(&root, "en-US");
    let items = navigate(NavKind::Folder, sub.to_str().unwrap(), &o, &NavOptions::default()).unwrap();
    let names: Vec<(&str, &str)> = items.iter().map(|i| (i.name.as_str(), i.kind.as_str())).collect();
    assert_eq!(names, [("monthly", "Folder"), ("monthly (combined)", "Combined")]);
    let files = super::load_m_with(&items[0].m, &o).unwrap();
    assert_eq!(files.row_count(), 3);
    assert!(files.column("Content").is_none());
    let combined = super::load_m_with(&items[1].m, &o).unwrap();
    assert_eq!(combined.row_count(), 3);
    assert_eq!(
        combined.column("Units").map(|c| c.data_type),
        Some(DataType::Int64)
    );
}

#[test]
fn files_outside_the_roots_and_wrong_kinds_are_errors() {
    let root = data_root();
    let other = data_root();
    let o = opts(&root, "en-US");
    assert!(
        navigate(
            NavKind::Csv,
            &path(&other, "sales.csv"),
            &o,
            &NavOptions::default()
        )
        .is_err()
    );
    let none = SourceOptions::default();
    assert!(
        navigate(
            NavKind::Csv,
            &path(&root, "sales.csv"),
            &none,
            &NavOptions::default()
        )
        .is_err()
    );
    // a CSV read as a workbook, a missing file
    assert!(
        navigate(
            NavKind::Excel,
            &path(&root, "sales.csv"),
            &o,
            &NavOptions::default()
        )
        .is_err()
    );
    assert!(
        navigate(
            NavKind::Json,
            &path(&root, "missing.json"),
            &o,
            &NavOptions::default()
        )
        .is_err()
    );
    assert_eq!(NavKind::from_extension("a/B.XLSX"), Some(NavKind::Excel));
    assert_eq!(NavKind::from_extension("a.tsv"), Some(NavKind::Csv));
    assert_eq!(NavKind::from_extension("a.pbix"), None);
}

#[test]
fn detected_types_follow_power_query() {
    let en = Culture::en_us();
    let fr = Culture::fr_fr();
    let t = |v: &[&str], c| entered::infer_texts(v.iter().copied(), c);
    assert_eq!(t(&["1", "-2", ""], en), EnteredType::Whole);
    assert_eq!(t(&["1.5", "2"], en), EnteredType::Number);
    assert_eq!(t(&["1,5", "2"], fr), EnteredType::Number);
    assert_eq!(t(&["1,5"], en), EnteredType::Text);
    assert_eq!(t(&["TRUE", "false"], en), EnteredType::Logical);
    assert_eq!(t(&["2026-01-05", "01/31/2026"], en), EnteredType::Date);
    assert_eq!(t(&["2026-01-05T10:30:00"], en), EnteredType::DateTime);
    assert_eq!(t(&["a", "1"], en), EnteredType::Text);
    assert_eq!(t(&["", " "], en), EnteredType::Text);
}

fn cells(rows: &[&[&str]]) -> Vec<Vec<Cell>> {
    rows.iter()
        .map(|r| {
            r.iter()
                .map(|c| {
                    if c.is_empty() {
                        Cell::Null
                    } else {
                        Cell::Text((*c).into())
                    }
                })
                .collect()
        })
        .collect()
}

#[test]
fn an_entered_table_loads_its_typed_values_in_any_culture_and_reads_back() {
    let columns = vec![
        ("Name".to_string(), Some(EnteredType::Text)),
        ("Qty".to_string(), None),
        ("Price".to_string(), None),
        ("When".to_string(), Some(EnteredType::Date)),
        ("Ok".to_string(), None),
        ("Fee \"x\" #1".to_string(), Some(EnteredType::Fixed)),
        ("At".to_string(), Some(EnteredType::DateTime)),
    ];
    let rows = cells(&[
        &[
            "Alpha",
            "3",
            "1,5",
            "05/01/2026",
            "true",
            "2,125",
            "2026-01-05T10:30:00",
        ],
        &["Beta \"b\"\n", "-7", "1000", "2024-02-29", "false", "", ""],
        &["", "", "", "", "", "3", "2026-01-05"],
    ]);
    for query in ["en-US", "fr-FR"] {
        let e = entered::build(&columns, &rows, Culture::fr_fr(), Culture::from_name(query)).unwrap();
        let types: Vec<EnteredType> = e.columns.iter().map(|c| c.1).collect();
        assert_eq!(
            types,
            [
                EnteredType::Text,
                EnteredType::Whole,
                EnteredType::Number,
                EnteredType::Date,
                EnteredType::Logical,
                EnteredType::Fixed,
                EnteredType::DateTime
            ]
        );
        assert_eq!(e.values[2][0], CellValue::Real(1.5));
        assert_eq!(e.values[5][0], CellValue::Decimal(21_250));
        assert!(entered::is_entered(&e.m));
        let o = SourceOptions {
            culture: Some(query.into()),
            ..SourceOptions::default()
        };
        let d = super::load_m_with(&e.m, &o).unwrap_or_else(|err| panic!("{query}: {err}\n{}", e.m));
        let loaded: Vec<Vec<CellValue>> = d.columns().iter().map(|c| c.values.clone()).collect();
        assert_eq!(loaded, e.values, "{query}");
        let back = entered::read(&e.m, &o).unwrap();
        assert_eq!(back.columns, e.columns);
        assert_eq!(back.texts, e.texts);
        assert_eq!(back.values, e.values);
    }
}

#[test]
fn desktop_enter_data_and_table_literals_read_back_other_queries_do_not() {
    // the Enter data query of tests_desktop.rs (desktop_t01_enterdata)
    let desktop = "let\n    Source = Table.FromRows(Json.Document(Binary.Decompress(Binary.FromText(\"XZBNC8IwDIb/Sul5XdN0Ku428QPFmzs5dqhS2aF+4DbQf2+TKQMhTdL2yZumVSWL8GicTKSNy6QT8nqiEXAa0+7Ze1knlVz4jiA1iw5TJIwQBaiAKtfF/rBicuOuV8cUa3nWRW0NadJR+dNc+sCilkkFqRl0Nc6JzWJ+caEd4NWjjfvR6OzIj7r1Ifw5LmBtk6DNfpH77NxNzBLxHXBsUDbDiO50jv5FNVZH+4Jv3zK2vTMlMhQURDp+hVWQlQZyCzkAXdLvCVnXHw==\", BinaryEncoding.Base64), Compression.Deflate)), let _t = ((type nullable text) meta [Serialized.Text = true]) in type table [Name = _t, Qty = _t, Price = _t, When = _t, Flag = _t]),\n    #\"Changed Type\" = Table.TransformColumnTypes(Source,{{\"Name\", type text}, {\"Qty\", Int64.Type}, {\"Price\", type number}, {\"When\", type date}, {\"Flag\", type logical}})\nin\n    #\"Changed Type\"";
    let o = SourceOptions {
        culture: Some("en-US".into()),
        ..SourceOptions::default()
    };
    let e = entered::read(desktop, &o).unwrap();
    let names: Vec<(&str, EnteredType)> = e.columns.iter().map(|(n, t)| (n.as_str(), *t)).collect();
    assert_eq!(
        names,
        [
            ("Name", EnteredType::Text),
            ("Qty", EnteredType::Whole),
            ("Price", EnteredType::Number),
            ("When", EnteredType::Date),
            ("Flag", EnteredType::Logical)
        ]
    );
    assert_eq!(e.texts.len(), 9);
    assert_eq!(
        e.texts[0],
        [
            Some("Alpha".to_string()),
            Some("3".into()),
            Some("1.5".into()),
            Some("2026-01-05".into()),
            Some("true".into())
        ]
    );
    let literal = "let\n    Source = #table(type table [A = text, #\"B c\" = Int64.Type], {\n        {\"x\", 1},\n        {null, 2}\n    })\nin\n    Source";
    let e = entered::read(literal, &o).unwrap();
    assert_eq!(
        e.columns,
        [
            ("A".into(), EnteredType::Text),
            ("B c".into(), EnteredType::Whole)
        ]
    );
    assert_eq!(e.texts[1], [None, Some("2".to_string())]);
    for not in [
        "let Source = Csv.Document(File.Contents(\"a.csv\")) in Source",
        "let Source = #table({\"A\"}, {{1}}), R = Table.RenameColumns(Source, {{\"A\", \"B\"}}) in R",
        "let Source = Other in Source",
        "#table({\"A\"}, {{1}}) & Excel.CurrentWorkbook()",
        "",
        "let",
    ] {
        assert!(entered::read(not, &o).is_err(), "{not}");
    }
}

/// The JSON rows packed in an Enter data query's `Binary.FromText("…")`.
fn packed_rows(m: &str) -> String {
    let b64 = m
        .split("Binary.FromText(\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .unwrap();
    let packed = super::lib::misc::base64_decode(b64).unwrap();
    String::from_utf8(miniz_oxide::inflate::decompress_to_vec(&packed).unwrap()).unwrap()
}

/// Power BI Desktop 2.159.1231.0 (2026-11-23): its own Enter data writes an empty cell as `""`
/// and never `null`, and its Create Table dialog (Edit of the Source step) fails on a query whose
/// rows hold `null` ("Object reference not set to an instance of an object").
#[test]
fn entered_rows_hold_empty_texts_not_nulls_as_desktop_writes_them() {
    let c = Culture::en_us();
    let columns = vec![
        ("Name".to_string(), Some(EnteredType::Text)),
        ("Qty".to_string(), Some(EnteredType::Whole)),
        ("Price".to_string(), Some(EnteredType::Number)),
    ];
    let rows = vec![
        vec![Cell::Text("alpha".into()), Cell::Number(1.0), Cell::Number(2.5)],
        vec![Cell::Text(String::new()), Cell::Number(3.0), Cell::Null],
        vec![Cell::Null, Cell::Null],
    ];
    let e = entered::build(&columns, &rows, c, c).unwrap();
    assert_eq!(
        packed_rows(&e.m),
        r#"[["alpha","1","2.5"],["","3",""],["","",""]]"#
    );
    // an empty cell is an empty text in a text column (Desktop has no null text there) and
    // null in a typed one, as Desktop loads its own rows
    assert_eq!(e.values[0][2], CellValue::Text(String::new()));
    assert_eq!(e.values[1][2], CellValue::Null);
    assert_eq!(e.values[2][1], CellValue::Null);
    let o = SourceOptions {
        culture: Some("en-US".into()),
        ..SourceOptions::default()
    };
    let d = super::load_m_with(&e.m, &o).unwrap();
    let loaded: Vec<Vec<CellValue>> = d.columns().iter().map(|c| c.values.clone()).collect();
    assert_eq!(loaded, e.values);
    let back = entered::read(&e.m, &o).unwrap();
    assert_eq!(back.values, e.values);
}

#[test]
fn hostile_entered_tables_are_errors() {
    let c = Culture::en_us();
    let col = |n: &str| (n.to_string(), Some(EnteredType::Whole));
    assert!(entered::build(&[], &[], c, c).is_err());
    assert!(entered::build(&[col("A"), col("a")], &[], c, c).is_err());
    assert!(entered::build(&[col(" ")], &[], c, c).is_err());
    assert!(entered::build(&[col("A")], &cells(&[&["x"]]), c, c).is_err());
    assert!(entered::build(&[col("A")], &[vec![Cell::Number(1.5)]], c, c).is_err());
    assert!(entered::build(&[col("A")], &[vec![Cell::Number(f64::NAN)]], c, c).is_err());
    assert!(entered::build(&[col("A")], &cells(&[&["1", "2"]]), c, c).is_err());
    let wide: Vec<_> = (0..=entered::MAX_COLUMNS)
        .map(|i| col(&format!("C{i}")))
        .collect();
    assert!(entered::build(&wide, &[], c, c).is_err());
    let many = vec![vec![Cell::Null]; entered::MAX_CELLS + 1];
    assert!(entered::build(&[col("A")], &many, c, c).is_err());
}
