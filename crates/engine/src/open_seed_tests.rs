//! `file.open` hands the backup it decompressed (to read the model) to the first query, which
//! reads the stored data from it instead of decompressing the `DataModel` part again.

use std::path::Path;

use serde_json::{Value, json};

use crate::Session;
use crate::tempdir::TempDir;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

const QUERY: &str =
    "EVALUATE ROW(\"s\", SUM(Sales[Amount]), \"n\", COUNTROWS(Sales), \"c\", DISTINCTCOUNT(Sales[Color]))";

/// A PBIX with one 300-row table, saved at `path`.
fn build(path: &Path, rows: usize) {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Seed"}));
    let data: Vec<String> = (0..rows)
        .map(|i| format!("{{{i}, \"{}\", {}}}", ["red", "green", "blue"][i % 3], i * 2))
        .collect();
    run(
        &mut s,
        "table.import",
        json!({"name": "Sales", "m": format!(
            "let Source = #table(type table [Id = Int64.Type, Color = text, Amount = Int64.Type], {{{}}}) in Source",
            data.join(", ")
        )}),
    );
    run(&mut s, "file.save_as", json!({"path": path}));
}

fn row(v: &Value) -> Value {
    v["results"][0]["rows"][0].clone()
}

#[test]
fn the_first_query_reads_the_backup_file_open_decompressed() {
    let dir = TempDir::new("open-seed");
    let path = dir.join("seed.pbix");
    build(&path, 300);
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": path}));
    assert_eq!(super::seed_state(&s), (true, false), "file.open keeps its backup");
    let seeded = row(&run(&mut s, "dax.evaluate", json!({"query": QUERY})));
    assert_eq!(super::seed_state(&s), (false, true), "the store took it over");
    assert_eq!(seeded, json!({"[s]": 89700, "[n]": 300, "[c]": 3}));

    // the same answers when the part is decompressed again (no backup handed over)
    let mut fresh = Session::new();
    run(&mut fresh, "file.open", json!({"path": path}));
    fresh.preview.lock().seeded = None;
    assert_eq!(
        row(&run(&mut fresh, "dax.evaluate", json!({"query": QUERY}))),
        seeded
    );
    assert_eq!(super::seed_state(&fresh), (false, true));
}

/// A backup recorded for other bytes than the session's `DataModel` part is never read.
#[test]
fn a_backup_for_another_part_is_ignored() {
    let dir = TempDir::new("open-seed-other");
    let path = dir.join("seed.pbix");
    build(&path, 300);
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": path}));
    // garbage under the identity of other bytes: reading it would fail
    s.preview.seed_backup(
        &bicraft_opc::Part::new("DataModel", b"not the DataModel part".to_vec(), true),
        vec![0u8; 64],
    );
    let v = run(&mut s, "dax.evaluate", json!({"query": QUERY}));
    assert_eq!(row(&v), json!({"[s]": 89700, "[n]": 300, "[c]": 3}));
    assert_eq!(super::seed_state(&s), (false, true));
}

/// Opening another file replaces the handed-over backup: the second file's data is read.
#[test]
fn opening_another_file_reads_its_own_data() {
    let dir = TempDir::new("open-seed-two");
    let (a, b) = (dir.join("a.pbix"), dir.join("b.pbix"));
    build(&a, 300);
    build(&b, 30);
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": a}));
    run(&mut s, "file.open", json!({"path": b}));
    let v = run(&mut s, "dax.evaluate", json!({"query": QUERY}));
    assert_eq!(row(&v), json!({"[s]": 870, "[n]": 30, "[c]": 3}));
    run(&mut s, "file.open", json!({"path": a}));
    let v = run(&mut s, "dax.evaluate", json!({"query": QUERY}));
    assert_eq!(row(&v), json!({"[s]": 89700, "[n]": 300, "[c]": 3}));
}
