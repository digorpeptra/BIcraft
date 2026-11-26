//! Round trip and validation of real PBIR reports. The inputs are local-only (client material):
//! they are never committed, copied into the repository or quoted.
//!
//! - `BICRAFT_TESTDATA`: every `*.pbix` in it that carries a PBIR `Report/definition/` is used;
//!   every `*.pbix`/`*.pbit` below it (any depth) is schema-validated.
//! - `BICRAFT_PBIR_DIR` (optional): a `<Name>.Report` folder of a PBIP project, opened read-only.
//!
//! Without them the test prints a note and passes. Only counts are printed.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::Value;

#[path = "../../../testsupport/tempdir.rs"]
mod tempdir;

type Docs = Vec<(String, Vec<u8>)>;

fn pretty(v: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    let mut ser = serde_json::Serializer::with_formatter(
        &mut out,
        serde_json::ser::PrettyFormatter::with_indent(b"  "),
    );
    serde::Serialize::serialize(v, &mut ser).unwrap();
    out
}

fn report_parts(pbix: &Path) -> Docs {
    let file = std::fs::File::open(pbix).unwrap();
    let mut zip = zip::ZipArchive::new(file).unwrap();
    let mut docs = Vec::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).unwrap();
        let name = entry.name().unwrap().into_owned();
        if let Some(rel) = name.strip_prefix("Report/")
            && (rel.starts_with("definition/") || rel.starts_with("StaticResources/"))
        {
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            docs.push((rel.to_string(), bytes));
        }
    }
    docs
}

fn walk(root: &Path, dir: &Path, out: &mut Docs) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            walk(root, &path, out);
        } else {
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            out.push((rel, std::fs::read(&path).unwrap()));
        }
    }
}

fn check(label: &str, docs: &Docs) -> bicraft_report::Report {
    let issues = bicraft_pbir::validate_documents(docs);
    for i in issues.iter().take(20) {
        eprintln!("  issue: {} {} {}", i.path, i.pointer, i.message);
    }
    assert!(issues.is_empty(), "{label}: {} validation issues", issues.len());

    let report = bicraft_pbir::read_documents(docs).unwrap();
    let visuals: usize = report.pages.iter().map(|p| p.visuals.len()).sum();
    let fields = report.field_refs().count();
    eprintln!(
        "{label}: {} documents, {} pages, {visuals} visuals, {fields} field bindings, {} bookmarks, {} resources",
        docs.len(),
        report.pages.len(),
        report.bookmarks.len(),
        report.resources.len()
    );

    let written = bicraft_pbir::write_documents(&report);
    let out: BTreeMap<&str, &Vec<u8>> = written.iter().map(|(p, b)| (p.as_str(), b)).collect();
    let mut json_docs = 0;
    for (path, bytes) in docs {
        let Some(new) = out.get(path.as_str()) else {
            panic!(
                "{label}: a document was not written back ({} chars path)",
                path.len()
            );
        };
        if path.starts_with("definition/") && path.ends_with(".json") {
            let a: Value = serde_json::from_slice(bytes).unwrap();
            let b: Value = serde_json::from_slice(new).unwrap();
            assert!(a == b, "{label}: JSON differs after round trip");
            // Same key order and Desktop layout: our bytes are the source re-indented.
            assert!(
                pretty(&a) == **new,
                "{label}: layout or key order differs after round trip"
            );
            json_docs += 1;
        } else {
            assert!(bytes == *new, "{label}: resource bytes differ");
        }
    }
    assert_eq!(written.len(), docs.len(), "{label}: extra documents written");
    eprintln!("{label}: {json_docs} JSON documents identical as values and in key order");

    // Our own output: stable, valid, and reads back to the same model.
    let again = bicraft_pbir::read_documents(&written).unwrap();
    assert!(again == report, "{label}: model differs after re-read");
    assert!(
        bicraft_pbir::write_documents(&again) == written,
        "{label}: second write differs"
    );
    let issues = bicraft_pbir::validate_documents(&written);
    assert!(
        issues.is_empty(),
        "{label}: {} issues in written documents",
        issues.len()
    );
    report
}

#[test]
fn real_reports_round_trip_and_validate() {
    let mut ran = false;
    if let Some(root) = bicraft_env::var_os("BICRAFT_TESTDATA").map(PathBuf::from) {
        let mut pbix: Vec<PathBuf> = std::fs::read_dir(&root)
            .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path()).collect())
            .unwrap_or_default();
        pbix.retain(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pbix")));
        pbix.sort();
        for (i, path) in pbix.iter().enumerate() {
            let docs = report_parts(path);
            if !docs.iter().any(|(p, _)| p == "definition/report.json") {
                continue;
            }
            check(&format!("pbix #{i}"), &docs);
            ran = true;
        }
    } else {
        eprintln!("note: BICRAFT_TESTDATA not set; skipping the PBIX reports");
    }

    if let Some(dir) = bicraft_env::var_os("BICRAFT_PBIR_DIR").map(PathBuf::from) {
        let mut docs = Vec::new();
        for top in ["definition", "StaticResources"] {
            if dir.join(top).is_dir() {
                walk(&dir, &dir.join(top), &mut docs);
            }
        }
        docs.sort();
        let report = check("pbip report folder", &docs);
        assert!(bicraft_pbir::read_dir(&dir).unwrap() == report);

        // write_dir into a guarded scratch folder (never into the source), removed on drop.
        let scratch = tempdir::TempDir::new("pbir-real");
        let tmp = scratch.join("out");
        bicraft_pbir::write_dir(&report, &tmp).unwrap();
        assert!(bicraft_pbir::read_dir(&tmp).unwrap() == report);
        ran = true;
    } else {
        eprintln!("note: BICRAFT_PBIR_DIR not set; skipping the PBIP report folder");
    }
    if !ran {
        eprintln!("note: no real PBIR report found; nothing checked");
    }
}

fn packages(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.is_dir() {
            packages(&path, out);
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("pbix") || e.eq_ignore_ascii_case("pbit"))
        {
            out.push(path);
        }
    }
}

/// No false schema errors on Desktop-written reports: every PBIR report under the test data,
/// at any depth, validates without issues. Only counts are printed.
#[test]
fn every_real_report_has_no_schema_issues() {
    let Some(root) = bicraft_env::var_os("BICRAFT_TESTDATA").map(PathBuf::from) else {
        eprintln!("note: BICRAFT_TESTDATA not set; skipping");
        return;
    };
    let mut files = Vec::new();
    packages(&root, &mut files);
    files.sort();
    let mut reports = 0;
    for (i, path) in files.iter().enumerate() {
        let docs = report_parts(path);
        if !docs.iter().any(|(p, _)| p == "definition/report.json") {
            continue;
        }
        let issues = bicraft_pbir::validate_documents(&docs);
        assert!(
            issues.is_empty(),
            "package #{i}: {} validation issues",
            issues.len()
        );
        reports += 1;
    }
    eprintln!("{reports} PBIR reports in {} packages: no issues", files.len());
}
