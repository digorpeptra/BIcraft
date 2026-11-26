//! `report.settings` (Desktop's Report settings of the current file): persistent filters,
//! personalize visuals, cross-report drillthrough, the Filters pane's visibility to readers and
//! "Allow users to change filter types", written to `report.json` (`settings`,
//! `objects.outspacePane`) and kept by a PBIP, a PBIX with PBIR parts and a PBIX with a legacy
//! layout (`docs/formats/pbir.md` ("Report settings")).

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::Session;

const SYNTHETIC_METADATA: &[u8] =
    include_bytes!("../../../metadata/tests/fixtures/synthetic_metadata.sqlitedb");

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn shop() -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Shop"}));
    run(
        &mut s,
        "table.import",
        json!({"name": "Sales", "m": "let Source = #table(type table [Region = text, Amount = Int64.Type], {{\"East\", 5}, {\"West\", 7}}) in Source"}),
    );
    run(&mut s, "report.new", json!({}));
    s
}

fn settings(s: &Session) -> Value {
    s.report.as_ref().unwrap().extra["settings"].clone()
}

fn objects(s: &Session) -> Value {
    s.report
        .as_ref()
        .unwrap()
        .extra
        .get("objects")
        .cloned()
        .unwrap_or(Value::Null)
}

/// The five settings set away from a new report's.
fn all_off() -> Value {
    json!({
        "persistent_filters": false,
        "personalize_visuals": true,
        "cross_report_drillthrough": true,
        "filter_pane_visible": false,
        "allow_change_filter_types": false,
    })
}

fn check_written(s: &Session, label: &str) {
    let st = settings(s);
    assert_eq!(st["isPersistentUserStateDisabled"], true, "{label}: {st}");
    assert_eq!(st["allowInlineExploration"], true, "{label}: {st}");
    assert_eq!(st["useCrossReportDrillthrough"], true, "{label}: {st}");
    assert_eq!(st["allowChangeFilterTypes"], false, "{label}: {st}");
    assert_eq!(
        objects(s)["outspacePane"],
        json!([{"properties": {"visible": {"expr": {"Literal": {"Value": "false"}}}}}]),
        "{label}"
    );
    let snap = s.snapshot();
    let read = &snap["report"]["settings"];
    for (k, v) in all_off().as_object().unwrap() {
        assert_eq!(&read[k], v, "{label}: {k} in {read}");
    }
}

#[test]
fn the_settings_are_written_read_back_and_undone() {
    let mut s = shop();
    // a new report: Desktop's defaults
    let read = run(&mut s, "report.settings", json!({}));
    assert_eq!(read["changed"], false);
    assert_eq!(
        (
            &read["persistent_filters"],
            &read["personalize_visuals"],
            &read["cross_report_drillthrough"],
            &read["filter_pane_visible"],
            &read["allow_change_filter_types"],
        ),
        (
            &json!(true),
            &json!(false),
            &json!(false),
            &json!(true),
            &json!(true)
        ),
        "{read}"
    );
    let out = run(&mut s, "report.settings", all_off());
    assert_eq!(out["changed"], true);
    check_written(&s, "session");
    // the same values again change nothing
    assert_eq!(run(&mut s, "report.settings", all_off())["changed"], false);
    // back: persistent filters and the pane's visibility leave the file, the others stay written
    run(
        &mut s,
        "report.settings",
        json!({"persistent_filters": true, "filter_pane_visible": true, "personalize_visuals": false}),
    );
    let st = settings(&s);
    assert!(st.get("isPersistentUserStateDisabled").is_none(), "{st}");
    assert_eq!(st["allowInlineExploration"], false);
    assert!(objects(&s).is_null(), "{}", objects(&s));
    // one undo step per change
    run(&mut s, "edit.undo", json!({}));
    check_written(&s, "undo");
}

#[test]
fn the_filters_pane_keeps_its_other_properties() {
    let mut s = shop();
    s.report.as_mut().unwrap().extra.insert(
        "objects".into(),
        json!({"outspacePane": [{"properties": {"expanded": {"expr": {"Literal": {"Value": "false"}}}}}]}),
    );
    run(&mut s, "report.settings", json!({"filter_pane_visible": false}));
    assert_eq!(
        objects(&s)["outspacePane"][0]["properties"],
        json!({"expanded": {"expr": {"Literal": {"Value": "false"}}},
               "visible": {"expr": {"Literal": {"Value": "false"}}}})
    );
    assert_eq!(s.snapshot()["report"]["settings"]["filter_pane_visible"], false);
    run(&mut s, "report.settings", json!({"filter_pane_visible": true}));
    assert_eq!(
        objects(&s)["outspacePane"],
        json!([{"properties": {"expanded": {"expr": {"Literal": {"Value": "false"}}}}}])
    );
}

/// The session after a save as `file` and a fresh open.
fn reopened(s: &mut Session, dir: &TempDir, file: &str) -> Session {
    run(s, "file.save_as", json!({"path": dir.join(file)}));
    let mut back = Session::new();
    run(&mut back, "file.open", json!({"path": dir.join(file)}));
    back
}

#[test]
fn pbip_and_pbix_keep_the_settings() {
    let dir = TempDir::new("report-settings-files");
    for file in ["Shop.pbip", "Shop.pbix"] {
        let mut s = shop();
        run(&mut s, "report.settings", all_off());
        let back = reopened(&mut s, &dir, file);
        check_written(&back, file);
    }
}

fn synthetic(dir: &TempDir, pbir: bool) -> PathBuf {
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

fn open(path: &Path) -> Session {
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": path}));
    s
}

#[test]
fn a_legacy_layout_and_pbir_parts_keep_the_settings() {
    let dir = TempDir::new("report-settings-legacy");
    for pbir in [false, true] {
        let path = synthetic(&dir, pbir);
        let mut s = open(&path);
        run(&mut s, "report.settings", all_off());
        run(&mut s, "file.save", json!({}));
        let back = open(&path);
        check_written(&back, if pbir { "pbir parts" } else { "legacy layout" });
    }
}

#[test]
fn bad_settings_are_refused() {
    let mut s = shop();
    for bad in [
        json!({"persistent_filters": "no"}),
        json!({"personalize_visuals": 1}),
        json!({"cross_report_drillthrough": "true"}),
        json!({"filter_pane_visible": []}),
        json!({"allow_change_filter_types": {}}),
        json!({"filter_pane": false}),
    ] {
        assert!(s.execute("report.settings", &bad).is_err(), "{bad}");
    }
    // a report whose objects are not an object is refused, not crashed on
    s.report
        .as_mut()
        .unwrap()
        .extra
        .insert("objects".into(), json!(3));
    assert!(
        s.execute("report.settings", &json!({"filter_pane_visible": false}))
            .is_err()
    );
    let mut empty = Session::new();
    assert!(
        empty
            .execute("report.settings", &json!({"persistent_filters": false}))
            .is_err()
    );
}
