//! The BIcraft Spec visual (docs/formats/spec-visual.md): `visual.set_spec` embeds the visual
//! and stores a checked spec that `report.render` draws; `visual.spec_template` and
//! `visual.scaffold {template: "spec"}`; and the golden comparison of the two layouts: the
//! renderer's Rust (`bicraft_render::spec`) against `spec_golden.json`, and the visual's
//! TypeScript (compiled by BIcraft's own bundler) against the Rust, run with Node when Node is on
//! the PATH (a development check; it prints a note and passes when Node is missing).
//! `BICRAFT_BLESS=1` rewrites the golden file from the Rust layout.

use std::path::{Path, PathBuf};

use bicraft_engine::Session;
use bicraft_engine::spec_visual::{SPEC_GUID, layout_script};
use bicraft_render::spec;
use serde_json::{Value, json};

use crate::tempdir::TempDir;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Runs a command that must fail and checks it left the report alone.
fn fails(s: &mut Session, id: &str, params: Value) -> String {
    let before = s.report.clone();
    let err = s
        .execute(id, &params)
        .expect_err(&format!("{id} {params} should fail"));
    assert_eq!(s.report, before, "{id} changed the report although it failed");
    err
}

// ---------------------------------------------------------------------------------------------
// Golden layouts
// ---------------------------------------------------------------------------------------------

/// The cases: a spec, data and a size. Specs that must be refused carry no size.
pub(crate) fn cases() -> Vec<(&'static str, Value, Value, f64, f64)> {
    let abc = json!({"categories": ["a", "b", "c"], "values": [[100, 200, 442], [30, null, 10]]});
    let months = json!({"categories": [{"date": "2024-01-01"}, {"date": "2024-02-01"}, {"date": "2024-03-01"}, {"date": "2024-04-01"}],
        "values": [[5.5, 7.25, -2, 12], [1, 2, 3, 4]]});
    let pts = json!({"categories": ["p", "q", "r", "s", "t"], "values": [[1.5, 3, 4.25, 8, 9.75], [10, 40, 20, 80, 55]]});
    let many: Vec<String> = (0..40).map(|i| format!("Category {i}")).collect();
    let many_vals: Vec<f64> = (0..40).map(|i| f64::from(i * 37 % 101) - 20.0).collect();
    let many = json!({"categories": many, "values": [many_vals]});
    // highlighted by another visual's selection (docs/formats/bicraft-visuals.md "Highlights"):
    // part of a, none of b, more than all of c (kept to the value), the second measure on b only
    let mut abc_hl = abc.clone();
    abc_hl["highlights"] = json!([[40, null, 500], [null, null, null]]);
    let mut negative_hl = json!({"categories": ["x", "y"], "values": [[-30, 50]]});
    negative_hl["highlights"] = json!([[-12, 50]]);
    vec![
        (
            "hl_bar",
            spec::template("bar").unwrap(),
            abc_hl.clone(),
            320.0,
            200.0,
        ),
        ("hl_combo", spec::template("combo").unwrap(), abc_hl, 400.0, 240.0),
        (
            "hl_bar_negative",
            json!({"marks": [{"type": "bar", "y": "value", "label": "value"}], "style": {"preset": "minimal"}}),
            negative_hl,
            240.0,
            180.0,
        ),
        // BIcraft's look (docs/formats/visual-look.md)
        (
            "look_bar_bold",
            {
                let mut s = spec::template("bar").unwrap();
                s["style"] = json!({"preset": "bold"});
                s
            },
            abc.clone(),
            320.0,
            200.0,
        ),
        (
            "look_combo_flat",
            {
                let mut s = spec::template("combo").unwrap();
                s["style"] = json!({"rounded": false, "shadow": false, "palette": "bicraft"});
                s
            },
            abc.clone(),
            400.0,
            240.0,
        ),
        // the look's second pass: value chips on the combo's labels, no card
        (
            "look2_combo_pills",
            {
                let mut s = spec::template("combo").unwrap();
                s["style"] = json!({"pills": true, "card": false});
                s
            },
            abc.clone(),
            400.0,
            240.0,
        ),
        (
            "look_bad_preset",
            json!({"style": {"preset": "neon"}, "marks": [{"type": "bar"}]}),
            json!({}),
            0.0,
            0.0,
        ),
        (
            "look_bad_flag",
            json!({"style": {"shadow": "yes"}, "marks": [{"type": "bar"}]}),
            json!({}),
            0.0,
            0.0,
        ),
        ("bar", spec::template("bar").unwrap(), abc.clone(), 320.0, 200.0),
        (
            "combo",
            spec::template("combo").unwrap(),
            abc.clone(),
            400.0,
            240.0,
        ),
        (
            "line_gap",
            json!({"marks": [{"type": "line", "y": "value.1", "name": "Second"}, {"type": "point", "y": "value.1"}]}),
            abc.clone(),
            300.0,
            180.0,
        ),
        (
            "area_time",
            json!({"x": {"scale": "time", "title": "Month"}, "y": {"title": "Amount", "zero": false},
            "marks": [{"type": "area", "y": "value"}, {"type": "line", "y": "value", "size": 1.5, "label": "value", "decimals": 1}]}),
            months.clone(),
            360.0,
            220.0,
        ),
        (
            "scatter",
            spec::template("scatter").unwrap(),
            pts.clone(),
            300.0,
            300.0,
        ),
        (
            "color_by_value",
            json!({"legend": false, "palette": ["#112233", "#445566"],
            "marks": [{"type": "bar", "color": {"field": "value", "range": ["#FFFFFF", "#000000"]}, "label": "category"},
                      {"type": "rule", "y": {"value": 150}, "color": "#FF0000", "size": 2},
                      {"type": "rule", "y": "value.1"}]}),
            abc.clone(),
            260.0,
            160.0,
        ),
        (
            "by_category",
            json!({"marks": [{"type": "point", "color": {"field": "category"}, "size": {"field": "value", "range": [2, 9]}},
            {"type": "text", "label": "value", "y": "value", "size": 9}]}),
            pts.clone(),
            280.0,
            200.0,
        ),
        (
            "rect_linear",
            json!({"x": {"scale": "linear", "zero": true}, "marks": [{"type": "rect", "x": {"value": 0}, "x2": "value", "y": "value.1", "y2": {"value": 5}, "opacity": 0.5},
            {"type": "rule", "x": {"value": 5}}]}),
            pts.clone(),
            300.0,
            200.0,
        ),
        (
            "crowded_negative",
            json!({"y": {"min": -40}, "marks": [{"type": "bar", "label": "value"}]}),
            many,
            500.0,
            220.0,
        ),
        (
            "no_axes_tiny",
            json!({"padding": 0, "x": {"axis": false}, "y": {"axis": false, "grid": false}, "marks": [{"type": "bar"}]}),
            abc.clone(),
            30.0,
            20.0,
        ),
        (
            "empty_data",
            spec::template("line").unwrap(),
            json!({}),
            200.0,
            120.0,
        ),
        (
            "too_small",
            spec::template("bar").unwrap(),
            abc.clone(),
            40.0,
            30.0,
        ),
        (
            "x_text_rule",
            json!({"marks": [{"type": "bar"}, {"type": "rule", "x": {"value": "b"}, "color": "#00AA00"}]}),
            abc,
            240.0,
            150.0,
        ),
        // the time-scale case Desktop drew (a legend, the last tick at the right edge)
        (
            "time_legend",
            json!({"x": {"scale": "time"}, "marks": [{"type": "line", "name": "Total"}, {"type": "point"}]}),
            json!({"categories": [{"date": "2025-12-20"}, {"date": "2026-02-01"}, {"date": "2026-03-18"}],
                "values": [[12, 30, 21]]}),
            300.0,
            180.0,
        ),
        // date labels in the category's format string (G105, 2026-11-24): time-axis ticks of
        // days or more, band labels, data labels and a legend by category
        (
            "time_dates_fr",
            json!({"x": {"scale": "time"}, "marks": [{"type": "line", "name": "Total"}, {"type": "point"}]}),
            json!({"categories": [{"date": "2025-12-20"}, {"date": "2026-02-01"}, {"date": "2026-03-18"}],
                "values": [[12, 30, 21]], "categoryFormat": "dd/mm/yyyy", "locale": "fr-FR"}),
            360.0,
            180.0,
        ),
        (
            "band_dates_months",
            json!({"marks": [{"type": "bar", "label": "category"}]}),
            json!({"categories": [{"date": "2024-01-01"}, {"date": "2024-02-01"}, {"date": "2024-03-01"}],
                "values": [[5, 7, 2]], "categoryFormat": "mmm yyyy", "locale": "en-US"}),
            320.0,
            200.0,
        ),
        (
            "legend_dates_fr",
            json!({"marks": [{"type": "bar", "color": {"field": "category"}}]}),
            json!({"categories": [{"date": "2024-01-31"}, {"date": "2024-02-29"}],
                "values": [[5, 7]], "categoryFormat": "Long Date", "locale": "fr-FR"}),
            420.0,
            200.0,
        ),
        // every bound measure as its own series (docs/formats/spec-visual.md "Series"): the bar
        // template over two named measures draws clustered bars with a legend; a spec that reads
        // one measure in a way that cannot be repeated says how many it leaves out
        (
            "series_bar_two",
            spec::template("bar").unwrap(),
            json!({"categories": ["North", "South", "East"], "values": [[416198.52, 159246.32, 675928.33], [5200, 1800, 7400]],
                "names": ["Sum of amount", "Sum of quantity"], "categoryName": "Region"}),
            420.0,
            240.0,
        ),
        (
            "series_line_three",
            spec::template("line").unwrap(),
            json!({"categories": ["a", "b", "c", "d"], "values": [[1, 3, 2, 4], [2, 2, 3, 1], [4, 1, 1, 2]],
                "names": ["One", "Two", "Three"]}),
            360.0,
            220.0,
        ),
        (
            "series_notice_by_category",
            spec::template("point").unwrap(),
            json!({"categories": ["a", "b", "c"], "values": [[1, 2, 3], [9, 8, 7]], "locale": "fr-FR"}),
            360.0,
            220.0,
        ),
        // the text settings (docs/formats/spec-visual.md "Text")
        (
            "text_labels_big",
            {
                let mut s = spec::template("bar").unwrap();
                s["text"] = json!({"labels": {"size": 16, "color": "#112233", "bold": false, "italic": true, "units": 0, "decimals": 1},
                    "y": {"size": 13, "units": 1000, "family": "Georgia, serif"},
                    "x": {"size": 9, "underline": true, "color": "#445566"}});
                s
            },
            json!({"categories": ["North", "South", "East"], "values": [[416198.52, 159246.32, 675928.33]]}),
            420.0,
            260.0,
        ),
        (
            "text_titles_legend_fr",
            {
                let mut s = spec::template("combo").unwrap();
                s["text"] = json!({"xTitle": {"show": true, "size": 14, "bold": true}, "yTitle": {"show": true},
                    "legend": {"size": 9, "color": "#202020"}, "labels": {"show": true}});
                s
            },
            json!({"categories": ["Nord", "Sud"], "values": [[1234567.891, 2500000], [12.5, 30]],
                "names": ["Somme de montant", "Somme de quantité"], "categoryName": "Région", "locale": "fr-FR"}),
            440.0,
            260.0,
        ),
        (
            "text_all_hidden",
            {
                let mut s = spec::template("combo").unwrap();
                s["text"] = json!({"x": {"show": false}, "y": {"show": false}, "legend": {"show": false}, "labels": {"show": false}});
                s
            },
            json!({"categories": ["a", "b", "c"], "values": [[100, 200, 442], [30, null, 10]]}),
            300.0,
            180.0,
        ),
        (
            "text_tiny_still_draws",
            {
                let mut s = spec::template("bar").unwrap();
                s["text"] = json!({"labels": {"size": 20}, "x": {"size": 14}, "y": {"size": 14}});
                s
            },
            json!({"categories": ["a", "b"], "values": [[3, 5]]}),
            90.0,
            70.0,
        ),
        // refused by both
        (
            "bad_text_size",
            json!({"marks": [{"type": "bar"}], "text": {"labels": {"size": 2}}}),
            json!({}),
            0.0,
            0.0,
        ),
        (
            "bad_text_key",
            json!({"marks": [{"type": "bar"}], "text": {"x": {"units": 1000}}}),
            json!({}),
            0.0,
            0.0,
        ),
        (
            "bad_text_units",
            json!({"marks": [{"type": "bar"}], "text": {"y": {"units": 10}}}),
            json!({}),
            0.0,
            0.0,
        ),
        (
            "bad_key",
            json!({"marks": [{"type": "bar", "colour": "#000000"}]}),
            json!({}),
            0.0,
            0.0,
        ),
        (
            "bad_y",
            json!({"marks": [{"type": "line", "y": "category"}]}),
            json!({}),
            0.0,
            0.0,
        ),
        (
            "bad_scale",
            json!({"x": {"scale": "linear"}, "marks": [{"type": "bar"}]}),
            json!({}),
            0.0,
            0.0,
        ),
        (
            "bad_ref",
            json!({"marks": [{"type": "point", "x": "value.99"}]}),
            json!({}),
            0.0,
            0.0,
        ),
        (
            "bad_rule",
            json!({"marks": [{"type": "rule", "x": "category", "y": "value"}]}),
            json!({}),
            0.0,
            0.0,
        ),
    ]
}

fn golden_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/report/spec_golden.json")
}

/// The Rust layout of every case: `{name: prims}` or `{name: {"error": message}}`.
fn rust_results() -> serde_json::Map<String, Value> {
    let mut out = serde_json::Map::new();
    for (name, sp, data, w, h) in cases() {
        let v = match spec::parse(&sp) {
            Ok(s) => spec::layout_json(&s, &spec::SpecData::from_json(&data), w, h),
            Err(e) => json!({"error": e}),
        };
        out.insert(name.into(), v);
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
fn the_rust_layout_matches_the_golden_file() {
    let got = rust_results();
    let path = golden_path();
    if bicraft_env::var_os("BICRAFT_BLESS").is_some() {
        let mut text = serde_json::to_string_pretty(&Value::Object(got.clone())).unwrap();
        text.push('\n');
        std::fs::write(&path, text).unwrap();
    }
    let want: Value = serde_json::from_str(
        &std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{}: {e} (BICRAFT_BLESS=1 writes it)", path.display())),
    )
    .unwrap();
    same(&Value::Object(got), &want, "golden").unwrap();
    // every drawn case draws something, except the visual too small for a plot
    for (name, v) in rust_results() {
        if let Some(a) = v.as_array() {
            assert_eq!(a.is_empty(), name == "too_small", "{name}");
        }
    }
}

/// Axis texts stay clear of the legend and inside the visual (both drawers, since the
/// TypeScript is held to the same primitives): the legend row ends above the top y tick
/// label, and every x tick label's estimated extent lies within `[0, width]`.
#[test]
fn axis_labels_clear_the_legend_and_stay_inside_the_visual() {
    let num = |p: &Value, k: &str| p[k].as_f64().unwrap();
    let mut checked = 0;
    let mut errors = Vec::new();
    for (name, sp, data, w, h) in cases() {
        let Ok(s) = spec::parse(&sp) else { continue };
        let prims = spec::layout_json(&s, &spec::SpecData::from_json(&data), w, h);
        let prims = prims.as_array().unwrap();
        let axis_text = |p: &&Value, anchor: &str| {
            p["t"] == "text"
                && (p["fill"] == "#555555" || p["fill"] == "#5F5E5A")
                && (num(p, "size") == 10.0 || num(p, "size") == 11.0)
                && num(p, "rotate") == 0.0
                && p["anchor"] == anchor
        };
        // the legend: its swatches and names (11 px text starting at the swatch)
        let legend_bottom = prims
            .iter()
            .filter_map(|p| match p["t"].as_str() {
                Some("rect") if num(p, "w") == 10.0 && num(p, "h") == 10.0 => Some(num(p, "y") + 10.0),
                Some("text") if p["anchor"] == "start" && num(p, "size") >= 11.0 => {
                    Some(num(p, "y") + num(p, "size") / 2.0)
                }
                _ => None,
            })
            .fold(f64::NEG_INFINITY, f64::max);
        let tick_top = prims
            .iter()
            .filter(|p| axis_text(p, "end"))
            .map(|p| num(p, "y") - num(p, "size") / 2.0)
            .fold(f64::INFINITY, f64::min);
        if legend_bottom.is_finite() && tick_top.is_finite() {
            if legend_bottom > tick_top {
                errors.push(format!(
                    "{name}: the legend ends at {legend_bottom}, the top y tick label starts at {tick_top}"
                ));
            }
            checked += 1;
        }
        for p in prims.iter().filter(|p| axis_text(p, "middle")) {
            let half = p["text"].as_str().unwrap().chars().count() as f64 * num(p, "size") * 0.55 / 2.0;
            let x = num(p, "x");
            if x - half < -0.01 || x + half > w + 0.01 {
                errors.push(format!(
                    "{name}: x tick label {} spans {}..{} outside 0..{w}",
                    p["text"],
                    x - half,
                    x + half
                ));
            }
        }
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
    assert!(
        checked >= 2,
        "the combo and time_legend cases carry a legend and a y axis"
    );
}

/// The visual's TypeScript, compiled by BIcraft's bundler and run by Node, lays out every case
/// as the renderer does (and refuses the same specs with the same messages).
#[test]
fn the_typescript_layout_matches_the_rust_one() {
    let node = std::process::Command::new("node").arg("--version").output();
    if !node.is_ok_and(|o| o.status.success()) {
        eprintln!("note: node is not on the PATH; the TypeScript layout is not compared (development check)");
        return;
    }
    let dir = TempDir::new("spec-golden");
    let js = layout_script().unwrap();
    let harness = format!(
        "var M = {js}\n\
         var fs = require('fs');\n\
         var cases = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));\n\
         var out = {{}};\n\
         for (var i = 0; i < cases.length; i++) {{\n\
           var c = cases[i];\n\
           try {{ out[c.name] = M.layout(M.parse(c.spec), M.dataFromJson(c.data), c.w, c.h); }}\n\
           catch (e) {{ out[c.name] = {{error: String(e && e.message ? e.message : e)}}; }}\n\
         }}\n\
         fs.writeFileSync(process.argv[3], JSON.stringify(out));\n"
    );
    let script = dir.join("harness.js");
    std::fs::write(&script, harness).unwrap();
    let input: Vec<Value> = cases()
        .into_iter()
        .map(|(name, sp, data, w, h)| json!({"name": name, "spec": sp, "data": data, "w": w, "h": h}))
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
    let ts: Value = serde_json::from_str(&std::fs::read_to_string(&out_path).unwrap()).unwrap();
    let rust = rust_results();
    let mut errors = Vec::new();
    for (name, want) in &rust {
        if let Err(e) = same(&ts[name], want, name) {
            errors.push(format!(
                "{e}\n  ts:   {}\n  rust: {}",
                clip(&ts[name]),
                clip(want)
            ));
        }
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

fn clip(v: &Value) -> String {
    v.to_string().chars().take(600).collect()
}

// ---------------------------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------------------------

fn session() -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Shop"}));
    run(
        &mut s,
        "table.import",
        json!({"name": "T", "m": "let Source = #table(type table [K = text, V = Int64.Type], {{\"a\", 100}, {\"b\", 200}, {\"c\", 442}}) in Source"}),
    );
    run(
        &mut s,
        "measure.add",
        json!({"table": "T", "name": "Total", "expression": "SUM(T[V])"}),
    );
    run(&mut s, "report.new", json!({}));
    s
}

fn page(s: &Session) -> String {
    s.report.as_ref().unwrap().pages[0].name.clone()
}

fn add_chart(s: &mut Session) -> String {
    let p = page(s);
    run(
        s,
        "visual.add",
        json!({"page": p, "visual_type": "clusteredColumnChart", "x": 10, "y": 10, "width": 400, "height": 260,
               "fields": [{"role": "Category", "field": "T[K]"}, {"role": "Y", "field": "[Total]"}]}),
    )["visual"]
        .as_str()
        .unwrap()
        .to_string()
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

fn render_status(s: &mut Session, name: &str) -> (String, String) {
    let p = page(s);
    let r = run(s, "report.render", json!({"page": p, "system_fonts": false}));
    let v = r["pages"][0]["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == name)
        .cloned()
        .unwrap_or_else(|| panic!("{name} not rendered: {r}"));
    (
        v["status"].as_str().unwrap_or("").to_string(),
        v["note"].as_str().unwrap_or("").to_string(),
    )
}

#[test]
fn set_spec_embeds_the_visual_and_the_renderer_draws_it() {
    let mut s = session();
    let v = add_chart(&mut s);
    let p = page(&s);
    let sp = run(&mut s, "visual.spec_template", json!({"kind": "combo"}))["spec"].clone();
    let out = run(
        &mut s,
        "visual.set_spec",
        json!({"page": p, "visual": v, "spec": sp}),
    );
    assert_eq!(out["installed"], true, "{out}");
    assert_eq!(out["converted_from"], "clusteredColumnChart");
    assert_eq!(out["visual_type"], SPEC_GUID);
    assert_eq!(
        out["fields"],
        json!([{"role": "category", "field": "T[K]"}, {"role": "values", "field": "[Total]"}])
    );
    // the combo reads value.1: one measure is bound
    assert!(out["warnings"][0].as_str().unwrap().contains("value.1"), "{out}");
    assert!(s.report_dirty);
    let listed = run(&mut s, "report.custom_visuals", json!({}));
    let e = &listed["embedded"][0];
    assert_eq!(e["name"], SPEC_GUID, "{listed}");
    assert_eq!(e["registered"], true, "{listed}");
    assert_eq!(e["issues"], json!([]), "{listed}");
    // the snapshot carries the spec, the renderer draws it (not a placeholder)
    let sv = snap_visual(&s, &v);
    assert_eq!(sv["spec"], sp, "{sv}");
    assert_eq!(sv["custom_visual"]["name"], SPEC_GUID);
    assert_eq!(sv["data"]["status"], "ok", "{}", sv["data"]);
    let (status, note) = render_status(&mut s, &v);
    assert_eq!(status, "drawn", "{note}");
    // a second spec: the package is already there, the spec replaced
    let line = run(&mut s, "visual.spec_template", json!({"kind": "line"}))["spec"].clone();
    let text = line.to_string();
    let again = run(
        &mut s,
        "visual.set_spec",
        json!({"page": p, "visual": v, "spec": text}),
    );
    assert_eq!(again["installed"], false);
    assert_eq!(again["converted_from"], Value::Null);
    assert_eq!(again["warnings"], json!([]));
    assert_eq!(snap_visual(&s, &v)["spec"], line);
    // undo takes the spec back, redo brings it again
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(snap_visual(&s, &v)["spec"], sp);
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(snap_visual(&s, &v)["spec"], line);
    // PDF export draws the same
    let dir = TempDir::new("spec-visual");
    let pdf = dir.join("r.pdf");
    let r = run(
        &mut s,
        "report.export_pdf",
        json!({"out": pdf, "system_fonts": false}),
    );
    assert_eq!(r["pages"][0]["visuals"][0]["status"], "drawn", "{r}");
    assert!(std::fs::metadata(&pdf).unwrap().len() > 1000);
    // saved and reopened: the package, the type and the spec
    for file in ["Shop.pbip", "Shop.pbix"] {
        run(&mut s, "file.save_as", json!({"path": dir.join(file)}));
        let mut back = Session::new();
        run(&mut back, "file.open", json!({"path": dir.join(file)}));
        assert_eq!(snap_visual(&back, &v)["spec"], line, "{file}");
        let l = run(&mut back, "report.custom_visuals", json!({}));
        assert_eq!(l["embedded"][0]["registered"], true, "{file}: {l}");
        assert_eq!(l["embedded"][0]["used_by"][0]["visual"], v.as_str(), "{file}");
        let again = run(
            &mut back,
            "visual.set_spec",
            json!({"page": p, "visual": v, "spec": line}),
        );
        assert_eq!(
            again["installed"], false,
            "{file}: the package survives byte for byte"
        );
    }
}

#[test]
fn a_spec_is_checked_before_anything_changes() {
    let mut s = session();
    let v = add_chart(&mut s);
    let p = page(&s);
    let bad = [
        (json!({"page": p, "visual": v}), "missing parameter `spec`"),
        (json!({"page": p, "visual": v, "spec": 3}), "JSON object"),
        (json!({"page": p, "visual": v, "spec": "{"}), "not valid JSON"),
        (
            json!({"page": p, "visual": v, "spec": {"marks": [{"type": "pie"}]}}),
            "marks[0].type",
        ),
        (
            json!({"page": p, "visual": v, "spec": {"marks": [{"type": "bar"}], "extra": 1}}),
            "unknown key `extra`",
        ),
        (
            json!({"page": p, "visual": v, "spec": "x".repeat(40_000)}),
            "bytes",
        ),
        (
            json!({"page": p, "visual": "nope", "spec": {"marks": [{"type": "bar"}]}}),
            "no visual named",
        ),
        (
            json!({"page": "nope", "visual": v, "spec": {"marks": [{"type": "bar"}]}}),
            "nope",
        ),
        (
            json!({"page": p, "visual": v, "spec": {"marks": [{"type": "bar"}]}, "x": 1}),
            "x",
        ),
    ];
    for (params, want) in bad {
        let e = fails(&mut s, "visual.set_spec", params.clone());
        assert!(e.contains(want), "{params}: {e}");
    }
    assert!(
        !s.report
            .as_ref()
            .unwrap()
            .resources
            .iter()
            .any(|r| r.path.contains(SPEC_GUID)),
        "nothing was embedded"
    );
    let e = s
        .execute("visual.spec_template", &json!({"kind": "pie"}))
        .unwrap_err();
    assert!(e.contains("scatter"), "{e}");
    for k in spec::TEMPLATES {
        let t = run(&mut s, "visual.spec_template", json!({"kind": k}));
        spec::parse(&t["spec"]).unwrap_or_else(|e| panic!("{k}: {e}"));
    }
    // no report
    let mut empty = Session::new();
    run(&mut empty, "model.new", json!({"name": "M"}));
    assert!(
        empty
            .execute(
                "visual.set_spec",
                &json!({"page": "p", "visual": "v", "spec": {"marks": [{"type": "bar"}]}})
            )
            .is_err()
    );
}

/// `visual.scaffold {template: "spec"}` writes the Spec visual's sources as a project that
/// builds and packs like any other.
#[test]
fn the_spec_template_scaffolds_a_project_that_packs() {
    let mut s = Session::new();
    let dir = TempDir::new("spec-scaffold");
    let proj = dir.join("myspec");
    let out = run(
        &mut s,
        "visual.scaffold",
        json!({"folder": proj, "name": "mySpec", "template": "spec"}),
    );
    assert_eq!(out["template"], "spec");
    assert!(
        out["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f == "src/spec.ts"),
        "{out}"
    );
    let caps: Value =
        serde_json::from_str(&std::fs::read_to_string(proj.join("capabilities.json")).unwrap()).unwrap();
    assert_eq!(
        caps["objects"]["spec"]["properties"]["json"]["type"],
        json!({"text": true})
    );
    // its names in English and French (stringResources/<locale>/resources.resjson), which the
    // package carries
    assert_eq!(caps["dataRoles"][0]["displayNameKey"], "Role_category");
    let fr: Value = serde_json::from_str(
        &std::fs::read_to_string(proj.join("stringResources/fr-FR/resources.resjson")).unwrap(),
    )
    .unwrap();
    assert_eq!(fr["Role_category"], "Catégorie");
    assert_eq!(fr["Object_spec_json"], "Spécification (JSON)");
    let packed = run(&mut s, "visual.pack", json!({"folder": proj}));
    let read =
        bicraft_pbix::pbiviz::read_pbiviz(&std::fs::read(packed["out"].as_str().unwrap()).unwrap()).unwrap();
    let body = read
        .files
        .iter()
        .find(|(n, _)| n.ends_with(".pbiviz.json"))
        .map(|(_, b)| serde_json::from_slice::<Value>(b).unwrap())
        .unwrap();
    assert_eq!(body["stringResources"]["fr-FR"], fr);
    assert_eq!(body["stringResources"]["en-US"]["Role_values"], "Values");
    assert_eq!(
        packed["modules"],
        json!([
            "src/runtime/contrast.ts",
            "src/runtime/data.ts",
            "src/runtime/format.ts",
            "src/runtime/formatting.ts",
            "src/runtime/interactions.ts",
            "src/runtime/keys.ts",
            "src/runtime/landing.ts",
            "src/runtime/paint.ts",
            "src/runtime/selection.ts",
            "src/runtime/strings.ts",
            "src/runtime/style.ts",
            "src/runtime/textsettings.ts",
            "src/runtime/tooltips.ts",
            "src/spec.ts",
            "src/visual.ts"
        ]),
        "{packed}"
    );
    let e = s
        .execute(
            "visual.scaffold",
            &json!({"folder": dir.join("x"), "name": "x", "template": "spec", "data_roles": ["a:Grouping"]}),
        )
        .unwrap_err();
    assert!(e.contains("own data roles"), "{e}");
    let e = s
        .execute(
            "visual.scaffold",
            &json!({"folder": dir.join("y"), "name": "y", "template": "pie"}),
        )
        .unwrap_err();
    assert!(e.contains("template"), "{e}");
}
