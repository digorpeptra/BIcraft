//! Tests of the `theme.*` commands (`theme_edit.rs`).

use bicraft_report::{Report, Resource};
use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Runs a command that must fail and checks it left the report and its dirty flag alone.
fn fails(s: &mut Session, id: &str, params: Value) -> String {
    let before = s.report.clone();
    let dirty = s.report_dirty;
    let err = s
        .execute(id, &params)
        .expect_err(&format!("{id} {params} should fail"));
    assert_eq!(s.report, before, "{id} changed the report although it failed");
    assert_eq!(s.report_dirty, dirty);
    err
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
        &mut {
            let mut copy = Session::new();
            copy.report = s.report.clone();
            copy
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

#[test]
fn get_on_a_report_without_theme() {
    let mut s = session();
    let out = run(&mut s, "theme.get", json!({}));
    assert_eq!(out["custom"], Value::Null);
    assert_eq!(out["base"], Value::Null);
    assert_eq!(out["resolved"], Value::Null);
    assert_eq!(out["schema_version"], "2.157");
    // theme.get is read-only: the revision does not move
    let rev = s.revision;
    run(&mut s, "theme.get", json!({}));
    assert_eq!(s.revision, rev);
    let mut none = Session::new();
    assert!(none.execute("theme.get", &json!({})).is_err());
}

#[test]
fn bad_parameters_change_nothing() {
    let mut s = session();
    let e = fails(&mut s, "theme.set_colors", json!({"background": "red"}));
    assert!(e.contains("#118DFF"), "{e}");
    fails(&mut s, "theme.set_colors", json!({"data_colors": []}));
    fails(&mut s, "theme.set_colors", json!({"data_colors": ["#123456", 3]}));
    fails(&mut s, "theme.set_colors", json!({"backgroud": "#123456"}));
    fails(&mut s, "theme.set_colors", json!({}));
    fails(
        &mut s,
        "theme.set_text_classes",
        json!({"title": {"font_size": -1}}),
    );
    fails(&mut s, "theme.set_text_classes", json!({"title": {"size": 12}}));
    fails(
        &mut s,
        "theme.set_text_classes",
        json!({"subtitle": {"font_size": 12}}),
    );
    fails(
        &mut s,
        "theme.set_visual_style",
        json!({"visual_type": "card", "object": "labels", "property": "fontSize"}),
    );
    fails(
        &mut s,
        "theme.set_visual_style",
        json!({"visual_type": "", "object": "labels", "property": "fontSize", "value": 1}),
    );
    fails(
        &mut s,
        "theme.set_visual_style",
        json!({"visual_type": "card", "object": "labels", "property": "color", "value": {"color": "blue"}}),
    );
    fails(&mut s, "theme.set", json!({"theme": [1, 2]}));
    fails(&mut s, "theme.set", json!({"theme": {"dataColors": ["#123456"]}}));
    fails(&mut s, "theme.set", json!({"theme": "{not json"}));
    // hostile report.json shapes
    s.report
        .as_mut()
        .unwrap()
        .extra
        .insert("resourcePackages".into(), json!("x"));
    let e = fails(&mut s, "theme.set_colors", json!({"good": "#00FF00"}));
    assert!(e.contains("resourcePackages"), "{e}");
}

#[test]
fn schema_validation_and_force() {
    let mut s = session();
    // card label sizes are 8..60 (visual catalog and report theme schema)
    let e = fails(
        &mut s,
        "theme.set_visual_style",
        json!({"visual_type": "card", "object": "labels", "property": "fontSize", "value": 100}),
    );
    assert!(e.contains("60"), "{e}");
    let out = run(
        &mut s,
        "theme.set_visual_style",
        json!({"visual_type": "card", "object": "labels", "property": "fontSize", "value": 100, "force": true}),
    );
    let w = out["warnings"].to_string();
    assert!(w.contains("theme schema") && w.contains("fontSize"), "{out}");
    run(&mut s, "theme.remove", json!({}));
    let e = fails(
        &mut s,
        "theme.set",
        json!({"theme": {"name": "x", "dataColors": ["red"]}}),
    );
    assert!(e.contains("/dataColors/0"), "{e}");
    let out = run(
        &mut s,
        "theme.set",
        json!({"theme": {"name": "x", "dataColors": ["red"]}, "force": true}),
    );
    assert_eq!(out["changed"], true);
    assert!(
        out["warnings"][0].as_str().unwrap().contains("/dataColors/0"),
        "{out}"
    );
    // a theme that already breaks the schema can still be edited: only new problems count
    let out = run(&mut s, "theme.set_colors", json!({"good": "#00FF00"}));
    assert_eq!(out["changed"], true);
    let got = run(&mut s, "theme.get", json!({}));
    assert_eq!(got["custom"]["issues"].as_array().unwrap().len(), 1, "{got}");
}

#[test]
fn text_classes() {
    let mut s = session();
    let out = run(
        &mut s,
        "theme.set_text_classes",
        json!({"title": {"font_face": "Georgia", "font_size": 16, "color": "#202020"},
               "label": {"font_face": "Segoe UI Light", "font_size": 11}}),
    );
    assert_eq!(out["resolved"]["font_family"], "Segoe UI Light");
    assert_eq!(out["resolved"]["font_size"], 11);
    let doc = stored_theme(&s);
    assert_eq!(
        doc["textClasses"]["title"],
        json!({"fontFace": "Georgia", "fontSize": 16, "color": "#202020"})
    );
    no_op(
        &mut s,
        "theme.set_text_classes",
        json!({"title": {"font_size": 16}}),
    );
    run(
        &mut s,
        "theme.set_text_classes",
        json!({"title": {"color": null}}),
    );
    assert_eq!(
        stored_theme(&s)["textClasses"]["title"],
        json!({"fontFace": "Georgia", "fontSize": 16})
    );
    run(
        &mut s,
        "theme.set_text_classes",
        json!({"title": null, "label": null}),
    );
    assert!(stored_theme(&s).get("textClasses").is_none());
    assert_valid(&mut s);
}

#[test]
fn unknown_visual_types_warn_and_catalog_mismatches_fail() {
    let mut s = session();
    let out = run(
        &mut s,
        "theme.set_visual_style",
        json!({"visual_type": "myCustomVisual1234", "object": "general", "property": "x", "value": 1}),
    );
    assert!(
        out["warnings"][0].as_str().unwrap().contains("custom visual"),
        "{out}"
    );
    let bad = json!({"visual_type": "card", "object": "labels", "property": "noSuchProperty", "value": 1});
    if bicraft_visuals::visual("card").is_some() {
        let e = fails(&mut s, "theme.set_visual_style", bad.clone());
        assert!(e.contains("noSuchProperty") && e.contains("fontSize"), "{e}");
        let e = fails(
            &mut s,
            "theme.set_visual_style",
            json!({"visual_type": "card", "object": "noSuchObject", "property": "show", "value": true}),
        );
        assert!(e.contains("noSuchObject"), "{e}");
        let e = fails(
            &mut s,
            "theme.set_visual_style",
            json!({"visual_type": "card", "object": "labels", "property": "bold", "value": "yes"}),
        );
        assert!(e.contains("bold"), "{e}");
    }
    let mut forced = bad;
    forced["force"] = json!(true);
    let out = run(&mut s, "theme.set_visual_style", forced);
    assert!(out["warnings"].to_string().contains("noSuchProperty"), "{out}");
}

#[test]
fn edits_without_a_custom_theme_and_nothing_to_change_create_nothing() {
    let mut s = session();
    no_op(&mut s, "theme.set_colors", json!({"good": null}));
    no_op(
        &mut s,
        "theme.set_visual_style",
        json!({"visual_type": "card", "object": "labels", "property": "fontSize", "value": null}),
    );
    assert!(report(&s).extra.get("resourcePackages").is_none());
}

#[test]
fn remove_goes_back_to_the_base_theme() {
    let mut s = session();
    no_op(&mut s, "theme.remove", json!({}));
    run(&mut s, "theme.set_colors", json!({"good": "#00FF00"}));
    let out = run(&mut s, "theme.remove", json!({}));
    assert_eq!(out["changed"], true);
    assert!(
        out["removed_resource"]
            .as_str()
            .unwrap()
            .starts_with("StaticResources/RegisteredResources/")
    );
    let r = report(&s);
    assert_eq!(r.theme_collection, Some(json!({})));
    assert!(r.extra.get("resourcePackages").is_none());
    assert!(custom_resources(r).is_empty());
    assert_valid(&mut s);
    no_op(&mut s, "theme.remove", json!({}));
}

#[test]
fn remove_keeps_other_resources() {
    let mut s = session();
    {
        let r = s.report.as_mut().unwrap();
        r.extra.insert(
            "resourcePackages".into(),
            json!([{"name": "RegisteredResources", "type": "RegisteredResources",
                    "items": [{"name": "logo.png", "path": "logo.png", "type": "Image"}]}]),
        );
        r.resources.push(Resource {
            path: "StaticResources/RegisteredResources/logo.png".into(),
            bytes: vec![1, 2, 3],
        });
    }
    run(&mut s, "theme.set_colors", json!({"good": "#00FF00"}));
    assert_eq!(
        report(&s).extra["resourcePackages"][0]["items"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    run(&mut s, "theme.remove", json!({}));
    let r = report(&s);
    assert_eq!(
        r.extra["resourcePackages"],
        json!([{"name": "RegisteredResources", "type": "RegisteredResources",
                "items": [{"name": "logo.png", "path": "logo.png", "type": "Image"}]}])
    );
    assert_eq!(r.resources.len(), 1);
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
fn legacy_report_schemas_get_a_version_string() {
    let mut s = session();
    with_base_theme(&mut s);
    let r = s.report.as_mut().unwrap();
    r.schemas.insert(
        "report".into(),
        json!("https://developer.microsoft.com/json-schemas/fabric/item/report/definition/report/2.1.0/schema.json"),
    );
    r.theme_collection = Some(
        json!({"baseTheme": {"name": "CY24SU10", "reportVersionAtImport": "5.55", "type": "SharedResources"}}),
    );
    run(&mut s, "theme.set_colors", json!({"good": "#00FF00"}));
    assert_eq!(
        report(&s).theme_collection.as_ref().unwrap()["customTheme"]["reportVersionAtImport"],
        "5.55"
    );
}

#[test]
fn a_custom_theme_without_a_base_theme_keeps_the_classic_look() {
    // Power BI Desktop 2.159.1231.0 (2026-11-24) drew a report whose only theme is a custom one
    // (data colours alone, no base theme) with the classic look: light grey regular titles, as
    // without any theme. The resolved theme says so; with a base theme it does not.
    let mut s = session();
    run(
        &mut s,
        "theme.set_colors",
        json!({"data_colors": ["#7B2CBF", "#FF8C00"]}),
    );
    let t = s.snapshot()["report"]["theme"].clone();
    assert_eq!(t["classic"], true, "{t:#}");
    assert_eq!(t["data_colors"], json!(["#7B2CBF", "#FF8C00"]));
    let mut b = session();
    with_base_theme(&mut b);
    run(&mut b, "theme.set_colors", json!({"data_colors": ["#7B2CBF"]}));
    let t = b.snapshot()["report"]["theme"].clone();
    assert!(t.get("classic").is_none(), "{t:#}");
}

#[test]
fn import_and_export() {
    let dir = temp("io");
    let mut s = session();
    let theme = json!({
        "name": "Harbor",
        "dataColors": ["#0B3C5D", "#328CC1", "#D9B310", "#1D2731"],
        "background": "#FFFFFF",
        "foreground": "#1D2731",
        "tableAccent": "#328CC1",
        "textClasses": {"title": {"fontFace": "Segoe UI Semibold", "fontSize": 14, "color": "#0B3C5D"}},
        "visualStyles": {"card": {"*": {"labels": [{"fontSize": 24}]}}}
    });
    let src = dir.join("harbor.json");
    // BOM and CRLF, as Windows editors write it
    let mut bytes = b"\xEF\xBB\xBF".to_vec();
    bytes.extend(
        serde_json::to_string_pretty(&theme)
            .unwrap()
            .replace('\n', "\r\n")
            .into_bytes(),
    );
    std::fs::write(&src, bytes).unwrap();
    let out = run(&mut s, "theme.import", json!({"path": src}));
    assert_eq!(out["changed"], true);
    assert!(out["name"].as_str().unwrap().starts_with("Harbor"));
    assert_eq!(out["resolved"]["name"], "Harbor");
    assert_eq!(stored_theme(&s), theme);

    let dst = dir.join("out.json");
    let out = run(&mut s, "theme.export", json!({"path": dst}));
    assert_eq!(out["source"], "custom");
    let exported: Value = serde_json::from_slice(&std::fs::read(&dst).unwrap()).unwrap();
    assert_eq!(exported, theme);
    // export refuses to overwrite unless asked
    assert!(s.execute("theme.export", &json!({"path": dst})).is_err());
    run(&mut s, "theme.export", json!({"path": dst, "overwrite": true}));
    // importing the exported file (or setting the same JSON) is a no-op
    no_op(&mut s, "theme.import", json!({"path": dst}));
    no_op(&mut s, "theme.set", json!({"theme": theme}));
    no_op(&mut s, "theme.set", json!({"theme": theme.to_string()}));

    // limits and errors
    fails(&mut s, "theme.import", json!({"path": dir.join("missing.json")}));
    let big = dir.join("big.json");
    std::fs::write(&big, vec![b' '; (4 << 20) + 1]).unwrap();
    let e = fails(&mut s, "theme.import", json!({"path": big}));
    assert!(e.contains("limit"), "{e}");
    let mut empty = session();
    assert!(
        empty
            .execute("theme.export", &json!({"path": dir.join("x.json")}))
            .is_err()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_viewer_shows_the_edited_theme() {
    let mut s = session();
    run(
        &mut s,
        "theme.set_colors",
        json!({"data_colors": ["#123456"], "foreground": "#222222"}),
    );
    run(
        &mut s,
        "theme.set_text_classes",
        json!({"label": {"font_face": "Georgia", "font_size": 12}}),
    );
    run(
        &mut s,
        "theme.set_visual_style",
        json!({"visual_type": "card", "object": "title", "property": "show", "value": true}),
    );
    run(
        &mut s,
        "theme.set_visual_style",
        json!({"visual_type": "*", "object": "title", "property": "fontColor", "value": {"color": "#AA0000"}}),
    );
    let snap = s.snapshot();
    let theme = &snap["report"]["theme"];
    assert_eq!(theme["data_colors"], json!(["#123456"]));
    assert_eq!(theme["foreground"], "#222222");
    assert_eq!(theme["font_family"], "Georgia");
    assert_eq!(theme["font_size"], 12);
    let card = &snap["report"]["pages"][0]["visuals"][0];
    assert_eq!(card["format"]["title"]["show"], true, "{card:#}");
    assert_eq!(card["format"]["title"]["color"], "#AA0000", "{card:#}");
    run(&mut s, "theme.remove", json!({}));
    assert_eq!(s.snapshot()["report"]["theme"], Value::Null);
}

#[test]
fn hostile_theme_collections() {
    let mut s = session();
    // customTheme pointing at a missing file: an edit starts a new theme and says so
    s.report.as_mut().unwrap().theme_collection = Some(json!({"customTheme": {
        "name": "gone.json", "reportVersionAtImport": {"visual": "2.12.0", "report": "3.3.0", "page": "2.1.0"},
        "type": "RegisteredResources"}}));
    let got = run(&mut s, "theme.get", json!({}));
    assert_eq!(got["custom"]["resource"], Value::Null);
    let out = run(&mut s, "theme.set_colors", json!({"good": "#00FF00"}));
    assert!(
        out["warnings"][0].as_str().unwrap().contains("gone.json"),
        "{out}"
    );
    assert_valid(&mut s);
    // a non-object themeCollection is refused
    s.report.as_mut().unwrap().theme_collection = Some(json!([1]));
    fails(&mut s, "theme.set_colors", json!({"good": "#00FF00"}));
    // a theme file holding garbage
    let mut s = session();
    run(&mut s, "theme.set_colors", json!({"good": "#00FF00"}));
    for res in &mut s.report.as_mut().unwrap().resources {
        res.bytes = b"\x00\xFFnot json".to_vec();
    }
    let got = run(&mut s, "theme.get", json!({}));
    assert_eq!(got["custom"]["theme"], Value::Null);
    run(&mut s, "theme.set_colors", json!({"bad": "#FF0000"}));
    assert_eq!(custom_resources(report(&s)).len(), 1);
    assert_valid(&mut s);
}

/// theme.set {builtin}: each of BIcraft's built-in themes installs without a schema warning,
/// theme.get lists them, the snapshot shows the theme, and PBIP and PBIX files keep it.
#[test]
fn builtin_themes_install_and_are_kept_in_files() {
    let mut s = session();
    let got = run(&mut s, "theme.get", json!({}));
    let ids: Vec<String> = got["builtin"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(ids.len(), 8, "{got}");
    for id in &ids {
        let out = run(&mut s, "theme.set", json!({"builtin": id}));
        assert_eq!(out["warnings"].as_array().map_or(0, Vec::len), 0, "{id}: {out}");
        let name = s.snapshot()["report"]["theme"]["name"].clone();
        assert!(name.as_str().unwrap().starts_with("BIcraft"), "{id}: {name}");
        assert_valid(&mut s);
        no_op(&mut s, "theme.set", json!({"builtin": id}));
    }
    fails(&mut s, "theme.set", json!({"builtin": "Default"}));
    fails(&mut s, "theme.set", json!({"builtin": 3}));
    fails(
        &mut s,
        "theme.set",
        json!({"builtin": "dusk", "theme": {"name": "x"}}),
    );
    fails(&mut s, "theme.set", json!({}));

    let dir = temp("builtin");
    for file in ["Themed.pbip", "Themed.pbix"] {
        let mut s = Session::new();
        run(&mut s, "model.new", json!({"name": "Themed"}));
        run(&mut s, "report.new", json!({}));
        run(&mut s, "theme.set", json!({"builtin": "dusk"}));
        let before = stored_theme(&s);
        run(&mut s, "file.save_as", json!({"path": dir.join(file)}));
        let mut back = Session::new();
        run(&mut back, "file.open", json!({"path": dir.join(file)}));
        assert_eq!(stored_theme(&back), before, "{file}");
        let snap = back.snapshot();
        assert_eq!(snap["report"]["theme"]["name"], "BIcraft Dusk", "{file}");
        assert_eq!(snap["report"]["theme"]["background"], "#1E2130", "{file}");
        assert_eq!(snap["report"]["pages"][0]["background"], "#1E2130", "{file}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// theme.customize (the Customize theme dialog's Apply): the name, colours, text classes and
/// visual styles in one edit and one undo step; the snapshot shows what the dialog reads; PBIP
/// and PBIX files keep it; bad parts fail and change nothing.
#[test]
fn customize_sets_everything_in_one_undo_step() {
    let params = json!({
        "name": "Harbour lights",
        "data_colors": ["#112233", "#445566", "#778899"],
        "good": "#1AAB40", "neutral": "#D9B300", "bad": "#D64554",
        "maximum": "#118DFF", "center": "#D9B300", "minimum": "#DEEFFF",
        "text_classes": {
            "label": {"font_face": "Segoe UI", "font_size": 11, "color": "#252423"},
            "title": {"font_face": "Georgia", "font_size": 14},
            "callout": {"font_size": 40, "color": "#123456"},
            "header": {"font_face": "Verdana"}
        },
        "styles": [
            {"visual_type": "*", "object": "background", "property": "color", "value": {"color": "#FAFAFA"}},
            {"visual_type": "*", "object": "background", "property": "transparency", "value": 10},
            {"visual_type": "*", "object": "border", "property": "show", "value": true},
            {"visual_type": "*", "object": "border", "property": "radius", "value": 6},
            {"visual_type": "*", "object": "visualHeader", "property": "background", "value": {"color": "#EEEEEE"}},
            {"visual_type": "*", "object": "visualTooltip", "property": "titleFontColor", "value": {"color": "#333333"}},
            {"visual_type": "page", "object": "outspace", "property": "color", "value": {"color": "#F3F2F1"}},
            {"visual_type": "page", "object": "background", "property": "transparency", "value": 0},
            {"visual_type": "page", "object": "outspacePane", "property": "backgroundColor", "value": {"color": "#FFFFFF"}},
            {"visual_type": "page", "object": "filterCard", "state": "Applied", "property": "backgroundColor", "value": {"color": "#E6E6E6"}},
            {"visual_type": "page", "object": "filterCard", "state": "Available", "property": "foregroundColor", "value": {"color": "#252423"}}
        ]
    });
    let mut s = session();
    let before = s.report.clone();
    let out = run(&mut s, "theme.customize", params.clone());
    assert_eq!(out["created"], true, "{out}");
    assert_eq!(out["warnings"].as_array().map_or(0, Vec::len), 0, "{out}");
    let doc = stored_theme(&s);
    assert_eq!(doc["name"], "Harbour lights");
    assert_eq!(doc["dataColors"], json!(["#112233", "#445566", "#778899"]));
    assert_eq!(doc["bad"], "#D64554");
    assert_eq!(doc["textClasses"]["title"]["fontFace"], "Georgia");
    assert_eq!(
        doc["visualStyles"]["*"]["*"]["background"][0],
        json!({"color": {"solid": {"color": "#FAFAFA"}}, "transparency": 10})
    );
    assert_eq!(
        doc["visualStyles"]["page"]["*"]["filterCard"],
        json!([{"$id": "Applied", "backgroundColor": {"solid": {"color": "#E6E6E6"}}},
               {"$id": "Available", "foregroundColor": {"solid": {"color": "#252423"}}}])
    );
    assert_valid(&mut s);
    // what the dialog reads back
    let t = s.snapshot()["report"]["theme"].clone();
    assert_eq!(t["name"], "Harbour lights");
    assert_eq!(t["good"], "#1AAB40");
    assert_eq!(t["minimum"], "#DEEFFF");
    assert_eq!(
        t["text_classes"]["callout"],
        json!({"font_size": 40, "color": "#123456"})
    );
    assert_eq!(t["text_classes"]["header"]["font_face"], "Verdana");
    assert_eq!(
        t["styles"]["*"]["background"],
        json!({"color": "#FAFAFA", "transparency": 10})
    );
    assert_eq!(t["styles"]["*"]["border"], json!({"show": true, "radius": 6}));
    assert_eq!(t["styles"]["page"]["outspace"]["color"], "#F3F2F1");
    assert_eq!(
        t["styles"]["page"]["filterCard.Applied"]["backgroundColor"],
        "#E6E6E6"
    );
    assert_eq!(
        t["styles"]["page"]["filterCard.Available"]["foregroundColor"],
        "#252423"
    );
    // the same again changes nothing
    no_op(&mut s, "theme.customize", params.clone());
    // one undo step
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.report, before);
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(stored_theme(&s), doc);
    // a later edit keeps the rest
    run(
        &mut s,
        "theme.customize",
        json!({"name": "Renamed", "text_classes": {"title": null}}),
    );
    let after = stored_theme(&s);
    assert_eq!(after["name"], "Renamed");
    assert_eq!(after["textClasses"].get("title"), None);
    assert_eq!(after["dataColors"], doc["dataColors"]);

    for bad in [
        json!({}),
        json!({"name": ""}),
        json!({"name": 3}),
        json!({"good": "green"}),
        json!({"text_classes": []}),
        json!({"text_classes": {"bogus": {}}}),
        json!({"text_classes": {"title": {"font_size": -1}}}),
        json!({"styles": {}}),
        json!({"styles": [3]}),
        json!({"styles": [{"visual_type": "*", "object": "border", "property": "show"}]}),
        json!({"styles": [{"visual_type": "card", "object": "nope", "property": "x", "value": 1}]}),
        json!({"styles": [{"visual_type": "*", "object": "border", "property": "show", "value": true, "extra": 1}]}),
        json!({"name": "Ok", "styles": [{"visual_type": "card", "object": "nope", "property": "x", "value": 1}]}),
        json!({"unknown": 1}),
    ] {
        fails(&mut s, "theme.customize", bad);
    }

    let dir = temp("customize");
    for file in ["Custom.pbip", "Custom.pbix"] {
        let mut s = Session::new();
        run(&mut s, "model.new", json!({"name": "Custom"}));
        run(&mut s, "report.new", json!({}));
        run(&mut s, "theme.customize", params.clone());
        let before = stored_theme(&s);
        let snap_before = s.snapshot()["report"]["theme"].clone();
        run(&mut s, "file.save_as", json!({"path": dir.join(file)}));
        let mut back = Session::new();
        run(&mut back, "file.open", json!({"path": dir.join(file)}));
        assert_eq!(stored_theme(&back), before, "{file}");
        assert_eq!(back.snapshot()["report"]["theme"], snap_before, "{file}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
