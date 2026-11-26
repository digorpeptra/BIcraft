//! Tests of `Session::revision`, `Session::last_command` and `Session::snapshot`.

use bicraft_report::{FieldBinding, FieldRef, Page, Position, Report, Visual};
use serde_json::{Value, json};

use bicraft_engine::Session;
use bicraft_engine::SnapshotLimits;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn keys(v: &Value) -> Vec<&str> {
    let mut k: Vec<&str> = v
        .as_object()
        .unwrap_or_else(|| panic!("not an object: {v}"))
        .keys()
        .map(String::as_str)
        .collect();
    k.sort_unstable();
    k
}

/// Checks the contract's required keys are present (producers may add more).
fn has_keys(v: &Value, want: &[&str]) {
    let k = keys(v);
    for w in want {
        assert!(k.contains(w), "missing `{w}` in {v}");
    }
}

fn check_top(snap: &Value) {
    has_keys(
        snap,
        &[
            "version",
            "revision",
            "source",
            "dirty",
            "last_command",
            "model",
            "report",
            "validation",
        ],
    );
    assert_eq!(snap["version"], 1);
    has_keys(&snap["source"], &["path", "kind", "format"]);
    has_keys(&snap["dirty"], &["model", "report", "queries"]);
    has_keys(&snap["validation"], &["model", "report"]);
}

/// Sales(Amount, Product) with [Total], Product(Product), one relationship.
/// A session holding only `report`.
fn with_report(report: Report) -> Session {
    let mut s = Session::new();
    s.report = Some(report);
    s
}

fn model_session() -> Session {
    let mut s = Session::new();
    for (id, p) in [
        ("model.new", json!({"name": "Shop", "culture": "fr-FR"})),
        ("table.add", json!({"name": "Sales"})),
        ("table.add", json!({"name": "Product"})),
        (
            "column.add",
            json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Product", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Product", "name": "Product", "data_type": "string"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])", "format_string": "#,0"}),
        ),
        (
            "relationship.add",
            json!({"from": "Sales[Product]", "to": "Product[Product]"}),
        ),
    ] {
        run(&mut s, id, p);
    }
    s
}

#[test]
fn revision_bumps_only_on_mutations() {
    let mut s = Session::new();
    assert_eq!(s.revision, 0);
    assert!(s.last_command.is_none());

    run(&mut s, "model.new", json!({"name": "M"}));
    assert_eq!(s.revision, 1);
    let last = s.last_command.clone().unwrap();
    assert_eq!((last.id.as_str(), last.ok, last.error), ("model.new", true, None));
    assert!(last.at_ms > 0);

    // Read-only commands.
    run(&mut s, "model.summary", json!({}));
    run(&mut s, "model.validate", json!({}));
    run(&mut s, "dax.validate", json!({"expression": "1+1"}));
    run(&mut s, "dax.validate_query", json!({"query": "EVALUATE {1}"}));
    run(&mut s, "file.inspect", json!({}));
    assert_eq!(s.revision, 1);
    assert_eq!(s.last_command.as_ref().unwrap().id, "file.inspect");

    // Failures change nothing but are recorded.
    assert!(s.execute("table.remove", &json!({"name": "Nope"})).is_err());
    assert!(s.execute("no.such.command", &json!({})).is_err());
    assert_eq!(s.revision, 1);
    let last = s.last_command.clone().unwrap();
    assert_eq!(last.id, "no.such.command");
    assert!(!last.ok);
    assert!(last.error.unwrap().contains("unknown command"));

    // Mutations.
    run(&mut s, "table.add", json!({"name": "T"}));
    run(&mut s, "report.new", json!({}));
    assert_eq!(s.revision, 3);
    run(&mut s, "report.summary", json!({}));
    run(&mut s, "report.validate", json!({}));
    assert_eq!(s.revision, 3);

    // A batch bumps once per successful mutating step.
    let steps = vec![
        ("table.add".to_string(), json!({"name": "U"})),
        ("model.summary".to_string(), json!({})),
        ("table.add".to_string(), json!({"name": "U"})),
    ];
    assert!(s.batch(&steps).is_err());
    assert_eq!(s.revision, 4);
    assert!(!s.last_command.as_ref().unwrap().ok);
}

#[test]
fn every_command_is_classified() {
    for id in bicraft_engine::READ_ONLY_COMMANDS {
        assert!(
            bicraft_engine::find_command(id).is_some(),
            "{id} is not a command"
        );
    }
    assert!(bicraft_engine::is_mutating("measure.add"));
    assert!(!bicraft_engine::is_mutating("report.summary"));
}

#[test]
fn snapshot_of_an_empty_session() {
    let snap = Session::new().snapshot();
    check_top(&snap);
    assert_eq!(snap["revision"], 0);
    assert_eq!(
        snap["source"],
        json!({"path": null, "kind": "none", "format": null, "desktop": {
            "path": null, "unsaved": false,
            "reason": "nothing is saved yet: save the session as a .pbix or .pbip file first (file.save_as)"}})
    );
    assert_eq!(
        snap["dirty"],
        json!({"model": false, "report": false, "queries": false})
    );
    assert_eq!(snap["last_command"], Value::Null);
    assert_eq!(snap["model"], Value::Null);
    assert_eq!(snap["report"], Value::Null);
    assert_eq!(snap["validation"], json!({"model": null, "report": null}));
}

#[test]
fn snapshot_of_a_model_only_session() {
    let mut s = model_session();
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Broken", "expression": "SUM(Sales[Nope])", "force": true}),
    );
    let snap = s.snapshot();
    check_top(&snap);
    assert_eq!(snap["revision"], s.revision);
    assert_eq!(snap["dirty"]["model"], true);
    assert_eq!(snap["last_command"]["id"], "measure.add");
    assert_eq!(snap["last_command"]["ok"], true);
    assert_eq!(snap["report"], Value::Null);

    let m = &snap["model"];
    has_keys(m, &["name", "culture", "tables", "relationships"]);
    assert_eq!(m["name"], "Shop");
    assert_eq!(m["culture"], "fr-FR");
    assert!(m.get("truncated").is_none());
    let sales = &m["tables"][0];
    has_keys(sales, &["name", "hidden", "columns", "measures"]);
    assert_eq!(sales["name"], "Sales");
    assert_eq!(
        sales["columns"][0],
        json!({"name": "Amount", "data_type": "double", "format_string": null, "calculated": false, "hidden": false,
               "display_folder": null, "summarize_by": "sum", "data_category": null,
               "description": null, "sort_by_column": null})
    );
    let total = &sales["measures"][0];
    has_keys(
        total,
        &[
            "name",
            "expression",
            "format_string",
            "hidden",
            "display_folder",
            "valid",
        ],
    );
    assert_eq!(total["expression"], "SUM(Sales[Amount])");
    assert_eq!(total["format_string"], "#,0");
    assert_eq!(total["valid"], true);
    assert_eq!(sales["measures"][1]["name"], "Broken");
    assert_eq!(sales["measures"][1]["valid"], false);

    let r = &m["relationships"][0];
    has_keys(
        r,
        &[
            "from_table",
            "from_column",
            "to_table",
            "to_column",
            "active",
            "cross_filter",
            "from_cardinality",
            "to_cardinality",
        ],
    );
    assert_eq!(r["from_table"], "Sales");
    assert_eq!(r["to_column"], "Product");
    assert_eq!(r["active"], true);
    assert_eq!(r["cross_filter"], "single");
    assert_eq!(r["from_cardinality"], "many");
    assert_eq!(r["to_cardinality"], "one");

    let v = &snap["validation"]["model"];
    has_keys(v, &["valid", "invalid_count", "objects"]);
    assert_eq!(v["valid"], false);
    assert_eq!(v["invalid_count"], 1);
    let o = &v["objects"][0];
    has_keys(o, &["table", "kind", "name", "diagnostics"]);
    assert_eq!(
        (o["table"].as_str(), o["name"].as_str()),
        (Some("Sales"), Some("Broken"))
    );
    assert!(!o["diagnostics"].as_array().unwrap().is_empty());
    assert_eq!(snap["validation"]["report"], Value::Null);
}

fn literal_title(text: &str) -> Value {
    json!({"title": [{"properties": {"text": {"expr": {"Literal": {"Value": text}}}}}]})
}

#[test]
fn snapshot_of_a_report_only_session() {
    let mut visual = Visual {
        name: "v1".into(),
        visual_type: Some("card".into()),
        position: Position {
            x: 10.0,
            y: 20.0,
            z: 1000.0,
            width: 300.0,
            height: 120.0,
            tab_order: Some(2),
        },
        fields: vec![FieldBinding {
            display_name: None,
            show_as: None,
            role: "Values".into(),
            field: FieldRef::Aggregation {
                table: "Sales".into(),
                column: "Amount".into(),
                function: "Sum".into(),
            },
        }],
        ..Default::default()
    };
    visual
        .visual_extra
        .insert("visualContainerObjects".into(), literal_title("'It''s total'"));
    let s = with_report(Report {
        pages: vec![Page {
            name: "p1".into(),
            display_name: "Overview".into(),
            width: 1280.0,
            height: 720.0,
            visibility: Some("HiddenInViewMode".into()),
            visuals: vec![visual],
            ..Default::default()
        }],
        ..Default::default()
    });
    let snap = s.snapshot();
    check_top(&snap);
    assert_eq!(snap["model"], Value::Null);
    assert_eq!(snap["validation"]["model"], Value::Null);
    let r = &snap["report"];
    has_keys(r, &["active_page", "pages"]);
    assert_eq!(r["active_page"], "p1");
    let p = &r["pages"][0];
    has_keys(
        p,
        &["name", "display_name", "width", "height", "hidden", "visuals"],
    );
    assert_eq!(p["display_name"], "Overview");
    assert_eq!(p["width"], 1280.0);
    assert_eq!(p["hidden"], true);
    let v = &p["visuals"][0];
    has_keys(
        v,
        &[
            "name",
            "type",
            "x",
            "y",
            "z",
            "width",
            "height",
            "tab_order",
            "title",
            "parent_group",
            "hidden",
            "fields",
            "issues",
        ],
    );
    assert_eq!(v["type"], "card");
    assert_eq!((v["x"].as_f64(), v["y"].as_f64()), (Some(10.0), Some(20.0)));
    assert_eq!(v["tab_order"], 2);
    assert_eq!(v["title"], "It's total");
    assert_eq!(
        v["fields"][0],
        json!({"role": "Values", "kind": "aggregation", "table": "Sales", "name": "Amount",
               "function": "Sum", "text": "Sum(Sales[Amount])", "label": "Sum of Amount",
               "ref": "Sum of Amount"})
    );
    assert_eq!(v["auto_title"], "Sum of Amount");
    assert_eq!(v["issues"], json!([]));
    let val = &snap["validation"]["report"];
    has_keys(val, &["valid", "error_count", "warning_count", "issues"]);
    assert_eq!(val["checked_against_model"], false);
}

#[test]
fn snapshot_of_a_model_and_report_session() {
    let mut s = model_session();
    run(&mut s, "report.new", json!({}));
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    let good = run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "card", "x": 0, "y": 0, "width": 200, "height": 100,
               "fields": [{"role": "Values", "field": "[Total]"}]}),
    );
    let bad = run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "tableEx", "x": 300, "y": 0, "width": 200, "height": 100,
               "fields": [{"role": "Values", "field": "Sales[Nope]"}], "force": true}),
    );
    let snap = s.snapshot();
    check_top(&snap);
    assert_eq!(
        snap["dirty"],
        json!({"model": true, "report": true, "queries": false})
    );
    let visuals = snap["report"]["pages"][0]["visuals"].as_array().unwrap();
    let find = |name: &Value| {
        visuals
            .iter()
            .find(|v| v["name"] == *name)
            .unwrap_or_else(|| panic!("no visual {name}"))
    };
    let good = find(&good["visual"]);
    assert_eq!(good["fields"][0]["kind"], "measure");
    assert_eq!(good["fields"][0]["table"], "Sales");
    assert_eq!(good["fields"][0]["text"], "[Total]");
    assert_eq!(good["issues"], json!([]));
    let bad = find(&bad["visual"]);
    let issue = &bad["issues"][0];
    assert_eq!(issue["severity"], "error");
    assert!(issue["message"].as_str().unwrap().contains("Nope"), "{issue}");
    let val = &snap["validation"]["report"];
    assert_eq!(val["valid"], false);
    assert_eq!(val["checked_against_model"], true);
    assert!(val["error_count"].as_u64().unwrap() >= 1);

    // Saving as a project sets the source and clears the dirty flags.
    let tmp = crate::tempdir::TempDir::new("engine-snapshot");
    let dir = tmp.path();
    let before = s.revision;
    run(&mut s, "project.new", json!({"dir": dir, "name": "Shop"}));
    assert_eq!(s.revision, before + 1);
    let snap = s.snapshot();
    assert_eq!(snap["source"]["kind"], "project");
    assert_eq!(snap["source"]["format"], "pbip-tmdl");
    assert_eq!(
        snap["dirty"],
        json!({"model": false, "report": false, "queries": false})
    );
}

#[test]
fn snapshot_caps_lists_and_flags_truncation() {
    let mut s = model_session();
    run(&mut s, "report.new", json!({}));
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    for i in 0..3 {
        run(
            &mut s,
            "visual.add",
            json!({"page": page, "visual_type": "card", "x": i * 10, "y": 0, "width": 5, "height": 5,
                   "fields": [{"role": "Values", "field": "[Total]"}, {"role": "Values", "field": "Sales[Nope]"}],
                   "force": true}),
        );
    }
    run(&mut s, "page.add", json!({"display_name": "Second"}));
    let limits = SnapshotLimits {
        tables: 1,
        columns_per_table: 1,
        measures_per_table: 0,
        relationships: 0,
        pages: 1,
        visuals_per_page: 2,
        fields_per_visual: 1,
        validation_issues: 1,
        expression_chars: 3,
        ..SnapshotLimits::default()
    };
    let snap = s.snapshot_with(&limits);
    let m = &snap["model"];
    assert_eq!(m["truncated"], true);
    assert_eq!(m["tables"].as_array().unwrap().len(), 1);
    assert_eq!(m["relationships"], json!([]));
    assert_eq!(m["tables"][0]["truncated"], true);
    assert_eq!(m["tables"][0]["columns"].as_array().unwrap().len(), 1);
    let r = &snap["report"];
    assert_eq!(r["truncated"], true);
    assert_eq!(r["pages"].as_array().unwrap().len(), 1);
    assert_eq!(r["pages"][0]["truncated"], true);
    assert_eq!(r["pages"][0]["visuals"].as_array().unwrap().len(), 2);
    assert_eq!(r["pages"][0]["visuals"][0]["truncated"], true);
    assert_eq!(r["pages"][0]["visuals"][0]["fields"].as_array().unwrap().len(), 1);
    let v = &snap["validation"]["report"];
    assert_eq!(v["truncated"], true);
    assert_eq!(v["issues"].as_array().unwrap().len(), 1);

    // Long expressions are cut.
    let limits = SnapshotLimits {
        expression_chars: 3,
        ..SnapshotLimits::default()
    };
    let total = &s.snapshot_with(&limits)["model"]["tables"][0]["measures"][0];
    assert_eq!(total["expression"], "SUM");
    assert_eq!(total["expression_truncated"], true);
    // The default caps leave this session alone.
    let full = s.snapshot();
    assert!(full["model"].get("truncated").is_none());
    assert!(full["report"].get("truncated").is_none());
}

#[test]
fn big_reports_skip_the_schema_check() {
    let mut s = model_session();
    run(&mut s, "report.new", json!({}));
    let snap = s.snapshot();
    assert_eq!(snap["validation"]["report"]["schema_checked"], true);
    let limits = SnapshotLimits {
        schema_check_visuals: 0,
        ..SnapshotLimits::default()
    };
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "card", "x": 0, "y": 0, "width": 5, "height": 5}),
    );
    let snap = s.snapshot_with(&limits);
    assert_eq!(snap["validation"]["report"]["schema_checked"], false);
    // The command always checks schemas.
    assert_eq!(run(&mut s, "report.validate", json!({}))["schema_checked"], true);
}

/// Timing on a real file (local only; skipped without `BICRAFT_TESTDATA`), as read and with its
/// pages repeated to 100+. Run in release to check the < 50 ms budget:
/// `cargo test -p bicraft-engine --release snapshot_timing -- --nocapture`.
#[test]
fn snapshot_timing_on_real_files() {
    let Some(root) = bicraft_env::var_os("BICRAFT_TESTDATA") else {
        eprintln!("BICRAFT_TESTDATA unset: skipping");
        return;
    };
    let path = std::path::Path::new(&root).join("sample1").join("report.pbix");
    if !path.exists() {
        eprintln!("{} missing: skipping", path.display());
        return;
    }
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": path}));
    let time = |s: &Session| {
        let _ = s.snapshot();
        let n = 5u32;
        let start = std::time::Instant::now();
        let mut bytes = 0;
        for _ in 0..n {
            bytes = serde_json::to_string(&s.snapshot()).unwrap().len();
        }
        (start.elapsed() / n, bytes)
    };
    let pages = |s: &Session| s.report.as_ref().map_or(0, |r| r.pages.len());
    let (each, bytes) = time(&s);
    eprintln!(
        "snapshot of sample1 ({} pages): {each:?} per call (incl. serialization), {bytes} bytes",
        pages(&s)
    );
    if let Some(r) = s.report.as_mut()
        && !r.pages.is_empty()
    {
        let original = r.pages.clone();
        let mut i = 0;
        while r.pages.len() < 100 {
            for p in &original {
                i += 1;
                let mut p = p.clone();
                p.name = format!("{}{i}", p.name);
                p.display_name = format!("{} {i}", p.display_name);
                r.pages.push(p);
            }
        }
        let (each, bytes) = time(&s);
        eprintln!(
            "snapshot of sample1 x{} pages: {each:?} per call, {bytes} bytes",
            pages(&s)
        );
    }
}

// ---------------------------------------------------------------------------------------------
// M8.2: theme, page background, visual format, visual data
// ---------------------------------------------------------------------------------------------

fn lit(v: &str) -> Value {
    json!({"expr": {"Literal": {"Value": v}}})
}

fn solid(v: &str) -> Value {
    json!({"solid": {"color": lit(v)}})
}

#[test]
fn snapshot_theme_background_and_format() {
    let custom = json!({
        "name": "Shop theme",
        "dataColors": ["#112233", "#445566", "#778899"],
        "foreground": "#101010",
        "textClasses": {"label": {"fontFace": "'Segoe UI Semibold', wf_segoe-ui_semibold", "fontSize": 11}},
        "visualStyles": {
            "*": {"*": {"border": [{"show": true, "radius": 4}], "title": [{"fontSize": 13}],
                        "categoryAxis": [{"fontSize": 14, "gridlineShow": false}]}},
            "clusteredColumnChart": {"*": {"valueAxis": [{"fontSize": 11}]}},
            "page": {"*": {"background": [{"color": {"solid": {"color": "#EEEEEE"}}, "transparency": 0}]}}
        }
    });
    let mut themed = Visual {
        name: "v1".into(),
        visual_type: Some("clusteredColumnChart".into()),
        ..Default::default()
    };
    themed.visual_extra.insert(
        "visualContainerObjects".into(),
        json!({
            "title": [{"properties": {"show": lit("true"), "fontSize": lit("14D"), "alignment": lit("'center'"),
                                      "fontColor": {"solid": {"color": {"expr": {"ThemeDataColor": {"ColorId": 3, "Percent": 0}}}}}}}],
            "background": [{"properties": {"show": lit("false"), "color": solid("'#FAFAFA'"), "transparency": lit("50D")}}]
        }),
    );
    themed.visual_extra.insert(
        "objects".into(),
        json!({"labels": [{"properties": {"show": lit("true")}}],
               "legend": [{"properties": {"show": lit("true"), "position": lit("'Top'")}}],
               "valueAxis": [{"properties": {"fontSize": lit("12D")}}]}),
    );
    let plain = Visual {
        name: "v2".into(),
        visual_type: Some("textbox".into()),
        ..Default::default()
    };
    let mut page = Page {
        name: "p1".into(),
        display_name: "One".into(),
        width: 1280.0,
        height: 720.0,
        visuals: vec![themed, plain],
        ..Default::default()
    };
    let second = Page {
        name: "p2".into(),
        display_name: "Two".into(),
        ..page.clone()
    };
    page.extra.insert(
        "objects".into(),
        json!({"background": [{"properties": {"color": solid("'#F3F2F1'"), "transparency": lit("0D")}}]}),
    );
    let s = with_report(Report {
        pages: vec![page, second],
        theme_collection: Some(json!({
            "baseTheme": {"name": "CY24SU10", "reportVersionAtImport": "5.61", "type": "SharedResources"},
            "customTheme": {"name": "Shop.json", "reportVersionAtImport": "5.61", "type": "RegisteredResources"}
        })),
        resources: vec![bicraft_report::Resource {
            path: "StaticResources/RegisteredResources/Shop.json".into(),
            bytes: custom.to_string().into_bytes(),
        }],
        ..Default::default()
    });
    let snap = s.snapshot();
    let r = &snap["report"];
    let theme = &r["theme"];
    assert_eq!(theme["name"], "Shop theme");
    assert_eq!(theme["data_colors"], json!(["#112233", "#445566", "#778899"]));
    assert_eq!(theme["foreground"], "#101010");
    // not set by the custom theme: the built-in default for the base theme
    assert_eq!(theme["background"], "#FFFFFF");
    assert_eq!(theme["font_family"], "Segoe UI Semibold");
    assert_eq!(theme["font_size"], 11);
    let p1 = &r["pages"][0];
    assert_eq!(p1["background"], "#F3F2F1");
    // a page without its own background takes the theme's page style
    assert_eq!(r["pages"][1]["background"], "#EEEEEE");
    let f = &p1["visuals"][0]["format"];
    assert_eq!(
        f["title"],
        json!({"show": true, "font_size": 14, "alignment": "center", "color": "#445566"})
    );
    assert_eq!(
        f["background"],
        json!({"show": false, "color": "#FAFAFA", "transparency": 50})
    );
    assert_eq!(f["border"], json!({"show": true, "radius": 4}));
    assert_eq!(f["data_labels"], true);
    assert_eq!(f["legend"], json!({"show": true, "position": "top"}));
    // axis label sizes: the visual's own only (Desktop 2.158 ignores the theme's, `*` or per type)
    assert_eq!(f["x_axis"], json!({"gridlines": false}));
    assert_eq!(f["y_axis"]["font_size"], 12);
    // theme defaults apply to every visual
    assert_eq!(p1["visuals"][1]["format"]["border"]["show"], true);
    // visuals without fields carry no data
    assert_eq!(p1["visuals"][0]["data"], Value::Null);

    // no theme collection: theme null
    let bare = with_report(Report::default());
    assert_eq!(bare.snapshot()["report"]["theme"], Value::Null);
}

#[test]
fn snapshot_carries_visual_data() {
    let mut s = model_session();
    run(&mut s, "report.new", json!({}));
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    let v = run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "card", "x": 0, "y": 0, "width": 200, "height": 100,
               "fields": [{"role": "Values", "field": "[Total]"}]}),
    );
    let snap = s.snapshot();
    let visual = snap["report"]["pages"][0]["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["name"] == v["visual"])
        .unwrap();
    // the tables have no partition: no data, and the reason says why
    assert_eq!(visual["data"]["status"], "unavailable", "{visual}");
    assert!(visual["data"]["reason"].is_string());
}

/// The data preview on a real file (local only; skipped without `BICRAFT_TESTDATA`). Prints
/// counts and timings only. Run in release for meaningful timings:
/// `cargo test -p bicraft-engine --release snapshot_data_on_real -- --nocapture`.
#[test]
fn snapshot_data_on_real_files() {
    let Some(root) = bicraft_env::var_os("BICRAFT_TESTDATA") else {
        eprintln!("BICRAFT_TESTDATA unset: skipping");
        return;
    };
    let path = std::path::Path::new(&root).join("sample1").join("report.pbix");
    if !path.exists() {
        eprintln!("{} missing: skipping", path.display());
        return;
    }
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": path}));
    let tally = |snap: &Value| {
        let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
        let mut reasons: std::collections::BTreeMap<String, usize> = Default::default();
        let mut rows = 0usize;
        for p in snap["report"]["pages"].as_array().unwrap() {
            for v in p["visuals"].as_array().unwrap() {
                let d = &v["data"];
                let status = d["status"].as_str().unwrap_or("none").to_string();
                if let Some(r) = d["rows"].as_array() {
                    rows += r.len();
                }
                if let Some(reason) = d["reason"].as_str() {
                    // the reason's shape only: names are client material
                    let shape: String = reason
                        .split(['\'', '[', ']'])
                        .step_by(2)
                        .collect::<Vec<_>>()
                        .join("...");
                    *reasons.entry(format!("{status}: {shape}")).or_default() += 1;
                }
                *counts.entry(status).or_default() += 1;
            }
        }
        (counts, reasons, rows)
    };
    let start = std::time::Instant::now();
    let first = s.snapshot();
    let t_first = start.elapsed();
    let start = std::time::Instant::now();
    let second = s.snapshot();
    let t_second = start.elapsed();
    let (counts, reasons, rows) = tally(&second);
    eprintln!(
        "data on sample1: first snapshot {t_first:?}, cached {t_second:?}; statuses {counts:?}; {rows} rows"
    );
    for (r, n) in &reasons {
        eprintln!("  {n} x {r}");
    }
    let theme = &first["report"]["theme"];
    assert!(theme.is_object(), "the sample has a theme");
    eprintln!(
        "theme: {} data colors; pages with a background: {}; visuals with a format: {}",
        theme["data_colors"].as_array().map_or(0, Vec::len),
        first["report"]["pages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["background"].is_string())
            .count(),
        first["report"]["pages"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|p| p["visuals"].as_array().unwrap())
            .filter(|v| v["format"].is_object())
            .count()
    );
    assert!(
        counts.get("ok").copied().unwrap_or(0) > 0,
        "some visuals evaluate"
    );
    assert_eq!(counts.get("error"), None, "no visual fails: {reasons:?}");

    // an edit that touches no data keeps the cached results
    let page = first["report"]["pages"][0]["name"].clone();
    let visual = first["report"]["pages"][0]["visuals"][0]["name"].clone();
    run(
        &mut s,
        "visual.set_position",
        json!({"page": page, "visual": visual, "x": 1, "y": 1}),
    );
    let start = std::time::Instant::now();
    let after = s.snapshot();
    eprintln!("snapshot after a visual move: {:?}", start.elapsed());
    assert_eq!(tally(&after).0, counts);
}

/// Pages carry their type as page.set_type names it (the app's page tabs keep it when they hide
/// or show a page).
#[test]
fn pages_carry_their_type() {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "M"}));
    run(&mut s, "report.new", json!({}));
    let first = s.report.as_ref().unwrap().pages[0].name.clone();
    run(&mut s, "page.add", json!({"display_name": "Tip"}));
    run(&mut s, "page.set_type", json!({"page": "Tip", "type": "tooltip"}));
    let snap = s.snapshot();
    let pages = snap["report"]["pages"].as_array().unwrap();
    assert_eq!(pages[0]["name"], first);
    assert_eq!(pages[0]["page_type"], "report");
    assert_eq!(pages[1]["page_type"], "tooltip");
}

/// The names the parameters forms pick from (docs/view-snapshot.md, "Names for the forms").
#[test]
fn the_snapshot_names_the_objects_the_forms_pick() {
    let mut s = model_session();
    for (id, p) in [
        (
            "hierarchy.add",
            json!({"table": "Product", "name": "Products", "levels": ["Product"]}),
        ),
        ("calculation_group.add", json!({"name": "Time"})),
        (
            "calculation_item.add",
            json!({"group": "Time", "name": "Current", "expression": "SELECTEDMEASURE()"}),
        ),
        ("role.add", json!({"name": "Readers"})),
        ("culture.add", json!({"name": "de-DE"})),
        (
            "expression.add",
            json!({"name": "Root", "expression": "\"C:\\data\""}),
        ),
        ("report.new", json!({})),
        (
            "report_measure.add",
            json!({"table": "Sales", "name": "Twice", "expression": "[Total] * 2"}),
        ),
    ] {
        run(&mut s, id, p);
    }
    let snap = s.snapshot();
    let m = &snap["model"];
    let table = |n: &str| {
        m["tables"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == n)
            .unwrap_or_else(|| panic!("no table {n}"))
            .clone()
    };
    assert_eq!(table("Product")["hierarchies"], json!(["Products"]));
    assert!(
        table("Sales").get("hierarchies").is_none(),
        "only tables that have some"
    );
    assert_eq!(table("Time")["calculation_items"], json!(["Current"]));
    assert!(
        table("Sales").get("calculation_items").is_none(),
        "only calculation groups"
    );
    assert_eq!(table("Time")["partitions"].as_array().unwrap().len(), 1);
    assert_eq!(m["roles"], json!(["Readers"]));
    assert!(
        m["cultures"].as_array().unwrap().contains(&json!("de-DE")),
        "{}",
        m["cultures"]
    );
    assert_eq!(m["expressions"], json!(["Root"]));
    assert_eq!(
        snap["report"]["measures"],
        json!([{"table": "Sales", "name": "Twice"}])
    );
    assert!(
        m["relationships"][0]["name"].is_string(),
        "relationships carry their names"
    );
}
