//! Real `metadata.sqlitedb` files: local only, never committed.
//!
//! Set `BICRAFT_METADATA_DB` to an extracted `metadata.sqlitedb`, or put `*.sqlitedb` files
//! under `$BICRAFT_TESTDATA/metadata/`. Without either the test prints a note and passes.

use std::path::PathBuf;

use bicraft_metadata::read_sqlitedb;

fn inputs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(p) = bicraft_env::var_os("BICRAFT_METADATA_DB") {
        out.push(PathBuf::from(p));
    }
    if let Some(root) = bicraft_env::var_os("BICRAFT_TESTDATA")
        && let Ok(entries) = std::fs::read_dir(PathBuf::from(root).join("metadata"))
    {
        let mut found: Vec<_> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("sqlitedb")))
            .collect();
        found.sort();
        out.extend(found);
    }
    out
}

#[test]
fn real_metadata_databases_map_cleanly() {
    let inputs = inputs();
    if inputs.is_empty() {
        eprintln!(
            "note: set BICRAFT_METADATA_DB (or BICRAFT_TESTDATA with metadata/*.sqlitedb) to run this test"
        );
        return;
    }
    for path in inputs {
        let Ok(bytes) = std::fs::read(&path) else {
            eprintln!("note: cannot read {}, skipped", path.display());
            continue;
        };
        let m = read_sqlitedb(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert!(!m.tables.is_empty(), "{}: no tables", path.display());
        for t in &m.tables {
            assert!(!t.name.is_empty());
            assert!(
                !(t.name.starts_with("H$") || t.name.starts_with("R$") || t.name.starts_with("U$")),
                "system table leaked"
            );
            assert!(t.columns.iter().all(|c| !c.name.is_empty()));
            assert!(t.measures.iter().all(|x| !x.name.is_empty()));
            assert!(t.partitions.iter().all(|p| !p.source_type.is_empty()));
        }
        let has = |table: &str, column: &str| {
            m.table(table)
                .is_some_and(|t| t.columns.iter().any(|c| c.name.eq_ignore_ascii_case(column)))
        };
        for r in &m.relationships {
            assert!(
                has(&r.from_table, &r.from_column) && has(&r.to_table, &r.to_column),
                "relationship endpoints must resolve"
            );
        }
        let json = serde_json::to_string(&m).unwrap();
        assert_eq!(
            serde_json::from_str::<bicraft_model::SemanticModel>(&json).unwrap(),
            m
        );
        let columns: usize = m.tables.iter().map(|t| t.columns.len()).sum();
        let measures: usize = m.tables.iter().map(|t| t.measures.len()).sum();
        eprintln!(
            "{}: {} tables, {columns} columns, {measures} measures, {} relationships",
            path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default(),
            m.tables.len(),
            m.relationships.len()
        );
    }
}

/// `apply_model` on real files: identity, one added measure, and the SQLite row adapter. Prints
/// only shapes and the values bicraft chose, never the file's own content.
#[test]
fn real_metadata_databases_apply_cleanly() {
    use bicraft_metadata::{MetaValue, apply_model_at, build_model, tables_from_sqlitedb, tables_to_rows};

    const NOW: i64 = 134_116_992_000_000_000;
    let inputs = inputs();
    if inputs.is_empty() {
        eprintln!(
            "note: set BICRAFT_METADATA_DB (or BICRAFT_TESTDATA with metadata/*.sqlitedb) to run this test"
        );
        return;
    }
    for path in inputs {
        let Ok(bytes) = std::fs::read(&path) else {
            eprintln!("note: cannot read {}, skipped", path.display());
            continue;
        };
        let base = tables_from_sqlitedb(&bytes).unwrap();
        let model = build_model(&base).unwrap();
        assert_eq!(apply_model_at(&base, &model, NOW).unwrap(), base, "identity");

        // The adapter reproduces every stored row of every loaded table.
        let db = bicraft_sqlite::Database::from_bytes(&bytes).unwrap();
        for info in db.tables() {
            let original = db.rows_vec(&info.name).unwrap();
            let rows = tables_to_rows(&base);
            let got = if info.columns.iter().any(|c| c.rowid_alias) {
                rows.rows_for(info).unwrap()
            } else {
                rows.rows_for_keyed(info, &original).unwrap()
            };
            if let Some(got) = got {
                assert_eq!(got, original, "rows of a loaded table");
            }
        }

        // Add `[bicraft-cli test] = 1` to the first table that has measures (else the first table).
        let mut edited = model.clone();
        let target = edited
            .tables
            .iter()
            .position(|t| !t.measures.is_empty())
            .unwrap_or(0);
        edited.tables[target].measures.push(bicraft_model::Measure {
            name: "bicraft test".into(),
            expression: "1".into(),
            ..Default::default()
        });
        let out = apply_model_at(&base, &edited, NOW).unwrap();
        let changed = tables_to_rows(&out).changed_tables(&base);
        assert!(
            changed.iter().all(|t| ["Measure", "DBPROPERTIES"].contains(t)),
            "only Measure and the ID counter change: {changed:?}"
        );
        let old = base.rows("Measure");
        let new = out.rows("Measure");
        assert_eq!(new.len(), old.len() + 1);
        assert_eq!(&new[..old.len()], old, "existing measure rows untouched");
        let added = &new[old.len()];
        let id_of = |r: &bicraft_metadata::MetaRow| match r.get("ID") {
            Some(MetaValue::Integer(i)) => Some(*i),
            _ => None,
        };
        let maxid = |t: &bicraft_metadata::MetadataTables| {
            t.rows("DBPROPERTIES")
                .iter()
                .find(|r| r.get("NAME") == Some(&MetaValue::from("MAXID")))
                .and_then(|r| match r.get("VALUE") {
                    Some(MetaValue::Integer(i)) => Some(*i),
                    _ => None,
                })
        };
        let new_id = id_of(added).unwrap();
        if let Some(before) = maxid(&base) {
            assert_eq!(new_id, before + 1, "IDs continue the DBPROPERTIES.MAXID counter");
            assert_eq!(maxid(&out), Some(new_id));
        }
        let back = build_model(&out).unwrap();
        let (table, m) = back.measure("bicraft test").unwrap();
        assert_eq!(
            (table.name.as_str(), m.expression.as_str()),
            (edited.tables[target].name.as_str(), "1")
        );

        eprintln!("new Measure row, column by column (existing rows summarized, not shown):");
        for (col, v) in added {
            let existing: Vec<&MetaValue> = old.iter().filter_map(|r| r.get(col)).collect();
            let uniform = existing.first().filter(|f| existing.iter().all(|x| x == *f));
            let shape = |v: &MetaValue| match v {
                MetaValue::Null => "NULL",
                MetaValue::Integer(_) => "integer",
                MetaValue::Real(_) => "real",
                MetaValue::Text(_) => "text",
                MetaValue::Blob(_) => "blob",
            };
            let ours = match col.as_str() {
                "Name"
                | "Expression"
                | "DataType"
                | "State"
                | "KPIID"
                | "IsSimpleMeasure"
                | "IsHidden"
                | "DetailRowsDefinitionID"
                | "FormatStringDefinitionID"
                | "ErrorMessage"
                | "Description"
                | "FormatString"
                | "DisplayFolder"
                | "DataCategory"
                | "SourceLineageTag" => format!("{v:?}"),
                "ModifiedTime" | "StructureModifiedTime" if *v == MetaValue::Integer(NOW) => "NOW".into(),
                "ID" => "MAXID+1".into(),
                _ => shape(v).into(),
            };
            let cmp = match uniform {
                Some(u) if *u == v => "equals the value all existing rows share".to_string(),
                Some(u) => format!("existing rows all hold another {}", shape(u)),
                None => format!(
                    "existing rows vary ({} rows; kinds {})",
                    existing.len(),
                    existing
                        .iter()
                        .map(|x| shape(x))
                        .collect::<std::collections::BTreeSet<_>>()
                        .into_iter()
                        .collect::<Vec<_>>()
                        .join("/")
                ),
            };
            eprintln!("  {col}: {ours} — {cmp}");
        }
    }
}
