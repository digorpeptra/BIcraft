//! Structural saves through the whole PBIX pipeline on a synthetic package (runs everywhere):
//! values -> VertiPaq encoder -> `H$`/`R$` system tables -> storage rows -> ABF -> XPress9, then
//! read back and decoded. The database holds the tables of the Desktop 2.158 schema that the
//! edits touch (docs/formats/abf.md) and nothing else; all content is our own.

use bicraft_model::{Column, DataType, Partition, Relationship, SemanticModel, Table};
use bicraft_opc::Package;
use bicraft_pbix::structural::{Cell, ColumnValues, DataModelEditor, StructuralData, TableValues};
use bicraft_pbix::{DATA_MODEL, DataModelLimits, read_model};

const NOW: i64 = 134_116_992_000_000_000;

fn editor(pkg: &Package) -> DataModelEditor {
    DataModelEditor::open(&pkg.get(DATA_MODEL).unwrap().data, &DataModelLimits::default()).unwrap()
}

fn write(pkg: &Package, model: &SemanticModel, data: &StructuralData) -> Package {
    let bytes = editor(pkg).write_at(model, data, Some(NOW + 10)).unwrap();
    let mut next = pkg.clone();
    next.put(DATA_MODEL, bytes, true);
    next
}

fn col(name: &str, t: DataType, tag: u32) -> Column {
    Column {
        name: name.into(),
        data_type: t,
        source_column: Some(name.into()),
        lineage_tag: Some(format!("00000000-0000-4000-8000-{tag:012}")),
        ..Default::default()
    }
}

fn table(name: &str, tag: u32, columns: Vec<Column>) -> Table {
    Table {
        name: name.into(),
        lineage_tag: Some(format!("00000000-0000-4000-9000-{tag:012}")),
        columns,
        partitions: vec![Partition {
            name: name.into(),
            mode: Some("import".into()),
            source_type: "m".into(),
            expression: Some(format!("#table({{}}, {{}}) // {name}")),
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn values(name: &str, v: Vec<Cell>) -> ColumnValues {
    ColumnValues {
        name: name.into(),
        values: v,
    }
}

fn text(s: &str) -> Cell {
    Cell::Text(s.into())
}

fn relationship(from: &str, to: &str) -> Relationship {
    Relationship {
        name: "00000000-0000-4000-a000-000000000001".into(),
        from_table: from.into(),
        from_column: "Key".into(),
        to_table: to.into(),
        to_column: "Key".into(),
        is_active: true,
        ..Default::default()
    }
}

#[test]
fn synthetic_pbix_structural_round_trip() {
    let pkg = bicraft_pbix::synthetic::pbix(&bicraft_pbix::synthetic::storage_database().unwrap()).unwrap();
    let base = read_model(&pkg).unwrap();
    assert!(base.tables.is_empty());

    // Two tables and a relationship.
    let mut m = base.clone();
    m.tables.push(table(
        "Dim",
        1,
        vec![col("Key", DataType::Int64, 1), col("Name", DataType::String, 2)],
    ));
    m.tables.push(table(
        "Fact",
        2,
        vec![col("Key", DataType::Int64, 3), col("Amount", DataType::Double, 4)],
    ));
    m.relationships.push(relationship("Fact", "Dim"));
    let dim = TableValues {
        name: "Dim".into(),
        columns: vec![
            values("Key", vec![Cell::Int(10), Cell::Int(20), Cell::Int(30)]),
            values("Name", vec![text("b"), text("a"), text("B")]),
        ],
    };
    let fact = TableValues {
        name: "Fact".into(),
        columns: vec![
            values(
                "Key",
                vec![
                    Cell::Int(20),
                    Cell::Int(10),
                    Cell::Int(20),
                    Cell::Int(99),
                    Cell::Null,
                ],
            ),
            values(
                "Amount",
                vec![
                    Cell::Real(1.5),
                    Cell::Real(2.0),
                    Cell::Null,
                    Cell::Real(4.0),
                    Cell::Real(5.0),
                ],
            ),
        ],
    };
    let data = StructuralData {
        tables: vec![dim.clone(), fact.clone()],
        columns: vec![],
    };
    let p1 = write(&pkg, &m, &data);
    let back = read_model(&p1).unwrap();
    assert_eq!(back.tables.len(), 2);
    assert_eq!(back.relationships.len(), 1);
    let e1 = editor(&p1);
    assert!(e1.changes(&back).is_empty());
    assert_eq!(e1.column_values("Dim", "Key").unwrap(), dim.columns[0].values);
    // "B" is the same dictionary entry as "b" for the engine: it reads back as the first spelling.
    assert_eq!(
        e1.column_values("Dim", "Name").unwrap(),
        vec![text("b"), text("a"), text("b")]
    );
    assert_eq!(e1.column_values("Fact", "Key").unwrap(), fact.columns[0].values);
    assert_eq!(
        e1.column_values("Fact", "Amount").unwrap(),
        fact.columns[1].values
    );

    // Renames, and new data for Fact (its relationship is rebuilt).
    let mut m2 = back.clone();
    let d = m2.tables.iter_mut().find(|t| t.name == "Dim").unwrap();
    d.name = "Dimension".into();
    d.columns.iter_mut().find(|c| c.name == "Name").unwrap().name = "Label".into();
    m2.relationships[0].to_table = "Dimension".into();
    let f = m2.tables.iter_mut().find(|t| t.name == "Fact").unwrap();
    f.partitions[0].expression = Some("#table({}, {}) // v2".into());
    let fact2 = TableValues {
        name: "Fact".into(),
        columns: vec![
            values("Key", vec![Cell::Int(30)]),
            values("Amount", vec![Cell::Real(7.0)]),
        ],
    };
    let ch = editor(&p1).changes(&m2);
    assert_eq!(
        ch.renamed_tables,
        vec![("Dim".to_string(), "Dimension".to_string())]
    );
    assert_eq!(ch.replaced_tables, vec!["Fact".to_string()]);
    let p2 = write(
        &p1,
        &m2,
        &StructuralData {
            tables: vec![fact2],
            columns: vec![],
        },
    );
    let back2 = read_model(&p2).unwrap();
    assert!(
        back2
            .table("Dimension")
            .and_then(|t| t.columns.iter().find(|c| c.name == "Label"))
            .is_some()
    );
    assert_eq!(back2.relationships.len(), 1);
    assert_eq!(back2.relationships[0].to_table, "Dimension");
    assert_eq!(
        editor(&p2).column_values("Fact", "Key").unwrap(),
        vec![Cell::Int(30)]
    );
    assert_eq!(editor(&p2).column_values("Dimension", "Label").unwrap().len(), 3);

    // Removals: every file the edits added for Fact goes with it.
    let mut m3 = back2.clone();
    m3.relationships.clear();
    m3.tables.retain(|t| t.name != "Fact");
    let p3 = write(&p2, &m3, &StructuralData::default());
    assert_eq!(read_model(&p3).unwrap().tables.len(), 1);
    let abf = bicraft_xpress9::decompress_datamodel(&p3.get(DATA_MODEL).unwrap().data, 1 << 30).unwrap();
    let abf = bicraft_abf::Abf::parse(&abf).unwrap();
    assert!(abf.files().iter().all(|f| !f.path.contains("Fact")));
    assert!(abf.files().iter().any(|f| f.path.contains("Dim")));
}

#[test]
fn synthetic_pbix_structural_refusals() {
    let pkg = bicraft_pbix::synthetic::pbix(&bicraft_pbix::synthetic::storage_database().unwrap()).unwrap();
    let mut m = read_model(&pkg).unwrap();
    m.tables
        .push(table("Dim", 1, vec![col("Key", DataType::Int64, 1)]));
    m.tables
        .push(table("Fact", 2, vec![col("Key", DataType::Int64, 2)]));
    m.relationships.push(relationship("Fact", "Dim"));
    let data = |dim: Vec<Cell>| StructuralData {
        tables: vec![
            TableValues {
                name: "Dim".into(),
                columns: vec![values("Key", dim)],
            },
            TableValues {
                name: "Fact".into(),
                columns: vec![values("Key", vec![Cell::Int(1)])],
            },
        ],
        columns: vec![],
    };
    let ed = editor(&pkg);
    // Duplicate and blank keys on the "one" side.
    let e = ed
        .write(&m, &data(vec![Cell::Int(1), Cell::Int(1)]))
        .unwrap_err()
        .to_string();
    assert!(e.contains("duplicate"), "{e}");
    let e = ed
        .write(&m, &data(vec![Cell::Int(1), Cell::Null]))
        .unwrap_err()
        .to_string();
    assert!(e.contains("blank"), "{e}");
    // Missing data, and values that do not fit the column type.
    let e = ed.write(&m, &StructuralData::default()).unwrap_err().to_string();
    assert!(e.contains("no data was supplied"), "{e}");
    let mut bad = data(vec![Cell::Int(1)]);
    bad.tables[1].columns[0].values = vec![text("x")];
    assert!(ed.write(&m, &bad).is_err());
    // One-to-many (the "one" side first) is refused; many-to-many is written.
    m.relationships[0].from_cardinality = Some("one".into());
    m.relationships[0].to_cardinality = Some("many".into());
    let e = ed.write(&m, &data(vec![Cell::Int(1)])).unwrap_err().to_string();
    assert!(e.contains("one-to-many"), "{e}");
}

/// `RelationshipIndexStorage.Flags` of the only relationship of `pkg`.
fn relationship_flags(pkg: &Package) -> i64 {
    let abf = bicraft_xpress9::decompress_datamodel(&pkg.get(DATA_MODEL).unwrap().data, 1 << 30).unwrap();
    let abf = bicraft_abf::Abf::parse(&abf).unwrap();
    let entry = abf.metadata_entry().unwrap();
    let t = bicraft_metadata::tables_from_sqlitedb_all(abf.data(entry).unwrap()).unwrap();
    let rows = t.rows("RelationshipIndexStorage");
    assert_eq!(rows.len(), 1);
    match rows[0].get("Flags") {
        Some(bicraft_metadata::MetaValue::Integer(f)) => *f,
        other => panic!("Flags: {other:?}"),
    }
}

/// Text keys match as the engine stores them (checked against Desktop 2.158, en-US): only ASCII
/// case merges, so `À` and `à` are two keys of the "one" side; imported keys lose trailing blanks
/// (`"A "` matches `a`). Unmatched and blank "many" keys set the index flag that gives the "one"
/// table its blank row.
#[test]
fn synthetic_pbix_text_relationship_keys() {
    let pkg = bicraft_pbix::synthetic::pbix(&bicraft_pbix::synthetic::storage_database().unwrap()).unwrap();
    let mut m = read_model(&pkg).unwrap();
    assert_eq!(m.culture.as_deref(), Some("en-US"));
    m.tables
        .push(table("Dim", 1, vec![col("Key", DataType::String, 1)]));
    m.tables
        .push(table("Fact", 2, vec![col("Key", DataType::String, 2)]));
    m.relationships.push(relationship("Fact", "Dim"));
    let data = |dim: Vec<Cell>, fact: Vec<Cell>| StructuralData {
        tables: vec![
            TableValues {
                name: "Dim".into(),
                columns: vec![values("Key", dim)],
            },
            TableValues {
                name: "Fact".into(),
                columns: vec![values("Key", fact)],
            },
        ],
        columns: vec![],
    };
    let dim = || vec![text("À"), text("à"), text("a"), text("Ω"), text("ω")];
    let p = write(&pkg, &m, &data(dim(), vec![text("à"), text("A "), text("ω")]));
    assert_eq!(relationship_flags(&p), 0);
    assert_eq!(
        editor(&p).column_values("Fact", "Key").unwrap(),
        vec![text("à"), text("A"), text("ω")]
    );
    let p = write(&pkg, &m, &data(dim(), vec![text("à"), text("c")]));
    assert_eq!(relationship_flags(&p), 1);
    let p = write(&pkg, &m, &data(dim(), vec![text("à"), Cell::Null]));
    assert_eq!(relationship_flags(&p), 1);
    // Equal once stored: duplicates on the "one" side.
    let e = editor(&pkg)
        .write(&m, &data(vec![text("a"), text("A ")], vec![text("a")]))
        .unwrap_err()
        .to_string();
    assert!(e.contains("duplicate"), "{e}");
}

/// Regression (G10): columns holding only blanks are written like the engine's (no
/// `.dictionary`, `StorageFileID` 0) and read back as blanks; an int64 column of at most 20 rows
/// per value is value-encoded.
#[test]
fn synthetic_pbix_columns_of_blanks() {
    let pkg = bicraft_pbix::synthetic::pbix(&bicraft_pbix::synthetic::storage_database().unwrap()).unwrap();
    let mut m = read_model(&pkg).unwrap();
    m.tables.push(table(
        "Blanks",
        7,
        vec![
            col("Key", DataType::Int64, 70),
            col("NoInt", DataType::Int64, 71),
            col("NoText", DataType::String, 72),
            col("NoReal", DataType::Double, 73),
        ],
    ));
    let rows = 6;
    let data = StructuralData {
        tables: vec![TableValues {
            name: "Blanks".into(),
            columns: vec![
                values("Key", (1..=rows).map(Cell::Int).collect()),
                values("NoInt", vec![Cell::Null; 6]),
                values("NoText", vec![Cell::Null; 6]),
                values("NoReal", vec![Cell::Null; 6]),
            ],
        }],
        columns: vec![],
    };
    let p = write(&pkg, &m, &data);
    let e = editor(&p);
    for c in ["NoInt", "NoText", "NoReal"] {
        assert_eq!(e.column_values("Blanks", c).unwrap(), vec![Cell::Null; 6], "{c}");
    }
    assert_eq!(
        e.column_values("Blanks", "Key").unwrap(),
        (1..=rows).map(Cell::Int).collect::<Vec<_>>()
    );
    let back = read_model(&p).unwrap();
    assert!(e.changes(&back).is_empty());
}

/// (system table, column, decoded values).
type SystemColumn = (String, String, Vec<u32>);

/// Every metadata row of `table` in `pkg`'s `metadata.sqlitedb`, and the decoded values of the
/// columns of the system tables whose name starts with `prefix`.
fn storage(pkg: &Package, table: &str, prefix: &str) -> (Vec<bicraft_metadata::MetaRow>, Vec<SystemColumn>) {
    use bicraft_metadata::MetaValue;
    let abf = bicraft_xpress9::decompress_datamodel(&pkg.get(DATA_MODEL).unwrap().data, 1 << 30).unwrap();
    let abf = bicraft_abf::Abf::parse(&abf).unwrap();
    let t =
        bicraft_metadata::tables_from_sqlitedb_all(abf.data(abf.metadata_entry().unwrap()).unwrap()).unwrap();
    let int = |r: &bicraft_metadata::MetaRow, c: &str| match r.get(c) {
        Some(MetaValue::Integer(i)) => *i,
        _ => 0,
    };
    let text = |r: &bicraft_metadata::MetaRow, c: &str| match r.get(c) {
        Some(MetaValue::Text(s)) => s.clone(),
        _ => String::new(),
    };
    let by_id = |table: &str, id: i64| t.rows(table).iter().find(|r| int(r, "ID") == id).unwrap().clone();
    let path = |file: i64| {
        let f = by_id("StorageFile", file);
        let folder = text(&by_id("StorageFolder", int(&f, "StorageFolderID")), "Path");
        format!("{folder}/{}", text(&f, "FileName")).replace('\\', "/")
    };
    let mut system = Vec::new();
    for st in t
        .rows("Table")
        .iter()
        .filter(|r| text(r, "Name").starts_with(prefix))
    {
        for c in t
            .rows("Column")
            .iter()
            .filter(|c| int(c, "TableID") == int(st, "ID"))
        {
            let cps = t
                .rows("ColumnPartitionStorage")
                .iter()
                .find(|r| int(r, "ColumnStorageID") == int(c, "ColumnStorageID"))
                .unwrap();
            let seg = by_id("SegmentStorage", int(cps, "SegmentStorageID"));
            let idf = abf.read(&path(int(cps, "StorageFileID"))).unwrap();
            let meta = bicraft_vertipaq::parse_idfmeta(abf.read(&path(int(&seg, "StorageFileID"))).unwrap())
                .unwrap();
            let ids =
                bicraft_vertipaq::decode_data_ids(idf, &meta, &bicraft_vertipaq::Limits::default()).unwrap();
            system.push((text(st, "Name"), text(c, "ExplicitName"), ids));
        }
    }
    (t.rows(table).to_vec(), system)
}

fn int_of(r: &bicraft_metadata::MetaRow, c: &str) -> i64 {
    match r.get(c) {
        Some(bicraft_metadata::MetaValue::Integer(i)) => *i,
        other => panic!("{c}: {other:?}"),
    }
}

/// Many-to-many and one-to-one relationships, stored as the engine stores them (Desktop 2.158
/// engine, structural check `rels`): many-to-many without storage (no index, both ends as they
/// are, blanks and duplicates allowed), one-to-one with two indexes (`RelationshipStorage2ID`), one
/// per direction, both ends unique.
#[test]
fn synthetic_pbix_relationship_cardinalities() {
    let pkg = bicraft_pbix::synthetic::pbix(&bicraft_pbix::synthetic::storage_database().unwrap()).unwrap();
    let mut m = read_model(&pkg).unwrap();
    m.tables.push(table(
        "A",
        1,
        vec![col("Key", DataType::Int64, 1), col("V", DataType::Int64, 2)],
    ));
    m.tables.push(table("B", 2, vec![col("Key", DataType::Int64, 3)]));
    m.tables.push(table("C", 3, vec![col("Key", DataType::Int64, 4)]));
    m.tables.push(table("D", 4, vec![col("Key", DataType::Int64, 5)]));
    let mut ab = relationship("A", "B");
    ab.to_cardinality = Some("many".into());
    ab.cross_filtering_behavior = Some("bothDirections".into());
    let mut cd = relationship("C", "D");
    cd.name = "00000000-0000-4000-a000-000000000002".into();
    cd.from_cardinality = Some("one".into());
    cd.to_cardinality = Some("one".into());
    m.relationships = vec![ab, cd];
    let ints = |v: &[Option<i64>]| {
        v.iter()
            .map(|x| x.map_or(Cell::Null, Cell::Int))
            .collect::<Vec<_>>()
    };
    let data = |d: Vec<Cell>| StructuralData {
        tables: vec![
            TableValues {
                name: "A".into(),
                columns: vec![
                    values("Key", ints(&[Some(1), Some(1), Some(2), None, Some(3)])),
                    values("V", ints(&[Some(10), Some(11), Some(12), Some(13), Some(14)])),
                ],
            },
            TableValues {
                name: "B".into(),
                columns: vec![values("Key", ints(&[Some(2), Some(1), Some(2), None, Some(4)]))],
            },
            TableValues {
                name: "C".into(),
                columns: vec![values("Key", ints(&[Some(3), Some(1), Some(2), Some(4)]))],
            },
            TableValues {
                name: "D".into(),
                columns: vec![values("Key", d)],
            },
        ],
        columns: vec![],
    };
    let p = write(&pkg, &m, &data(ints(&[Some(5), Some(2), Some(3), Some(4)])));
    let back = read_model(&p).unwrap();
    assert!(editor(&p).changes(&back).is_empty());
    assert_eq!(back.relationships.len(), 2);
    let (rels, system) = storage(&p, "Relationship", "R$");
    let by_name = |n: &str| {
        rels.iter()
            .find(|r| matches!(r.get("Name"), Some(bicraft_metadata::MetaValue::Text(s)) if s == n))
            .unwrap()
    };
    let ab = by_name("00000000-0000-4000-a000-000000000001");
    assert_eq!(int_of(ab, "RelationshipStorageID"), 0);
    assert_eq!(int_of(ab, "RelationshipStorage2ID"), 0);
    assert_eq!(int_of(ab, "RefreshedTime"), 31_240_512_000_000_000);
    assert_eq!(int_of(ab, "CrossFilteringBehavior"), 2);
    let cd = by_name("00000000-0000-4000-a000-000000000002");
    assert_ne!(int_of(cd, "RelationshipStorageID"), 0);
    assert_ne!(int_of(cd, "RelationshipStorage2ID"), 0);
    // Two R$ tables, one per direction, named after the table each maps from.
    assert_eq!(system.len(), 2, "{system:?}");
    let forward = system.iter().find(|(t, _, _)| t.starts_with("R$C (")).unwrap();
    let reverse = system.iter().find(|(t, _, _)| t.starts_with("R$D (")).unwrap();
    // C keys 3,1,2,4 (hash ids 3..6) → D rows of 5,2,3,4: 3 → row 2, 1 → none, 2 → row 1, 4 → row 3.
    assert_eq!(forward.2, vec![0, 0, 0, 3, 0, 2, 4]);
    // D keys 5,2,3,4 → C rows: 5 → none, 2 → row 2, 3 → row 0, 4 → row 3.
    assert_eq!(reverse.2, vec![0, 0, 0, 0, 3, 1, 4]);
    assert_eq!(editor(&p).column_values("A", "Key").unwrap().len(), 5);
    // Removing the one-to-one relationship takes both indexes.
    let mut m2 = back.clone();
    m2.relationships.retain(|r| r.name.ends_with('1'));
    let p2 = write(&p, &m2, &StructuralData::default());
    assert!(storage(&p2, "RelationshipStorage", "R$").0.is_empty());
    assert!(storage(&p2, "Relationship", "R$").1.is_empty());
    // A one-to-one end must be unique on both sides.
    let e = editor(&pkg)
        .write(&m, &data(ints(&[Some(5), Some(5), Some(3), Some(4)])))
        .unwrap_err()
        .to_string();
    assert!(e.contains("duplicate"), "{e}");
}

/// User hierarchies: a new table's hierarchy and one added to an existing table, written with the
/// `U$` node table the engine builds (Desktop 2.158 engine, structural check `hier`: same data,
/// same nodes), then removed.
#[test]
fn synthetic_pbix_user_hierarchies() {
    let pkg = bicraft_pbix::synthetic::pbix(&bicraft_pbix::synthetic::storage_database().unwrap()).unwrap();
    let mut m = read_model(&pkg).unwrap();
    let mut g = table(
        "G",
        1,
        vec![
            col("Country", DataType::String, 1),
            col("City", DataType::String, 2),
            col("Year", DataType::Int64, 3),
        ],
    );
    let geo = serde_json::json!({"name": "Geo", "levels": [
        {"name": "Country", "ordinal": 0, "column": "Country"},
        {"name": "City", "ordinal": 1, "column": "City"}]});
    g.extra.insert("hierarchies".into(), serde_json::json!([geo]));
    m.tables.push(g);
    let t = |s: Option<&str>| s.map_or(Cell::Null, text);
    let rows = [
        (Some("France"), Some("Paris"), 2020),
        (Some("France"), Some("Lyon"), 2021),
        (Some("Spain"), Some("Madrid"), 2020),
        (Some("Spain"), Some("Lyon"), 2020),
        (None, Some("Oslo"), 2022),
        (Some("France"), Some("Paris"), 2022),
        (Some("Italy"), None, 2021),
        (Some("france"), Some("Nice"), 2021),
    ];
    let data = StructuralData {
        tables: vec![TableValues {
            name: "G".into(),
            columns: vec![
                values("Country", rows.iter().map(|r| t(r.0)).collect()),
                values("City", rows.iter().map(|r| t(r.1)).collect()),
                values("Year", rows.iter().map(|r| Cell::Int(r.2)).collect()),
            ],
        }],
        columns: vec![],
    };
    let p = write(&pkg, &m, &data);
    let back = read_model(&p).unwrap();
    assert!(
        editor(&p).changes(&back).is_empty(),
        "{:?}",
        editor(&p).changes(&back)
    );
    let (hs, system) = storage(&p, "HierarchyStorage", "U$");
    assert_eq!(hs.len(), 1);
    let def = match hs[0].get("LevelDefinition") {
        Some(bicraft_metadata::MetaValue::Text(s)) => s.clone(),
        other => panic!("{other:?}"),
    };
    assert!(
        def.starts_with("$Country (") && def.contains(")$0$City (") && def.ends_with(")$4$"),
        "{def}"
    );
    let col = |n: &str| system.iter().find(|(_, c, _)| c == n).unwrap().2.clone();
    // The engine's nodes for these rows: countries (blank, France, Italy, Spain), then their cities.
    assert_eq!(col("MULTI_LEVEL_ID"), vec![2, 3, 5, 4, 6, 4, 7, 3, 2, 4, 5]);
    assert_eq!(col("PARENT_POS"), vec![0, 0, 0, 0, 1, 2, 2, 2, 3, 4, 4]);
    assert_eq!(col("FIRST_CHILD_POS"), vec![5, 6, 9, 10, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(col("CHILD_COUNT"), vec![2, 4, 2, 3, 0, 0, 0, 0, 0, 0, 0]);

    // A hierarchy added to the stored table (levels read from the stored H$ tables), with a
    // level on a value-encoded column.
    let mut m2 = back.clone();
    let g2 = m2.tables.iter_mut().find(|t| t.name == "G").unwrap();
    let hs2 = g2.extra.get_mut("hierarchies").unwrap().as_array_mut().unwrap();
    hs2.push(serde_json::json!({"name": "Time", "levels": [
        {"name": "Y", "ordinal": 0, "column": "Year"},
        {"name": "Town", "ordinal": 1, "column": "City"}]}));
    let ch = editor(&p).changes(&m2);
    assert_eq!(ch.added_hierarchies, vec![("G".to_string(), "Time".to_string())]);
    let p2 = write(&p, &m2, &StructuralData::default());
    let back2 = read_model(&p2).unwrap();
    assert!(editor(&p2).changes(&back2).is_empty());
    let (hs, system) = storage(&p2, "HierarchyStorage", "U$");
    assert_eq!(hs.len(), 2);
    assert_eq!(system.len(), 8);
    // Years 2020, 2021, 2022, then their towns (3, 3, 2): 3 + 8 nodes.
    let time = system
        .iter()
        .find(|(t, c, _)| t.contains("Time") && c == "PARENT_POS")
        .unwrap();
    assert_eq!(time.2, vec![0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 3]);

    // Removing a hierarchy takes its U$ table; restructuring one rebuilds it.
    let mut m3 = back2.clone();
    let g3 = m3.tables.iter_mut().find(|t| t.name == "G").unwrap();
    g3.extra.insert(
        "hierarchies".into(),
        serde_json::json!([{"name": "Geo", "levels": [
        {"name": "Town", "ordinal": 0, "column": "City"}]}]),
    );
    let ch = editor(&p2).changes(&m3);
    assert_eq!(ch.removed_hierarchies.len(), 2);
    assert_eq!(ch.added_hierarchies, vec![("G".to_string(), "Geo".to_string())]);
    let p3 = write(&p2, &m3, &StructuralData::default());
    let back3 = read_model(&p3).unwrap();
    assert!(editor(&p3).changes(&back3).is_empty());
    let (hs, system) = storage(&p3, "HierarchyStorage", "U$");
    assert_eq!(hs.len(), 1);
    assert_eq!(system.len(), 4);
    let abf = bicraft_xpress9::decompress_datamodel(&p3.get(DATA_MODEL).unwrap().data, 1 << 30).unwrap();
    let abf = bicraft_abf::Abf::parse(&abf).unwrap();
    assert_eq!(abf.files().iter().filter(|f| f.path.contains("U$")).count(), 8);

    // A level on a column without an attribute hierarchy is refused.
    let mut bad = back3.clone();
    let gb = bad.tables.iter_mut().find(|t| t.name == "G").unwrap();
    gb.columns
        .iter_mut()
        .find(|c| c.name == "Year")
        .unwrap()
        .extra
        .insert("isAvailableInMdx".into(), serde_json::json!(false));
    gb.extra.insert(
        "hierarchies".into(),
        serde_json::json!([{"name": "Y", "levels": [
        {"name": "Y", "ordinal": 0, "column": "Year"}]}]),
    );
    assert!(editor(&p3).write(&bad, &StructuralData::default()).is_err());
}

/// A new table of two partitions (Desktop 2.158 engine, structural check `parts`): one segment
/// file pair per column and partition named after the partition's position, which is also its
/// `PartitionStorage.DataObjectId` (the engine refuses the image otherwise), row numbers
/// continuing across partitions, one dictionary per column.
#[test]
fn synthetic_pbix_table_of_several_partitions() {
    use bicraft_pbix::structural::PartitionRows;
    let pkg = bicraft_pbix::synthetic::pbix(&bicraft_pbix::synthetic::storage_database().unwrap()).unwrap();
    let mut m = read_model(&pkg).unwrap();
    let mut t = table(
        "P",
        9,
        vec![col("K", DataType::Int64, 90), col("S", DataType::String, 91)],
    );
    let mut p2 = t.partitions[0].clone();
    p2.name = "P2".into();
    p2.expression = Some("#table({}, {}) // P2".into());
    t.partitions.push(p2);
    m.tables.push(t);
    let data = StructuralData {
        tables: vec![TableValues {
            name: "P".into(),
            columns: vec![
                values("K", (1..=5).map(Cell::Int).collect()),
                values("S", ["a", "b", "a", "c", "d"].into_iter().map(text).collect()),
            ],
        }],
        columns: vec![],
    };
    let ed = editor(&pkg);
    let e = ed.write(&m, &data).unwrap_err().to_string();
    assert!(e.contains("rows of each"), "{e}");
    let rows = |r: Vec<u64>| {
        [PartitionRows {
            table: "P".into(),
            rows: r,
        }]
    };
    assert!(
        ed.write_partitioned(&m, &data, &rows(vec![2, 2]), Some(NOW))
            .is_err()
    );
    let bytes = ed
        .write_partitioned(&m, &data, &rows(vec![2, 3]), Some(NOW + 10))
        .unwrap();
    let mut p = pkg.clone();
    p.put(DATA_MODEL, bytes, true);
    let back = read_model(&p).unwrap();
    assert_eq!(back.table("P").unwrap().partitions.len(), 2);
    assert!(editor(&p).changes(&back).is_empty());
    assert_eq!(
        editor(&p).column_values("P", "K").unwrap(),
        (1..=5).map(Cell::Int).collect::<Vec<_>>()
    );
    let (storages, _) = storage(&p, "PartitionStorage", "");
    let mut positions: Vec<(i64, i64)> = storages
        .iter()
        .filter(|r| matches!(r.get("Name"), Some(bicraft_metadata::MetaValue::Text(n)) if n.starts_with('P')))
        .map(|r| (int_of(r, "StoragePosition"), int_of(r, "DataObjectId")))
        .collect();
    positions.sort_unstable();
    assert_eq!(positions, vec![(0, 0), (1, 1)]);
    let abf = bicraft_xpress9::decompress_datamodel(&p.get(DATA_MODEL).unwrap().data, 1 << 30).unwrap();
    let abf = bicraft_abf::Abf::parse(&abf).unwrap();
    let names: Vec<&str> = abf
        .files()
        .iter()
        .map(|f| f.path.as_str())
        .filter(|f| f.contains(".S ("))
        .collect();
    assert!(
        names.iter().any(|f| f.ends_with(".0.idf")) && names.iter().any(|f| f.ends_with(".1.idf")),
        "{names:?}"
    );
    assert_eq!(names.iter().filter(|f| f.ends_with(".dictionary")).count(), 1);
    // Row numbers continue: the second partition's row-number segment starts at row 2.
    assert_eq!(
        editor(&p)
            .column_values("P", bicraft_metadata::ROW_NUMBER_NAME)
            .unwrap(),
        (0..5).map(Cell::Int).collect::<Vec<_>>()
    );
}
