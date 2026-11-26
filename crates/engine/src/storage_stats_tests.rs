//! The storage statistics tests that need the engine's private items (`storage_stats::from_files`, `vpax::iso_utc`); the others are the `model`
//! integration test (`tests/model/storage_stats_tests.rs`), under the same module path.

use std::path::Path;

use serde_json::{Value, json};

use crate::Session;
use crate::tempdir::TempDir;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Two related tables, one with a user hierarchy, saved as a PBIX at `path`.
fn build(path: &Path) -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Stats"}));
    let rows: Vec<String> = (0..500)
        .map(|i| {
            format!(
                "{{{i}, \"{}\", {}, {}}}",
                ["red", "green", "blue"][i % 3],
                i % 7,
                (i as f64) * 1.25
            )
        })
        .collect();
    run(
        &mut s,
        "table.import",
        json!({"name": "Sales", "m": format!(
            "let Source = #table(type table [Id = Int64.Type, Color = text, Product = Int64.Type, Amount = number], {{{}}}) in Source",
            rows.join(", ")
        )}),
    );
    run(
        &mut s,
        "table.import",
        json!({"name": "Product", "m": "let Source = #table(type table [Product = Int64.Type, Name = text, Group = text], {{0, \"a\", \"x\"}, {1, \"b\", \"x\"}, {2, \"c\", \"y\"}, {3, \"d\", \"y\"}, {4, \"e\", \"z\"}, {5, \"f\", \"z\"}, {6, \"g\", \"z\"}}) in Source"}),
    );
    run(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[Product]", "to": "Product[Product]"}),
    );
    run(
        &mut s,
        "hierarchy.add",
        json!({"table": "Product", "name": "Groups", "levels": ["Group", "Name"]}),
    );
    run(&mut s, "file.save_as", json!({"path": path}));
    s
}

#[test]
fn streamed_reading_gives_the_same_statistics() {
    let dir = TempDir::new("stats-stream");
    let path = dir.join("stats.pbix");
    build(&path);
    let pkg = bicraft_opc::Package::open(&path).unwrap();
    let part = pkg.get(bicraft_pbix::DATA_MODEL).unwrap();
    let want = |f: &bicraft_abf::FileEntry| f.path.to_ascii_lowercase().ends_with(".idfmeta");
    let limits = bicraft_pbix::DataModelLimits::default();
    let memory = bicraft_pbix::storage::read_bytes(&part.data, &want, &limits).unwrap();
    assert!(!memory.streamed);
    // Limits below the backup's size force the streamed reader.
    let tiny = bicraft_pbix::DataModelLimits {
        max_bytes: 1024,
        max_ratio: 1,
        min_allowance: 0,
    };
    let streamed = bicraft_pbix::storage::read_bytes(&part.data, &want, &tiny).unwrap();
    assert!(streamed.streamed);
    assert_eq!(memory.contents, streamed.contents);
    assert_eq!(memory.sqlitedb, streamed.sqlitedb);
    let a = crate::storage_stats::from_files(&memory).unwrap();
    let mut b = crate::storage_stats::from_files(&streamed).unwrap();
    b.streamed = false;
    assert_eq!(a, b);

    // Hostile input: a truncated part is an error, not a panic.
    let cut = part.data.get(..part.data.len() / 2).unwrap();
    assert!(bicraft_pbix::storage::read_bytes(cut, &want, &tiny).is_err());
    assert!(bicraft_pbix::storage::read_bytes(cut, &want, &limits).is_err());
}

#[test]
fn iso_dates() {
    assert_eq!(crate::vpax::iso_utc(0), "1970-01-01T00:00:00Z");
    assert_eq!(crate::vpax::iso_utc(951_782_400), "2000-02-29T00:00:00Z");
    assert_eq!(crate::vpax::iso_utc(1_791_979_261), "2026-10-14T12:01:01Z");
}
