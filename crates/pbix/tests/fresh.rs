//! A `DataModel` built from scratch (M5): every table, column, relationship, measure and role of
//! a model that never had a `DataModel` written into a new backup, then read back by bicraft's
//! own reader (model and values). Synthetic models only.

use bicraft_model::{
    Column, DataType, Measure, Partition, Relationship, Role, SemanticModel, Table, TablePermission,
};
use bicraft_pbix::structural::{ColumnValues, DataModelEditor, StructuralData, TableValues};
use bicraft_vertipaq::CellValue;
use serde_json::json;

const NOW: i64 = 134_364_379_681_883_581;

fn column(name: &str, data_type: DataType) -> Column {
    Column {
        name: name.into(),
        data_type,
        source_column: Some(name.into()),
        ..Column::default()
    }
}

fn m_table(name: &str, columns: Vec<Column>, m: &str) -> Table {
    Table {
        name: name.into(),
        columns,
        partitions: vec![Partition {
            name: name.into(),
            source_type: "m".into(),
            expression: Some(m.into()),
            mode: Some("import".into()),
            ..Partition::default()
        }],
        ..Table::default()
    }
}

fn model() -> SemanticModel {
    let mut m = SemanticModel {
        name: "Fresh".into(),
        compatibility_level: Some(1606),
        culture: Some("en-US".into()),
        ..SemanticModel::default()
    };
    m.extra
        .insert("defaultPowerBIDataSourceVersion".into(), json!("powerBI_V3"));
    m.extra.insert("sourceQueryCulture".into(), json!("en-US"));
    let mut products = m_table(
        "Product",
        vec![column("Key", DataType::Int64), column("Name", DataType::String)],
        "let Source = #table(type table [Key = Int64.Type, Name = text], {{1, \"a\"}, {2, \"b\"}}) in Source",
    );
    products.measures.push(Measure {
        name: "Products".into(),
        expression: "COUNTROWS(Product)".into(),
        ..Measure::default()
    });
    let sales = m_table(
        "Sales",
        vec![
            column("Key", DataType::Int64),
            column("Amount", DataType::Double),
            column("When", DataType::DateTime),
            column("Ok", DataType::Boolean),
        ],
        "let Source = #table(type table [Key = Int64.Type, Amount = number, When = datetime, Ok = logical], {}) in Source",
    );
    m.tables = vec![products, sales];
    m.relationships.push(Relationship {
        name: "8f0c7a4e-2b1d-4c3e-9a5f-6d7e8f9a0b1c".into(),
        from_table: "Sales".into(),
        from_column: "Key".into(),
        to_table: "Product".into(),
        to_column: "Key".into(),
        is_active: true,
        ..Relationship::default()
    });
    m.roles.push(Role {
        name: "Only a".into(),
        model_permission: Some("read".into()),
        table_permissions: vec![TablePermission {
            table: "Product".into(),
            filter_expression: Some("[Name] = \"a\"".into()),
            ..TablePermission::default()
        }],
        ..Role::default()
    });
    m
}

fn values(name: &str, v: Vec<CellValue>) -> ColumnValues {
    ColumnValues {
        name: name.into(),
        values: v,
    }
}

fn data() -> StructuralData {
    use CellValue::*;
    StructuralData {
        tables: vec![
            TableValues {
                name: "Product".into(),
                columns: vec![
                    values("Key", vec![Int(1), Int(2)]),
                    values("Name", vec![Text("a".into()), Text("b".into())]),
                ],
            },
            TableValues {
                name: "Sales".into(),
                columns: vec![
                    values("Key", vec![Int(1), Int(2), Int(2), Null]),
                    values("Amount", vec![Real(1.5), Real(-2.0), Null, Real(10.25)]),
                    values(
                        "When",
                        vec![DateTime(45000.5), Null, DateTime(45001.0), DateTime(1.0)],
                    ),
                    values("Ok", vec![Bool(true), Bool(false), Null, Bool(true)]),
                ],
            },
        ],
        columns: Vec::new(),
    }
}

#[test]
fn a_model_built_from_scratch_reads_back() {
    let m = model();
    let part = bicraft_pbix::fresh::build_data_model_at(&m, &data(), NOW).unwrap();
    let editor = DataModelEditor::open(&part, &bicraft_pbix::DataModelLimits::default()).unwrap();
    let back = editor.base();
    let names: Vec<&str> = back.tables.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["Product", "Sales"]);
    assert_eq!(back.relationships.len(), 1);
    assert_eq!(back.roles.len(), 1);
    assert_eq!(
        back.roles[0].table_permissions[0].filter_expression,
        m.roles[0].table_permissions[0].filter_expression
    );
    assert_eq!(back.tables[0].measures[0].expression, "COUNTROWS(Product)");
    assert_eq!(
        back.tables[1].partitions[0].expression,
        m.tables[1].partitions[0].expression
    );
    assert_eq!(back.extra["defaultPowerBIDataSourceVersion"], "powerBI_V3");
    // Only the engine's row-number columns differ (the reader returns them, models leave them out).
    let ch = editor.changes(&m);
    assert!(ch.added_tables.is_empty() && ch.added_columns.is_empty() && ch.added_relationships.is_empty());
    assert!(
        ch.removed_columns
            .iter()
            .all(|(_, c)| c.starts_with("RowNumber-")),
        "{ch:?}"
    );
    for t in &data().tables {
        assert_eq!(
            editor.row_count(&t.name).unwrap(),
            Some(t.columns[0].values.len() as u64)
        );
        for c in &t.columns {
            assert_eq!(
                editor.column_values(&t.name, &c.name).unwrap(),
                c.values,
                "{}[{}]",
                t.name,
                c.name
            );
        }
    }
    // A later metadata edit works like on any PBIX.
    let mut edited = back.clone();
    edited.tables[1].measures.push(Measure {
        name: "Total".into(),
        expression: "SUM(Sales[Amount])".into(),
        ..Measure::default()
    });
    let again = editor
        .write_at(&edited, &StructuralData::default(), Some(NOW + 1))
        .unwrap();
    let reread = bicraft_pbix::read_data_model(&again).unwrap();
    assert_eq!(reread.tables[1].measures.len(), 1);
}

#[test]
fn reproducible() {
    let a = bicraft_pbix::fresh::build_data_model_at(&model(), &data(), NOW).unwrap();
    let b = bicraft_pbix::fresh::build_data_model_at(&model(), &data(), NOW).unwrap();
    assert_eq!(a, b);
}

#[test]
fn empty_model() {
    let m = SemanticModel::default();
    let part = bicraft_pbix::fresh::build_data_model_at(&m, &StructuralData::default(), NOW).unwrap();
    let back = bicraft_pbix::read_data_model(&part).unwrap();
    assert!(back.tables.is_empty());
}

#[test]
fn missing_values_are_refused() {
    let err =
        bicraft_pbix::fresh::build_data_model_at(&model(), &StructuralData::default(), NOW).unwrap_err();
    assert!(err.to_string().contains("Product"), "{err}");
}

/// `SegmentMapStorage` rows of a `DataModel` part: (`RecordCount`, `SegmentCount`, `Type`).
fn segment_maps(part: &[u8]) -> Vec<(i64, i64, i64)> {
    let abf = bicraft_xpress9::decompress_datamodel(part, 1 << 30).unwrap();
    let abf = bicraft_abf::Abf::parse(&abf).unwrap();
    let db = bicraft_sqlite::Database::from_bytes(abf.metadata_sqlitedb().unwrap()).unwrap();
    let mut maps: Vec<(i64, i64, i64)> = db
        .query_all("SegmentMapStorage")
        .unwrap()
        .iter()
        .map(|r| {
            let v = |k: &str| r[k].as_i64().unwrap();
            (v("RecordCount"), v("SegmentCount"), v("Type"))
        })
        .collect();
    maps.sort_unstable();
    maps
}

/// G50: a user partition of several 2^20-row segments has `SegmentMapStorage.Type` 1, as Desktop
/// writes it; Desktop's consistency check refused type 3 there (Power BI Desktop 2.160.1151.0,
/// 1,048,577 and 10,000,000 rows). One segment keeps type 3, the `H$` table keeps type 2.
#[test]
fn a_partition_of_several_segments_has_segment_map_type_1() {
    let rows: i64 = (1 << 20) + 1;
    let mut m = SemanticModel {
        name: "Big".into(),
        compatibility_level: Some(1606),
        culture: Some("en-US".into()),
        ..SemanticModel::default()
    };
    m.tables = vec![
        m_table(
            "Small",
            vec![column("qty", DataType::Int64)],
            "let Source = 1 in Source",
        ),
        m_table(
            "Big",
            vec![column("qty", DataType::Int64)],
            "let Source = 1 in Source",
        ),
    ];
    let data = StructuralData {
        tables: vec![
            TableValues {
                name: "Small".into(),
                columns: vec![values("qty", (0..3).map(CellValue::Int).collect())],
            },
            TableValues {
                name: "Big".into(),
                columns: vec![values(
                    "qty",
                    (0..rows).map(|i| CellValue::Int(1 + i % 9)).collect(),
                )],
            },
        ],
        columns: Vec::new(),
    };
    let part = bicraft_pbix::fresh::build_data_model_at(&m, &data, NOW).unwrap();
    let maps = segment_maps(&part);
    assert!(maps.contains(&(rows, 2, 1)), "{maps:?}");
    assert!(maps.contains(&(3, 1, 3)), "{maps:?}");
    // The H$ table of Big[qty]: 9 values + 3 records, type 2.
    assert!(maps.contains(&(12, 2, 2)), "{maps:?}");
    assert!(
        maps.iter().all(|&(_, segments, kind)| kind != 3 || segments == 1),
        "{maps:?}"
    );
    let editor = DataModelEditor::open(&part, &bicraft_pbix::DataModelLimits::default()).unwrap();
    assert_eq!(editor.row_count("Big").unwrap(), Some(rows as u64));
}
