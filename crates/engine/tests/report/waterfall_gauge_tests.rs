//! BIcraft Waterfall+ and BIcraft Gauge / KPI+ (docs/formats/chart-pack.md "Waterfall+" and
//! "Gauge / KPI+"), two visuals of BIcraft's chart pack: the renderer's Rust layouts against
//! `waterfall_gauge_golden.json` (a case for each option, hostile inputs, highlights), the
//! visuals' TypeScript layouts against the Rust under Node (a development check that prints a
//! note and passes when Node is missing), both packaged visuals run under Node with a stand-in
//! host (landing page, tooltips, selection, high contrast, format pane), and both visuals added
//! to a report by name, drawn by `report.render` and PDF export, formatted and kept through PBIX
//! and PBIP. `BICRAFT_BLESS=1` rewrites the golden file from the Rust layouts.

use std::path::{Path, PathBuf};

use bicraft_engine::Session;
use bicraft_engine::chart_pack::{VISUALS, find, install, layout_script, package_bytes};
use bicraft_render::chart_pack;
use serde_json::{Value, json};

use crate::chart_pack_tests::{DOM, has_node, package_json};
use crate::tempdir::TempDir;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

// ---------------------------------------------------------------------------------------------
// Golden layouts
// ---------------------------------------------------------------------------------------------

/// Monthly steps: [category, group, subtotal flag, target, value].
fn steps() -> Value {
    json!([
        ["Jan", "Q1", null, null, 120],
        ["Feb", "Q1", null, null, -45],
        ["Mar", "Q1", null, null, 30],
        ["Apr", "Q2", null, null, -80],
        ["May", "Q2", null, null, 65],
        ["Jun", "Q2", null, null, 20]
    ])
}

fn with_target(target: f64) -> Value {
    let mut rows = steps();
    for r in rows.as_array_mut().unwrap() {
        r[3] = json!(target / 6.0);
    }
    rows
}

/// [trend, value, target, min, max, t1, t2, t3, previous] by month.
fn trend() -> Value {
    json!([
        ["2024-01-01T00:00:00", 52, 70, null, null, null, null, null, null],
        ["2024-02-01T00:00:00", 61, 70, null, null, null, null, null, null],
        ["2024-03-01T00:00:00", 58, 75, null, null, null, null, null, null],
        ["2024-04-01T00:00:00", 66, 75, null, null, null, null, null, null],
        ["2024-05-01T00:00:00", 79, 80, null, null, null, null, null, null]
    ])
}

fn one(value: Value, target: Value, min: Value, max: Value, t: [Value; 3]) -> Value {
    let [t1, t2, t3] = t;
    json!([[null, value, target, min, max, t1, t2, t3, null]])
}

pub(crate) fn cases() -> Vec<(&'static str, &'static str, Value, f64, f64)> {
    let hl: Vec<Value> = steps()
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let v = r[4].as_f64().unwrap();
            json!([if i % 2 == 0 { json!(v / 2.0) } else { Value::Null }])
        })
        .collect();
    let measures = json!([
        [null, null, null, 900, 500, 320, -140, -60, 85],
        [null, null, null, null, 250, 80, -40, null, 15]
    ]);
    let three = || [json!(40), json!(70), json!(90)];
    vec![
        // Waterfall+
        (
            "wf_basic",
            "waterfall",
            json!({"rows": steps(), "formats": ["", "", "", "", "#,0"]}),
            640.0,
            320.0,
        ),
        (
            "wf_horizontal",
            "waterfall",
            json!({"rows": steps(), "options": {"orientation": "horizontal"}}),
            520.0,
            360.0,
        ),
        (
            "wf_groups",
            "waterfall",
            json!({"rows": steps(), "options": {"labelMode": "both"}}),
            720.0,
            320.0,
        ),
        (
            "wf_flag_every",
            "waterfall",
            json!({"rows": [
                ["A", null, null, null, 10], ["B", null, "yes", null, 5], ["C", null, 0, null, -3],
                ["D", null, false, null, 8], ["E", null, null, null, 2], ["F", null, 1, null, -4]
            ], "options": {"subtotalEvery": 4}}),
            640.0,
            300.0,
        ),
        (
            "wf_color_category",
            "waterfall",
            json!({"rows": steps(), "options": {"colorBy": "category", "showConnectors": false}}),
            640.0,
            300.0,
        ),
        (
            "wf_color_rule",
            "waterfall",
            json!({"rows": steps(), "options": {"colorBy": "rule", "ruleValue": 25, "ruleAbove": "#112233", "totalFill": "#7030a0"}}),
            640.0,
            300.0,
        ),
        (
            "wf_target",
            "waterfall",
            json!({"rows": with_target(150.0), "formats": ["", "", "", "#,0", "$#,0.00"]}),
            640.0,
            320.0,
        ),
        (
            "wf_target_horizontal_fr",
            "waterfall",
            json!({"rows": with_target(90.0), "formats": ["", "", "", "", "#,0.0"], "lang": "fr",
                   "options": {"orientation": "horizontal", "labelMode": "percent", "targetFill": "#C00000"}}),
            560.0,
            360.0,
        ),
        (
            "wf_measures",
            "waterfall",
            json!({"rows": measures, "names": ["Revenue", "Cost of sales", "Opex", "Other"],
                   "formats": ["", "", "", "", "#,0", "#,0", "#,0", "#,0"], "options": {"subtotalEvery": 2}}),
            640.0,
            320.0,
        ),
        (
            "wf_no_total_no_labels",
            "waterfall",
            json!({"rows": steps(), "options": {"showTotal": false, "labelMode": "none", "fontSize": 14}}),
            480.0,
            260.0,
        ),
        (
            "wf_bold_theme",
            "waterfall",
            json!({"rows": steps(), "options": {"stylePreset": "bold", "stylePills": true}, "theme": {"colors": ["#7030A0", "#00A3A3"]}}),
            640.0,
            320.0,
        ),
        (
            "wf_minimal_dark",
            "waterfall",
            json!({"rows": steps(), "options": {"stylePreset": "minimal"}, "theme": {"colors": ["#4A5BE8"], "background": "#1B1A19"}}),
            640.0,
            320.0,
        ),
        (
            "hl_waterfall",
            "waterfall",
            json!({"rows": steps(), "highlights": hl}),
            640.0,
            320.0,
        ),
        (
            "wf_hostile",
            "waterfall",
            json!({"rows": [
                ["zero", null, null, null, 0], ["neg", null, null, null, -12.5], ["text", null, null, null, "x"],
                ["null", null, null, null, null], ["huge", null, null, null, 1e300], [null, null, null, null, -7],
                [true, null, null, "t", 3], [4.5, null, null, null, -0.25]
            ], "options": {"fontSize": 1e9, "subtotalEvery": -3, "colorBy": 5, "orientation": "sideways"}}),
            500.0,
            300.0,
        ),
        (
            "wf_all_negative",
            "waterfall",
            json!({"rows": [["a", null, null, -5, -10], ["b", null, null, -5, -20], ["c", null, null, null, -5]],
                   "options": {"orientation": "horizontal", "labelMode": "both"}}),
            480.0,
            240.0,
        ),
        // thousands in French: the value axis groups its digits as the tooltip does (G117)
        (
            "wf_thousands_fr",
            "waterfall",
            json!({"rows": [
                ["Jan", null, null, null, 1200], ["Feb", null, null, null, -450], ["Mar", null, null, null, 300.5]
            ], "formats": ["", "", "", "", "#,0"], "lang": "fr"}),
            480.0,
            280.0,
        ),
        // a small fall near the axis's start: its label after the bar, clear of its name (G117)
        (
            "wf_small_fall_horizontal",
            "waterfall",
            json!({"rows": [
                ["Start", null, null, null, 10], ["Dip", null, null, null, -5], ["Rise", null, null, null, 900],
                ["Drop", null, null, null, -300]
            ], "options": {"orientation": "horizontal"}}),
            420.0,
            240.0,
        ),
        (
            "empty_waterfall",
            "waterfall",
            json!({"rows": [["a", null, null, 3, "x"], ["b", null, null, 4, null]]}),
            300.0,
            200.0,
        ),
        // Gauge / KPI+
        (
            "gauge_arc",
            "gauge",
            json!({"rows": one(json!(72), json!(80), json!(0), json!(100), three())}),
            360.0,
            240.0,
        ),
        (
            "gauge_arc_auto",
            "gauge",
            json!({"rows": one(json!(1234.5), Value::Null, Value::Null, Value::Null, [Value::Null, Value::Null, Value::Null]),
                   "formats": ["", "#,0"]}),
            300.0,
            220.0,
        ),
        (
            "gauge_arc_trend",
            "gauge",
            json!({"rows": trend(), "formats": ["mmm", "0"], "options": {"threshold1": 50, "threshold2": 75, "max": 100}}),
            360.0,
            300.0,
        ),
        (
            "gauge_bullet",
            "gauge",
            json!({"rows": trend(), "formats": ["", "0.0"], "options": {"gaugeStyle": "bullet", "threshold1": 60, "max": 100, "compareTo": "previous"}}),
            480.0,
            200.0,
        ),
        (
            "gauge_bullet_one",
            "gauge",
            json!({"rows": one(json!(0.62), json!(0.7), json!(0), json!(1), [json!(0.5), Value::Null, Value::Null]),
                   "formats": ["", "0%"], "options": {"gaugeStyle": "bullet"}}),
            420.0,
            110.0,
        ),
        (
            "gauge_kpi",
            "gauge",
            json!({"rows": trend(), "formats": ["", "#,0"], "options": {"gaugeStyle": "kpi"}}),
            320.0,
            220.0,
        ),
        (
            "gauge_kpi_fr_lower",
            "gauge",
            json!({"rows": trend(), "formats": ["", "#,0.0"], "lang": "fr",
                   "options": {"gaugeStyle": "kpi", "compareTo": "previous", "lowerIsBetter": true}}),
            320.0,
            220.0,
        ),
        (
            "gauge_kpi_no_trend",
            "gauge",
            json!({"rows": trend(), "options": {"gaugeStyle": "kpi", "showTrend": false}}),
            260.0,
            140.0,
        ),
        (
            "gauge_bands",
            "gauge",
            json!({"rows": one(json!(55), json!(60), json!(0), json!(100), three()),
                   "options": {"band1": "#FFC7CE", "band3": "#C6EFCE", "valueByBand": true, "targetFill": "#000000"}}),
            360.0,
            240.0,
        ),
        (
            "gauge_value_by_band_default",
            "gauge",
            json!({"rows": one(json!(95), Value::Null, json!(0), json!(100), three()), "options": {"valueByBand": true, "lowerIsBetter": true}}),
            360.0,
            240.0,
        ),
        (
            "gauge_roles_range",
            "gauge",
            json!({"rows": one(json!(-15), json!(10), json!(-50), json!(50), [json!(-20), json!(0), json!(80)]),
                   "options": {"min": 0, "max": 5, "threshold1": 1}}),
            360.0,
            240.0,
        ),
        (
            "gauge_bold_dark",
            "gauge",
            json!({"rows": trend(), "options": {"stylePreset": "bold", "threshold1": 60}, "theme": {"colors": ["#E07B00"], "background": "#201F1E"}}),
            360.0,
            280.0,
        ),
        (
            "gauge_minimal",
            "gauge",
            json!({"rows": one(json!(30), json!(50), Value::Null, json!(60), [Value::Null, Value::Null, Value::Null]), "options": {"stylePreset": "minimal"}}),
            300.0,
            200.0,
        ),
        (
            "hl_gauge",
            "gauge",
            json!({"rows": trend(), "highlights": [26, null, null, 0, 40]}),
            360.0,
            300.0,
        ),
        (
            "hl_gauge_kpi",
            "gauge",
            json!({"rows": trend(), "highlights": [26, null, null, null, null], "options": {"gaugeStyle": "kpi"}}),
            320.0,
            220.0,
        ),
        (
            "gauge_many",
            "gauge",
            json!({"rows": (0..100).map(|i| json!([format!("p{i}"), (i * 37 % 100) as f64, null, null, null, null, null, null, null])).collect::<Vec<_>>()}),
            360.0,
            300.0,
        ),
        (
            "gauge_small",
            "gauge",
            json!({"rows": one(json!(7), Value::Null, Value::Null, Value::Null, [Value::Null, Value::Null, Value::Null])}),
            120.0,
            50.0,
        ),
        (
            "gauge_hostile",
            "gauge",
            json!({"rows": [["a", "x", 1e300, "m", -1, 5, 5, 5, null], ["b", -0.0, null, 3, 2, 1e308, -1e308, null, "p"]],
                   "options": {"min": "zero", "max": -5, "gaugeStyle": 3, "band1": "#GGGGGG", "compareTo": "previous"}}),
            300.0,
            200.0,
        ),
        (
            "empty_gauge",
            "gauge",
            json!({"rows": [["a", "x", 3], [null, null, 4]]}),
            300.0,
            200.0,
        ),
    ]
}

fn golden_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/report/waterfall_gauge_golden.json")
}

/// The golden file's text: one primitive or hit per line, so a change shows as a short diff.
fn golden_text(got: &serde_json::Map<String, Value>) -> String {
    let list = |v: &Value| {
        let items: Vec<String> = v
            .as_array()
            .map(|l| l.iter().map(|x| format!("      {x}")).collect())
            .unwrap_or_default();
        if items.is_empty() {
            "[]".to_string()
        } else {
            format!("[\n{}\n    ]", items.join(",\n"))
        }
    };
    let cases: Vec<String> = got
        .iter()
        .map(|(k, v)| {
            format!(
                "  {}: {{\n    \"prims\": {},\n    \"hits\": {}\n  }}",
                json!(k),
                list(&v["prims"]),
                list(&v["hits"])
            )
        })
        .collect();
    format!("{{\n{}\n}}\n", cases.join(",\n"))
}

fn rust_results() -> serde_json::Map<String, Value> {
    let mut out = serde_json::Map::new();
    for (name, kind, input, w, h) in cases() {
        out.insert(name.into(), chart_pack::layout(kind, &input, w, h).json());
    }
    out
}

/// The same JSON, numbers within 0.011 (both round to hundredths).
fn same(a: &Value, b: &Value, at: &str) -> Result<(), String> {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            let (x, y) = (x.as_f64().unwrap_or(f64::NAN), y.as_f64().unwrap_or(f64::NAN));
            if (x - y).abs() <= 0.011 {
                Ok(())
            } else {
                Err(format!("{at}: {x} != {y}"))
            }
        }
        (Value::Array(x), Value::Array(y)) => {
            if x.len() != y.len() {
                return Err(format!("{at}: {} items != {}", x.len(), y.len()));
            }
            for (i, (p, q)) in x.iter().zip(y).enumerate() {
                same(p, q, &format!("{at}[{i}]"))?;
            }
            Ok(())
        }
        (Value::Object(x), Value::Object(y)) => {
            let mut kx: Vec<&String> = x.keys().collect();
            let mut ky: Vec<&String> = y.keys().collect();
            kx.sort();
            ky.sort();
            if kx != ky {
                return Err(format!("{at}: keys {kx:?} != {ky:?}"));
            }
            for k in kx {
                same(&x[k], &y[k], &format!("{at}.{k}"))?;
            }
            Ok(())
        }
        _ if a == b => Ok(()),
        _ => Err(format!("{at}: {a} != {b}")),
    }
}

#[test]
fn the_rust_layouts_match_the_golden_file() {
    let got = rust_results();
    let path = golden_path();
    if bicraft_env::var_os("BICRAFT_BLESS").is_some() {
        std::fs::write(&path, golden_text(&got)).unwrap();
    }
    let want: Value = serde_json::from_str(
        &std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{}: {e} (BICRAFT_BLESS=1 writes it)", path.display())),
    )
    .unwrap();
    same(&Value::Object(got.clone()), &want, "golden").unwrap();
    for (name, v) in &got {
        let n = v["prims"].as_array().unwrap().len();
        assert_eq!(n == 0, name.starts_with("empty"), "{name}: {n} primitives");
        for h in v["hits"].as_array().unwrap() {
            assert!(h["prim"].as_u64().unwrap() < n as u64, "{name}: {h}");
        }
    }
}

/// Every text stays inside the visual, every number drawn is finite.
#[test]
fn texts_stay_inside_and_numbers_are_finite() {
    let mut errors = Vec::new();
    for (name, kind, input, w, h) in cases() {
        let out = chart_pack::layout(kind, &input, w, h).json();
        if out.to_string().contains("null") && !name.starts_with("empty") {
            // a `null` would be a NaN written out (hits carry `null` for an absent target)
            for p in out["prims"].as_array().unwrap() {
                assert!(!p.to_string().contains("null"), "{name}: {p}");
            }
        }
        for p in out["prims"].as_array().unwrap() {
            if p["t"] != "text" {
                continue;
            }
            let x = p["x"].as_f64().unwrap();
            let y = p["y"].as_f64().unwrap();
            let size = p["size"].as_f64().unwrap();
            let wd = p["text"].as_str().unwrap().chars().count() as f64 * size * 0.55;
            let (l, r) = match p["anchor"].as_str().unwrap() {
                "start" => (x, x + wd),
                "end" => (x - wd, x),
                _ => (x - wd / 2.0, x + wd / 2.0),
            };
            if l < -0.01 || r > w + 0.01 || y - size / 2.0 < -0.01 || y + size / 2.0 > h + 0.01 {
                errors.push(format!("{name}: {p} outside {w}x{h}"));
            }
        }
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

/// Every case laid out by the visuals' TypeScript under Node, `None` without Node.
fn ts_results() -> Option<Value> {
    if !has_node() {
        return None;
    }
    let dir = TempDir::new("waterfall-gauge-ts");
    let harness = format!(
        "var M = {}\n\
         var fs = require('fs');\n\
         var cases = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));\n\
         var out = {{}};\n\
         for (var i = 0; i < cases.length; i++) {{\n\
           var c = cases[i];\n\
           out[c.name] = M.layout(c.kind, c.input, c.w, c.h);\n\
         }}\n\
         fs.writeFileSync(process.argv[3], JSON.stringify(out));\n",
        layout_script().unwrap()
    );
    let script = dir.join("harness.js");
    std::fs::write(&script, harness).unwrap();
    let input: Vec<Value> = cases()
        .into_iter()
        .map(|(name, kind, input, w, h)| json!({"name": name, "kind": kind, "input": input, "w": w, "h": h}))
        .collect();
    let cases_path = dir.join("cases.json");
    std::fs::write(&cases_path, Value::Array(input).to_string()).unwrap();
    let out_path = dir.join("out.json");
    let o = std::process::Command::new("node")
        .arg(&script)
        .arg(&cases_path)
        .arg(&out_path)
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "node failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    Some(serde_json::from_str(&std::fs::read_to_string(&out_path).unwrap()).unwrap())
}

#[test]
fn the_typescript_layouts_match_the_rust_ones() {
    let Some(ts) = ts_results() else {
        eprintln!(
            "note: node is not on the PATH; the Waterfall+ and Gauge / KPI+ TypeScript layouts are not compared (development check)"
        );
        return;
    };
    let mut errors = Vec::new();
    for (name, want) in &rust_results() {
        if let Err(e) = same(&ts[name], want, name) {
            errors.push(e);
        }
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

/// The value axis groups its digits by the culture, and a small fall's label in the horizontal
/// layout keeps clear of its category name (G117, seen in Power BI Desktop 2.160.1151.0).
#[test]
fn waterfall_axis_follows_the_culture_and_fall_labels_clear_the_names() {
    let r = rust_results();
    let texts = |name: &str| -> Vec<Value> {
        r[name]["prims"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["t"] == "text")
            .cloned()
            .collect()
    };
    let h = texts("wf_small_fall_horizontal");
    let find = |s: &str| {
        h.iter()
            .find(|p| p["text"] == s)
            .unwrap_or_else(|| panic!("{s}: {h:?}"))
    };
    let (label, name) = (find("-5"), find("Dip"));
    let name_x = name["x"].as_f64().unwrap();
    let left = if label["anchor"] == "start" {
        label["x"].as_f64().unwrap()
    } else {
        label["x"].as_f64().unwrap() - 2.0 * label["size"].as_f64().unwrap() * 0.55
    };
    assert!(left >= name_x + 8.0, "-5 from {left}, Dip ends at {name_x}");
    let fr = texts("wf_thousands_fr");
    let all: Vec<&str> = fr.iter().map(|p| p["text"].as_str().unwrap()).collect();
    assert!(all.contains(&"1\u{a0}000"), "{all:?}");
    assert!(
        !all.iter().any(|s| s.contains(',') && !s.contains('%')),
        "{all:?}"
    );
}

/// The Waterfall+'s running totals: each step floats from the total before it, the subtotals
/// and the total stand on zero at the running total, and the groups close with a subtotal.
#[test]
fn waterfall_steps_run_and_subtotals_close_groups() {
    let out = chart_pack::layout("waterfall", &json!({"rows": steps()}), 640.0, 320.0);
    let bars: Vec<&Value> = out.hits.iter().filter(|h| h.get("item").is_some()).collect();
    let kinds: Vec<&str> = bars.iter().map(|h| h["kind"].as_str().unwrap()).collect();
    assert_eq!(
        kinds,
        ["step", "step", "step", "sub", "step", "step", "step", "total"]
    );
    let mut run = 0.0;
    for h in &bars {
        let (start, end, value) = (
            h["start"].as_f64().unwrap(),
            h["end"].as_f64().unwrap(),
            h["value"].as_f64().unwrap(),
        );
        if h["kind"] == "step" {
            assert_eq!(start, run, "{h}");
            run += value;
            assert_eq!(end, run, "{h}");
        } else {
            assert_eq!((start, end, value), (0.0, run, run), "{h}");
        }
    }
    assert_eq!(bars[3]["label"], "Q1");
    assert_eq!(run, 110.0);
    // several measures: each one a step, its total over the rows
    let out = chart_pack::layout(
        "waterfall",
        &json!({"rows": [[null, null, null, null, 10, -4], [null, null, null, null, 5, -1]], "names": ["In", "Out"]}),
        400.0,
        300.0,
    );
    let vals: Vec<(String, f64, i64)> = out
        .hits
        .iter()
        .filter(|h| h.get("item").is_some())
        .map(|h| {
            (
                h["label"].as_str().unwrap().to_string(),
                h["value"].as_f64().unwrap(),
                h["row"].as_i64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        vals,
        [
            ("In".into(), 15.0, -1),
            ("Out".into(), -5.0, -1),
            ("Total".into(), 10.0, -1)
        ]
    );
}

// ---------------------------------------------------------------------------------------------
// The packages under Node
// ---------------------------------------------------------------------------------------------

const VISUAL_CASES: &str = r##"
var CASES = {
  waterfall: { cats: [col("Month", "category", ["Jan", "Feb", "Mar"])], vals: [col("Profit", "values", [10, -4, 6], "#,0"), col("Goal", "target", [5, 5, 5], "#,0")],
    missing: [col("Month", "category", ["Jan"])], target: 1 },
  gauge: { cats: [col("Day", "trend", [new Date(2024, 0, 1), new Date(2024, 0, 2), new Date(2024, 0, 3)], "yyyy-MM-dd")],
    vals: [col("Sales", "value", [50, 60, 72], "#,0"), col("Goal", "target", [80, 80, 80], "#,0")],
    missing: [col("Day", "trend", [new Date(2024, 0, 1)])], target: 0 }
};
var out = {};
Object.keys(CASES).forEach(function (kind) {
  var c = CASES[kind]; var r = {};
  var element = new El("div"); var host = makeHost("en-US");
  var v = PLUGINS[kind].create({ element: element, host: host });
  v.update({ dataViews: [{ metadata: { columns: [] } }], viewport: { width: 400, height: 300 } });
  r.landing = element.all("p").map(function (p) { return p.textContent; }).join("|");
  var frEl = new El("div"); var fr = PLUGINS[kind].create({ element: frEl, host: makeHost("fr-FR") });
  fr.update({ dataViews: [dataView(c.missing, [])], viewport: { width: 400, height: 300 } });
  r.missing = frEl.all("p").map(function (p) { return p.textContent; }).join("|");
  host.cats = c.cats;
  v.update({ dataViews: [dataView(c.cats, c.vals)], viewport: { width: 600, height: 300 } });
  var svg = element.all("svg")[0];
  var points = svg.all("rect").concat(svg.all("circle"), svg.all("text"), svg.all("polygon"), svg.all("line")).filter(function (e) { return e.attrs.tabindex === "0"; });
  r.targets = points.length;
  r.role = svg.attrs.role;
  var t = points[c.target];
  r.label = t.attrs["aria-label"];
  fire(t, "mousemove");
  r.tip = host.log[host.log.length - 1];
  fire(t, "click");
  r.selected = host.sel.map(function (s) { return s.k; });
  fire(t, "contextmenu");
  r.menu = host.log[host.log.length - 1][1];
  var hcEl = new El("div"); var hc = PLUGINS[kind].create({ element: hcEl, host: makeHost("en-US", true) });
  hc.update({ dataViews: [dataView(c.cats, c.vals)], viewport: { width: 600, height: 300 } });
  var anyText = hcEl.all("svg")[0].all("text")[0];
  r.contrast = anyText ? anyText.attrs.fill : null;
  r.cards = v.getFormattingModel().cards.map(function (k) { return k.uid; });
  var frModel = fr.getFormattingModel();
  r.fr_cards = frModel.cards.map(function (k) { return k.displayName; });
  out[kind] = r;
});
require("fs").writeFileSync(process.argv[2], JSON.stringify(out));
"##;

/// Each packaged visual under Node with a stand-in host and document: the landing page without
/// fields, a message when its required role is missing, its points with tooltips, selection,
/// the context menu, high contrast and the format pane in English and French.
#[test]
fn the_packaged_visuals_run_under_node() {
    if !has_node() {
        eprintln!(
            "note: node is not on the PATH; the Waterfall+ and Gauge / KPI+ visuals are not run (development check)"
        );
        return;
    }
    let dir = TempDir::new("waterfall-gauge-visuals");
    let mut script = String::from(DOM);
    for kind in ["waterfall", "gauge"] {
        let d = find(kind).unwrap();
        let pkg = package_json(d.guid, &package_bytes(d).unwrap());
        script.push_str(pkg["content"]["js"].as_str().unwrap());
        script.push_str(&format!("\nPLUGINS[{kind:?}] = {}.default;\n", d.guid));
    }
    script.push_str(VISUAL_CASES);
    let path = dir.join("visuals.js");
    std::fs::write(&path, script).unwrap();
    let out_path = dir.join("out.json");
    let o = std::process::Command::new("node")
        .arg(&path)
        .arg(&out_path)
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "node failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    let got: Value = serde_json::from_str(&std::fs::read_to_string(&out_path).unwrap()).unwrap();
    let want = json!({
        "waterfall": {
            "landing": "BIcraft Waterfall+|Add fields to Values to draw this visual.",
            "missing": "Ajoutez des champs dans Valeurs pour dessiner ce visuel.",
            "role": "listbox",
            "label": "Month: Feb, Profit: -4, Running total: 6, Change: -40.0%",
            "tip": ["show", 4], "selected": ["Feb"], "menu": "Feb",
            "contrast": "#FFFF00", "cards": ["waterfall", "colors", "labels", "target", "style"],
            "fr_cards": ["Cascade", "Couleurs", "Étiquettes", "Cible", "Style"]
        },
        "gauge": {
            "landing": "BIcraft Gauge / KPI+|Add fields to Value to draw this visual.",
            "missing": "Ajoutez des champs dans Valeur pour dessiner ce visuel.",
            "role": "listbox",
            "label": "Day: 2024-01-01, Sales: 50",
            "tip": ["show", 2], "selected": ["2024-01-01"], "menu": "2024-01-01",
            "contrast": "#FFFF00", "cards": ["gauge", "range", "colors", "style"],
            "fr_cards": ["Jauge", "Échelle", "Couleurs", "Style"]
        }
    });
    let mut errors = Vec::new();
    for k in ["waterfall", "gauge"] {
        for (key, v) in want[k].as_object().unwrap() {
            if &got[k][key] != v {
                errors.push(format!("{k}.{key}: got {}, want {v}", got[k][key]));
            }
        }
        assert!(got[k]["targets"].as_u64().unwrap() >= 3, "{k}: {}", got[k]);
    }
    assert!(errors.is_empty(), "{}\nall: {got}", errors.join("\n"));
}

// ---------------------------------------------------------------------------------------------
// In a report
// ---------------------------------------------------------------------------------------------

fn session() -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Plus"}));
    run(
        &mut s,
        "table.import",
        json!({"name": "F", "m": "let Source = #table(type table [Month = text, Qtr = text, Day = date, Amount = Int64.Type, Cost = Int64.Type], {{\"Jan\", \"Q1\", #date(2024, 1, 31), 120, 40}, {\"Feb\", \"Q1\", #date(2024, 2, 29), -45, 30}, {\"Apr\", \"Q2\", #date(2024, 4, 30), 65, 20}}) in Source"}),
    );
    for (name, e) in [
        ("Profit", "SUM(F[Amount])"),
        ("Costs", "-SUM(F[Cost])"),
        ("Goal", "150"),
    ] {
        run(
            &mut s,
            "measure.add",
            json!({"table": "F", "name": name, "expression": e}),
        );
    }
    run(&mut s, "report.new", json!({}));
    s
}

fn page(s: &Session) -> String {
    s.report.as_ref().unwrap().pages[0].name.clone()
}

fn snap_visual(s: &Session, name: &str) -> Value {
    s.snapshot()["report"]["pages"][0]["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == name)
        .cloned()
        .unwrap()
}

fn drawn(s: &mut Session, names: &[&str]) {
    let p = page(s);
    let r = run(s, "report.render", json!({"page": p, "system_fonts": false}));
    for name in names {
        let v = r["pages"][0]["visuals"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["name"] == *name)
            .unwrap();
        assert_eq!(v["status"], "drawn", "{v}");
    }
}

#[test]
fn both_visuals_are_added_by_name_drawn_formatted_and_kept() {
    let mut s = session();
    let p = page(&s);
    let add = |s: &mut Session, name: &str, y: i64, fields: Value| {
        let out = run(
            s,
            "visual.add",
            json!({"page": p, "visual_type": name, "x": 10, "y": y, "width": 600, "height": 240, "fields": fields}),
        );
        assert_eq!(out["installed"], y != 250, "{out}");
        out["visual"].as_str().unwrap().to_string()
    };
    let wf = add(
        &mut s,
        "bicraftWaterfallPlus",
        0,
        json!([{"role": "category", "field": "F[Month]"}, {"role": "group", "field": "F[Qtr]"},
               {"role": "values", "field": "[Profit]"}, {"role": "target", "field": "[Goal]"}]),
    );
    // several measures in the value role
    let bridge = add(
        &mut s,
        "bicraftWaterfallPlus",
        250,
        json!([{"role": "values", "field": "[Profit]"}, {"role": "values", "field": "[Costs]"}]),
    );
    let gauge = add(
        &mut s,
        "bicraftGaugeKpi",
        500,
        json!([{"role": "trend", "field": "F[Day]"}, {"role": "value", "field": "[Profit]"}, {"role": "target", "field": "[Goal]"}]),
    );
    for (v, kind) in [(&wf, "waterfall"), (&bridge, "waterfall"), (&gauge, "gauge")] {
        let sv = snap_visual(&s, v);
        assert_eq!(sv["chart_pack"]["kind"], kind, "{sv}");
        assert_eq!(sv["data"]["status"], "ok", "{}", sv["data"]);
    }
    // the renderer draws one step per measure of the value role, named by its measure
    let fonts = bicraft_render::FontBook::bundled_only();
    let o = bicraft_render::RenderOptions {
        system_fonts: false,
        hits: true,
        ..bicraft_render::RenderOptions::default()
    };
    let hits = bicraft_render::render_page(
        &s.snapshot(),
        0,
        &std::collections::HashMap::<String, Vec<u8>>::new(),
        &fonts,
        &o,
    )
    .unwrap()
    .hits;
    let mut tips: Vec<Vec<(String, String)>> = Vec::new();
    for h in hits.into_iter().filter(|h| h.visual == bridge) {
        if !h.tip.is_empty() && !tips.contains(&h.tip) {
            tips.push(h.tip);
        }
    }
    let step = |name: &str| {
        tips.iter()
            .find(|t| t.first().is_some_and(|x| x.1 == name))
            .cloned()
    };
    assert_eq!(
        step("Profit").map(|t| t.get(1).cloned()),
        Some(Some(("Profit".to_string(), "+140".to_string()))),
        "{tips:?}"
    );
    assert_eq!(
        step("Costs").map(|t| t.get(2).cloned()),
        Some(Some(("Running total".to_string(), "50".to_string()))),
        "{tips:?}"
    );
    drawn(&mut s, &[&wf, &bridge, &gauge]);
    // the format pane's properties reach the layouts
    run(
        &mut s,
        "visual.format",
        json!({"page": p, "visual": wf, "objects": [
            {"object": "waterfall", "property": "orientation", "value": "horizontal"},
            {"object": "waterfall", "property": "subtotalEvery", "value": 2},
            {"object": "colors", "property": "colorBy", "value": "rule"},
            {"object": "colors", "property": "ruleAbove", "value": {"color": "#112233"}},
            {"object": "labels", "property": "mode", "value": "both"}
        ]}),
    );
    let wf_want = json!({"orientation": "horizontal", "subtotalEvery": 2.0, "colorBy": "rule", "ruleAbove": "#112233", "labelMode": "both"});
    assert_eq!(snap_visual(&s, &wf)["chart_pack"]["options"], wf_want);
    run(
        &mut s,
        "visual.format",
        json!({"page": p, "visual": gauge, "objects": [
            {"object": "gauge", "property": "style", "value": "kpi"},
            {"object": "range", "property": "threshold1", "value": 50},
            {"object": "colors", "property": "band2", "value": {"color": "#C6EFCE"}},
            {"object": "gauge", "property": "compareTo", "value": "nowhere"}
        ]}),
    );
    let gauge_want = json!({"gaugeStyle": "kpi", "threshold1": 50.0, "band2": "#C6EFCE"});
    assert_eq!(snap_visual(&s, &gauge)["chart_pack"]["options"], gauge_want);
    drawn(&mut s, &[&wf, &bridge, &gauge]);
    let dir = TempDir::new("waterfall-gauge-rt");
    let pdf = dir.join("r.pdf");
    let r = run(
        &mut s,
        "report.export_pdf",
        json!({"out": pdf, "system_fonts": false}),
    );
    for v in r["pages"][0]["visuals"].as_array().unwrap() {
        assert_eq!(v["status"], "drawn", "pdf {v}");
    }
    for file in ["plus.pbip", "plus.pbix"] {
        let path = dir.join(file);
        run(&mut s, "file.save_as", json!({"path": path}));
        let mut t = Session::new();
        run(&mut t, "file.open", json!({"path": path}));
        assert_eq!(snap_visual(&t, &wf)["chart_pack"]["options"], wf_want, "{file}");
        assert_eq!(
            snap_visual(&t, &gauge)["chart_pack"]["options"],
            gauge_want,
            "{file}"
        );
        for kind in ["waterfall", "gauge"] {
            assert!(!install(&mut t, kind).unwrap(), "{file}: {kind} was kept");
        }
        drawn(&mut t, &[&wf, &bridge, &gauge]);
    }
    // both are in the pack and registered with their versions
    for (kind, version) in [("waterfall", "1.0.3.0"), ("gauge", "1.0.2.0")] {
        let d = find(kind).unwrap();
        assert_eq!(d.version, version);
        assert!(VISUALS.iter().any(|x| x.guid == d.guid));
    }
}
