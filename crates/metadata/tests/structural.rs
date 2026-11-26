//! Structural edits over synthetic metadata: new tables, columns and relationships with their
//! storage rows and files, then removals that cascade back to nothing. All data is invented.

use bicraft_metadata::{
    AttributeHierarchyData, ColumnStatistics, ColumnStorageData, DictionaryData, EncodedData,
    EncodedNewColumn, EncodedRelationship, EncodedTable, Error, MetaRow, MetaValue, MetadataTables,
    PartitionSegments, ROW_NUMBER_NAME, SegmentMap, StructuralEdit, SystemColumnData, SystemTableData,
    apply_structural_at, build_model, dangling_references, read_sqlitedb, storage_file_paths,
    tables_from_sqlitedb_all, write_structural_sqlitedb_at,
};
use bicraft_model::{Column, DataType, Measure, Partition, Relationship, SemanticModel, Table};
use bicraft_sqlite::{DatabaseBuilder, Value};
use serde_json::json;

/// 2026-01-01T00:00:00Z as FILETIME ticks.
const NOW: i64 = 134_116_992_000_000_000;
const LATER: i64 = NOW + 10_000_000;

// --- synthetic encoded data ----------------------------------------------------------------------

fn stats(rows: i64, distinct: i64, db_type: i64) -> ColumnStatistics {
    ColumnStatistics {
        distinct_states: distinct,
        min_data_id: 3,
        max_data_id: 2 + distinct,
        original_min_segment_data_id: 2,
        rle_sort_order: -1,
        row_count: rows,
        has_nulls: false,
        rle_runs: 0,
        others_rle_runs: 1,
        db_type,
        xm_type: if db_type == 130 { 2 } else { 0 },
        compression_type: 0,
        compression_param: 0,
    }
}

fn system_table(kind: i64, distinct: i64, columns: &[&str], tag: &str) -> SystemTableData {
    SystemTableData {
        segment_map: SegmentMap {
            kind,
            record_count: distinct + 3,
            segment_count: if kind == 2 { 2 } else { 1 },
            records_per_segment: distinct,
        },
        columns: columns
            .iter()
            .map(|n| SystemColumnData {
                name: (*n).to_string(),
                idf: format!("{tag}-{n}-idf").into_bytes(),
                idfmeta: format!("{tag}-{n}-meta").into_bytes(),
                segment_count: if kind == 2 { 2 } else { 1 },
                settings: None,
            })
            .collect(),
    }
}

/// A hash-encoded column: `.dictionary`, `H$` with `POS_TO_ID` and `ID_TO_POS`.
fn hash_column(tag: &str, rows: i64, distinct: i64) -> ColumnStorageData {
    ColumnStorageData {
        idf: format!("{tag}-idf").into_bytes(),
        idfmeta: format!("{tag}-idfmeta").into_bytes(),
        segment_count: 1,
        dictionary: DictionaryData {
            file: Some(format!("{tag}-dictionary").into_bytes()),
            kind: 1,
            data_type: 2,
            base_id: 2,
            magnitude: 0.0,
            last_id: 2 + distinct,
            is_nullable: false,
            is_unique: false,
            is_operating_on_32: false,
            flags: 3,
            size: 17_000,
        },
        statistics: stats(rows, distinct, 130),
        hierarchy: Some(AttributeHierarchyData {
            materialization_type: 0,
            distinct_data_count: distinct,
            min_value: Some("a".into()),
            max_value: Some("z".into()),
            string_value_max_length: 5,
            hidx: None,
            system_table: Some(system_table(2, distinct, &["POS_TO_ID", "ID_TO_POS"], tag)),
        }),
    }
}

/// A value-encoded integer column: no dictionary file, `.hidx`, `H$` with `POS_TO_ID` only.
fn value_column(tag: &str, rows: i64, distinct: i64) -> ColumnStorageData {
    ColumnStorageData {
        idf: format!("{tag}-idf").into_bytes(),
        idfmeta: format!("{tag}-idfmeta").into_bytes(),
        segment_count: 1,
        dictionary: DictionaryData {
            file: None,
            kind: 2,
            data_type: 6,
            base_id: -2,
            magnitude: 1.0,
            last_id: 2,
            is_nullable: true,
            is_unique: false,
            is_operating_on_32: false,
            flags: 0,
            size: 144,
        },
        statistics: stats(rows, distinct, 20),
        hierarchy: Some(AttributeHierarchyData {
            materialization_type: 1,
            distinct_data_count: distinct,
            min_value: Some("1".into()),
            max_value: Some("9".into()),
            string_value_max_length: 0,
            hidx: Some(format!("{tag}-hidx").into_bytes()),
            system_table: Some(system_table(2, distinct, &["POS_TO_ID"], tag)),
        }),
    }
}

/// A column without an attribute hierarchy.
fn plain_column(tag: &str, rows: i64) -> ColumnStorageData {
    let mut c = value_column(tag, rows, rows);
    c.hierarchy = None;
    c
}

fn row_number(tag: &str, rows: i64) -> ColumnStorageData {
    let mut c = value_column(tag, rows, rows);
    c.dictionary.base_id = -3;
    c.hierarchy = Some(AttributeHierarchyData {
        materialization_type: 3,
        distinct_data_count: rows,
        min_value: Some("0".into()),
        max_value: Some((rows - 1).to_string()),
        string_value_max_length: 0,
        hidx: None,
        system_table: None,
    });
    c
}

// --- base metadata -------------------------------------------------------------------------------

fn row(cells: &[(&str, MetaValue)]) -> MetaRow {
    cells.iter().map(|(k, v)| ((*k).to_string(), v.clone())).collect()
}

fn base() -> MetadataTables {
    let mut t = MetadataTables::new();
    t.insert_table(
        "Model",
        vec![row(&[
            ("ID", 1.into()),
            ("Name", "Model".into()),
            ("Culture", "en-US".into()),
        ])],
    );
    t.insert_table(
        "DBPROPERTIES",
        vec![
            row(&[("NAME", "MAXID".into()), ("VALUE", 1.into())]),
            row(&[("NAME", "LASTCOMMITTED".into()), ("VALUE", 1.into())]),
        ],
    );
    t
}

fn calculated_table_column(name: &str, data_type: DataType, hidden: bool) -> Column {
    let mut extra = serde_json::Map::new();
    extra.insert("type".into(), json!("calculatedTableColumn"));
    Column {
        name: name.into(),
        data_type,
        source_column: Some(format!("[{name}]")),
        is_hidden: hidden,
        extra,
        ..Default::default()
    }
}

fn data_column(name: &str, data_type: DataType) -> Column {
    Column {
        name: name.into(),
        data_type,
        source_column: Some(name.into()),
        summarize_by: Some("none".into()),
        ..Default::default()
    }
}

/// Two new tables (an M one with data columns, a calculated one), a relationship, a measure.
fn grown(base_model: &SemanticModel) -> (SemanticModel, EncodedData) {
    let mut m = base_model.clone();
    let mut note = data_column("Note", DataType::String);
    note.extra.insert("isAvailableInMdx".into(), json!(false));
    m.tables.push(Table {
        name: "Sales".into(),
        description: Some("synthetic sales".into()),
        columns: vec![
            data_column("Region", DataType::String),
            note,
            data_column("Amount", DataType::Int64),
        ],
        measures: vec![Measure {
            name: "Total".into(),
            expression: "SUM(Sales[Amount])".into(),
            ..Default::default()
        }],
        partitions: vec![Partition {
            name: "Sales".into(),
            mode: Some("import".into()),
            source_type: "m".into(),
            expression: Some("let Source = 1 in Source".into()),
            ..Default::default()
        }],
        ..Default::default()
    });
    m.tables.push(Table {
        name: "Region_List".into(),
        columns: vec![calculated_table_column("Region", DataType::String, false)],
        partitions: vec![Partition {
            name: "Region_List".into(),
            mode: Some("import".into()),
            source_type: "calculated".into(),
            expression: Some("DATATABLE(\"Region\", STRING, {{\"a\"}, {\"b\"}})".into()),
            ..Default::default()
        }],
        lineage_tag: Some("aaaaaaaa-0000-4000-8000-000000000001".into()),
        ..Default::default()
    });
    m.relationships.push(Relationship {
        name: "0b5f0e0a-1111-4222-8333-444455556666".into(),
        from_table: "Sales".into(),
        from_column: "Region".into(),
        to_table: "Region_List".into(),
        to_column: "Region".into(),
        is_active: true,
        ..Default::default()
    });
    let data = EncodedData {
        tables: vec![
            EncodedTable {
                name: "Sales".into(),
                segment_map: SegmentMap::single(4),
                row_number: row_number("sales-rn", 4),
                columns: vec![
                    ("Region".into(), hash_column("sales-region", 4, 2)),
                    ("Note".into(), plain_column("sales-note", 4)),
                    ("Amount".into(), value_column("sales-amount", 4, 3)),
                ],
                partitions: Vec::new(),
            },
            EncodedTable {
                name: "region_list".into(),
                segment_map: SegmentMap::single(2),
                row_number: row_number("region-rn", 2),
                columns: vec![("REGION".into(), hash_column("region-region", 2, 2))],
                partitions: Vec::new(),
            },
        ],
        columns: Vec::new(),
        relationships: vec![EncodedRelationship {
            name: "0b5f0e0a-1111-4222-8333-444455556666".into(),
            index: system_table(3, 2, &["INDEX"], "rel"),
            unmatched: true,
            reverse: None,
        }],
        hierarchies: Vec::new(),
    };
    (m, data)
}

fn rows_where<'a>(t: &'a MetadataTables, table: &str, col: &str, v: &MetaValue) -> Vec<&'a MetaRow> {
    t.rows(table).iter().filter(|r| r.get(col) == Some(v)).collect()
}

fn int(r: &MetaRow, col: &str) -> i64 {
    match r.get(col) {
        Some(MetaValue::Integer(i)) => *i,
        other => panic!("{col}: {other:?}"),
    }
}

fn txt<'a>(r: &'a MetaRow, col: &str) -> &'a str {
    match r.get(col) {
        Some(MetaValue::Text(s)) => s,
        other => panic!("{col}: {other:?}"),
    }
}

fn by_id<'a>(t: &'a MetadataTables, table: &str, id: i64) -> &'a MetaRow {
    rows_where(t, table, "ID", &id.into())
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("{table} {id} missing"))
}

/// The column row of a user table by name.
fn column<'a>(t: &'a MetadataTables, table: &str, name: &str) -> &'a MetaRow {
    let tid = int(rows_where(t, "Table", "Name", &table.into())[0], "ID");
    t.rows("Column")
        .iter()
        .find(|r| {
            r.get("TableID") == Some(&tid.into())
                && [r.get("ExplicitName"), r.get("InferredName")].contains(&Some(&name.into()))
        })
        .unwrap_or_else(|| panic!("column {table}[{name}] missing"))
}

fn grow() -> (MetadataTables, StructuralEdit) {
    let b = base();
    let (m, data) = grown(&build_model(&b).unwrap());
    let edit = apply_structural_at(&b, &m, &data, NOW).unwrap();
    (b, edit)
}

/// The engine's spelling `isAvailableInMDX` (written by older bicraft versions) is still read;
/// the written model reads back with TOM's `isAvailableInMdx`.
#[test]
fn legacy_available_in_mdx_spelling_is_accepted() {
    let b = base();
    let (mut m, data) = grown(&build_model(&b).unwrap());
    let note = m
        .table_mut("Sales")
        .unwrap()
        .columns
        .iter_mut()
        .find(|c| c.name == "Note")
        .unwrap();
    let v = note.extra.shift_remove("isAvailableInMdx").unwrap();
    note.extra.insert("isAvailableInMDX".into(), v);
    let legacy = apply_structural_at(&b, &m, &data, NOW).unwrap();
    let (_, current) = grow();
    assert_eq!(legacy.tables, current.tables);
    assert_eq!(legacy.model, current.model);
    let sales = legacy.model.table("Sales").unwrap();
    let extra = &sales.columns.iter().find(|c| c.name == "Note").unwrap().extra;
    assert_eq!(extra.get("isAvailableInMdx"), Some(&json!(false)));
    assert!(!extra.contains_key("isAvailableInMDX"));
}

// --- additions -----------------------------------------------------------------------------------

#[test]
fn new_tables_get_every_storage_row_and_file() {
    let (b, edit) = grow();
    let t = &edit.tables;
    assert_eq!(
        build_model(t).unwrap(),
        edit.model,
        "rows read back as the written model"
    );
    assert!(dangling_references(t).is_empty(), "{:?}", dangling_references(t));
    // Every planned file has a StorageFile row and vice versa.
    let mut planned: Vec<&str> = edit.plan.add.iter().map(|f| f.path.as_str()).collect();
    planned.sort_unstable();
    assert_eq!(planned, storage_file_paths(t));
    assert!(edit.plan.remove.is_empty());
    // Files: per user column idf + idfmeta; dictionaries of hash columns; hidx of value
    // columns; idf + idfmeta per system column (H$ x 4 tables with 2+1+2+... columns, R$ 1).
    let user_columns = 4 + 2; // two row numbers
    let dictionaries = 2;
    let hidx = 1; // Sales[Amount]; row numbers have none
    let system_columns = 2 /* Sales[Region] */ + 1 /* Amount */ + 2 /* Region_List[Region] */ + 1 /* R$ */;
    assert_eq!(
        edit.plan.add.len(),
        2 * user_columns + dictionaries + hidx + 2 * system_columns
    );

    // The model as written: row-number columns and generated lineage tags.
    let sales = edit.model.table("Sales").unwrap();
    assert_eq!(sales.columns[0].name, ROW_NUMBER_NAME);
    assert_eq!(sales.columns[0].extra.get("type"), Some(&json!("rowNumber")));
    assert!(sales.lineage_tag.as_deref().is_some_and(|l| l.len() == 36));
    assert_eq!(
        edit.model.table("Region_List").unwrap().lineage_tag.as_deref(),
        Some("aaaaaaaa-0000-4000-8000-000000000001")
    );
    assert_eq!(sales.measures[0].name, "Total");

    // Names and folders follow Desktop's patterns.
    let sales_row = rows_where(t, "Table", "Name", &"Sales".into())[0];
    let sales_id = int(sales_row, "ID");
    assert_eq!(int(sales_row, "SystemFlags"), 0);
    let ts = by_id(t, "TableStorage", int(sales_row, "TableStorageID"));
    assert_eq!(txt(ts, "Name"), format!("Sales ({sales_id})"));
    assert_eq!(int(ts, "Settings"), 4353);
    let folder = by_id(t, "StorageFolder", int(ts, "StorageFolderID"));
    assert_eq!(txt(folder, "Path"), format!("Sales ({sales_id}).tbl"));
    assert_eq!(int(folder, "OwnerType"), 18);
    let region_list = rows_where(t, "Table", "Name", &"Region_List".into())[0];
    let rl_id = int(region_list, "ID");
    assert_eq!(int(region_list, "SystemFlags"), 2, "calculated table");
    assert_eq!(
        txt(
            by_id(t, "TableStorage", int(region_list, "TableStorageID")),
            "Name"
        ),
        format!("Region List ({rl_id})"),
        "storage names replace `_` with a space"
    );

    // Columns: flags, positions, usage.
    let flags = |table: &str, name: &str| {
        let c = column(t, table, name);
        let cs = by_id(t, "ColumnStorage", int(c, "ColumnStorageID"));
        (
            int(cs, "ColumnFlags"),
            int(cs, "StoragePosition"),
            int(cs, "Statistics_Usage"),
            int(cs, "Statistics_EncodingHint"),
        )
    };
    assert_eq!(flags("Sales", ROW_NUMBER_NAME), (31, 0, 3, 1));
    assert_eq!(flags("Sales", "Region"), (8, 1, 3, 0));
    assert_eq!(flags("Sales", "Note"), (0, 2, 3, 0));
    assert_eq!(flags("Sales", "Amount"), (8, 3, 3, 0));
    assert_eq!(
        flags("Region_List", "Region"),
        (11, 1, 0, 1),
        "the one end of the relationship"
    );
    let rl_region = column(t, "Region_List", "Region");
    assert_eq!(int(rl_region, "Type"), 4);
    assert_eq!(int(rl_region, "SystemFlags"), 2);
    let rn = column(t, "Sales", ROW_NUMBER_NAME);
    assert_eq!(
        (int(rn, "Type"), int(rn, "InferredDataType"), int(rn, "IsKey")),
        (3, 19, 1)
    );

    // Attribute hierarchies: H$ tables, hidx, a column without one.
    let ahs_of = |table: &str, name: &str| {
        let c = column(t, table, name);
        let ah = by_id(t, "AttributeHierarchy", int(c, "AttributeHierarchyID"));
        by_id(
            t,
            "AttributeHierarchyStorage",
            int(ah, "AttributeHierarchyStorageID"),
        )
        .clone()
    };
    let region_ahs = ahs_of("Sales", "Region");
    assert_eq!(
        (
            int(&region_ahs, "ColumnPositionToData"),
            int(&region_ahs, "ColumnDataToPosition")
        ),
        (0, 1)
    );
    let h = by_id(t, "Table", int(&region_ahs, "SystemTableID"));
    let region_col_id = int(column(t, "Sales", "Region"), "ID");
    let h_name = format!("H$Sales ({sales_id})$Region ({region_col_id})");
    assert_eq!(txt(h, "Name"), h_name);
    assert_eq!((int(h, "SystemFlags"), int(h, "ModelID")), (1, 0));
    let h_folder = by_id(
        t,
        "StorageFolder",
        int(
            by_id(t, "TableStorage", int(h, "TableStorageID")),
            "StorageFolderID",
        ),
    );
    assert_eq!(txt(h_folder, "Path"), format!("{h_name}$({}).tbl", int(h, "ID")));
    let amount_ahs = ahs_of("Sales", "Amount");
    assert_eq!(int(&amount_ahs, "MaterializationType"), 1);
    assert_eq!(int(&amount_ahs, "ColumnDataToPosition"), -1);
    let hidx = by_id(t, "StorageFile", int(&amount_ahs, "StorageFileID"));
    let amount_id = int(column(t, "Sales", "Amount"), "ID");
    assert_eq!(
        txt(hidx, "FileName"),
        format!("1.H$Sales ({sales_id})$Amount ({amount_id}).hidx")
    );
    let note_ahs = ahs_of("Sales", "Note");
    assert_eq!(int(&note_ahs, "MaterializationType"), -1);
    assert_eq!(int(&note_ahs, "SystemTableID"), 0);
    assert_eq!(int(column(t, "Sales", "Note"), "IsAvailableInMDX"), 0);

    // The relationship: storage, index, R$ table.
    let rel = &t.rows("Relationship")[0];
    let rs = by_id(t, "RelationshipStorage", int(rel, "RelationshipStorageID"));
    let ris = by_id(
        t,
        "RelationshipIndexStorage",
        int(rs, "RelationshipIndexStorageID"),
    );
    assert_eq!(int(ris, "RecordCount"), 5);
    assert_eq!(int(ris, "Flags"), 1);
    let r_table = by_id(t, "Table", int(ris, "SystemTableID"));
    assert_eq!(
        txt(r_table, "Name"),
        format!("R$Sales ({sales_id})${}", txt(rs, "Name"))
    );
    assert!(txt(rs, "Name").starts_with("0b5f0e0a 1111 4222 8333 444455556666 ("));

    // IDs: unique, MAXID follows.
    let maxid = t
        .rows("DBPROPERTIES")
        .iter()
        .find(|r| r.get("NAME") == Some(&"MAXID".into()))
        .map(|r| int(r, "VALUE"))
        .unwrap();
    let all_ids: Vec<i64> = t
        .table_names()
        .filter(|n| *n != "DBPROPERTIES")
        .flat_map(|n| t.rows(n).iter().filter_map(|r| r.get("ID").cloned()))
        .map(|v| match v {
            MetaValue::Integer(i) => i,
            _ => 0,
        })
        .collect();
    assert_eq!(all_ids.iter().max().copied(), Some(maxid));
    let mut sorted = all_ids.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), all_ids.len());
    assert_ne!(b, *t);

    // Deterministic.
    let (_, again) = grow();
    assert_eq!(again, edit);
}

#[test]
fn a_new_column_joins_an_existing_table() {
    let (_, grown) = grow();
    let mut m = grown.model.clone();
    let mut qty = data_column("Qty", DataType::Int64);
    qty.description = Some("quantity".into());
    m.table_mut("Sales").unwrap().columns.push(qty);
    let data = EncodedData {
        columns: vec![EncodedNewColumn {
            table: "sales".into(),
            column: "QTY".into(),
            data: value_column("sales-qty", 4, 4),
            partitions: Vec::new(),
        }],
        ..Default::default()
    };
    let edit = apply_structural_at(&grown.tables, &m, &data, LATER).unwrap();
    let t = &edit.tables;
    assert_eq!(build_model(t).unwrap(), edit.model);
    assert!(dangling_references(t).is_empty());
    let qty = column(t, "Sales", "Qty");
    let cs = by_id(t, "ColumnStorage", int(qty, "ColumnStorageID"));
    assert_eq!(int(cs, "StoragePosition"), 4);
    assert_eq!(txt(qty, "LineageTag").len(), 36, "a generated lineage tag");
    assert_eq!(
        edit.plan.add.len(),
        2 + 1 + 2,
        "idf, idfmeta, hidx, H$ POS_TO_ID files"
    );
    let sales = rows_where(t, "Table", "Name", &"Sales".into())[0];
    assert_eq!(int(sales, "StructureModifiedTime"), LATER);
    assert_eq!(
        edit.model
            .table("Sales")
            .unwrap()
            .column("Qty")
            .unwrap()
            .description
            .as_deref(),
        Some("quantity")
    );

    // A column whose row count does not match the partition is refused.
    let mut short = data.clone();
    short.columns[0].data.statistics.row_count = 3;
    let err = apply_structural_at(&grown.tables, &m, &short, LATER).unwrap_err();
    assert!(
        matches!(err, Error::UnsupportedChange(ref v) if v[0].what.contains("rows")),
        "{err}"
    );
}

/// Regression (G10, incremental refresh): a new column of a table of several partitions gets
/// one `ColumnPartitionStorage` + `SegmentStorage` and one `.k.idf`/`.k.idfmeta` pair per
/// partition, in that partition's folder, and shares one dictionary and `H$`. Mismatched
/// partition counts or row counts are refused.
#[test]
fn a_new_column_joins_every_partition_of_a_table() {
    let (_, grown) = grow();
    let mut t = grown.tables.clone();
    // A second partition for Sales (as the engine stores policy-range partitions).
    let sales_id = int(rows_where(&t, "Table", "Name", &"Sales".into())[0], "ID");
    let p = rows_where(&t, "Partition", "TableID", &sales_id.into())[0].clone();
    let ps = by_id(&t, "PartitionStorage", int(&p, "PartitionStorageID")).clone();
    let sm = by_id(&t, "SegmentMapStorage", int(&ps, "SegmentMapStorageID")).clone();
    let folder = by_id(&t, "StorageFolder", int(&ps, "StorageFolderID")).clone();
    let first_rows = int(&sm, "RecordCount");
    let (p2, ps2, sm2, f2) = (900_001i64, 900_002i64, 900_003i64, 900_004i64);
    let set = |r: &MetaRow, cells: &[(&str, MetaValue)]| {
        let mut r = r.clone();
        for (k, v) in cells {
            r.insert((*k).to_string(), v.clone());
        }
        r
    };
    let path = txt(&folder, "Path").replace(&format!("{}.prt", int(&p, "ID")), &format!("{p2}.prt"));
    t.push_row(
        "Partition",
        set(
            &p,
            &[
                ("ID", p2.into()),
                ("Name", "Sales 2".into()),
                ("PartitionStorageID", ps2.into()),
            ],
        ),
    );
    t.push_row(
        "PartitionStorage",
        set(
            &ps,
            &[
                ("ID", ps2.into()),
                ("PartitionID", p2.into()),
                ("StoragePosition", 1.into()),
                ("SegmentMapStorageID", sm2.into()),
                ("StorageFolderID", f2.into()),
            ],
        ),
    );
    t.push_row(
        "SegmentMapStorage",
        set(
            &sm,
            &[
                ("ID", sm2.into()),
                ("PartitionStorageID", ps2.into()),
                ("RecordCount", 6.into()),
            ],
        ),
    );
    t.push_row(
        "StorageFolder",
        set(&folder, &[("ID", f2.into()), ("Path", path.as_str().into())]),
    );
    let base_model = build_model(&t).unwrap();
    assert_eq!(base_model.table("Sales").unwrap().partitions.len(), 2);

    let mut m = base_model.clone();
    m.table_mut("Sales")
        .unwrap()
        .columns
        .push(data_column("Qty", DataType::Int64));
    let segments = |a: i64, b: i64| {
        vec![
            PartitionSegments {
                idf: b"p0-idf".to_vec(),
                idfmeta: b"p0-idfmeta".to_vec(),
                segment_count: 1,
                rows: a,
            },
            PartitionSegments {
                idf: b"p1-idf".to_vec(),
                idfmeta: b"p1-idfmeta".to_vec(),
                segment_count: 1,
                rows: b,
            },
        ]
    };
    let total = first_rows + 6;
    let data = EncodedData {
        columns: vec![EncodedNewColumn {
            table: "Sales".into(),
            column: "Qty".into(),
            data: value_column("sales-qty", total, 4),
            partitions: segments(first_rows, 6),
        }],
        ..Default::default()
    };
    let edit = apply_structural_at(&t, &m, &data, LATER).unwrap();
    let qty = column(&edit.tables, "Sales", "Qty");
    let cs = int(qty, "ColumnStorageID");
    let cps = rows_where(
        &edit.tables,
        "ColumnPartitionStorage",
        "ColumnStorageID",
        &cs.into(),
    );
    assert_eq!(cps.len(), 2);
    let mut by_partition: Vec<i64> = cps.iter().map(|r| int(r, "PartitionStorageID")).collect();
    by_partition.sort_unstable();
    assert_eq!(by_partition, vec![int(&ps, "ID"), ps2]);
    let files: Vec<(&str, &[u8])> = edit
        .plan
        .add
        .iter()
        .filter(|f| f.path.contains("Qty ("))
        .map(|f| (f.path.as_str(), f.bytes.as_slice()))
        .collect();
    let second = files
        .iter()
        .find(|(p, _)| p.ends_with(".1.idf"))
        .expect("the second partition's .idf");
    assert!(second.0.contains(&format!("{p2}.prt/")), "{}", second.0);
    assert_eq!(second.1, b"p1-idf");
    assert!(
        files
            .iter()
            .any(|(p, b)| p.ends_with(".0.idfmeta") && *b == b"p0-idfmeta")
    );
    assert!(
        files
            .iter()
            .any(|(p, b)| p.ends_with(".1.idfmeta") && *b == b"p1-idfmeta")
    );
    assert_eq!(files.iter().filter(|(p, _)| p.ends_with(".hidx")).count(), 1);
    assert!(dangling_references(&edit.tables).is_empty());

    // One segment set for two partitions, or rows that do not match a partition: refused.
    for bad in [
        vec![segments(first_rows, 6).remove(0)],
        segments(first_rows + 1, 5),
        Vec::new(),
    ] {
        let mut d = data.clone();
        d.columns[0].partitions = bad;
        let err = apply_structural_at(&t, &m, &d, LATER).unwrap_err();
        assert!(matches!(err, Error::UnsupportedChange(_)), "{err}");
    }
}

// --- removals ------------------------------------------------------------------------------------

#[test]
fn removing_a_column_cascades_and_compacts_positions() {
    let (_, grown) = grow();
    let before = &grown.tables;
    let mut m = grown.model.clone();
    m.table_mut("Sales").unwrap().columns.retain(|c| c.name != "Note");
    let edit = apply_structural_at(before, &m, &EncodedData::default(), LATER).unwrap();
    let t = &edit.tables;
    assert_eq!(build_model(t).unwrap(), m);
    assert!(dangling_references(t).is_empty());
    assert!(edit.plan.add.is_empty());
    let gone: Vec<String> = storage_file_paths(before)
        .into_iter()
        .filter(|p| !storage_file_paths(t).contains(p))
        .collect();
    let mut planned = edit.plan.remove.clone();
    planned.sort();
    assert_eq!(planned, gone);
    assert_eq!(
        planned.len(),
        2,
        "idf and idfmeta of a value column without hierarchy"
    );
    // Amount moved from position 3 to 2.
    let amount = column(t, "Sales", "Amount");
    assert_eq!(
        int(
            by_id(t, "ColumnStorage", int(amount, "ColumnStorageID")),
            "StoragePosition"
        ),
        2
    );

    // Removing a hash column drops its dictionary and its H$ table.
    let mut m2 = m.clone();
    m2.table_mut("Sales")
        .unwrap()
        .columns
        .retain(|c| c.name != "Amount");
    let edit2 = apply_structural_at(t, &m2, &EncodedData::default(), LATER).unwrap();
    assert_eq!(
        edit2.plan.remove.len(),
        2 + 1 + 2,
        "idf, idfmeta, hidx, H$ POS_TO_ID files"
    );
    assert!(
        edit2
            .tables
            .rows("Table")
            .iter()
            .all(|r| !txt(r, "Name").contains("$Amount ("))
    );
    assert!(dangling_references(&edit2.tables).is_empty());
}

#[test]
fn removing_relationships_and_tables_returns_to_the_base() {
    let (b, grown) = grow();
    let mut m = grown.model.clone();
    m.relationships.clear();
    let no_rel = apply_structural_at(&grown.tables, &m, &EncodedData::default(), LATER).unwrap();
    assert_eq!(no_rel.plan.remove.len(), 2, "R$ INDEX idf and idfmeta");
    assert!(no_rel.tables.rows("RelationshipStorage").is_empty());
    assert!(no_rel.tables.rows("RelationshipIndexStorage").is_empty());
    assert!(dangling_references(&no_rel.tables).is_empty());

    m.tables.retain(|t| t.name != "Sales" && t.name != "Region_List");
    let empty = apply_structural_at(&no_rel.tables, &m, &EncodedData::default(), LATER).unwrap();
    let t = &empty.tables;
    assert!(storage_file_paths(t).is_empty());
    let mut all_removed = no_rel.plan.remove.clone();
    all_removed.extend(empty.plan.remove.clone());
    all_removed.sort();
    let mut all_added: Vec<String> = grown.plan.add.iter().map(|f| f.path.clone()).collect();
    all_added.sort();
    assert_eq!(all_removed, all_added, "everything added is removed again");
    for name in b.table_names().chain(t.table_names()) {
        if name == "DBPROPERTIES" {
            continue;
        }
        assert_eq!(t.rows(name), b.rows(name), "{name} is back to the base");
    }
}

#[test]
fn structural_changes_that_cannot_be_written_are_refused() {
    let (_, grown) = grow();
    let refused = |m: &SemanticModel, data: &EncodedData| -> Vec<String> {
        match apply_structural_at(&grown.tables, m, data, LATER) {
            Err(Error::UnsupportedChange(v)) => v.iter().map(ToString::to_string).collect(),
            other => panic!("expected a refusal, got {other:?}"),
        }
    };
    let none = EncodedData::default();
    // The row-number column.
    let mut m = grown.model.clone();
    m.table_mut("Sales").unwrap().columns.remove(0);
    assert!(refused(&m, &none)[0].contains("row-number"));
    // A column a kept relationship uses.
    let mut m = grown.model.clone();
    m.table_mut("Sales")
        .unwrap()
        .columns
        .retain(|c| c.name != "Region");
    assert!(refused(&m, &none).iter().any(|s| s.contains("relationship")));
    // A calculated-table column.
    let mut m = grown.model.clone();
    m.relationships.clear();
    m.table_mut("Region_List")
        .unwrap()
        .columns
        .retain(|c| c.name != "Region");
    assert!(refused(&m, &none).iter().any(|s| s.contains("calculated-table")));
    // A table a kept relationship uses.
    let mut m = grown.model.clone();
    m.tables.retain(|t| t.name != "Region_List");
    assert!(refused(&m, &none).iter().any(|s| s.contains("relationship")));
    // New objects without encoded data.
    let mut m = grown.model.clone();
    m.tables.push(Table {
        name: "Empty".into(),
        partitions: vec![Partition {
            name: "Empty".into(),
            source_type: "m".into(),
            expression: Some("x".into()),
            ..Default::default()
        }],
        ..Default::default()
    });
    m.table_mut("Sales")
        .unwrap()
        .columns
        .push(data_column("Extra", DataType::Double));
    m.relationships.push(Relationship {
        name: "r2".into(),
        from_table: "Sales".into(),
        from_column: "Amount".into(),
        to_table: "Sales".into(),
        to_column: "Amount".into(),
        is_active: false,
        ..Default::default()
    });
    let msgs = refused(&m, &none);
    assert!(
        msgs.iter()
            .any(|s| s.contains("table 'Empty'") && s.contains("encoded data"))
    );
    assert!(
        msgs.iter()
            .any(|s| s.contains("[Extra]") && s.contains("encoded data"))
    );
    assert!(
        msgs.iter()
            .any(|s| s.contains("'r2'") && s.contains("encoded index"))
    );
    assert!(
        msgs.iter()
            .any(|s| s.contains("'r2'") && s.contains("hash-encoded"))
    );
    // Nothing is written when anything is refused: the base is untouched (pure function).
}

#[test]
fn a_model_without_structural_change_is_a_metadata_edit() {
    let (_, grown) = grow();
    let mut m = grown.model.clone();
    m.table_mut("Sales").unwrap().description = Some("changed".into());
    let edit = apply_structural_at(&grown.tables, &m, &EncodedData::default(), LATER).unwrap();
    assert!(edit.plan.is_empty());
    assert_eq!(build_model(&edit.tables).unwrap(), m);
    assert_eq!(
        edit.tables,
        bicraft_metadata::apply_model_at(&grown.tables, &m, LATER).unwrap()
    );
}

// --- through SQLite --------------------------------------------------------------------------------

/// The column lists of the 2.158 schema (docs/formats/abf.md) for the tables these tests touch.
const SCHEMA: &[(&str, &str)] = &[
    ("Model", "Name T, Description T, Culture T, ModifiedTime I"),
    (
        "Table",
        "ModelID I, Name T, DataCategory T, Description T, IsHidden I, TableStorageID I, ModifiedTime I, StructureModifiedTime I, SystemFlags I, ShowAsVariationsOnly I, IsPrivate I, DefaultDetailRowsDefinitionID I, AlternateSourcePrecedence I, RefreshPolicyID I, CalculationGroupID I, ExcludeFromModelRefresh I, LineageTag T, SourceLineageTag T, SystemManaged I, ExcludeFromAutomaticAggregations I, DirectLakeIndexingBehavior I",
    ),
    (
        "Column",
        "TableID I, ExplicitName T, InferredName T, ExplicitDataType I, InferredDataType I, DataCategory T, Description T, IsHidden I, State I, IsUnique I, IsKey I, IsNullable I, Alignment I, TableDetailPosition I, IsDefaultLabel I, IsDefaultImage I, SummarizeBy I, ColumnStorageID I, Type I, SourceColumn T, ColumnOriginID I, Expression T, FormatString T, IsAvailableInMDX I, SortByColumnID I, AttributeHierarchyID I, ModifiedTime I, StructureModifiedTime I, RefreshedTime I, SystemFlags I, KeepUniqueRows I, DisplayOrdinal I, ErrorMessage T, SourceProviderType T, DisplayFolder T, EncodingHint I, RelatedColumnDetailsID I, AlternateOfID I, LineageTag T, SourceLineageTag T, ExpressionContext I, StringIndexingBehavior I, FullTextIndexingBehavior I",
    ),
    (
        "Measure",
        "TableID I, Name T, Description T, DataType I, Expression T, FormatString T, IsHidden I, State I, ModifiedTime I, StructureModifiedTime I, KPIID I, IsSimpleMeasure I, ErrorMessage T, DisplayFolder T, DetailRowsDefinitionID I, DataCategory T, FormatStringDefinitionID I, LineageTag T, SourceLineageTag T",
    ),
    (
        "Partition",
        "TableID I, Name T, Description T, DataSourceID I, QueryDefinition T, State I, Type I, PartitionStorageID I, Mode I, DataView I, ModifiedTime I, RefreshedTime I, SystemFlags I, ErrorMessage T, RetainDataTillForceCalculate I, RangeStart R, RangeEnd R, RangeGranularity I, RefreshBookmark T, QueryGroupID I, ExpressionSourceID I, MAttributes T, DataCoverageDefinitionID I, SchemaName T",
    ),
    (
        "Relationship",
        "ModelID I, Name T, IsActive I, Type I, CrossFilteringBehavior I, JoinOnDateBehavior I, RelyOnReferentialIntegrity I, FromTableID I, FromColumnID I, FromCardinality I, ToTableID I, ToColumnID I, ToCardinality I, State I, RelationshipStorageID I, RelationshipStorage2ID I, ModifiedTime I, RefreshedTime I, SecurityFilteringBehavior I",
    ),
    (
        "Annotation",
        "ObjectID I, ObjectType I, Name T, Value T, ModifiedTime I",
    ),
    (
        "AttributeHierarchy",
        "ColumnID I, State I, AttributeHierarchyStorageID I, ModifiedTime I, RefreshedTime I",
    ),
    ("StorageFolder", "OwnerID I, OwnerType I, Path T"),
    (
        "StorageFile",
        "OwnerID I, OwnerType I, StorageFolderID I, FileName T",
    ),
    (
        "TableStorage",
        "TableID I, Name T, Version I, Settings I, RIViolationCount I, StorageFolderID I",
    ),
    (
        "ColumnStorage",
        "ColumnID I, Name T, StoragePosition I, DictionaryStorageID I, Settings I, ColumnFlags I, Collation T, OrderByColumn T, Locale I, BinaryCharacters I, Statistics_DistinctStates I, Statistics_MinDataID I, Statistics_MaxDataID I, Statistics_OriginalMinSegmentDataID I, Statistics_RLESortOrder I, Statistics_RowCount I, Statistics_HasNulls I, Statistics_RLERuns I, Statistics_OthersRLERuns I, Statistics_Usage I, Statistics_DBType I, Statistics_XMType I, Statistics_CompressionType I, Statistics_CompressionParam I, Statistics_EncodingHint I, IsDeltaPartitionColumn I, DeltaColumnMappingPhysicalName T, DeltaColumnMappingId I, FramedSourceColumn T, DeltaColumnPrimitiveType I, DeltaTableColumnStorageID I, StringIndexStorageID I, ColumnIndexStorageID I, FullTextIndexStorageID I",
    ),
    (
        "PartitionStorage",
        "PartitionID I, Name T, StoragePosition I, SegmentMapStorageID I, DataObjectId I, StorageFolderID I, DeltaTableMetadataStorageID I",
    ),
    (
        "SegmentMapStorage",
        "PartitionStorageID I, Type I, RecordCount I, SegmentCount I, RecordsPerSegment I",
    ),
    (
        "DictionaryStorage",
        "ColumnStorageID I, Type I, DataType I, DataVersion I, BaseId I, Magnitude R, LastId I, IsNullable I, IsUnique I, IsOperatingOn32 I, DictionaryFlags I, StorageFileID I, Size I",
    ),
    (
        "ColumnPartitionStorage",
        "ColumnStorageID I, PartitionStorageID I, DataVersion I, State I, SegmentStorageID I, StorageFileID I",
    ),
    (
        "SegmentStorage",
        "ColumnPartitionStorageID I, SegmentCount I, StorageFileID I",
    ),
    (
        "RelationshipStorage",
        "RelationshipID I, Name T, DefinitionType I, Cardinality I, Flags I, RelationshipIndexStorageID I",
    ),
    (
        "RelationshipIndexStorage",
        "RelationshipStorageID I, IndexType I, Flags I, RecordCount I, SecondaryRecordCount I, StorageFolderID I, StorageFileID I, SystemTableID I, SecondarySystemTableID I",
    ),
    (
        "AttributeHierarchyStorage",
        "AttributeHierarchyID I, SortOrder I, OptimizationLevel I, MaterializationType I, ColumnPositionToData I, ColumnDataToPosition I, DistinctDataCount I, DataVersion I, StorageFileID I, SystemTableID I, HasStatistics I, MinValue T, MaxValue T, StringValueMaxLength I",
    ),
];

fn empty_database() -> Vec<u8> {
    let mut b = DatabaseBuilder::new(4096).unwrap();
    for (name, cols) in SCHEMA {
        let cols: Vec<String> = cols
            .split(", ")
            .map(|c| {
                let (n, t) = c.rsplit_once(' ').unwrap();
                let ty = match t {
                    "I" => "INTEGER",
                    "T" => "TEXT",
                    _ => "REAL",
                };
                format!("[{n}] {ty}")
            })
            .collect();
        let sql = format!(
            "CREATE TABLE [{name}] ([ID] INTEGER, {}, PRIMARY KEY([ID] ASC))",
            cols.join(", ")
        );
        b.create_table(name, &sql).unwrap();
    }
    b.create_table(
        "DBPROPERTIES",
        "CREATE TABLE [DBPROPERTIES] ([NAME] STRING PRIMARY KEY, [VALUE] INTEGER)",
    )
    .unwrap();
    b.insert(
        "Model",
        1,
        vec![
            Value::Integer(1),
            Value::Text("Model".into()),
            Value::Null,
            Value::Text("en-US".into()),
            Value::Integer(NOW - 1),
        ],
    )
    .unwrap();
    b.insert(
        "DBPROPERTIES",
        1,
        vec![Value::Text("MAXID".into()), Value::Integer(1)],
    )
    .unwrap();
    b.insert(
        "DBPROPERTIES",
        2,
        vec![Value::Text("LASTCOMMITTED".into()), Value::Integer(NOW - 1)],
    )
    .unwrap();
    b.insert(
        "DBPROPERTIES",
        3,
        vec![Value::Text("LASTCUBEDATACHANGE".into()), Value::Integer(NOW - 1)],
    )
    .unwrap();
    b.finish().unwrap()
}

#[test]
fn structural_writes_go_through_sqlite() {
    let db = empty_database();
    let base_model = read_sqlitedb(&db).unwrap();
    let (m, data) = grown(&base_model);
    let w = write_structural_sqlitedb_at(&db, &m, &data, NOW).unwrap();
    assert_eq!(read_sqlitedb(&w.sqlitedb).unwrap(), w.model);
    let back = tables_from_sqlitedb_all(&w.sqlitedb).unwrap();
    assert!(dangling_references(&back).is_empty());
    let mut planned: Vec<&str> = w.plan.add.iter().map(|f| f.path.as_str()).collect();
    planned.sort_unstable();
    assert_eq!(planned, storage_file_paths(&back));
    // The same rows as the in-memory edit (DBPROPERTIES aside: its commit markers move).
    let edit = apply_structural_at(&tables_from_sqlitedb_all(&db).unwrap(), &m, &data, NOW).unwrap();
    for name in back.table_names().filter(|n| *n != "DBPROPERTIES") {
        assert_eq!(back.rows(name), edit.tables.rows(name), "{name}");
    }

    // And back down: remove everything again.
    let mut shrunk = w.model.clone();
    shrunk.relationships.clear();
    shrunk.tables.clear();
    let w2 = write_structural_sqlitedb_at(&w.sqlitedb, &shrunk, &EncodedData::default(), LATER).unwrap();
    assert_eq!(w2.plan.remove.len(), w.plan.add.len());
    let back2 = tables_from_sqlitedb_all(&w2.sqlitedb).unwrap();
    assert!(storage_file_paths(&back2).is_empty());
    assert_eq!(read_sqlitedb(&w2.sqlitedb).unwrap(), base_model);

    // New data moves the last data change forward; a removal adds none and leaves it.
    let marker = |db: &[u8]| {
        tables_from_sqlitedb_all(db)
            .unwrap()
            .rows("DBPROPERTIES")
            .iter()
            .find(|r| r.get("NAME") == Some(&MetaValue::Text("LASTCUBEDATACHANGE".into())))
            .and_then(|r| r.get("VALUE").cloned())
    };
    assert_eq!(marker(&db), Some(MetaValue::Integer(NOW - 1)));
    assert_eq!(marker(&w.sqlitedb), Some(MetaValue::Integer(NOW)));
    assert_eq!(marker(&w2.sqlitedb), Some(MetaValue::Integer(NOW)));
}
