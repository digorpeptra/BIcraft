//! The theme edit tests that need the engine's private items (`theme::resolve`); the others are the `report`
//! integration test (`tests/report/theme_edit_tests.rs`), under the same module path.

use std::path::PathBuf;

use bicraft_report::{Report, Resource};
use serde_json::{Value, json};

use crate::Session;
use crate::tempdir::TempDir;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Runs a command that must change nothing (report, dirty flag) and returns its result.
fn no_op(s: &mut Session, id: &str, params: Value) -> Value {
    let before = s.report.clone();
    let dirty = s.report_dirty;
    let out = run(s, id, params);
    assert_eq!(out["changed"], false, "{out}");
    assert_eq!(s.report, before);
    assert_eq!(s.report_dirty, dirty);
    out
}

fn temp(name: &str) -> TempDir {
    TempDir::new(&format!("engine-theme-{name}"))
}

/// A blank report with a card, saved state (not dirty).
fn session() -> Session {
    let mut s = Session::new();
    run(&mut s, "report.new", json!({}));
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "card", "x": 10, "y": 10, "width": 200, "height": 100, "force": true}),
    );
    s.report_dirty = false;
    s
}

fn report(s: &Session) -> &Report {
    s.report.as_ref().unwrap()
}

/// The custom theme document as stored in the report.
fn stored_theme(s: &Session) -> Value {
    let out = run(
        &mut Session {
            report: s.report.clone(),
            ..Session::new()
        },
        "theme.get",
        json!({}),
    );
    out["custom"]["theme"].clone()
}

fn custom_resources(r: &Report) -> Vec<&Resource> {
    r.resources
        .iter()
        .filter(|x| x.path.starts_with("StaticResources/RegisteredResources/"))
        .collect()
}

/// report.validate (with schemas) finds no error.
fn assert_valid(s: &mut Session) {
    let v = run(s, "report.validate", json!({}));
    assert_eq!(v["error_count"], 0, "{v:#}");
    assert_eq!(v["schema_issues"].as_array().map_or(0, Vec::len), 0, "{v:#}");
}

/// A report whose base theme is embedded, as Desktop saves it.
fn with_base_theme(s: &mut Session) {
    let r = s.report.as_mut().unwrap();
    r.theme_collection = Some(json!({"baseTheme": {
        "name": "CY24SU10",
        "reportVersionAtImport": {"visual": "1.8.50", "report": "2.0.50", "page": "1.3.50"},
        "type": "SharedResources"}}));
    r.extra.insert(
        "resourcePackages".into(),
        json!([{"name": "SharedResources", "type": "SharedResources",
                "items": [{"name": "CY24SU10", "path": "BaseThemes/CY24SU10.json", "type": "BaseTheme"}]}]),
    );
    r.resources.push(Resource {
        path: "StaticResources/SharedResources/BaseThemes/CY24SU10.json".into(),
        bytes: serde_json::to_vec(&json!({
            "name": "CY24SU10",
            "dataColors": ["#010203", "#040506"],
            "foreground": "#111111",
            "textClasses": {"label": {"fontFace": "Segoe UI", "fontSize": 9}}
        }))
        .unwrap(),
    });
}

#[test]
fn set_colors_creates_the_custom_theme_as_desktop_does() {
    let mut s = session();
    let out = run(
        &mut s,
        "theme.set_colors",
        json!({"data_colors": ["#112233", "#445566", "#778899"], "background": "#FAFAFA"}),
    );
    assert_eq!(out["changed"], true);
    assert_eq!(out["created"], true);
    assert!(s.report_dirty);
    let name = out["name"].as_str().unwrap().to_string();
    assert!(
        name.starts_with("Customtheme") && name.ends_with(".json"),
        "{name}"
    );
    let r = report(&s);
    // themeCollection.customTheme {name, reportVersionAtImport, type}
    let ct = &r.theme_collection.as_ref().unwrap()["customTheme"];
    assert_eq!(ct["name"], name.as_str());
    assert_eq!(ct["type"], "RegisteredResources");
    assert_eq!(
        ct["reportVersionAtImport"],
        json!({"visual": "2.12.0", "report": "3.3.0", "page": "2.1.0"})
    );
    // resourcePackages (before settings, as Desktop orders report.json), its item, and the file
    assert_eq!(
        r.extra.keys().collect::<Vec<_>>(),
        ["resourcePackages", "settings"]
    );
    assert_eq!(
        r.extra["resourcePackages"],
        json!([{"name": "RegisteredResources", "type": "RegisteredResources",
                "items": [{"name": name, "path": name, "type": "CustomTheme"}]}])
    );
    let files = custom_resources(r);
    assert_eq!(files.len(), 1);
    assert_eq!(
        files[0].path,
        format!("StaticResources/RegisteredResources/{name}")
    );
    let doc: Value = serde_json::from_slice(&files[0].bytes).unwrap();
    assert_eq!(doc["name"], "Custom theme");
    assert_eq!(doc["dataColors"], json!(["#112233", "#445566", "#778899"]));
    assert_eq!(
        out["resolved"]["data_colors"],
        json!(["#112233", "#445566", "#778899"])
    );
    assert_eq!(out["resolved"]["background"], "#FAFAFA");
    assert_valid(&mut s);
    assert_round_trip(report(&s));

    // Idempotent.
    no_op(
        &mut s,
        "theme.set_colors",
        json!({"data_colors": ["#112233", "#445566", "#778899"], "background": "#FAFAFA"}),
    );
}

#[test]
fn edits_replace_the_file_without_leaving_the_old_one() {
    let mut s = session();
    let first = run(&mut s, "theme.set_colors", json!({"good": "#00FF00"}));
    let second = run(&mut s, "theme.set_colors", json!({"bad": "#FF0000"}));
    assert_ne!(first["name"], second["name"]);
    assert_eq!(second["removed_resource"], first["resource"]);
    assert_eq!(second["created"], false);
    let r = report(&s);
    assert_eq!(custom_resources(r).len(), 1);
    let items = r.extra["resourcePackages"][0]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], second["name"]);
    let doc = stored_theme(&s);
    assert_eq!(doc["good"], "#00FF00");
    assert_eq!(doc["bad"], "#FF0000");
    // null removes a key
    run(&mut s, "theme.set_colors", json!({"good": null}));
    assert!(stored_theme(&s).get("good").is_none());
    assert_valid(&mut s);
    assert_round_trip(report(&s));
}

#[test]
fn visual_styles_and_cleanup() {
    let mut s = session();
    run(
        &mut s,
        "theme.set_visual_style",
        json!({"visual_type": "card", "object": "labels", "property": "fontSize", "value": 20}),
    );
    run(
        &mut s,
        "theme.set_visual_style",
        json!({"visual_type": "*", "object": "title", "property": "fontColor", "value": {"color": "#AA0000"}}),
    );
    run(
        &mut s,
        "theme.set_visual_style",
        json!({"visual_type": "actionButton", "object": "fill", "property": "fillColor",
               "value": {"color": "#00AA00"}, "state": "hover"}),
    );
    run(
        &mut s,
        "theme.set_visual_style",
        json!({"visual_type": "actionButton", "object": "fill", "property": "show", "value": true}),
    );
    let doc = stored_theme(&s);
    assert_eq!(
        doc["visualStyles"]["card"]["*"]["labels"],
        json!([{"fontSize": 20}])
    );
    assert_eq!(
        doc["visualStyles"]["*"]["*"]["title"],
        json!([{"fontColor": {"solid": {"color": "#AA0000"}}}])
    );
    // the stateless entry comes first
    assert_eq!(
        doc["visualStyles"]["actionButton"]["*"]["fill"],
        json!([{"show": true}, {"$id": "hover", "fillColor": {"solid": {"color": "#00AA00"}}}])
    );
    assert_valid(&mut s);
    assert_round_trip(report(&s));
    no_op(
        &mut s,
        "theme.set_visual_style",
        json!({"visual_type": "card", "object": "labels", "property": "fontSize", "value": 20}),
    );

    // Removing every style leaves no empty containers.
    for (vt, object, property, state) in [
        ("card", "labels", "fontSize", None),
        ("*", "title", "fontColor", None),
        ("actionButton", "fill", "fillColor", Some("hover")),
        ("actionButton", "fill", "show", None),
    ] {
        run(
            &mut s,
            "theme.set_visual_style",
            json!({"visual_type": vt, "object": object, "property": property, "value": null, "state": state}),
        );
    }
    assert!(stored_theme(&s).get("visualStyles").is_none());
    // removing what is not there changes nothing
    no_op(
        &mut s,
        "theme.set_visual_style",
        json!({"visual_type": "card", "object": "labels", "property": "fontSize", "value": null}),
    );
}

#[test]
fn a_new_custom_theme_starts_from_the_base_theme() {
    let mut s = session();
    with_base_theme(&mut s);
    let got = run(&mut s, "theme.get", json!({}));
    assert_eq!(got["base"]["name"], "CY24SU10");
    assert_eq!(
        got["base"]["resource"],
        "StaticResources/SharedResources/BaseThemes/CY24SU10.json"
    );
    assert_eq!(got["resolved"]["data_colors"], json!(["#010203", "#040506"]));
    run(
        &mut s,
        "theme.set_text_classes",
        json!({"callout": {"font_size": 40}}),
    );
    let doc = stored_theme(&s);
    assert_eq!(doc["dataColors"], json!(["#010203", "#040506"]));
    assert_eq!(doc["foreground"], "#111111");
    assert_eq!(doc["textClasses"], json!({"callout": {"fontSize": 40}}));
    let r = report(&s);
    let tc = r.theme_collection.as_ref().unwrap();
    // baseTheme stays first, customTheme follows
    assert_eq!(
        tc.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["baseTheme", "customTheme"]
    );
    let packages = r.extra["resourcePackages"].as_array().unwrap();
    assert_eq!(packages.len(), 2);
    assert_eq!(packages[1]["name"], "RegisteredResources");
    assert_valid(&mut s);
    assert_round_trip(report(&s));
    // export without a custom theme gives the base theme
    run(&mut s, "theme.remove", json!({}));
    let dir = temp("base-export");
    let out = run(&mut s, "theme.export", json!({"path": dir.join("base.json")}));
    assert_eq!(out["source"], "base");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The PBIR fixture's custom theme is replaced in place on disk, and its old file deleted.
#[test]
fn editing_a_pbir_report_on_disk() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../pbir/tests/fixtures/sales.Report");
    let dir = temp("pbir");
    let target = dir.join("sales.Report");
    let r = bicraft_pbir::read_dir(&fixture).unwrap();
    bicraft_pbir::write_dir(&r, &target).unwrap();
    let old = target.join("StaticResources/RegisteredResources/bicraft-test-theme.json");
    assert!(old.is_file());

    let mut s = Session::new();
    s.report = Some(r);
    let got = run(&mut s, "theme.get", json!({}));
    assert_eq!(got["custom"]["name"], "bicraft-test-theme.json");
    assert_eq!(got["custom"]["theme_name"], "bicraft test theme");
    let out = run(&mut s, "theme.set_colors", json!({"table_accent": "#1F6FB4"}));
    assert_eq!(
        out["removed_resource"],
        "StaticResources/RegisteredResources/bicraft-test-theme.json"
    );
    let name = out["name"].as_str().unwrap().to_string();
    assert!(name.starts_with("bicrafttesttheme"), "{name}");
    // the logo stays registered
    let items = report(&s).extra["resourcePackages"][0]["items"].clone();
    assert_eq!(items[0]["name"], "bicraft-logo.png");
    assert_eq!(items[1]["name"], name.as_str());
    assert_valid(&mut s);
    assert_round_trip(report(&s));

    bicraft_pbir::write_dir(report(&s), &target).unwrap();
    assert!(!old.exists(), "the replaced theme file is deleted");
    assert!(
        target
            .join("StaticResources/RegisteredResources")
            .join(&name)
            .is_file()
    );
    assert!(
        target
            .join("StaticResources/RegisteredResources/bicraft-logo.png")
            .is_file()
    );
    let back = bicraft_pbir::read_dir(&target).unwrap();
    assert_eq!(back.theme_collection, report(&s).theme_collection);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Writes the report as PBIR, reads it back, and checks the theme survives.
fn assert_round_trip(r: &Report) {
    let docs = bicraft_pbir::write_documents(r);
    let issues = bicraft_pbir::validate_documents(&docs);
    assert!(issues.is_empty(), "{issues:#?}");
    let back = bicraft_pbir::read_documents(&docs).unwrap();
    assert_eq!(back.theme_collection, r.theme_collection);
    assert_eq!(
        back.extra.get("resourcePackages"),
        r.extra.get("resourcePackages")
    );
    // the reader lists resources in its own order
    let sorted = |list: &[Resource]| {
        let mut v = list.to_vec();
        v.sort_by(|a, b| a.path.cmp(&b.path));
        v
    };
    assert_eq!(sorted(&back.resources), sorted(&r.resources));
    assert_eq!(crate::theme::resolve(&back), crate::theme::resolve(r));
    // and the written documents are stable
    let mut again = bicraft_pbir::write_documents(&back);
    let mut docs = docs;
    again.sort();
    docs.sort();
    assert!(again == docs, "PBIR write is not stable after a read");
}
