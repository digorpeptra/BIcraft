//! Real reports, local only (client material: never committed, copied or quoted).
//!
//! - `BICRAFT_TESTDATA`: folder whose top-level `*.pbix` / `*.pbit` files are opened.
//! - `BICRAFT_TESTDATA_PBIP`: a `.pbip` file (or project folder) opened read-only.
//!
//! Each file is opened, then `report.summary` and `report.validate` run; fields are checked
//! against the file's own model, or the PBIP's model when the package's cannot be read. Only
//! counts are printed. Without the variables the test prints a note and passes.

use std::path::{Path, PathBuf};

use bicraft_engine::Session;
use serde_json::{Value, json};

fn count(v: &Value, key: &str) -> u64 {
    v[key].as_u64().unwrap_or(0)
}

/// Opens `path`, summarises and validates its report, and prints the counts.
fn check(path: &Path, fallback_model: Option<&bicraft_model::SemanticModel>) {
    let mut s = Session::new();
    s.execute("file.open", &json!({"path": path}))
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert!(
        s.report.is_some(),
        "{}: report not read: {:?}",
        path.display(),
        s.report_error
    );
    if s.model.is_none()
        && let Some(m) = fallback_model
    {
        s.model = Some(m.clone());
    }
    let summary = s.execute("report.summary", &json!({})).unwrap();
    let validation = s.execute("report.validate", &json!({})).unwrap();
    assert!(count(&summary, "page_count") > 0);
    let bound: usize = summary["pages"]
        .as_array()
        .map(|pages| {
            pages
                .iter()
                .flat_map(|p| p["visuals"].as_array().into_iter().flatten())
                .map(|v| v["fields"].as_array().map_or(0, Vec::len))
                .sum()
        })
        .unwrap_or(0);
    eprintln!(
        "{}: {} pages, {} visuals, {} visual types, {} distinct fields ({} bindings); \
         validate: checked_against_model={}, {} errors, {} warnings, {} schema issues",
        path.extension().and_then(|e| e.to_str()).unwrap_or("project"),
        count(&summary, "page_count"),
        count(&summary, "visual_count"),
        summary["visual_types"].as_object().map_or(0, |o| o.len()),
        summary["fields"].as_array().map_or(0, Vec::len),
        bound,
        validation["checked_against_model"],
        count(&validation, "error_count"),
        count(&validation, "warning_count"),
        validation["schema_issues"].as_array().map_or(0, Vec::len),
    );
}

#[test]
fn real_reports_summarise_and_validate() {
    let pbip = bicraft_env::var_os("BICRAFT_TESTDATA_PBIP").map(PathBuf::from);
    let root = bicraft_env::var_os("BICRAFT_TESTDATA").map(PathBuf::from);
    if pbip.is_none() && root.is_none() {
        eprintln!("note: set BICRAFT_TESTDATA and/or BICRAFT_TESTDATA_PBIP to run this test");
        return;
    }
    let mut model = None;
    if let Some(pbip) = pbip.filter(|p| p.exists()) {
        let mut s = Session::new();
        s.execute("file.open", &json!({"path": pbip})).unwrap();
        model = s.model.clone();
        check(&pbip, None);
    }
    let mut packages: Vec<PathBuf> = root
        .and_then(|r| std::fs::read_dir(r).ok())
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("pbix") || e.eq_ignore_ascii_case("pbit"))
        })
        .collect();
    packages.sort();
    for p in &packages {
        check(p, model.as_ref());
    }
}
