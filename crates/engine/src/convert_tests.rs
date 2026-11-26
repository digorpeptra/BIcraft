//! The convert tests that need the engine's private items (`pbir_form`, `dropped_warning`, `has_legacy_keys`); the others are the `files`
//! integration test (`tests/files/convert_tests.rs`), under the same module path.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::*;
use crate::tempdir::TempDir;

const SYNTHETIC_METADATA: &[u8] = include_bytes!("../../metadata/tests/fixtures/synthetic_metadata.sqlitedb");

fn write_pkg(path: &Path, pkg: &bicraft_opc::Package) {
    std::fs::write(path, pkg.to_bytes().unwrap()).unwrap();
}
fn temp(name: &str) -> TempDir {
    TempDir::new(&format!("engine-convert-{name}"))
}

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn open(path: &Path) -> Session {
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": path}));
    assert_eq!(s.model_error, None, "{}", path.display());
    s
}

/// A legacy layout whose visual has a phone layout and properties PBIR has no place for.
const RICH_LAYOUT: &str = r#"{"config":"{}","layoutOptimization":0,"sections":[{"name":"ReportSection","displayName":"Page 1","width":1280,"height":720,"displayOption":1,"config":"{}","filters":"[]","visualContainers":[{"x":10,"y":10,"z":0,"width":200,"height":100,"id":7,"query":"{}","dataTransforms":"{}","filters":"[]","config":"{\"name\":\"v1\",\"layouts\":[{\"id\":0,\"position\":{\"x\":10,\"y\":10,\"z\":0,\"width\":200,\"height\":100}},{\"id\":1,\"position\":{\"x\":0,\"y\":0,\"z\":0,\"width\":100,\"height\":50}}],\"singleVisual\":{\"visualType\":\"textbox\",\"objects\":{}}}"}]}]}"#;

fn rich_legacy_pbix(dir: &Path) -> PathBuf {
    let mut pkg = bicraft_pbix::synthetic::pbix(SYNTHETIC_METADATA).unwrap();
    pkg.put(
        "Report/Layout",
        bicraft_opc::encode_text(RICH_LAYOUT, bicraft_opc::Encoding::Utf16Le),
        false,
    );
    let path = dir.join("Rich.pbix");
    write_pkg(&path, &pkg);
    path
}

/// Every file under `dir`, with its text when it is UTF-8.
fn texts(dir: &Path) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(t) = std::fs::read_to_string(&p) {
                out.push((p, t));
            }
        }
    }
    out
}

#[test]
fn legacy_only_keys_become_pbir_documents_or_are_counted() {
    let mut report = bicraft_report::Report {
        pages: vec![bicraft_report::Page {
            name: "p".into(),
            visuals: vec![bicraft_report::Visual {
                name: "v".into(),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    report.extra.insert(
        bicraft_layout::BOOKMARKS_METADATA_KEY.into(),
        json!({"items": []}),
    );
    report
        .extra
        .insert(bicraft_layout::UNMAPPED_KEY.into(), json!({"a": 1, "b": 2}));
    report.pages[0].visuals[0].extra.insert(
        bicraft_layout::MOBILE_STATE_KEY.into(),
        json!({"position": {"x": 0}}),
    );
    let (form, dropped) = pbir_form(&report);
    assert_eq!(dropped, ["report: a", "report: b"]);
    let warning = dropped_warning(&dropped).unwrap();
    assert!(
        warning.contains("2 legacy-only") && warning.contains("report: b"),
        "{warning}"
    );
    let report = form.into_owned();
    // Nothing legacy left: the PBIR documents hold no `legacy:` key.
    for (path, bytes) in bicraft_pbir::write_documents(&report) {
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains("legacy:"), "{path}: {text}");
    }
    // A report without legacy keys is borrowed as is.
    assert!(matches!(pbir_form(&report).0, std::borrow::Cow::Borrowed(_)));
    assert!(report.extra.is_empty() && report.pages[0].visuals[0].extra.is_empty());
    let paths: Vec<&str> = report.resources.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "definition/bookmarks/bookmarks.json",
            "definition/pages/p/visuals/v/mobile.json"
        ]
    );
    let mobile: Value = serde_json::from_slice(&report.resources[1].bytes).unwrap();
    assert!(
        mobile["$schema"]
            .as_str()
            .unwrap()
            .contains("visualContainerMobileState")
    );
}

#[test]
fn legacy_keys_are_never_written_as_pbir() {
    let dir = temp("legacy-keys");
    let src = rich_legacy_pbix(&dir);
    let mut s = open(&src);
    let visual = &s.report.as_ref().unwrap().pages[0].visuals[0];
    assert!(
        visual.extra.contains_key(bicraft_layout::MOBILE_STATE_KEY),
        "the fixture has a phone layout"
    );
    assert!(has_legacy_keys(s.report.as_ref().unwrap()));
    // report.validate checks the PBIR form: no schema complaint about `legacy:` keys.
    let v = run(&mut s, "report.validate", json!({}));
    assert!(!v.to_string().contains("legacy:"), "{v}");

    // Conversion to PBIR: the phone layout becomes mobile.json, the rest is listed as dropped.
    let out = run(
        &mut s,
        "file.save_as",
        json!({"path": dir.join("conv").join("P.pbip"), "report_format": "pbir", "cache": false}),
    );
    let warnings = out["warnings"].to_string();
    assert!(
        warnings.contains("dropped") && warnings.contains("visual ReportSection/v1"),
        "{warnings}"
    );
    let report_dir = dir.join("conv").join("P.Report");
    let files = texts(&report_dir);
    assert!(
        files.iter().all(|(_, t)| !t.contains("legacy:")),
        "a legacy: key was written"
    );
    assert!(
        files
            .iter()
            .any(|(p, t)| p.ends_with("mobile.json") && t.contains("visualContainerMobileState")),
        "no mobile.json"
    );

    // project.new and file.save write the PBIR form too.
    let mut s = open(&src);
    let out = run(
        &mut s,
        "project.new",
        json!({"dir": dir.join("new"), "name": "N", "format": "tmsl"}),
    );
    assert!(out["warnings"].to_string().contains("dropped"), "{out}");
    assert!(
        texts(&dir.join("new").join("N.Report"))
            .iter()
            .all(|(_, t)| !t.contains("legacy:"))
    );

    let mut s = open(&src);
    run(
        &mut s,
        "file.save_as",
        json!({"path": dir.join("keep").join("K.pbip"), "cache": false}),
    );
    assert!(dir.join("keep/K.Report/report.json").is_file(), "kept legacy");
    let mut p = open(&dir.join("keep").join("K.pbip"));
    run(&mut p, "page.add", json!({"display_name": "Second"}));
    let out = run(&mut p, "file.save", json!({}));
    assert_eq!(out["report_converted_to_pbir"], true, "{out}");
    assert!(out["warnings"].to_string().contains("dropped"), "{out}");
    assert!(
        texts(&dir.join("keep").join("K.Report"))
            .iter()
            .all(|(_, t)| !t.contains("legacy:"))
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// G13: report measures and phone formatting as Desktop converts them (legacy-layout-bb.md).
#[test]
fn legacy_report_measures_and_phone_objects_convert_like_desktop() {
    let mut report = bicraft_report::Report {
        pages: vec![bicraft_report::Page {
            name: "p".into(),
            visuals: vec![bicraft_report::Visual {
                name: "v".into(),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    report.extra.insert(
        bicraft_layout::UNMAPPED_KEY.into(),
        json!({"config.modelExtensions": [{"name": "extension", "entities": [{"name": "Sales",
            "extends": "Sales", "measures": [{"name": "M", "dataType": 3, "expression": "1",
            "formatInformation": {"formatString": "0.0", "format": "General"}}]}]}]}),
    );
    report.pages[0].visuals[0].extra.insert(
        bicraft_layout::MOBILE_STATE_KEY.into(),
        json!({"position": {"x": 0, "y": 0, "width": 10, "height": 10},
               "objects": {"labels": []}, "visualContainerObjects": {"title": []}}),
    );
    let (form, dropped) = pbir_form(&report);
    assert_eq!(
        dropped,
        [
            "visual p/v: phone objects (mobileObjects in the layout)",
            "visual p/v: phone visualContainerObjects (mobileObjects in the layout)"
        ]
    );
    let report = form.into_owned();
    let doc = |path: &str| -> Value {
        let r = report.resources.iter().find(|r| r.path == path).unwrap();
        serde_json::from_slice(&r.bytes).unwrap()
    };
    let ext = doc("definition/reportExtensions.json");
    assert!(ext["$schema"].as_str().unwrap().contains("reportExtension/"));
    assert_eq!(
        ext["entities"],
        json!([{"name": "Sales", "measures": [
            {"name": "M", "dataType": "Double", "expression": "1", "formatString": "0.0"}]}])
    );
    let mobile = doc("definition/pages/p/visuals/v/mobile.json");
    assert!(mobile.get("objects").is_none() && mobile.get("visualContainerObjects").is_none());
    assert_eq!(mobile["position"]["width"], 10);
}
