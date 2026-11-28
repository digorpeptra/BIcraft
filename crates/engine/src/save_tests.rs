//! The save tests that need the engine's private items (`save::build_package`,
//! `visual_extras::mobile_position`); the others are the `files` integration test
//! (`tests/files/save_tests.rs`), under the same module path.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::tempdir::TempDir;
use crate::{Session, Source};

fn temp(name: &str) -> TempDir {
    TempDir::new(&format!("engine-save-{name}"))
}

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

const SYNTHETIC_METADATA: &[u8] = include_bytes!("../../metadata/tests/fixtures/synthetic_metadata.sqlitedb");

/// A synthetic PBIX (model: tables Calendar and Sales) written to `dir/name`.
fn synthetic_pbix(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    let pkg = bicraft_pbix::synthetic::pbix(SYNTHETIC_METADATA).unwrap();
    std::fs::write(&path, pkg.to_bytes().unwrap()).unwrap();
    path
}

fn reopen(path: &Path) -> Session {
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": path}));
    assert_eq!(s.model_error, None);
    s
}

/// A synthetic PBIX with a PBIR report (page `p1`, card `v1`) written to `dir/name`.
fn synthetic_pbir_pbix(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    let pkg = bicraft_pbix::synthetic::pbir_pbix(SYNTHETIC_METADATA).unwrap();
    std::fs::write(&path, pkg.to_bytes().unwrap()).unwrap();
    path
}

fn part(path: &Path, name: &str) -> Option<Vec<u8>> {
    let pkg = bicraft_opc::Package::open(path).unwrap();
    pkg.get(name).map(|p| p.data.clone())
}

fn part_names(path: &Path) -> Vec<String> {
    let pkg = bicraft_opc::Package::open(path).unwrap();
    pkg.parts.into_iter().map(|p| p.name).collect()
}

/// Every M2.5 edit survives a save into a PBIX holding PBIR parts: series and conditional
/// selectors, analytics, axes, phone layouts, interactions, tooltip and drillthrough pages, the
/// report-page tooltip, the custom theme.
#[test]
fn m25_edits_are_saved_into_a_pbir_pbix() {
    let dir = temp("m25-pbir");
    let path = synthetic_pbir_pbix(&dir, "Shop.pbix");
    let mut s = reopen(&path);
    let (chart, table) = format_report(&mut s, "Overview");
    let tip = run(&mut s, "page.add", json!({"display_name": "Tip"}))["page"]
        .as_str()
        .unwrap()
        .to_string();
    run(
        &mut s,
        "page.set_type",
        json!({"page": tip, "type": "tooltip", "fields": ["Calendar[Month]"]}),
    );
    let detail = run(&mut s, "page.add", json!({"display_name": "Detail"}))["page"]
        .as_str()
        .unwrap()
        .to_string();
    run(
        &mut s,
        "page.set_type",
        json!({"page": detail, "type": "drillthrough", "fields": ["Calendar[Month]"], "back_button": true}),
    );
    run(
        &mut s,
        "visual.tooltip_page",
        json!({"page": "Overview", "visual": chart, "tooltip_page": tip}),
    );
    run(
        &mut s,
        "theme.set_colors",
        json!({"data_colors": ["#123456", "#654321"]}),
    );
    let out = run(&mut s, "file.save", json!({"backup": false}));
    assert_eq!(out["report_format"], "pbir", "{out}");

    let names = part_names(&path);
    let has = |suffix: &str| names.iter().any(|n| n.ends_with(suffix));
    assert!(has(&format!("/visuals/{chart}/mobile.json")), "{names:?}");
    assert!(
        names
            .iter()
            .any(|n| n.starts_with("Report/StaticResources/RegisteredResources/") && n.ends_with(".json")),
        "{names:?}"
    );
    let mut fresh = reopen(&path);
    assert_eq!(fresh.report, s.report);
    assert_formatted(&mut fresh, "Overview", &chart, &table);
    let r = fresh.report.as_ref().unwrap();
    for (page, kind) in [(&tip, "Tooltip"), (&detail, "Drillthrough")] {
        let p = r.page(page).unwrap();
        assert_eq!(p.extra["type"], kind);
        assert_eq!(p.extra["pageBinding"]["type"], kind);
    }
    assert!(
        r.page(&detail)
            .unwrap()
            .visuals
            .iter()
            .any(|v| v.visual_type.as_deref() == Some("actionButton"))
    );
    assert_eq!(
        r.theme_collection.as_ref().unwrap()["customTheme"]["type"],
        "RegisteredResources"
    );
    assert!(
        r.extra["resourcePackages"]
            .to_string()
            .contains("\"CustomTheme\"")
    );
    let theme = run(&mut fresh, "theme.get", json!({}));
    assert!(theme.to_string().contains("#123456"), "{theme}");
    let check = run(&mut fresh, "report.validate", json!({}));
    assert_eq!(check["schema_issues"], json!([]), "{check}");
    assert_eq!(check["error_count"], 0, "{check}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// M2.5 edits in a PBIX whose report is a legacy `Report/Layout`: formatting, phone layouts,
/// interactions and renames are written in the legacy form; what it cannot hold (page bindings,
/// themes) is refused, naming each, and nothing is saved.
#[test]
fn m25_edits_in_a_legacy_pbix() {
    let dir = temp("m25-legacy");
    let path = synthetic_pbix(&dir, "Shop.pbix");
    let mut s = reopen(&path);
    let (chart, table) = format_report(&mut s, "Page 1");
    let out = run(&mut s, "file.save", json!({"backup": false}));
    assert_eq!(out["report_format"], "legacy", "{out}");
    assert!(
        !part_names(&path)
            .iter()
            .any(|n| n.starts_with("Report/definition/"))
    );
    let mut fresh = reopen(&path);
    assert_eq!(fresh.report, s.report);
    assert_formatted(&mut fresh, "Page 1", &chart, &table);
    let layout = || {
        let data = part(&path, "Report/Layout").unwrap();
        bicraft_opc::decode_text("Report/Layout", &data).unwrap().0
    };
    // the phone layout is the container's layout with id 1
    assert!(
        layout().contains(r#"\"id\":1,\"position\":{\"x\":0,\"y\":10,\"z\":0,\"height\":200,\"width\":320}"#),
        "{}",
        layout()
    );

    // Phone layout edits patch the container; removing it drops the layout.
    let mut s = fresh;
    let moved = run(
        &mut s,
        "visual.mobile",
        json!({"page": "Page 1", "visual": chart, "x": 40}),
    );
    assert_eq!(moved["changed"], true, "{moved}");
    assert_eq!(moved["mobile"]["x"], 40);
    let again = run(
        &mut s,
        "visual.mobile",
        json!({"page": "Page 1", "visual": chart, "x": 40}),
    );
    assert_eq!(again["changed"], false, "{again}");
    run(&mut s, "file.save", json!({"backup": false}));
    assert!(
        layout().contains(r#"\"id\":1,\"position\":{\"x\":40,"#),
        "{}",
        layout()
    );
    run(
        &mut s,
        "visual.mobile",
        json!({"page": "Page 1", "visual": table, "x": 0, "y": 300, "width": 320, "height": 100}),
    );
    run(
        &mut s,
        "visual.mobile",
        json!({"page": "Page 1", "visual": chart, "layout": null}),
    );
    run(&mut s, "file.save", json!({"backup": false}));
    let mut fresh = reopen(&path);
    let r = fresh.report.as_ref().unwrap();
    let page = r.pages[0].name.clone();
    assert_eq!(crate::visual_extras::mobile_position(r, &page, &chart), None);
    assert_eq!(
        crate::visual_extras::mobile_position(r, &page, &table),
        Some(json!({"x": 0, "y": 300, "width": 320, "height": 100}))
    );

    // A measure rename follows into the legacy queryRefs and selectors.
    run(
        &mut fresh,
        "measure.set",
        json!({"name": "Total", "new_name": "Revenue", "update_report": true}),
    );
    run(&mut fresh, "file.save", json!({"backup": false}));
    let mut renamed = reopen(&path);
    let props = run(
        &mut renamed,
        "visual.props",
        json!({"page": "Page 1", "visual": chart}),
    )["props"]
        .clone();
    assert_eq!(
        props["series"][0],
        json!({"target": "[Revenue]", "color": "#118DFF"}),
        "{props}"
    );
    assert!(layout().contains(r#"\"metadata\":\"Sales.Revenue\""#));
    assert!(!layout().contains("Sales.Total"));

    // Page bindings and themes have no legacy form: refused, each named, nothing saved.
    let before = std::fs::read(&path).unwrap();
    run(
        &mut renamed,
        "page.set_type",
        json!({"page": "Page 1", "type": "tooltip"}),
    );
    run(
        &mut renamed,
        "theme.set_colors",
        json!({"data_colors": ["#123456"]}),
    );
    let e = renamed.execute("file.save", &json!({})).unwrap_err();
    for what in ["page binding", "theme", "resourcePackages"] {
        assert!(e.contains(what), "{what}: {e}");
    }
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(renamed.report_dirty);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Report-only save of the local sample PBIX: building the package with the model rewritten
/// (what every package save did before report saves landed) against leaving `DataModel` as is.
/// `BICRAFT_TESTDATA=… cargo test -p bicraft-engine --release report_only_save_timing -- --ignored --nocapture`
#[test]
#[ignore = "timing; run in release"]
fn report_only_save_timing() {
    let Some(root) = bicraft_env::var_os("BICRAFT_TESTDATA") else {
        eprintln!("BICRAFT_TESTDATA is not set: skipped");
        return;
    };
    let path = PathBuf::from(root).join("sample1").join("report.pbix");
    if !path.exists() {
        eprintln!("{} is missing: skipped", path.display());
        return;
    }
    let mut s = reopen(&path);
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    let visual = s.report.as_ref().unwrap().pages[0].visuals[0].name.clone();
    run(
        &mut s,
        "visual.set_position",
        json!({"page": page, "visual": visual, "x": 7}),
    );
    let Some(Source::Package { package, .. }) = &s.source else {
        panic!("not a package")
    };
    let (model, report) = (s.model.as_ref().unwrap(), s.report.as_ref().unwrap());
    let time = |m: Option<&bicraft_model::SemanticModel>| {
        let start = std::time::Instant::now();
        for _ in 0..5 {
            crate::save::build_package(package, m, Some(report)).unwrap();
        }
        start.elapsed() / 5
    };
    let with_model = time(Some(model));
    let report_only = time(None);
    eprintln!(
        "report-only save of the sample: model rewritten {with_model:?}, DataModel kept {report_only:?}"
    );
}

/// Chart and table formatting of M2.5 on `page`: series colors and labels, analytics lines, an
/// axis, a conditional format, a phone layout, interactions. Returns (chart, table).
fn format_report(s: &mut Session, page: &str) -> (String, String) {
    let add = |s: &mut Session, visual_type: &str, x: u32, fields: Value| -> String {
        run(
            s,
            "visual.add",
            json!({"page": page, "visual_type": visual_type, "x": x, "y": 200, "width": 300,
                   "height": 200, "fields": fields}),
        )["visual"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let chart = add(
        s,
        "clusteredColumnChart",
        10,
        json!([{"role": "Category", "field": "Calendar[Month]"}, {"role": "Y", "field": "[Total]"},
               {"role": "Y", "field": "[Rows]"}]),
    );
    let table = add(
        s,
        "tableEx",
        400,
        json!([{"role": "Values", "field": "Calendar[Month]"}, {"role": "Values", "field": "[Total]"}]),
    );
    run(
        s,
        "visual.format",
        json!({"page": page, "visual": chart,
               "colors": {"[Total]": "#118DFF", "Calendar[Month]=Jan": "#E66C37"},
               "series": [{"target": "[Rows]", "labels": {"color": "#444444"}}]}),
    );
    run(
        s,
        "visual.analytics",
        json!({"page": page, "visual": chart, "lines": [{"kind": "constant", "value": 5},
                                                         {"kind": "average", "measure": "[Total]"}]}),
    );
    run(
        s,
        "visual.axis",
        json!({"page": page, "visual": chart, "axis": "y", "min": 0, "gridlines": false}),
    );
    run(
        s,
        "visual.conditional",
        json!({"page": page, "visual": table, "target": "background", "series": "[Total]",
               "rule": {"gradient": {"field": "[Total]", "min": "#FFFFFF", "max": "#00AA00"}}}),
    );
    // whole-visual conditional formats (visualContainerObjects) and the data labels' colour
    run(
        s,
        "visual.conditional",
        json!({"page": page, "visual": chart, "conditional": [
            {"target": "title_text", "rule": {"field_value": {"field": "[Rows]"}}},
            {"target": "title_color", "rule": {"rules": {"field": "[Total]",
                "cases": [{"op": ">", "value": 10, "color": "#AA0000"}], "else": "#00AA00"}}},
            {"target": "visual_background", "rule": {"gradient": {"field": "[Total]", "min": "#FFFFFF", "max": "#0000AA"}}},
            {"target": "label_color", "rule": {"gradient": {"field": "[Total]", "min": "#111111", "max": "#EEEEEE"}}}]}),
    );
    run(
        s,
        "visual.mobile",
        json!({"page": page, "visual": chart, "x": 0, "y": 10, "width": 320, "height": 200}),
    );
    run(
        s,
        "page.interactions",
        json!({"page": page, "rules": [{"source": chart, "target": table, "type": "none"}]}),
    );
    (chart, table)
}

/// What [`format_report`] wrote, as `visual.props` and the page reads it.
fn assert_formatted(s: &mut Session, page: &str, chart: &str, table: &str) {
    let props = run(s, "visual.props", json!({"page": page, "visual": chart}))["props"].clone();
    assert_eq!(
        props["series"],
        json!([{"target": "[Total]", "color": "#118DFF"}, {"target": "Calendar[Month]=Jan", "color": "#E66C37"},
               {"target": "[Rows]", "labels": {"color": "#444444"}}]),
        "{props}"
    );
    assert_eq!(props["analytics"].as_array().unwrap().len(), 2, "{props}");
    assert_eq!(props["axes"]["y"]["min"], 0.0, "{props}");
    let targets: Vec<&str> = props["conditional"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["target"].as_str())
        .collect();
    for t in ["title_text", "title_color", "visual_background", "label_color"] {
        assert!(targets.contains(&t), "{t} missing: {props}");
    }
    let props = run(s, "visual.props", json!({"page": page, "visual": table}))["props"].clone();
    assert_eq!(
        props["conditional"][0]["rule"]["gradient"]["max"], "#00AA00",
        "{props}"
    );
    let r = s.report.as_ref().unwrap();
    assert_eq!(
        crate::visual_extras::mobile_position(r, &r.page(page).unwrap().name, chart),
        Some(json!({"x": 0, "y": 10, "width": 320, "height": 200}))
    );
    assert_eq!(
        r.page(page).unwrap().extra["visualInteractions"],
        json!([{"source": chart, "target": table, "type": "NoFilter"}])
    );
}

/// Places `chart` and `table` of `page` on the phone layout, the chart formatted for it (a
/// smaller title, a background); returns what `visual.props {layout: mobile}` reads for the chart.
fn mobile_edits(s: &mut Session, page: &str, chart: &str, table: &str) -> Value {
    run(
        s,
        "visual.mobile",
        json!({"page": page, "visuals": [
            {"visual": chart, "x": 0, "y": 0, "width": 323, "height": 200},
            {"visual": table, "x": 0, "y": 208, "width": 323, "height": 240}]}),
    );
    run(
        s,
        "visual.mobile_format",
        json!({"page": page, "visual": chart, "title": {"show": true, "font_size": 9},
               "background": {"color": "#FFEEDD"}}),
    );
    run(
        s,
        "visual.props",
        json!({"page": page, "visual": chart, "layout": "mobile"}),
    )["props"]
        .clone()
}

/// What [`mobile_edits`] wrote, read back after a reopen.
fn assert_mobile(s: &mut Session, page: &str, chart: &str, table: &str, props: &Value) {
    let r = s.report.as_ref().unwrap();
    let name = r.page(page).unwrap().name.clone();
    assert_eq!(
        crate::visual_extras::mobile_position(r, &name, table),
        Some(json!({"x": 0, "y": 208, "width": 323, "height": 240}))
    );
    let back = run(
        s,
        "visual.props",
        json!({"page": page, "visual": chart, "layout": "mobile"}),
    );
    assert_eq!(&back["props"], props, "{back}");
    assert_eq!(back["props"]["title"]["font_size"], 9.0, "{back}");
    // the desktop layout keeps its own title
    let desk = run(s, "visual.props", json!({"page": page, "visual": chart}));
    assert_ne!(desk["props"]["title"]["font_size"], 9.0, "{desk}");
}

/// The phone layout and its formatting survive PBIP (PBIR files), a PBIX with PBIR parts and a
/// PBIX whose report is a legacy layout (the container's phone layout and `mobileObjects` /
/// `mobileVcObjects`).
#[test]
fn mobile_layout_and_format_round_trip_through_pbir_and_pbix() {
    let dir = temp("mobile-round-trip");
    // PBIR: a project folder, then a PBIX with PBIR parts
    let path = synthetic_pbir_pbix(&dir, "Shop.pbix");
    let mut s = reopen(&path);
    let (chart, table) = format_report(&mut s, "Overview");
    let props = mobile_edits(&mut s, "Overview", &chart, &table);
    for out in [dir.join("pbip").join("Shop.pbip"), dir.join("Saved.pbix")] {
        run(&mut s, "file.save_as", json!({"path": out}));
        let mut back = reopen(&out);
        assert_mobile(&mut back, "Overview", &chart, &table, &props);
        let check = run(&mut back, "report.validate", json!({}));
        assert_eq!(check["schema_issues"], json!([]), "{out:?}: {check}");
    }
    // a legacy layout
    let path = synthetic_pbix(&dir, "Legacy.pbix");
    let mut s = reopen(&path);
    let (chart, table) = format_report(&mut s, "Page 1");
    let props = mobile_edits(&mut s, "Page 1", &chart, &table);
    let out = run(&mut s, "file.save", json!({"backup": false}));
    assert_eq!(out["report_format"], "legacy", "{out}");
    let layout = {
        let data = part(&path, "Report/Layout").unwrap();
        bicraft_opc::decode_text("Report/Layout", &data).unwrap().0
    };
    assert!(layout.contains(r#"\"mobileVcObjects\""#), "{layout}");
    let mut back = reopen(&path);
    assert_mobile(&mut back, "Page 1", &chart, &table, &props);
    // edited again after the reopen: the legacy slot takes it
    run(
        &mut back,
        "visual.mobile_format",
        json!({"page": "Page 1", "visual": chart, "clear": true}),
    );
    run(&mut back, "file.save", json!({"backup": false}));
    let mut again = reopen(&path);
    let p = run(
        &mut again,
        "visual.props",
        json!({"page": "Page 1", "visual": chart, "layout": "mobile"}),
    );
    assert_eq!(p["mobile_objects"], json!([]), "{p}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Every format the conditional-formatting dialog writes on a table and a matrix (a gradient
/// with numbers, a middle colour and blank handling, applied to values and totals; rules with
/// two bounds; icons with their layout and alignment; data bars with both colours, the axis,
/// right to left and Show bar only; a web URL; a gradient with Percent stops) reads back the same after a save into a PBIP
/// folder, a PBIX with PBIR parts and a PBIX with a legacy layout.
#[test]
fn dialog_conditional_formats_round_trip_through_pbip_and_pbix() {
    let dir = temp("cf-dialog-round-trip");
    let formats = json!([
        {"target": "background", "series": "[Total]", "apply_to": "values_and_totals",
         "rule": {"gradient": {"field": "[Total]", "min": {"color": "#FFFFFF", "value": 0.0},
                  "mid": "#FFFF00", "max": {"color": "#00AA00", "value": 1000.0}, "empty": "#EEEEEE"}}},
        {"target": "font_color", "series": "[Total]",
         "rule": {"rules": {"field": "[Total]", "cases": [
             {"gte": 0.0, "lt": 100.0, "color": "#AA0000"},
             {"op": ">=", "value": 100.0, "color": "#00AA00"}]}}},
        {"target": "icon", "series": "[Total]",
         "rule": {"icons": {"field": "[Total]", "position": "after", "align": "middle", "cases": [
             {"op": ">=", "value": 500.0, "icon": "TrafficHigh"},
             {"gte": 100.0, "lt": 500.0, "icon": "TrafficMedium"},
             {"op": "<", "value": 100.0, "icon": "TrafficLow"}]}}},
        {"target": "data_bars", "series": "[Rows]",
         "rule": {"data_bars": {"positive": "#42A5F5", "negative": "#D64550", "axis": "#333333",
                  "reverse": true, "show_bar_only": true, "min": -10.0, "max": 10.0}}},
        {"target": "web_url", "series": "Calendar[Month]", "rule": {"web_url": {"field": "Calendar[Month]"}}},
        {"target": "background", "series": "[Rows]",
         "rule": {"gradient": {"field": "[Rows]", "min": {"color": "#FFFFFF", "value": 10.0, "percent": true},
                  "mid": {"color": "#FFFF00", "value": 50.0, "percent": true}, "max": "#00AA00"}}}
    ]);
    let setup = |s: &mut Session, page: &str| -> Vec<String> {
        let fields = json!([{"role": "Values", "field": "Calendar[Month]"},
                            {"role": "Values", "field": "[Total]"}, {"role": "Values", "field": "[Rows]"}]);
        let mut out = Vec::new();
        for (vt, fields) in [
            ("tableEx", fields.clone()),
            (
                "pivotTable",
                json!([{"role": "Rows", "field": "Calendar[Month]"},
                       {"role": "Values", "field": "[Total]"}, {"role": "Values", "field": "[Rows]"}]),
            ),
        ] {
            let v = run(
                s,
                "visual.add",
                json!({"page": page, "visual_type": vt, "x": 10, "y": 10, "width": 300,
                       "height": 200, "fields": fields}),
            )["visual"]
                .as_str()
                .unwrap()
                .to_string();
            run(
                s,
                "visual.conditional",
                json!({"page": page, "visual": v, "conditional": formats}),
            );
            out.push(v);
        }
        out
    };
    let props = |s: &mut Session, page: &str, v: &str| -> Value {
        run(s, "visual.props", json!({"page": page, "visual": v}))["props"]["conditional"].clone()
    };
    let path = synthetic_pbir_pbix(&dir, "Shop.pbix");
    let mut s = reopen(&path);
    let visuals = setup(&mut s, "Overview");
    let before: Vec<Value> = visuals.iter().map(|v| props(&mut s, "Overview", v)).collect();
    for b in &before {
        assert_eq!(b.as_array().map(Vec::len), Some(6), "{b}");
    }
    for out in [dir.join("pbip").join("Shop.pbip"), dir.join("Saved.pbix")] {
        run(&mut s, "file.save_as", json!({"path": out}));
        let mut back = reopen(&out);
        for (v, b) in visuals.iter().zip(&before) {
            assert_eq!(&props(&mut back, "Overview", v), b, "{out:?}");
        }
        let check = run(&mut back, "report.validate", json!({}));
        assert_eq!(check["schema_issues"], json!([]), "{out:?}: {check}");
    }
    // a legacy layout
    let path = synthetic_pbix(&dir, "Legacy.pbix");
    let mut s = reopen(&path);
    let visuals = setup(&mut s, "Page 1");
    let before: Vec<Value> = visuals.iter().map(|v| props(&mut s, "Page 1", v)).collect();
    let out = run(&mut s, "file.save", json!({"backup": false}));
    assert_eq!(out["report_format"], "legacy", "{out}");
    let mut back = reopen(&path);
    for (v, b) in visuals.iter().zip(&before) {
        assert_eq!(&props(&mut back, "Page 1", v), b);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A thin PBIX (no model, a live connection to a published model) is read for its report, and
/// says that its model is published in the Power BI service and not opened: a known limitation,
/// not pursued by the maintainers' decision (docs/gaps.md G12).
#[test]
fn a_thin_pbix_says_its_model_is_published_and_not_opened() {
    let dir = temp("thin");
    let full = reopen(&synthetic_pbir_pbix(&dir, "Shop.pbix"));
    let remote = bicraft_pbixwriter::RemoteModel {
        connection: "powerbi://api.powerbi.com/v1.0/myorg/Finance".into(),
        model: "Sales Model".into(),
    };
    let pkg = bicraft_pbixwriter::write_thin_pbix(full.report.as_ref().unwrap(), &remote).unwrap();
    let path = dir.join("Thin.pbix");
    std::fs::write(&path, pkg.to_bytes().unwrap()).unwrap();

    let mut s = Session::new();
    let out = run(&mut s, "file.open", json!({"path": path}));
    assert!(s.model.is_none());
    assert!(
        out["report"]["pages"].as_u64().unwrap() >= 1,
        "the report is read: {out}"
    );
    let live = json!({
        "type": "pbiServiceXmlaStyleLive",
        "data_source": "powerbi://api.powerbi.com/v1.0/myorg/Finance",
        "catalog": "Sales Model",
    });
    assert_eq!(out["live_connection"], live, "{out}");
    let why = out["model_error"].as_str().unwrap();
    for part in [
        "thin report",
        "published in the Power BI service",
        "pbiServiceXmlaStyleLive",
        "powerbi://api.powerbi.com/v1.0/myorg/Finance",
        "Sales Model",
        "does not sign in",
        "not pursued",
        "G12",
    ] {
        assert!(why.contains(part), "`{part}` in: {why}");
    }
    // the same words wherever a model is needed, and in the viewer's snapshot
    let e = s.execute("model.summary", &json!({})).unwrap_err();
    assert!(e.contains("not pursued"), "{e}");
    let snap = s.snapshot();
    assert_eq!(snap["source"]["live_connection"], live);
    assert_eq!(snap["model_error"], json!(why));
}
