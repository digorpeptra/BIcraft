//! G1 leftovers through the commands: user hierarchies, data columns added to import tables,
//! tables of several partitions and calculated-table columns removed, saved into a PBIX.

use super::*;
use crate::tempdir::TempDir;
use bicraft_model::Partition;
use bicraft_pbix::structural::{ColumnValues, PartitionRows, StructuralData, TableValues};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn open(path: &Path) -> Session {
    let mut s = Session::new();
    run(&mut s, "file.open", json!({ "path": path }));
    assert_eq!(s.model_error, None);
    s
}

fn editor(path: &Path) -> DataModelEditor {
    let pkg = Package::open(path).unwrap();
    DataModelEditor::open(
        &pkg.get(bicraft_pbix::DATA_MODEL).unwrap().data,
        &Default::default(),
    )
    .unwrap()
}

fn stored(path: &Path, table: &str, column: &str) -> Vec<CellValue> {
    editor(path).column_values(table, column).unwrap()
}

/// Logical paths of the files of the PBIX's backup.
fn files(path: &Path) -> Vec<String> {
    let pkg = Package::open(path).unwrap();
    let abf =
        bicraft_xpress9::decompress_datamodel(&pkg.get(bicraft_pbix::DATA_MODEL).unwrap().data, 1 << 30)
            .unwrap();
    bicraft_abf::Abf::parse(&abf)
        .unwrap()
        .files()
        .iter()
        .map(|f| f.path.clone())
        .collect()
}

fn synthetic(dir: &TempDir, name: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    let pkg = bicraft_pbix::synthetic::pbix(&bicraft_pbix::synthetic::storage_database().unwrap()).unwrap();
    std::fs::write(&path, pkg.to_bytes().unwrap()).unwrap();
    path
}

fn text(s: &str) -> CellValue {
    CellValue::Text(s.into())
}

#[test]
fn hierarchies_are_added_and_removed_in_a_pbix() {
    let dir = TempDir::new("engine-struct-hier");
    let path = synthetic(&dir, "Geo.pbix");
    let mut s = open(&path);
    run(
        &mut s,
        "table.import",
        json!({"name": "Geo", "csv": "Country,City,Year\nFrance,Paris,2020\nSpain,Madrid,2021\nFrance,Lyon,2020\n"}),
    );
    run(
        &mut s,
        "hierarchy.add",
        json!({"table": "Geo", "name": "Place", "levels": ["Country", {"name": "Town", "column": "City"}]}),
    );
    // Errors: unknown column, repeated column, duplicate name.
    assert!(
        s.execute(
            "hierarchy.add",
            &json!({"table": "Geo", "name": "X", "levels": ["Nope"]})
        )
        .is_err()
    );
    assert!(
        s.execute(
            "hierarchy.add",
            &json!({"table": "Geo", "name": "X", "levels": ["City", "city"]})
        )
        .is_err()
    );
    assert!(
        s.execute(
            "hierarchy.add",
            &json!({"table": "Geo", "name": "place", "levels": ["Year"]})
        )
        .is_err()
    );
    run(&mut s, "file.save", json!({"backup": false}));
    let s2 = open(&path);
    let geo = s2.model.as_ref().unwrap().table("Geo").unwrap().clone();
    let hs = geo.extra.get("hierarchies").and_then(Value::as_array).unwrap();
    assert_eq!(hs.len(), 1);
    assert_eq!(hs[0]["levels"][1]["column"], "City");
    assert_eq!(files(&path).iter().filter(|f| f.contains("U$")).count(), 8);

    // A second hierarchy on the stored table, then the first one removed.
    let mut s3 = open(&path);
    run(
        &mut s3,
        "hierarchy.add",
        json!({"table": "Geo", "name": "Time", "levels": ["Year", "City"], "hidden": true}),
    );
    run(
        &mut s3,
        "hierarchy.remove",
        json!({"table": "Geo", "name": "Place"}),
    );
    assert!(
        s3.execute("hierarchy.remove", &json!({"table": "Geo", "name": "Place"}))
            .is_err()
    );
    run(&mut s3, "file.save", json!({"backup": false}));
    let s4 = open(&path);
    let geo = s4.model.as_ref().unwrap().table("Geo").unwrap().clone();
    let hs = geo.extra.get("hierarchies").and_then(Value::as_array).unwrap();
    assert_eq!(hs.len(), 1);
    assert_eq!(hs[0]["name"], "Time");
    assert_eq!(hs[0]["isHidden"], true);
    let u: Vec<String> = files(&path).into_iter().filter(|f| f.contains("U$")).collect();
    assert_eq!(u.len(), 8);
    assert!(u.iter().all(|f| f.contains("Time")), "{u:?}");
    let back = s4.model.as_ref().unwrap().clone();
    assert!(editor(&path).changes(&back).is_empty());
}

const T_QUERY: &str = "#table(type table [K = Int64.Type, L = text, X = number], {{1, \"a\", 1.5}, {2, \"b\", 2.5}, {3, \"a\", 3.5}})";

/// A table `T` whose `m` partition returns K, L and X, stored with only K and L, its rows in
/// the order `keys` (the query's are 1, 2, 3).
fn import_table_without_x(dir: &TempDir, keys: &[i64]) -> std::path::PathBuf {
    let path = synthetic(dir, "Query.pbix");
    let pkg = Package::open(&path).unwrap();
    let part = &pkg.get(bicraft_pbix::DATA_MODEL).unwrap().data;
    let ed = DataModelEditor::open(part, &Default::default()).unwrap();
    let mut m = ed.base().clone();
    let col = |name: &str, t: DataType| Column {
        name: name.into(),
        data_type: t,
        source_column: Some(name.into()),
        lineage_tag: Some(crate::edit::new_guid()),
        ..Default::default()
    };
    m.tables.push(Table {
        name: "T".into(),
        lineage_tag: Some(crate::edit::new_guid()),
        columns: vec![col("K", DataType::Int64), col("L", DataType::String)],
        partitions: vec![Partition {
            name: "T".into(),
            mode: Some("import".into()),
            source_type: "m".into(),
            expression: Some(T_QUERY.into()),
            ..Default::default()
        }],
        ..Default::default()
    });
    let label = |k: i64| text(if k == 2 { "b" } else { "a" });
    let data = StructuralData {
        tables: vec![TableValues {
            name: "T".into(),
            columns: vec![
                ColumnValues {
                    name: "K".into(),
                    values: keys.iter().map(|&k| CellValue::Int(k)).collect(),
                },
                ColumnValues {
                    name: "L".into(),
                    values: keys.iter().map(|&k| label(k)).collect(),
                },
            ],
        }],
        columns: vec![],
    };
    let bytes = ed.write(&m, &data).unwrap();
    let mut pkg = pkg.clone();
    pkg.put(bicraft_pbix::DATA_MODEL, bytes, true);
    std::fs::write(&path, pkg.to_bytes().unwrap()).unwrap();
    path
}

#[test]
fn data_column_added_to_an_import_table_is_read_from_its_query() {
    // Stored in the query's order.
    let dir = TempDir::new("engine-struct-addcol");
    let path = import_table_without_x(&dir, &[1, 2, 3]);
    let mut s = open(&path);
    run(
        &mut s,
        "column.add",
        json!({"table": "T", "name": "X", "data_type": "double"}),
    );
    run(&mut s, "file.save", json!({"backup": false}));
    assert_eq!(
        stored(&path, "T", "X"),
        vec![CellValue::Real(1.5), CellValue::Real(2.5), CellValue::Real(3.5)]
    );
    assert!(
        editor(&path)
            .changes(open(&path).model.as_ref().unwrap())
            .is_empty()
    );

    // Stored in another order: the new values follow the stored rows.
    let dir = TempDir::new("engine-struct-addcol-order");
    let path = import_table_without_x(&dir, &[3, 1, 2]);
    let mut s = open(&path);
    run(
        &mut s,
        "column.add",
        json!({"table": "T", "name": "X", "data_type": "double"}),
    );
    run(&mut s, "file.save", json!({"backup": false}));
    assert_eq!(
        stored(&path, "T", "K"),
        vec![CellValue::Int(3), CellValue::Int(1), CellValue::Int(2)]
    );
    assert_eq!(
        stored(&path, "T", "X"),
        vec![CellValue::Real(3.5), CellValue::Real(1.5), CellValue::Real(2.5)]
    );

    // The query no longer returns the stored rows: refused, nothing written.
    let dir = TempDir::new("engine-struct-addcol-stale");
    let path = import_table_without_x(&dir, &[1, 2, 9]);
    let bytes = std::fs::read(&path).unwrap();
    let mut s = open(&path);
    run(
        &mut s,
        "column.add",
        json!({"table": "T", "name": "X", "data_type": "double"}),
    );
    let err = s.execute("file.save", &json!({})).unwrap_err();
    assert!(err.contains("no longer returns the stored rows"), "{err}");
    assert!(err.contains("table.set_data"), "{err}");
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    // A column the query does not return.
    let mut s = open(&path);
    run(
        &mut s,
        "column.add",
        json!({"table": "T", "name": "Y", "data_type": "double"}),
    );
    let err = s.execute("file.save", &json!({})).unwrap_err();
    assert!(err.contains("'T'[Y]"), "{err}");
}

fn partition(name: &str, rows: &[(i64, &str)]) -> Partition {
    let body: Vec<String> = rows.iter().map(|(k, s)| format!("{{{k}, \"{s}\"}}")).collect();
    Partition {
        name: name.into(),
        mode: Some("import".into()),
        source_type: "m".into(),
        expression: Some(format!(
            "#table(type table [K = Int64.Type, S = text, Z = Int64.Type], {{{}}})",
            body.iter()
                .zip(rows)
                .map(|(b, (k, _))| format!("{}, {}}}", b.trim_end_matches('}'), k * 10))
                .collect::<Vec<_>>()
                .join(", ")
        )),
        ..Default::default()
    }
}

/// A table of two `m` partitions: written from each partition's query, given new data when a
/// partition's query changes, and a data column added to it.
#[test]
fn tables_of_several_partitions_are_written_per_partition() {
    let dir = TempDir::new("engine-struct-parts");
    let path = synthetic(&dir, "Parts.pbix");
    let pkg = Package::open(&path).unwrap();
    let part = &pkg.get(bicraft_pbix::DATA_MODEL).unwrap().data;
    let ed = DataModelEditor::open(part, &Default::default()).unwrap();
    let mut m = ed.base().clone();
    let col = |name: &str, t: DataType| Column {
        name: name.into(),
        data_type: t,
        source_column: Some(name.into()),
        lineage_tag: Some(crate::edit::new_guid()),
        ..Default::default()
    };
    m.tables.push(Table {
        name: "P".into(),
        lineage_tag: Some(crate::edit::new_guid()),
        columns: vec![col("K", DataType::Int64), col("S", DataType::String)],
        partitions: vec![
            partition("P1", &[(1, "a"), (2, "b")]),
            partition("P2", &[(3, "a"), (4, "c"), (5, "d")]),
        ],
        ..Default::default()
    });
    let data = StructuralData {
        tables: vec![TableValues {
            name: "P".into(),
            columns: vec![
                ColumnValues {
                    name: "K".into(),
                    values: (1..=5).map(CellValue::Int).collect(),
                },
                ColumnValues {
                    name: "S".into(),
                    values: ["a", "b", "a", "c", "d"].into_iter().map(text).collect(),
                },
            ],
        }],
        columns: vec![],
    };
    // Without the rows of each partition: refused.
    assert!(ed.write(&m, &data).is_err());
    let rows = [PartitionRows {
        table: "P".into(),
        rows: vec![2, 3],
    }];
    let bytes = ed.write_partitioned(&m, &data, &rows, None).unwrap();
    let mut pkg = pkg.clone();
    pkg.put(bicraft_pbix::DATA_MODEL, bytes, true);
    std::fs::write(&path, pkg.to_bytes().unwrap()).unwrap();
    assert_eq!(
        stored(&path, "P", "K"),
        (1..=5).map(CellValue::Int).collect::<Vec<_>>()
    );
    let idf = |path: &Path| {
        files(path)
            .into_iter()
            .filter(|f| f.starts_with("P (") && f.ends_with(".idf") && !f.contains("RowNumber"))
            .count()
    };
    // Two columns, one segment file per partition.
    assert_eq!(idf(&path), 4);

    // The second partition's query changes: both partitions are written again from their queries.
    let mut s = open(&path);
    {
        let model = s.model.as_mut().unwrap();
        let p = model.tables.iter_mut().find(|t| t.name == "P").unwrap();
        assert_eq!(p.partitions.len(), 2);
        p.partitions[1] = partition("P2", &[(7, "e"), (8, "a")]);
    }
    s.dirty = true;
    run(&mut s, "file.save", json!({"backup": false}));
    assert_eq!(
        stored(&path, "P", "K"),
        [1, 2, 7, 8].into_iter().map(CellValue::Int).collect::<Vec<_>>()
    );
    assert_eq!(
        stored(&path, "P", "S"),
        ["a", "b", "e", "a"].into_iter().map(text).collect::<Vec<_>>()
    );
    let back = open(&path);
    assert_eq!(
        back.model.as_ref().unwrap().table("P").unwrap().partitions.len(),
        2
    );
    assert_eq!(idf(&path), 4);

    // A data column every partition's query returns.
    let mut s = open(&path);
    run(
        &mut s,
        "column.add",
        json!({"table": "P", "name": "Z", "data_type": "int64"}),
    );
    run(&mut s, "file.save", json!({"backup": false}));
    assert_eq!(
        stored(&path, "P", "Z"),
        [10, 20, 70, 80]
            .into_iter()
            .map(CellValue::Int)
            .collect::<Vec<_>>()
    );
    assert!(
        editor(&path)
            .changes(open(&path).model.as_ref().unwrap())
            .is_empty()
    );
}

/// Removing a column of a calculated table rewrites its expression to select the others (the
/// engine infers the columns from the expression).
#[test]
fn calculated_table_column_removal_rewrites_the_expression() {
    let dir = TempDir::new("engine-struct-calc-col");
    let path = synthetic(&dir, "Calc.pbix");
    let mut s = open(&path);
    run(
        &mut s,
        "table.import",
        json!({"name": "C", "dax": "DATATABLE(\"K\", INTEGER, \"V\", STRING, \"W\", DOUBLE, {{1, \"x\", 0.5}, {2, \"y\", 1.5}})"}),
    );
    run(&mut s, "file.save", json!({"backup": false}));
    let mut s = open(&path);
    let out = run(&mut s, "column.remove", json!({"table": "C", "name": "V"}));
    let expr = out["expression_rewritten"].as_str().unwrap().to_string();
    assert!(expr.starts_with("SELECTCOLUMNS("), "{expr}");
    run(&mut s, "file.save", json!({"backup": false}));
    let back = open(&path);
    let t = back.model.as_ref().unwrap().table("C").unwrap().clone();
    let names: Vec<&str> = t
        .columns
        .iter()
        .filter(|c| !is_row_number(c))
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(names, ["K", "W"]);
    assert_eq!(t.partitions[0].expression.as_deref(), Some(expr.as_str()));
    assert_eq!(
        stored(&path, "C", "W"),
        vec![CellValue::Real(0.5), CellValue::Real(1.5)]
    );
    assert!(editor(&path).changes(back.model.as_ref().unwrap()).is_empty());
}
