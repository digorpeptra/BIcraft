//! Structural saves of a real Desktop PBIX (`bicraft_pbix::structural`): local only.
//!
//! Reads `$BICRAFT_TESTDATA/sample1/report.pbix`, applies structural edits (a new import table
//! with a relationship to an existing table and a measure, a removed column, a calculated
//! table, renames, replaced data), writes each result, reads it back and decodes the new
//! columns. Outputs go to `$BICRAFT_TESTDATA/m5/struct_*.pbix`. Without the variable or the
//! file the test prints a note and passes. Only counts and paths are printed, never content.

use std::path::{Path, PathBuf};

use bicraft_model::{Column, DataType, Measure, Partition, Relationship, SemanticModel, Table};
use bicraft_opc::Package;
use bicraft_pbix::structural::{
    Cell, ColumnValues, DataModelEditor, NewColumnValues, StructuralData, TableValues,
};
use bicraft_pbix::{DATA_MODEL, DataModelLimits, read_model};

const SAMPLE: &str = "sample1/report.pbix";
const NOW: i64 = 134_000_000_000_000_000;

fn sample() -> Option<(PathBuf, PathBuf)> {
    let Some(dir) = bicraft_env::var_os("BICRAFT_TESTDATA").map(PathBuf::from) else {
        eprintln!("note: BICRAFT_TESTDATA is not set; skipping the real structural checks");
        return None;
    };
    let file = dir.join(SAMPLE);
    if !file.exists() {
        eprintln!("note: {SAMPLE} not found under BICRAFT_TESTDATA; skipping");
        return None;
    }
    Some((file, dir.join("m5")))
}

fn editor(pkg: &Package) -> DataModelEditor {
    let part = &pkg.get(DATA_MODEL).expect("DataModel").data;
    DataModelEditor::open(part, &DataModelLimits::default()).expect("open")
}

/// Writes `model` + `data` into `pkg`, saves it as `out/name`, reopens it.
fn write(pkg: &Package, model: &SemanticModel, data: &StructuralData, out: &Path, name: &str) -> Package {
    let bytes = editor(pkg)
        .write_at(model, data, Some(NOW))
        .expect("structural write");
    let mut next = pkg.clone();
    next.put(DATA_MODEL, bytes, true);
    let path = out.join(name);
    std::fs::write(&path, next.to_bytes().expect("zip")).expect("write");
    println!("wrote {}", path.display());
    Package::open(&path).expect("reopen")
}

fn int_col(name: &str) -> Column {
    Column {
        name: name.into(),
        data_type: DataType::Int64,
        source_column: Some(name.into()),
        lineage_tag: Some(format!("00000000-0000-4000-8000-{:012x}", name.len())),
        ..Default::default()
    }
}

#[test]
fn real_pbix_structural_saves() {
    let Some((file, out)) = sample() else { return };
    std::fs::create_dir_all(&out).expect("create output folder");
    let pkg = Package::open(&file).expect("open sample");
    let ed = editor(&pkg);
    let base = ed.base().clone();

    // A "one" end of an existing relationship, hash-encoded and unique: the key of the new table.
    let rel = base
        .relationships
        .iter()
        .find(|r| {
            r.is_active
                && base
                    .table(&r.to_table)
                    .and_then(|t| t.columns.iter().find(|c| c.name == r.to_column))
                    .is_some_and(|c| matches!(c.data_type, DataType::Int64 | DataType::String))
        })
        .expect("a relationship to an int or text key");
    let (dim, key) = (rel.to_table.clone(), rel.to_column.clone());
    let key_type = base
        .table(&dim)
        .and_then(|t| t.columns.iter().find(|c| c.name == key))
        .map(|c| c.data_type)
        .expect("key column");
    let keys: Vec<Cell> = ed
        .column_values(&dim, &key)
        .expect("decode the key")
        .into_iter()
        .filter(|v| *v != Cell::Null)
        .take(5)
        .collect();
    assert!(!keys.is_empty());
    println!("dimension key: {} values sampled", keys.len());

    // 1. New import table + relationship to the dimension + measure.
    let rows = 40usize;
    let fact_keys: Vec<Cell> = (0..rows).map(|i| keys[i % keys.len()].clone()).collect();
    let amounts: Vec<Cell> = (0..rows).map(|i| Cell::Int(i as i64 + 1)).collect();
    let mut m1 = base.clone();
    let mut key_col = int_col("Key");
    key_col.data_type = key_type;
    let mut fact = Table {
        name: "bicraft Fact".into(),
        lineage_tag: Some("11111111-0000-4000-8000-000000000001".into()),
        columns: vec![key_col, int_col("Amount")],
        partitions: vec![Partition {
            name: "bicraft Fact".into(),
            mode: Some("import".into()),
            source_type: "m".into(),
            expression: Some("#table({\"Key\", \"Amount\"}, {})".into()),
            ..Default::default()
        }],
        ..Default::default()
    };
    fact.measures.push(Measure {
        name: "bicraft Total".into(),
        expression: "SUM('bicraft Fact'[Amount])".into(),
        ..Default::default()
    });
    m1.tables.push(fact);
    m1.relationships.push(Relationship {
        name: "11111111-0000-4000-8000-0000000000aa".into(),
        from_table: "bicraft Fact".into(),
        from_column: "Key".into(),
        to_table: dim.clone(),
        to_column: key.clone(),
        is_active: true,
        ..Default::default()
    });
    let data1 = StructuralData {
        tables: vec![TableValues {
            name: "bicraft Fact".into(),
            columns: vec![
                ColumnValues {
                    name: "Key".into(),
                    values: fact_keys.clone(),
                },
                ColumnValues {
                    name: "Amount".into(),
                    values: amounts.clone(),
                },
            ],
        }],
        columns: vec![],
    };
    let p1 = write(&pkg, &m1, &data1, &out, "struct_add_table.pbix");
    let back = read_model(&p1).expect("read back");
    assert!(back.table("bicraft Fact").is_some());
    assert!(back.relationships.iter().any(|r| r.from_table == "bicraft Fact"));
    assert!(back.measure("bicraft Total").is_some());
    let e1 = editor(&p1);
    assert_eq!(e1.column_values("bicraft Fact", "Key").unwrap(), fact_keys);
    assert_eq!(e1.column_values("bicraft Fact", "Amount").unwrap(), amounts);
    assert!(e1.changes(&back).is_empty());

    // 2. A calculated column added to the new table, evaluated by the caller.
    let mut m2 = back.clone();
    let t = m2.tables.iter_mut().find(|t| t.name == "bicraft Fact").unwrap();
    t.columns.push(Column {
        name: "Double".into(),
        data_type: DataType::Int64,
        expression: Some("'bicraft Fact'[Amount] * 2".into()),
        lineage_tag: Some("11111111-0000-4000-8000-000000000002".into()),
        ..Default::default()
    });
    let doubled: Vec<Cell> = (0..rows).map(|i| Cell::Int(2 * (i as i64 + 1))).collect();
    let data2 = StructuralData {
        tables: vec![],
        columns: vec![NewColumnValues {
            table: "bicraft Fact".into(),
            column: "Double".into(),
            values: doubled.clone(),
        }],
    };
    let p2 = write(&p1, &m2, &data2, &out, "struct_add_column.pbix");
    assert_eq!(
        editor(&p2).column_values("bicraft Fact", "Double").unwrap(),
        doubled
    );

    // 3. Renames: the new table and one of its columns.
    let mut m3 = read_model(&p2).unwrap();
    for r in &mut m3.relationships {
        if r.from_table == "bicraft Fact" {
            r.from_table = "bicraft Sales".into();
        }
    }
    let t = m3.tables.iter_mut().find(|t| t.name == "bicraft Fact").unwrap();
    t.name = "bicraft Sales".into();
    t.columns.iter_mut().find(|c| c.name == "Amount").unwrap().name = "Qty".into();
    let ch = editor(&p2).changes(&m3);
    assert_eq!(ch.renamed_tables.len(), 1);
    assert_eq!(ch.renamed_columns.len(), 1);
    let p3 = write(&p2, &m3, &StructuralData::default(), &out, "struct_rename.pbix");
    let back3 = read_model(&p3).unwrap();
    assert!(
        back3
            .table("bicraft Sales")
            .and_then(|t| t.columns.iter().find(|c| c.name == "Qty"))
            .is_some()
    );
    assert_eq!(
        editor(&p3).column_values("bicraft Sales", "Qty").unwrap(),
        amounts
    );

    // 4. Replaced data (two stages: the relationship is rebuilt).
    let mut m4 = back3.clone();
    let t = m4.tables.iter_mut().find(|t| t.name == "bicraft Sales").unwrap();
    t.partitions[0].expression = Some("#table({\"Key\", \"Qty\"}, {})".into());
    let rows4 = 7usize;
    let k4: Vec<Cell> = (0..rows4).map(|i| keys[(i + 1) % keys.len()].clone()).collect();
    let q4: Vec<Cell> = (0..rows4).map(|i| Cell::Int(100 + i as i64)).collect();
    let d4: Vec<Cell> = q4
        .iter()
        .map(|v| match v {
            Cell::Int(n) => Cell::Int(2 * n),
            o => o.clone(),
        })
        .collect();
    let data4 = StructuralData {
        tables: vec![TableValues {
            name: "bicraft Sales".into(),
            columns: vec![
                ColumnValues {
                    name: "Key".into(),
                    values: k4.clone(),
                },
                ColumnValues {
                    name: "Qty".into(),
                    values: q4.clone(),
                },
                ColumnValues {
                    name: "Double".into(),
                    values: d4,
                },
            ],
        }],
        columns: vec![],
    };
    assert_eq!(
        editor(&p3).changes(&m4).replaced_tables,
        vec!["bicraft Sales".to_string()]
    );
    let p4 = write(&p3, &m4, &data4, &out, "struct_replace.pbix");
    let back4 = read_model(&p4).unwrap();
    assert!(
        back4
            .relationships
            .iter()
            .any(|r| r.from_table == "bicraft Sales")
    );
    assert!(back4.measure("bicraft Total").is_some());
    assert_eq!(editor(&p4).column_values("bicraft Sales", "Qty").unwrap(), q4);

    // 5. Remove a column: the first one the writer accepts.
    let mut removed = None;
    'outer: for t in &base.tables {
        for c in &t.columns {
            let used = base.relationships.iter().any(|r| {
                (r.from_table == t.name && r.from_column == c.name)
                    || (r.to_table == t.name && r.to_column == c.name)
            });
            if used
                || c.extra.contains_key("variations")
                || t.columns
                    .iter()
                    .any(|o| o.sort_by_column.as_deref() == Some(&c.name))
            {
                continue;
            }
            let mut m5 = base.clone();
            m5.tables
                .iter_mut()
                .find(|x| x.name == t.name)
                .unwrap()
                .columns
                .retain(|x| x.name != c.name);
            if editor(&pkg)
                .write_at(&m5, &StructuralData::default(), Some(NOW))
                .is_ok()
            {
                removed = Some((t.name.clone(), c.name.clone(), m5));
                break 'outer;
            }
        }
    }
    let (rt, rc, m5) = removed.expect("a removable column");
    let p5 = write(
        &pkg,
        &m5,
        &StructuralData::default(),
        &out,
        "struct_remove_column.pbix",
    );
    assert!(
        read_model(&p5)
            .unwrap()
            .table(&rt)
            .unwrap()
            .columns
            .iter()
            .all(|c| c.name != rc)
    );

    // 6. Calculated table with supplied (evaluated) data.
    let mut m6 = base.clone();
    let calc_col = |name: &str, t: DataType| {
        let mut c = int_col(name);
        c.data_type = t;
        c.source_column = Some(format!("[{name}]"));
        c.extra
            .insert("type".into(), serde_json::json!("calculatedTableColumn"));
        c.extra.insert("isNameInferred".into(), serde_json::json!(true));
        c.extra
            .insert("isDataTypeInferred".into(), serde_json::json!(true));
        c
    };
    m6.tables.push(Table {
        name: "bicraft Calc".into(),
        lineage_tag: Some("11111111-0000-4000-8000-000000000003".into()),
        columns: vec![
            calc_col("N", DataType::Int64),
            calc_col("Label", DataType::String),
        ],
        partitions: vec![Partition {
            name: "bicraft Calc".into(),
            mode: Some("import".into()),
            source_type: "calculated".into(),
            expression: Some("DATATABLE(\"N\", INTEGER, \"Label\", STRING, {{1, \"a\"}, {2, \"b\"}})".into()),
            ..Default::default()
        }],
        ..Default::default()
    });
    let data6 = StructuralData {
        tables: vec![TableValues {
            name: "bicraft Calc".into(),
            columns: vec![
                ColumnValues {
                    name: "N".into(),
                    values: vec![Cell::Int(1), Cell::Int(2)],
                },
                ColumnValues {
                    name: "Label".into(),
                    values: vec![Cell::Text("a".into()), Cell::Text("b".into())],
                },
            ],
        }],
        columns: vec![],
    };
    let p6 = write(&pkg, &m6, &data6, &out, "struct_calc_table.pbix");
    assert_eq!(
        editor(&p6).column_values("bicraft Calc", "Label").unwrap(),
        vec![Cell::Text("a".into()), Cell::Text("b".into())]
    );

    // Refusals: missing data, duplicate keys on the "one" side.
    let mut bad = base.clone();
    bad.tables.push(Table {
        name: "bicraft Bad".into(),
        columns: vec![int_col("K")],
        partitions: m6.tables.last().unwrap().partitions.clone(),
        ..Default::default()
    });
    assert!(editor(&pkg).write(&bad, &StructuralData::default()).is_err());
}
