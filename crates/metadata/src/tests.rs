//! Unit tests over synthetic, hand-written metadata rows (all names invented).

#![allow(clippy::indexing_slicing)]

use bicraft_model::DataType;
use serde_json::json;

use super::*;

fn row(cells: &[(&str, MetaValue)]) -> MetaRow {
    cells.iter().map(|(k, v)| ((*k).to_string(), v.clone())).collect()
}

fn i(v: i64) -> MetaValue {
    MetaValue::Integer(v)
}

fn s(v: &str) -> MetaValue {
    MetaValue::Text(v.to_string())
}

/// A small but complete model: two user tables, one system table, every column type, a
/// relationship, partitions, a shared expression, a role, a hierarchy, a perspective, a culture.
fn sample() -> MetadataTables {
    let mut t = MetadataTables::new();
    t.push_row(
        "Model",
        row(&[
            ("ID", i(1)),
            ("Name", s("Model")),
            ("Culture", s("en-US")),
            ("DefaultMode", i(0)),
            ("DefaultPowerBIDataSourceVersion", i(2)),
            ("DataAccessOptions", s(r#"{"legacyRedirects": true}"#)),
            ("SourceQueryCulture", s("en-US")),
            ("ValueFilterBehavior", i(1)),
            ("DataSourceDefaultMaxConnections", i(10)),
            ("MaxParallelismPerRefresh", i(-1)),
            ("DiscourageImplicitMeasures", i(1)),
        ]),
    );
    // Inserted out of ID order on purpose.
    t.push_row(
        "Table",
        row(&[
            ("ID", i(20)),
            ("ModelID", i(1)),
            ("Name", s("Sales")),
            ("SystemFlags", i(0)),
            ("IsHidden", i(0)),
            ("LineageTag", s("lt-sales")),
        ]),
    );
    t.push_row(
        "Table",
        row(&[
            ("ID", i(10)),
            ("ModelID", i(1)),
            ("Name", s("Product")),
            ("SystemFlags", i(2)),
            ("IsHidden", i(1)),
            ("DataCategory", s("Time")),
            ("Description", s("Products")),
        ]),
    );
    t.push_row(
        "Table",
        row(&[
            ("ID", i(900)),
            ("ModelID", i(0)),
            ("Name", s("H$Product$Id")),
            ("SystemFlags", i(1)),
        ]),
    );
    let col = |id: i64, table: i64, extra: &[(&str, MetaValue)]| {
        let mut r = row(&[("ID", i(id)), ("TableID", i(table))]);
        r.extend(row(extra));
        r
    };
    t.insert_table(
        "Column",
        vec![
            col(
                11,
                10,
                &[
                    ("ExplicitName", s("RowNumber-1")),
                    ("Type", i(3)),
                    ("ExplicitDataType", i(6)),
                    ("IsHidden", i(1)),
                    ("IsUnique", i(1)),
                    ("IsKey", i(1)),
                    ("IsNullable", i(0)),
                    ("SummarizeBy", i(1)),
                ],
            ),
            col(
                12,
                10,
                &[
                    ("InferredName", s("Id")),
                    ("Type", i(4)),
                    ("ExplicitDataType", i(1)),
                    ("InferredDataType", i(6)),
                    ("SourceColumn", s("[Id]")),
                    ("SummarizeBy", i(2)),
                    ("IsAvailableInMDX", i(0)),
                    ("LineageTag", s("lt-id")),
                ],
            ),
            col(
                13,
                10,
                &[
                    ("ExplicitName", s("Label")),
                    ("Type", i(4)),
                    ("ExplicitDataType", i(2)),
                    ("InferredDataType", i(2)),
                    ("SourceColumn", s("[Label]")),
                    ("SortByColumnID", i(12)),
                ],
            ),
            col(
                21,
                20,
                &[
                    ("ExplicitName", s("Amount")),
                    ("Type", i(1)),
                    ("ExplicitDataType", i(10)),
                    ("SourceColumn", s("amount")),
                    ("FormatString", s("0.00")),
                    ("SummarizeBy", i(3)),
                    ("DisplayFolder", s("Money")),
                    ("Alignment", i(1)),
                    ("TableDetailPosition", i(-1)),
                ],
            ),
            col(
                22,
                20,
                &[
                    ("ExplicitName", s("Double")),
                    ("Type", i(2)),
                    ("ExplicitDataType", i(8)),
                    ("Expression", s("[Amount] * 2")),
                    ("Alignment", i(3)),
                    ("EncodingHint", i(2)),
                ],
            ),
            col(
                23,
                20,
                &[
                    ("ExplicitName", s("ProductId")),
                    ("Type", i(1)),
                    ("ExplicitDataType", i(6)),
                ],
            ),
            // Column of the system table: never surfaces.
            col(901, 900, &[("ExplicitName", s("POS_TO_ID")), ("Type", i(1))]),
        ],
    );
    t.insert_table(
        "Measure",
        vec![
            row(&[
                ("ID", i(31)),
                ("TableID", i(20)),
                ("Name", s("Total")),
                ("Expression", s("SUM(Sales[Amount])")),
                ("FormatString", s("#,0")),
                ("KPIID", i(0)),
                ("FormatStringDefinitionID", i(41)),
            ]),
            row(&[
                ("ID", i(30)),
                ("TableID", i(20)),
                ("Name", s("Count")),
                ("Expression", s("COUNTROWS(Sales)")),
                ("IsHidden", i(1)),
                ("KPIID", i(50)),
            ]),
        ],
    );
    t.push_row(
        "FormatStringDefinition",
        row(&[
            ("ID", i(41)),
            ("ObjectID", i(31)),
            ("ObjectType", i(8)),
            ("Expression", s("\"0\"")),
        ]),
    );
    t.push_row(
        "KPI",
        row(&[
            ("ID", i(50)),
            ("MeasureID", i(30)),
            ("TargetExpression", s("100")),
        ]),
    );
    t.insert_table(
        "Partition",
        vec![
            row(&[
                ("ID", i(61)),
                ("TableID", i(20)),
                ("Name", s("Sales")),
                ("Type", i(4)),
                ("Mode", i(0)),
                ("DataView", i(3)),
                ("QueryDefinition", s("let\n    Source = 1\nin\n    Source")),
                ("QueryGroupID", i(80)),
            ]),
            row(&[
                ("ID", i(60)),
                ("TableID", i(10)),
                ("Name", s("Product")),
                ("Type", i(2)),
                ("Mode", i(2)),
                ("QueryDefinition", s("DATATABLE(\"Id\", INTEGER, {{1}})")),
            ]),
            row(&[
                ("ID", i(902)),
                ("TableID", i(900)),
                ("Name", s("sys")),
                ("Type", i(3)),
            ]),
        ],
    );
    t.push_row(
        "Relationship",
        row(&[
            ("ID", i(70)),
            ("ModelID", i(1)),
            ("Name", s("rel-1")),
            ("IsActive", i(0)),
            ("CrossFilteringBehavior", i(2)),
            ("JoinOnDateBehavior", i(1)),
            ("SecurityFilteringBehavior", i(1)),
            ("FromTableID", i(20)),
            ("FromColumnID", i(23)),
            ("FromCardinality", i(2)),
            ("ToTableID", i(10)),
            ("ToColumnID", i(12)),
            ("ToCardinality", i(1)),
        ]),
    );
    t.push_row(
        "QueryGroup",
        row(&[("ID", i(80)), ("ModelID", i(1)), ("Folder", s("Facts"))]),
    );
    t.push_row(
        "Expression",
        row(&[
            ("ID", i(90)),
            ("ModelID", i(1)),
            ("Name", s("Server")),
            ("Kind", i(0)),
            ("Expression", s("\"localhost\" meta [IsParameterQuery=true]")),
            ("QueryGroupID", i(80)),
            ("LineageTag", s("lt-server")),
        ]),
    );
    t.push_row(
        "ExtendedProperty",
        row(&[
            ("ID", i(91)),
            ("ObjectID", i(90)),
            ("ObjectType", i(41)),
            ("Name", s("ParameterMetadata")),
            ("Type", i(1)),
            ("Value", s(r#"{"version":0}"#)),
        ]),
    );
    t.push_row(
        "Role",
        row(&[
            ("ID", i(100)),
            ("ModelID", i(1)),
            ("Name", s("Readers")),
            ("ModelPermission", i(2)),
        ]),
    );
    t.push_row(
        "TablePermission",
        row(&[
            ("ID", i(101)),
            ("RoleID", i(100)),
            ("TableID", i(20)),
            ("FilterExpression", s("Sales[Amount] > 0")),
            ("MetadataPermission", i(0)),
        ]),
    );
    t.push_row(
        "RoleMembership",
        row(&[
            ("ID", i(102)),
            ("RoleID", i(100)),
            ("MemberName", s("someone")),
            ("MemberType", i(2)),
        ]),
    );
    t.push_row(
        "Hierarchy",
        row(&[
            ("ID", i(110)),
            ("TableID", i(10)),
            ("Name", s("H")),
            ("LineageTag", s("lt-h")),
        ]),
    );
    t.insert_table(
        "Level",
        vec![
            row(&[
                ("ID", i(111)),
                ("HierarchyID", i(110)),
                ("Ordinal", i(1)),
                ("Name", s("L1")),
                ("ColumnID", i(13)),
            ]),
            row(&[
                ("ID", i(112)),
                ("HierarchyID", i(110)),
                ("Ordinal", i(0)),
                ("Name", s("L0")),
                ("ColumnID", i(12)),
            ]),
        ],
    );
    t.push_row(
        "Culture",
        row(&[("ID", i(120)), ("ModelID", i(1)), ("Name", s("en-US"))]),
    );
    t.push_row(
        "Perspective",
        row(&[("ID", i(130)), ("ModelID", i(1)), ("Name", s("P"))]),
    );
    t.push_row(
        "PerspectiveTable",
        row(&[
            ("ID", i(131)),
            ("PerspectiveID", i(130)),
            ("TableID", i(20)),
            ("IncludeAll", i(0)),
        ]),
    );
    t.push_row(
        "PerspectiveMeasure",
        row(&[
            ("ID", i(132)),
            ("PerspectiveTableID", i(131)),
            ("MeasureID", i(31)),
        ]),
    );
    let ann = |id: i64, ty: i64, obj: i64, name: &str, value: &str| {
        row(&[
            ("ID", i(id)),
            ("ObjectType", i(ty)),
            ("ObjectID", i(obj)),
            ("Name", s(name)),
            ("Value", s(value)),
        ])
    };
    t.insert_table(
        "Annotation",
        vec![
            ann(141, 1, 1, "B", "2"),
            ann(140, 1, 1, "A", "1"),
            ann(142, 3, 20, "T", "t"),
            ann(143, 4, 12, "SummarizationSetBy", "User"),
            ann(144, 6, 61, "P", "p"),
            ann(145, 8, 31, "M", "m"),
            ann(146, 7, 70, "R", "r"),
            ann(147, 4, 9999, "Dangling", "x"),
        ],
    );
    t
}

#[test]
fn maps_a_complete_model() {
    let m = build_model(&sample()).unwrap();
    assert_eq!(m.name, "");
    assert_eq!(m.compatibility_level, None);
    assert_eq!(m.culture.as_deref(), Some("en-US"));
    // Annotations in ID order.
    let names: Vec<_> = m.annotations.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["A", "B"]);
    assert_eq!(
        serde_json::Value::Object(m.extra.clone()),
        json!({
            "name": "Model",
            "dataAccessOptions": {"legacyRedirects": true},
            "discourageImplicitMeasures": true,
            "defaultPowerBIDataSourceVersion": "powerBI_V3",
            "sourceQueryCulture": "en-US",
            "valueFilterBehavior": "independent",
            "cultures": [{"name": "en-US"}],
            "perspectives": [{"name": "P", "tables": [{"name": "Sales", "measures": [{"name": "Total"}]}]}],
            "queryGroups": [{"folder": "Facts"}],
        })
    );

    // Tables in ID order; the system table is gone.
    let tables: Vec<_> = m.tables.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(tables, ["Product", "Sales"]);

    let product = &m.tables[0];
    assert!(product.is_hidden);
    assert_eq!(product.description.as_deref(), Some("Products"));
    assert_eq!(product.extra.get("dataCategory"), Some(&json!("Time")));
    assert_eq!(
        product.extra.get("hierarchies"),
        Some(&json!([{
            "name": "H",
            "lineageTag": "lt-h",
            "levels": [
                {"name": "L0", "ordinal": 0, "column": "Id"},
                {"name": "L1", "ordinal": 1, "column": "Label"},
            ],
        }]))
    );
    let row_number = &product.columns[0];
    assert!(row_number.is_row_number());
    assert_eq!(row_number.data_type, DataType::Int64);
    assert!(row_number.is_hidden);
    assert_eq!(row_number.summarize_by, None);
    assert_eq!(
        serde_json::Value::Object(row_number.extra.clone()),
        json!({"type": "rowNumber", "isUnique": true, "isKey": true, "isNullable": false})
    );
    let id = &product.columns[1];
    assert_eq!(id.name, "Id");
    assert_eq!(id.data_type, DataType::Int64);
    assert_eq!(id.source_column.as_deref(), Some("[Id]"));
    assert_eq!(id.summarize_by.as_deref(), Some("none"));
    assert_eq!(id.lineage_tag.as_deref(), Some("lt-id"));
    assert_eq!(id.annotations.len(), 1);
    assert_eq!(
        serde_json::Value::Object(id.extra.clone()),
        json!({
            "type": "calculatedTableColumn",
            "isNameInferred": true,
            "isDataTypeInferred": true,
            "isAvailableInMdx": false,
        })
    );
    let label = &product.columns[2];
    assert_eq!(label.sort_by_column.as_deref(), Some("Id"));
    assert_eq!(label.extra.get("isNameInferred"), None);
    assert_eq!(product.partitions[0].source_type, "calculated");
    assert_eq!(
        product.partitions[0].mode, None,
        "mode `default` is TOM's default"
    );

    let sales = &m.tables[1];
    assert_eq!(sales.lineage_tag.as_deref(), Some("lt-sales"));
    assert_eq!(sales.annotations[0].name, "T");
    let amount = &sales.columns[0];
    assert_eq!(amount.data_type, DataType::Decimal);
    assert_eq!(amount.source_column.as_deref(), Some("amount"));
    assert_eq!(amount.format_string.as_deref(), Some("0.00"));
    assert_eq!(amount.display_folder.as_deref(), Some("Money"));
    assert_eq!(amount.summarize_by.as_deref(), Some("sum"));
    assert!(amount.extra.is_empty(), "{:?}", amount.extra);
    let double = &sales.columns[1];
    assert_eq!(double.expression.as_deref(), Some("[Amount] * 2"));
    assert_eq!(
        serde_json::Value::Object(double.extra.clone()),
        json!({"alignment": "right", "encodingHint": "value"})
    );
    // Measures in ID order.
    let measures: Vec<_> = sales.measures.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(measures, ["Count", "Total"]);
    assert!(sales.measures[0].is_hidden);
    assert_eq!(
        sales.measures[0].extra.get("kpi"),
        Some(&json!({"targetExpression": "100"}))
    );
    let total = &sales.measures[1];
    assert_eq!(total.expression, "SUM(Sales[Amount])");
    assert_eq!(total.format_string.as_deref(), Some("#,0"));
    assert_eq!(total.annotations[0].name, "M");
    assert_eq!(
        total.extra.get("formatStringDefinition"),
        Some(&json!({"expression": "\"0\""}))
    );
    let p = &sales.partitions[0];
    assert_eq!(p.name, "Sales");
    assert_eq!(p.source_type, "m");
    assert_eq!(p.mode.as_deref(), Some("import"));
    assert_eq!(
        p.expression.as_deref(),
        Some("let\n    Source = 1\nin\n    Source")
    );
    assert_eq!(
        serde_json::Value::Object(p.extra.clone()),
        json!({"queryGroup": "Facts", "annotations": [{"name": "P", "value": "p"}]})
    );

    let r = &m.relationships[0];
    assert_eq!(
        (
            r.from_table.as_str(),
            r.from_column.as_str(),
            r.to_table.as_str(),
            r.to_column.as_str()
        ),
        ("Sales", "ProductId", "Product", "Id")
    );
    assert!(!r.is_active);
    assert_eq!(r.cross_filtering_behavior.as_deref(), Some("bothDirections"));
    assert_eq!(r.from_cardinality, None);
    assert_eq!(r.to_cardinality, None);
    assert!(r.extra.is_empty());
    assert_eq!(r.annotations[0].name, "R");

    let e = &m.expressions[0];
    assert_eq!((e.name.as_str(), e.kind.as_str()), ("Server", "m"));
    assert_eq!(e.lineage_tag.as_deref(), Some("lt-server"));
    assert_eq!(
        serde_json::Value::Object(e.extra.clone()),
        json!({
            "queryGroup": "Facts",
            "extendedProperties": [{"type": "json", "name": "ParameterMetadata", "value": {"version": 0}}],
        })
    );

    let role = &m.roles[0];
    assert_eq!(role.model_permission.as_deref(), Some("read"));
    assert_eq!(role.table_permissions[0].table, "Sales");
    assert_eq!(
        role.table_permissions[0].filter_expression.as_deref(),
        Some("Sales[Amount] > 0")
    );
    assert_eq!(
        role.extra.get("members"),
        Some(&json!([{"memberName": "someone", "memberType": "user"}]))
    );
}

#[test]
fn model_survives_a_serde_round_trip() {
    let m = build_model(&sample()).unwrap();
    let text = serde_json::to_string(&m).unwrap();
    assert_eq!(
        serde_json::from_str::<bicraft_model::SemanticModel>(&text).unwrap(),
        m
    );
}

#[test]
fn missing_model_is_an_error() {
    assert!(matches!(build_model(&MetadataTables::new()), Err(Error::NoModel)));
    let mut t = sample();
    t.insert_table("Model", Vec::new());
    assert!(matches!(build_model(&t), Err(Error::NoModel)));
}

#[test]
fn a_model_row_alone_gives_an_empty_model() {
    let mut t = MetadataTables::new();
    t.push_row("Model", row(&[("ID", i(1))]));
    let m = build_model(&t).unwrap();
    assert!(m.tables.is_empty() && m.relationships.is_empty() && m.extra.is_empty());
}

#[test]
fn table_and_column_names_are_case_insensitive() {
    let mut t = MetadataTables::new();
    t.push_row("model", row(&[("id", i(1))]));
    t.push_row("TABLE", row(&[("Id", i(2)), ("modelid", i(1)), ("NAME", s("x"))]));
    let m = build_model(&t).unwrap();
    assert_eq!(m.tables[0].name, "x");
}

#[test]
fn dangling_references_are_dropped() {
    let mut t = sample();
    // A relationship to a column that does not exist, and one to a system-table column.
    t.push_row(
        "Relationship",
        row(&[
            ("ID", i(71)),
            ("ModelID", i(1)),
            ("FromColumnID", i(424242)),
            ("ToColumnID", i(12)),
        ]),
    );
    t.push_row(
        "Relationship",
        row(&[
            ("ID", i(72)),
            ("ModelID", i(1)),
            ("FromColumnID", i(901)),
            ("ToColumnID", i(12)),
        ]),
    );
    // Children of tables that do not exist.
    t.push_row(
        "Column",
        row(&[("ID", i(500)), ("TableID", i(777)), ("ExplicitName", s("orphan"))]),
    );
    t.push_row(
        "Measure",
        row(&[("ID", i(501)), ("TableID", i(777)), ("Name", s("orphan"))]),
    );
    // References that point nowhere.
    t.push_row(
        "Column",
        row(&[
            ("ID", i(502)),
            ("TableID", i(20)),
            ("ExplicitName", s("Sorted")),
            ("SortByColumnID", i(999_999)),
        ]),
    );
    t.push_row(
        "Measure",
        row(&[
            ("ID", i(503)),
            ("TableID", i(20)),
            ("Name", s("M2")),
            ("KPIID", i(999)),
            ("DetailRowsDefinitionID", i(998)),
        ]),
    );
    t.push_row(
        "TablePermission",
        row(&[("ID", i(504)), ("RoleID", i(100)), ("TableID", i(777))]),
    );
    t.push_row(
        "Level",
        row(&[("ID", i(505)), ("HierarchyID", i(110)), ("ColumnID", i(424242))]),
    );
    let m = build_model(&t).unwrap();
    assert_eq!(m.relationships.len(), 1);
    let sales = m.table("Sales").unwrap();
    assert_eq!(sales.column("Sorted").unwrap().sort_by_column, None);
    assert!(sales.measure("M2").unwrap().extra.is_empty());
    assert!(
        m.tables
            .iter()
            .all(|t| t.column("orphan").is_none() && t.measure("orphan").is_none())
    );
    assert_eq!(m.roles[0].table_permissions.len(), 1);
}

#[test]
fn wrong_types_read_as_absent_without_panicking() {
    let weird = [
        MetaValue::Null,
        MetaValue::Blob(vec![0xff, 0x00, 0x80]),
        MetaValue::Real(f64::NAN),
        MetaValue::Real(f64::INFINITY),
        MetaValue::Real(1e300),
        MetaValue::Real(-1e300),
        MetaValue::Integer(i64::MIN),
        MetaValue::Integer(i64::MAX),
        MetaValue::Text("not a number".into()),
        MetaValue::Text(String::new()),
    ];
    for v in &weird {
        let mut t = sample();
        for table in TABLES {
            let rows: Vec<MetaRow> = t
                .rows(table)
                .iter()
                .map(|r| r.keys().map(|k| (k.clone(), v.clone())).collect())
                .collect();
            if !rows.is_empty() {
                t.insert_table(*table, rows);
            }
        }
        // Every cell of every row now holds the same odd value: must not panic.
        let _ = build_model(&t);
        // And with only some columns broken.
        let mut t = sample();
        t.push_row(
            "Column",
            row(&[("ID", v.clone()), ("TableID", i(20)), ("Type", v.clone())]),
        );
        t.push_row(
            "Annotation",
            row(&[
                ("ObjectType", v.clone()),
                ("ObjectID", v.clone()),
                ("Name", v.clone()),
            ]),
        );
        assert!(build_model(&t).is_ok());
    }
}

#[test]
fn integral_reals_and_numeric_text_are_accepted_as_integers() {
    let mut t = MetadataTables::new();
    t.push_row("Model", row(&[("ID", MetaValue::Real(1.0))]));
    t.push_row(
        "Table",
        row(&[
            ("ID", s(" 5 ")),
            ("ModelID", i(1)),
            ("Name", s("T")),
            ("IsHidden", MetaValue::Real(1.0)),
        ]),
    );
    let m = build_model(&t).unwrap();
    assert!(m.tables[0].is_hidden);
}

#[test]
fn unknown_enum_codes_are_left_out() {
    let mut t = MetadataTables::new();
    t.push_row("Model", row(&[("ID", i(1)), ("ValueFilterBehavior", i(77))]));
    t.push_row("Table", row(&[("ID", i(2)), ("ModelID", i(1)), ("Name", s("T"))]));
    t.push_row(
        "Column",
        row(&[
            ("ID", i(3)),
            ("TableID", i(2)),
            ("ExplicitName", s("C")),
            ("Type", i(42)),
            ("ExplicitDataType", i(99)),
            ("SummarizeBy", i(99)),
        ]),
    );
    t.push_row(
        "Partition",
        row(&[
            ("ID", i(4)),
            ("TableID", i(2)),
            ("Name", s("P")),
            ("Type", i(99)),
            ("Mode", i(99)),
        ]),
    );
    let m = build_model(&t).unwrap();
    assert!(m.extra.is_empty());
    let c = &m.tables[0].columns[0];
    assert_eq!(c.data_type, DataType::Unknown);
    assert_eq!(c.summarize_by, None);
    assert!(c.extra.is_empty());
    let p = &m.tables[0].partitions[0];
    assert_eq!((p.source_type.as_str(), p.mode.as_deref()), ("", None));
}

#[test]
fn garbage_bytes_are_an_sqlite_error() {
    assert!(matches!(read_sqlitedb(b""), Err(Error::Sqlite(_))));
    assert!(matches!(read_sqlitedb(&[0u8; 4096]), Err(Error::Sqlite(_))));
}

#[test]
fn column_variations_are_mapped() {
    let mut t = MetadataTables::new();
    t.push_row("Model", row(&[("ID", i(1)), ("Name", s("Model"))]));
    for (id, name) in [(10, "Sales"), (20, "LocalDateTable_x")] {
        t.push_row(
            "Table",
            row(&[
                ("ID", i(id)),
                ("ModelID", i(1)),
                ("Name", s(name)),
                ("SystemFlags", i(0)),
            ]),
        );
    }
    for (id, table, name, ty) in [
        (11, 10, "OrderDate", 9),
        (12, 10, "Amount", 10),
        (21, 20, "Date", 9),
        (22, 20, "Year", 6),
    ] {
        t.push_row(
            "Column",
            row(&[
                ("ID", i(id)),
                ("TableID", i(table)),
                ("ExplicitName", s(name)),
                ("Type", i(1)),
                ("ExplicitDataType", i(ty)),
            ]),
        );
    }
    t.push_row(
        "Hierarchy",
        row(&[("ID", i(30)), ("TableID", i(20)), ("Name", s("Date Hierarchy"))]),
    );
    t.push_row(
        "Relationship",
        row(&[
            ("ID", i(40)),
            ("ModelID", i(1)),
            ("Name", s("rel-guid")),
            ("FromTableID", i(10)),
            ("FromColumnID", i(11)),
            ("ToTableID", i(20)),
            ("ToColumnID", i(21)),
        ]),
    );
    t.push_row(
        "Variation",
        row(&[
            ("ID", i(50)),
            ("ColumnID", i(11)),
            ("Name", s("Variation")),
            ("RelationshipID", i(40)),
            ("DefaultHierarchyID", i(30)),
            ("DefaultColumnID", i(0)),
            ("IsDefault", i(1)),
        ]),
    );
    let m = build_model(&t).unwrap();
    let sales = m.table("Sales").unwrap();
    let date = sales.column("OrderDate").unwrap();
    assert_eq!(
        date.extra["variations"],
        json!([{
            "name": "Variation",
            "relationship": "rel-guid",
            "defaultHierarchy": {"table": "LocalDateTable_x", "hierarchy": "Date Hierarchy"},
            "isDefault": true
        }])
    );
    assert!(sales.column("Amount").unwrap().extra.get("variations").is_none());
    let (t, c) = m.variation_column("Sales", "orderdate", "YEAR").unwrap();
    assert_eq!((t.name.as_str(), c.name.as_str()), ("LocalDateTable_x", "Year"));
    assert!(m.variation_column("Sales", "Amount", "Year").is_none());
}
