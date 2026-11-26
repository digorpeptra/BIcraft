//! Structural edits of a real Desktop PBIX: local only, never committed.
//!
//! Reads `$BICRAFT_TESTDATA/sample1/report.pbix`, removes one calculated column that nothing
//! references (and, separately, one table that nothing references), applies the storage plan to
//! the ABF, re-compresses the `DataModel` and writes `remove_column.pbix` / `remove_table.pbix`
//! to `$BICRAFT_TESTDATA/m5/` for a manual check in Power BI Desktop. Without the variable or
//! the file the test prints a note and passes. Only counts and output paths are printed, never
//! names or content of the model.

use std::path::{Path, PathBuf};

use bicraft_abf::{Abf, FileKind};
use bicraft_metadata::{
    EncodedData, dangling_references, read_sqlitedb, storage_file_paths, tables_from_sqlitedb_all,
    write_structural_sqlitedb_at,
};
use bicraft_model::{SemanticModel, Table};
use bicraft_opc::Package;

const SAMPLE: &str = "sample1/report.pbix";
const DATA_MODEL: &str = "DataModel";
/// 2026-10-13T12:00:00Z as FILETIME ticks.
const NOW: i64 = 134_363_808_000_000_000;

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

/// Every DAX/M text of the model, lower-cased, to look for references by name.
fn all_expressions(m: &SemanticModel) -> Vec<String> {
    let mut out = Vec::new();
    for t in &m.tables {
        out.extend(t.measures.iter().map(|x| x.expression.to_lowercase()));
        out.extend(
            t.columns
                .iter()
                .filter_map(|c| c.expression.as_ref())
                .map(|e| e.to_lowercase()),
        );
        out.extend(
            t.partitions
                .iter()
                .filter_map(|p| p.expression.as_ref())
                .map(|e| e.to_lowercase()),
        );
        if let Some(h) = t.extra.get("hierarchies") {
            out.push(h.to_string().to_lowercase());
        }
    }
    out.extend(
        m.roles
            .iter()
            .flat_map(|r| &r.table_permissions)
            .filter_map(|p| p.filter_expression.as_ref())
            .map(|e| e.to_lowercase()),
    );
    out
}

/// The last calculated column that no expression, relationship, sort-by or hierarchy names.
fn removable_column(m: &SemanticModel) -> Option<(usize, usize)> {
    let texts = all_expressions(m);
    let mut found = None;
    for (ti, t) in m.tables.iter().enumerate() {
        for (ci, c) in t.columns.iter().enumerate() {
            if c.expression.is_none() || c.extra.contains_key("type") {
                continue;
            }
            let lname = c.name.to_lowercase();
            let bracket = format!("[{lname}]");
            let quoted = format!("\"{lname}\"");
            let in_rel = m.relationships.iter().any(|r| {
                (r.from_table == t.name && r.from_column == c.name)
                    || (r.to_table == t.name && r.to_column == c.name)
            });
            let sorts = t.columns.iter().any(|o| {
                o.sort_by_column
                    .as_deref()
                    .is_some_and(|s| s.eq_ignore_ascii_case(&c.name))
            });
            let named = texts.iter().any(|x| x.contains(&bracket) || x.contains(&quoted));
            if !in_rel && !sorts && !named {
                found = Some((ti, ci));
            }
        }
    }
    found
}

/// A table without measures that no relationship, expression or other table names.
fn removable_table(m: &SemanticModel) -> Option<usize> {
    let texts = all_expressions(m);
    let mut found = None;
    for (ti, t) in m.tables.iter().enumerate() {
        let in_rel = m
            .relationships
            .iter()
            .any(|r| r.from_table == t.name || r.to_table == t.name);
        let lname = t.name.to_lowercase();
        let named = texts
            .iter()
            .any(|x| x.contains(&format!("'{lname}'")) || x.contains(&format!("{lname}[")));
        if !in_rel && !named && t.measures.is_empty() && !t.extra.contains_key("calculationGroup") {
            found = Some(ti);
        }
    }
    found
}

/// Applies a structural edit to the package, checks the result and writes it to `out/name`.
fn save_structural(pkg: &Package, edited: &SemanticModel, out: &Path, name: &str) {
    let part = &pkg.get(DATA_MODEL).expect("DataModel").data;
    let info = bicraft_xpress9::inspect_datamodel(part).expect("inspect");
    let abf_bytes = bicraft_xpress9::decompress_datamodel(part, 4 << 30).expect("decompress");
    let abf = Abf::parse(&abf_bytes).expect("abf");
    let sqlite = abf.metadata_sqlitedb().expect("metadata");
    let before = tables_from_sqlitedb_all(sqlite).expect("tables");

    let w =
        write_structural_sqlitedb_at(sqlite, edited, &EncodedData::default(), NOW).expect("structural write");
    assert!(w.plan.add.is_empty());
    assert!(!w.plan.remove.is_empty(), "a structural removal drops files");
    let after = tables_from_sqlitedb_all(&w.sqlitedb).expect("tables back");
    // References: none dangles, before or after (the Desktop file has none either).
    assert!(
        dangling_references(&before).is_empty(),
        "the base has dangling references"
    );
    let dangling = dangling_references(&after);
    assert!(dangling.is_empty(), "{} dangling references", dangling.len());
    // The plan removes exactly the files whose StorageFile rows went away.
    let (files_before, files_after) = (storage_file_paths(&before), storage_file_paths(&after));
    let mut gone: Vec<String> = files_before
        .iter()
        .filter(|p| !files_after.contains(p))
        .cloned()
        .collect();
    gone.sort();
    let mut planned = w.plan.remove.clone();
    planned.sort();
    assert_eq!(gone, planned, "plan and StorageFile rows disagree");

    // The container.
    let mut writer = abf.to_writer().expect("writer");
    for p in &w.plan.remove {
        writer.remove(p).expect("remove planned file");
    }
    writer
        .replace_metadata_sqlitedb(w.sqlitedb.clone())
        .expect("replace metadata");
    let new_abf = writer.finish().expect("write abf");
    let back = Abf::parse(&new_abf).expect("abf back");
    let meta = back.metadata_entry().expect("metadata entry").clone();
    let mut stored: Vec<String> = back
        .files()
        .iter()
        .filter(|f| f.file_group_class == meta.file_group_class && f.kind != FileKind::MetadataSqlite)
        .map(|f| f.path.clone())
        .collect();
    stored.sort();
    assert_eq!(stored, files_after, "ABF files and StorageFile rows disagree");

    let compressed = bicraft_xpress9::compress_datamodel(&new_abf, Some(&info)).expect("compress");
    assert!(
        bicraft_xpress9::decompress_datamodel(&compressed, new_abf.len()).expect("decompress back")
            == new_abf
    );
    let mut out_pkg = pkg.clone();
    out_pkg.put(DATA_MODEL, compressed, true);
    let path = out.join(name);
    std::fs::write(&path, out_pkg.to_bytes().expect("zip")).expect("write pbix");

    // Read back through the files: the model is the edited one.
    let reopened = Package::open(&path).expect("reopen");
    let part = &reopened.get(DATA_MODEL).expect("DataModel").data;
    let abf_back = bicraft_xpress9::decompress_datamodel(part, 4 << 30).expect("decompress");
    let abf_back = Abf::parse(&abf_back).expect("abf");
    let model_back = read_sqlitedb(abf_back.metadata_sqlitedb().expect("metadata")).expect("model back");
    assert_eq!(model_back, w.model, "the saved model differs from the edited one");
    println!(
        "wrote {} ({} tables changed, {} files removed)",
        path.display(),
        before
            .table_names()
            .filter(|t| before.rows(t) != after.rows(t))
            .count(),
        w.plan.remove.len()
    );
}

#[test]
fn real_pbix_structural_removals() {
    let Some((file, out)) = sample() else { return };
    std::fs::create_dir_all(&out).expect("create m5 output folder");
    let pkg = Package::open(&file).expect("open sample");
    let part = &pkg.get(DATA_MODEL).expect("DataModel").data;
    let abf_bytes = bicraft_xpress9::decompress_datamodel(part, 4 << 30).expect("decompress");
    let abf = Abf::parse(&abf_bytes).expect("abf");
    let model = read_sqlitedb(abf.metadata_sqlitedb().expect("metadata")).expect("model");

    match removable_column(&model) {
        Some((ti, ci)) => {
            let mut edited = model.clone();
            edited.tables[ti].columns.remove(ci);
            save_structural(&pkg, &edited, &out, "remove_column.pbix");
        }
        None => eprintln!("note: no unreferenced calculated column; remove_column skipped"),
    }
    match removable_table(&model) {
        Some(ti) => {
            let mut edited = model.clone();
            let removed: Table = edited.tables.remove(ti);
            assert!(!removed.name.is_empty());
            save_structural(&pkg, &edited, &out, "remove_table.pbix");
        }
        None => eprintln!("note: no unreferenced table; remove_table skipped"),
    }
}
