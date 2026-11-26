//! `report.validate` errors and `report.repair` for a field projected under two queryRefs (the
//! suffixed form older bicraft saved), in a legacy layout and in PBIR.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::Session;

const SYNTHETIC_METADATA: &[u8] =
    include_bytes!("../../../metadata/tests/fixtures/synthetic_metadata.sqlitedb");
const KEPT: &str = "Sum(Sales.Amount)";
const DUPLICATE: &str = "Sum(Sales.Amount)1";

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn open(path: &Path) -> Session {
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": path}));
    s
}

fn write(dir: &TempDir, pbir: bool) -> PathBuf {
    let path = dir.join(if pbir { "Pbir.pbix" } else { "Legacy.pbix" });
    let pkg = if pbir {
        bicraft_pbix::synthetic::pbir_pbix(SYNTHETIC_METADATA)
    } else {
        bicraft_pbix::synthetic::pbix(SYNTHETIC_METADATA)
    }
    .unwrap();
    std::fs::write(&path, pkg.to_bytes().unwrap()).unwrap();
    path
}

/// The report parts as text (the legacy layout is UTF-16LE).
fn report_text(path: &Path) -> String {
    let pkg = bicraft_opc::Package::open(path).unwrap();
    let mut out = String::new();
    for p in &pkg.parts {
        if p.name == "Report/Layout" {
            let units: Vec<u16> = p
                .data
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_le_bytes(*c))
                .collect();
            out.push_str(&String::from_utf16_lossy(&units));
        } else if p.name.starts_with("Report/definition/") {
            out.push_str(&String::from_utf8_lossy(&p.data));
        }
    }
    out
}

fn duplicate_issues(s: &mut Session) -> Vec<Value> {
    let v = run(s, "report.validate", json!({}));
    v["issues"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["query_ref"].is_string())
        .cloned()
        .collect()
}

fn visual_query(s: &Session, name: &str) -> Value {
    let r = s.report.as_ref().unwrap();
    let v = r
        .pages
        .iter()
        .flat_map(|p| &p.visuals)
        .find(|v| v.name == name)
        .unwrap();
    v.visual_extra["query"].clone()
}

/// A file holding a scatter chart whose Size projects Y's field under a suffixed queryRef, with
/// a formatting entry selecting the suffixed one. Returns the file and the visual name.
fn broken_file(dir: &TempDir, pbir: bool) -> (PathBuf, String) {
    let path = write(dir, pbir);
    let mut s = open(&path);
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    let added = run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "scatterChart", "x": 300, "y": 10, "width": 300, "height": 200,
               "fields": [{"role": "Category", "field": "Sales[Amount]"},
                          {"role": "Y", "field": "Sum(Sales[Amount])"},
                          {"role": "Size", "field": "Sum(Sales[Amount])"}]}),
    );
    let name = added["visual"].as_str().unwrap().to_string();
    {
        let r = s.report.as_mut().unwrap();
        let v = r
            .pages
            .iter_mut()
            .flat_map(|p| p.visuals.iter_mut())
            .find(|v| v.name == name)
            .unwrap();
        let q = v
            .visual_extra
            .entry("query")
            .or_insert_with(|| bicraft_report::query::regenerate(None, &v.fields).unwrap());
        let size = &mut q["queryState"]["Size"]["projections"][0];
        assert_eq!(
            size["queryRef"], KEPT,
            "the current writer reuses the first queryRef"
        );
        size["queryRef"] = json!(DUPLICATE);
        size["nativeQueryRef"] = json!("Sum of Amount1");
        v.visual_extra.insert(
            "objects".into(),
            json!({"dataPoint": [{"properties": {"fill": {"solid": {"color": {"expr": {"Literal": {"Value": "'#118DFF'"}}}}}},
                                  "selector": {"metadata": DUPLICATE}}]}),
        );
    }
    run(&mut s, "file.save", json!({}));
    assert!(report_text(&path).contains(DUPLICATE), "the broken form is saved");
    (path, name)
}

fn flag_and_repair(pbir: bool) {
    let dir = TempDir::new(&format!("report-repair-{pbir}"));
    let (path, name) = broken_file(&dir, pbir);

    let mut s = open(&path);
    let issues = duplicate_issues(&mut s);
    assert_eq!(issues.len(), 1, "{issues:?}");
    let issue = &issues[0];
    assert_eq!(issue["visual"], name.as_str());
    assert_eq!(issue["query_ref"], DUPLICATE);
    let message = issue["message"].as_str().unwrap();
    assert!(
        message.contains(&name) && message.contains("report.repair") && message.contains(KEPT),
        "{message}"
    );
    assert_eq!(run(&mut s, "report.validate", json!({}))["valid"], false);

    let out = run(&mut s, "report.repair", json!({}));
    assert_eq!(out["count"], 1, "{out}");
    assert_eq!(out["fixed"][0]["kept"], KEPT);
    assert_eq!(out["fixed"][0]["replaced"], DUPLICATE);
    assert!(duplicate_issues(&mut s).is_empty());
    // Idempotent.
    assert_eq!(run(&mut s, "report.repair", json!({}))["count"], 0);
    run(&mut s, "file.save", json!({}));

    let text = report_text(&path);
    assert!(!text.contains(DUPLICATE), "no suffixed queryRef left");
    let mut fresh = open(&path);
    assert!(duplicate_issues(&mut fresh).is_empty());
    let q = visual_query(&fresh, &name);
    for role in ["Y", "Size"] {
        assert_eq!(
            q["queryState"][role]["projections"][0]["queryRef"], KEPT,
            "{role}"
        );
    }
    let v = fresh
        .report
        .as_ref()
        .unwrap()
        .pages
        .iter()
        .flat_map(|p| &p.visuals)
        .find(|v| v.name == name)
        .unwrap();
    assert_eq!(
        v.visual_extra["objects"]["dataPoint"][0]["selector"]["metadata"],
        KEPT
    );
    if !pbir {
        // The legacy prototypeQuery selects the field once.
        assert_eq!(
            text.matches(&format!("\\\"Name\\\":\\\"{KEPT}\\\"")).count(),
            1,
            "{text}"
        );
    }
}

#[test]
fn duplicate_query_refs_are_flagged_and_repaired_in_a_legacy_layout() {
    flag_and_repair(false);
}

#[test]
fn duplicate_query_refs_are_flagged_and_repaired_in_pbir() {
    flag_and_repair(true);
}

#[test]
fn rebinding_repairs_and_scoped_repair_checks_its_target() {
    let dir = TempDir::new("report-repair-rebind");
    let (path, name) = broken_file(&dir, true);
    let mut s = open(&path);
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    // A scoped repair of another visual changes nothing; unknown targets are errors.
    assert!(
        s.execute("report.repair", &json!({"page": page, "visual": "nope"}))
            .is_err()
    );
    assert!(s.execute("report.repair", &json!({"visual": name})).is_err());
    assert!(s.execute("report.repair", &json!({"page": "nope"})).is_err());
    // Re-binding the visual regenerates its query without the duplicate.
    run(
        &mut s,
        "visual.bind",
        json!({"page": page, "visual": name, "role": "Details", "field": "Sales[Amount]", "force": true}),
    );
    assert!(duplicate_issues(&mut s).is_empty());
    // The repair still points the formatting selector at the kept queryRef.
    assert_eq!(
        run(&mut s, "report.repair", json!({"page": page, "visual": name}))["count"],
        1
    );
    let r = s.report.as_ref().unwrap();
    let v = r
        .pages
        .iter()
        .flat_map(|p| &p.visuals)
        .find(|v| v.name == name)
        .unwrap();
    assert_eq!(
        v.visual_extra["objects"]["dataPoint"][0]["selector"]["metadata"],
        KEPT
    );
    let q = &v.visual_extra["query"];
    assert_eq!(
        q["queryState"]["Details"]["projections"][0]["queryRef"],
        "Sales.Amount"
    );
    assert_eq!(q["queryState"]["Size"]["projections"][0]["queryRef"], KEPT);
    assert_eq!(
        run(&mut s, "report.repair", json!({"page": page, "visual": name}))["count"],
        0
    );
}

#[test]
fn same_names_of_different_fields_are_not_flagged() {
    let dir = TempDir::new("report-repair-names");
    let path = write(&dir, true);
    let mut s = open(&path);
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    let added = run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "tableEx", "x": 300, "y": 10, "width": 300, "height": 200,
               "fields": [{"role": "Values", "field": "Sum(Sales[Amount])"},
                          {"role": "Values", "field": "Average(Sales[Amount])"}]}),
    );
    // Rename the second projection's queryRef to the first plus a suffix: still two fields.
    {
        let r = s.report.as_mut().unwrap();
        let name = added["visual"].as_str().unwrap();
        let v = r
            .pages
            .iter_mut()
            .flat_map(|p| p.visuals.iter_mut())
            .find(|v| v.name == name)
            .unwrap();
        let q = v
            .visual_extra
            .entry("query")
            .or_insert_with(|| bicraft_report::query::regenerate(None, &v.fields).unwrap());
        q["queryState"]["Values"]["projections"][1]["queryRef"] = json!(DUPLICATE);
    }
    assert!(duplicate_issues(&mut s).is_empty());
    assert_eq!(run(&mut s, "report.repair", json!({}))["count"], 0);
}
