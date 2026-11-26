//! Engine-verified fixtures (`tests/engine`, see `SOURCE.txt` there and
//! `docs/formats/metadata-sqlitedb.md`, "Engine oracle").
//!
//! Each `<case>.sqlitedb` was written by the Analysis Services engine of Power BI Desktop from our
//! own `<case>.bim`; `<case>.model.json` is bicraft's reading of it, checked equal to the model
//! the engine scripts with TOM. Each `<case>.<edit>.request.json` is an edited model, and
//! `<case>.<edit>.expected.json` bicraft's reading of what `write_sqlitedb_at` wrote for it, checked
//! equal to what the engine scripts after loading the written file. These tests replay both
//! directions without the engine.

use std::path::PathBuf;

use bicraft_metadata::{
    MetaValue, build_model, encrypted_secrets, read_sqlitedb, tables_from_sqlitedb, write_sqlitedb_at,
};
use bicraft_model::SemanticModel;

/// The edit time the metadata oracle writes with (FILETIME ticks).
const FIXED_TIME: i64 = 134_000_000_000_000_000;

const CASES: &[(&str, &[&str])] = &[
    ("objects", &["add_and_change", "remove", "calendars"]),
    ("sources", &["sources"]),
    ("properties", &["aggregations"]),
    ("variations", &["variation_props", "variation_rename"]),
    ("preview", &["preview_change", "preview_remove"]),
];

fn path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("engine")
        .join(name)
}

fn model(name: &str) -> SemanticModel {
    let text = std::fs::read_to_string(path(name)).unwrap();
    serde_json::from_str(&text).unwrap()
}

#[test]
fn engine_written_metadata_reads_as_the_engine_scripts_it() {
    for (case, _) in CASES {
        let bytes = std::fs::read(path(&format!("{case}.sqlitedb"))).unwrap();
        let got = read_sqlitedb(&bytes).unwrap();
        assert_eq!(got, model(&format!("{case}.model.json")), "{case}");
    }
}

#[test]
fn an_unchanged_model_writes_the_same_bytes() {
    for (case, _) in CASES {
        let bytes = std::fs::read(path(&format!("{case}.sqlitedb"))).unwrap();
        let m = read_sqlitedb(&bytes).unwrap();
        assert_eq!(
            write_sqlitedb_at(&bytes, &m, FIXED_TIME).unwrap(),
            bytes,
            "{case}"
        );
        // and the rows apply onto themselves unchanged
        let tables = tables_from_sqlitedb(&bytes).unwrap();
        assert_eq!(
            bicraft_metadata::apply_model(&tables, &m).unwrap(),
            tables,
            "{case}"
        );
    }
}

#[test]
fn edits_write_what_the_engine_loaded() {
    for (case, edits) in CASES {
        let bytes = std::fs::read(path(&format!("{case}.sqlitedb"))).unwrap();
        for edit in *edits {
            let request = model(&format!("{case}.{edit}.request.json"));
            let written = write_sqlitedb_at(&bytes, &request, FIXED_TIME)
                .unwrap_or_else(|e| panic!("{case}.{edit}: {e}"));
            let got = read_sqlitedb(&written).unwrap();
            assert_eq!(
                got,
                model(&format!("{case}.{edit}.expected.json")),
                "{case}.{edit}"
            );
            // Writing the result again changes nothing.
            assert_eq!(
                write_sqlitedb_at(&written, &got, FIXED_TIME).unwrap(),
                written,
                "{case}.{edit}"
            );
        }
    }
}

/// G16: the preview edits wrote exactly what was asked for the new kinds (sets, perspective
/// sets, binding infos, analytics AI metadata, excluded artifacts).
#[test]
fn preview_edits_write_the_requested_objects() {
    for edit in ["preview_change", "preview_remove"] {
        let mut request = model(&format!("preview.{edit}.request.json"));
        let expected = model(&format!("preview.{edit}.expected.json"));
        // `isDynamic: false` is the default, which reads back as absent (as TOM writes it).
        for t in &mut request.tables {
            if let Some(serde_json::Value::Array(sets)) = t.extra.get_mut("sets") {
                for set in sets.iter_mut().filter_map(serde_json::Value::as_object_mut) {
                    if set.get("isDynamic") == Some(&serde_json::Value::Bool(false)) {
                        set.shift_remove("isDynamic");
                    }
                }
            }
        }
        for key in [
            "perspectives",
            "bindingInfoCollection",
            "analyticsAIMetadata",
            "excludedArtifacts",
        ] {
            assert_eq!(request.extra.get(key), expected.extra.get(key), "{edit}: {key}");
        }
        assert_eq!(request.tables, expected.tables, "{edit}");
        assert_eq!(request.expressions, expected.expressions, "{edit}");
    }
}

/// G16: encrypted data-source secrets are read opaquely and kept byte for byte by every write,
/// including edits of the data sources that hold them (the engine still decrypts them after
/// loading the written file: metadata oracle, case `sources`).
#[test]
fn encrypted_secrets_survive_every_write() {
    let bytes = std::fs::read(path("sources.sqlitedb")).unwrap();
    let before = encrypted_secrets(&tables_from_sqlitedb(&bytes).unwrap());
    let found: Vec<(&str, &str)> = before
        .iter()
        .map(|s| (s.data_source.as_str(), s.property))
        .collect();
    assert_eq!(
        found,
        [("SQL/srv;db", "credential"), ("LegacyDS", "connectionString")]
    );
    for s in &before {
        assert!(s.stored.starts_with("CAAAAAAAAA"), "{}", s.stored);
    }
    let request = model("sources.sources.request.json");
    let written = write_sqlitedb_at(&bytes, &request, FIXED_TIME).unwrap();
    assert_eq!(
        encrypted_secrets(&tables_from_sqlitedb(&written).unwrap()),
        before
    );
    // The model never shows them.
    let text = serde_json::to_string(&read_sqlitedb(&written).unwrap()).unwrap();
    assert!(!text.contains("CAAAAAAAAA"));
    assert!(!text.contains("connectionString") && !text.contains("\"credential\""));
}

/// G16: Direct Lake partitions. Desktop's engine refuses them, so this flips our own engine-written
/// DirectQuery entity partition (same rows otherwise: `Type` 5, `QueryDefinition` = entity name,
/// `ExpressionSourceID`, `SchemaName`) to `Mode` 5, as TOM describes Direct Lake.
#[test]
fn direct_lake_partitions_read_and_survive_metadata_edits() {
    let bytes = std::fs::read(path("preview.sqlitedb")).unwrap();
    let mut tables = tables_from_sqlitedb(&bytes).unwrap();
    let mut rows = tables.rows("Partition").to_vec();
    let row = rows
        .iter_mut()
        .find(|r| r.get("Name") == Some(&MetaValue::Text("Sales".into())))
        .unwrap();
    assert_eq!(row.get("Mode"), Some(&MetaValue::Integer(1)));
    row.insert("Mode".into(), MetaValue::Integer(5));
    tables.insert_table("Partition", rows);
    let m = build_model(&tables).unwrap();
    let sales = m.tables.iter().find(|t| t.name == "Sales").unwrap();
    let p = &sales.partitions[0];
    assert_eq!(p.mode.as_deref(), Some("directLake"));
    assert_eq!(p.source_type, "entity");
    assert_eq!(
        serde_json::Value::Object(p.source_extra.clone()),
        serde_json::json!({"entityName": "sales", "expressionSource": "DatabaseQuery", "schemaName": "dbo"})
    );
    assert_eq!(
        sales.extra.get("directLakeIndexingBehavior"),
        Some(&serde_json::json!("explicit"))
    );
    // Unchanged rows apply onto themselves; a metadata edit leaves the partition as it is.
    assert_eq!(bicraft_metadata::apply_model(&tables, &m).unwrap(), tables);
    let mut edited = m.clone();
    let t = edited.tables.iter_mut().find(|t| t.name == "Sales").unwrap();
    t.measures[0].description = Some("direct lake total".into());
    let out = bicraft_metadata::apply_model_at(&tables, &edited, FIXED_TIME).unwrap();
    assert_eq!(out.rows("Partition"), tables.rows("Partition"));
    assert_eq!(build_model(&out).unwrap(), edited);
}

/// G16 regression: removing a table also removes its sets, their dependents and the perspective
/// entries that name them (the structural writer used to leave `Set` rows dangling).
#[test]
fn removing_a_table_removes_its_sets() {
    let bytes = std::fs::read(path("preview.sqlitedb")).unwrap();
    let base = bicraft_metadata::tables_from_sqlitedb_all(&bytes).unwrap();
    let mut m = read_sqlitedb(&bytes).unwrap();
    m.tables.retain(|t| t.name != "Sales");
    m.extra.shift_remove("perspectives");
    let edit = bicraft_metadata::apply_structural_at(
        &base,
        &m,
        &bicraft_metadata::EncodedData::default(),
        FIXED_TIME,
    )
    .unwrap();
    let out = &edit.tables;
    assert!(out.rows("Set").is_empty());
    assert!(out.rows("PerspectiveSet").is_empty());
    let set_dependents = |table: &str| {
        out.rows(table)
            .iter()
            .filter(|r| r.get("ObjectType") == Some(&MetaValue::Integer(38)))
            .count()
    };
    assert_eq!(set_dependents("Annotation"), 0);
    assert_eq!(set_dependents("ExtendedProperty"), 0);
    // The table's excluded artifacts went with it; the model's and the expression's stay.
    assert_eq!(out.rows("ExcludedArtifact").len(), 2);
    assert!(bicraft_metadata::dangling_references(out).is_empty());
}
