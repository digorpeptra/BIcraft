//! BIcraft's own chart visuals (docs/formats/bicraft-visuals.md): the golden comparison of the
//! two layouts of each, the renderer's Rust (`bicraft_render::bvis`) against `bvis_golden.json`
//! and the visual's TypeScript (compiled by BIcraft's own bundler) against the Rust, run with
//! Node when Node is on the PATH (a development check; it prints a note and passes without
//! Node); the packages' capabilities against the API schema; and `visual.set_bicraft_visual`:
//! the package embedded, fields converted, options checked, the renderer and PDF export drawing
//! the visual, PBIP and PBIX round trips. `BICRAFT_BLESS=1` rewrites the golden file.

use std::path::{Path, PathBuf};

use bicraft_engine::Session;
use bicraft_engine::bvis::{DEFS, def, layout_script, package_bytes};
use bicraft_render::bvis;
use serde_json::{Value, json};

use crate::tempdir::TempDir;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

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

fn col(role: &str, name: &str, values: Value) -> Value {
    json!({"role": role, "name": name, "values": values})
}

/// A value column with its `highlights` (another visual's selection highlighting the visual,
/// docs/formats/bicraft-visuals.md "Highlights").
fn hl_col(role: &str, name: &str, values: Value, highlights: Value) -> Value {
    json!({"role": role, "name": name, "values": values, "highlights": highlights})
}

/// The cases: a kind, data, options and a size.
pub(crate) fn cases() -> Vec<(&'static str, &'static str, Value, Value, f64, f64)> {
    let amounts: Vec<f64> = (0..30).map(|i| f64::from((i * 37) % 97) + 0.5).collect();
    let weights: Vec<Value> = (0..30)
        .map(|i| if i % 7 == 3 { json!(null) } else { json!(1 + i % 4) })
        .collect();
    let hist = json!({"columns": [col("values", "Amount", json!(amounts)), col("frequency", "Rows", json!(weights))]});
    let distinct = json!({"columns": [col("values", "Score", json!([1, 2, 2.5, 3, 4, 7, 8, 8.5, 9, 10]))]});
    let bullets = json!({"columns": [
        col("category", "Region", json!(["North", "South", "East and West"])),
        col("value", "Sales", json!([270, 120, 310])),
        col("target", "Target", json!([250, 200, null]))
    ]});
    let banded = json!({"columns": [
        col("value", "Score", json!([72.5])),
        col("bands", "Poor", json!([40])),
        col("bands", "Good", json!([80])),
        col("target", "Goal", json!([75])),
        col("maximum", "Max", json!([100]))
    ]});
    let negative = json!({"columns": [
        col("category", "K", json!(["a", "b"])),
        col("value", "Margin", json!([-30, 45])),
        col("target", "Goal", json!([10, 40]))
    ]});
    let mut cats = Vec::new();
    let mut items = Vec::new();
    let mut vals = Vec::new();
    for (g, n) in [("Alpha", 9), ("Beta", 6), ("Gamma", 1)] {
        for i in 0..n {
            cats.push(json!(g));
            items.push(json!(format!("{g}{i}")));
            vals.push(json!(f64::from(i * i) + if g == "Beta" { 10.0 } else { 0.0 }));
        }
    }
    vals[8] = json!(250);
    let boxes = json!({"columns": [col("category", "Group", json!(cats)), col("sample", "Item", json!(items)), col("value", "Amount", json!(vals))]});
    let one_box = json!({"columns": [col("sample", "Item", json!(["a", "b", "c", "d", "e"])), col("value", "Amount", json!([4, 1, 3, 2, 5.5]))]});
    let pairs = json!({"columns": [
        col("category", "Country", json!(["France", "Germany", "Italy", "Spain"])),
        col("start", "2015", json!([62.5, 70, 55, 48])),
        col("end", "2025", json!([68, 66, null, 59.25]))
    ]});
    // close values on both sides: the slope chart's labels must move apart
    let crowded = json!({"columns": [
        col("category", "Team", json!(["Ann", "Bob", "Cy", "Dee", "Eve", "Fay"])),
        col("start", "Q1", json!([50, 50.5, 51, 51.2, 70, 20])),
        col("end", "Q4", json!([30, 30.2, 31, 60, 60.1, 10]))
    ]});
    // thirteen rows 11.4 px apart: the close start values' labels must move apart, the ends'
    // labels (far apart along the scale) stay on their rows (G110, 2026-11-24)
    let dense = json!({"columns": [
        col("category", "Team", json!((0..13).map(|i| format!("T{i}")).collect::<Vec<_>>())),
        col("start", "Q1", json!((0..13).map(|i| 40.0 + f64::from(i % 2) * 0.5).collect::<Vec<_>>())),
        col("end", "Q4", json!((0..13).map(|i| 60.0 + f64::from(i) * 5.0).collect::<Vec<_>>()))
    ]});
    // twenty rows of close values in a short visual: more labels than the plot can hold, so the
    // ones that cannot fit are left out instead of pushed above the plot (G110, 2026-11-25)
    let overfull = json!({"columns": [
        col("category", "Team", json!((0..20).map(|i| format!("T{i}")).collect::<Vec<_>>())),
        col("start", "Q1", json!((0..20).map(|i| 40.0 + f64::from(i % 3) * 0.5).collect::<Vec<_>>())),
        col("end", "Q4", json!((0..20).map(|i| 60.0 + f64::from(i % 2) * 0.4).collect::<Vec<_>>()))
    ]});
    // four regions' revenue last year and this year, in millions
    let wide = json!({"columns": [
        col("category", "Region", json!(["North", "South", "East", "West"])),
        col("start", "Revenue PY", json!([1_985_400, 2_120_300, 1_600_000, 1_802_000])),
        col("end", "Revenue", json!([2_400_000, 2_301_900, 1_712_400, 2_050_100]))
    ]});
    // date categories written with their column's format string (G110, 2026-11-24)
    let dated = |format: &str, locale: &str| {
        json!({"locale": locale, "columns": [
            {"role": "category", "name": "Month", "format": format, "values": ["2024-01-31", "2024-02-29", "2024-03-05 14:07"]},
            col("value", "Sales", json!([270, 120, 310])),
            col("target", "Target", json!([250, 200, 300]))
        ]})
    };
    let dated_pairs = json!({"locale": "en-US", "columns": [
        {"role": "category", "name": "Day", "format": "Long Date", "values": ["2024-11-25", "2024-12-30"]},
        col("start", "Start", json!([10, 20])),
        col("end", "End", json!([15, 12]))
    ]});
    // two dates of one month: two boxes, both labelled with the month
    let dated_boxes = json!({"locale": "fr-FR", "columns": [
        {"role": "category", "name": "Day", "format": "mmmm yyyy", "values": ["2024-01-02", "2024-01-02", "2024-01-20", "2024-01-20", "2024-02-01"]},
        col("value", "Amount", json!([1, 3, 2, 8, 5]))
    ]});
    // highlighted by another visual's selection: the bullet's parts, the others' units dimmed
    let hl_bullets = json!({"columns": [
        col("category", "Region", json!(["North", "South", "East and West"])),
        hl_col("value", "Sales", json!([270, 120, 310]), json!([135, null, 400])),
        hl_col("target", "Target", json!([250, 200, null]), json!([250, null, null]))
    ]});
    let hl_negative = json!({"columns": [
        col("category", "K", json!(["a", "b"])),
        hl_col("value", "Margin", json!([-30, 45]), json!([-10, 0])),
        col("target", "Goal", json!([10, 40]))
    ]});
    let hl_weights: Vec<Value> = weights
        .iter()
        .enumerate()
        .map(|(i, w)| if i % 4 == 0 { w.clone() } else { json!(null) })
        .collect();
    let hl_hist = json!({"columns": [col("values", "Amount", json!(amounts)), hl_col("frequency", "Rows", json!(weights), json!(hl_weights))]});
    let beta: Vec<Value> = cats
        .iter()
        .zip(&vals)
        .map(|(c, v)| if c == "Beta" { v.clone() } else { json!(null) })
        .collect();
    let hl_boxes = json!({"columns": [col("category", "Group", json!(cats)), col("sample", "Item", json!(items)), hl_col("value", "Amount", json!(vals), json!(beta))]});
    let hl_pairs = json!({"columns": [
        col("category", "Country", json!(["France", "Germany", "Italy", "Spain"])),
        hl_col("start", "2015", json!([62.5, 70, 55, 48]), json!([30, null, null, 48])),
        hl_col("end", "2025", json!([68, 66, null, 59.25]), json!([null, null, null, 20]))
    ]});
    vec![
        (
            "hl_bullet",
            "bullet",
            hl_bullets.clone(),
            json!({"labels": {"show": true}}),
            400.0,
            200.0,
        ),
        (
            "hl_bullet_vertical",
            "bullet",
            hl_bullets,
            json!({"style": {"vertical": true, "preset": "bold"}}),
            300.0,
            240.0,
        ),
        (
            "hl_bullet_negative",
            "bullet",
            hl_negative,
            json!({}),
            300.0,
            120.0,
        ),
        (
            "hl_hist",
            "histogram",
            hl_hist,
            json!({"bins": {"count": 8}}),
            360.0,
            220.0,
        ),
        ("hl_box", "box_plot", hl_boxes, json!({}), 360.0, 240.0),
        (
            "hl_dumbbell",
            "dumbbell",
            hl_pairs.clone(),
            json!({}),
            400.0,
            200.0,
        ),
        (
            "hl_slope",
            "dumbbell",
            hl_pairs,
            json!({"style": {"slope": true}}),
            360.0,
            240.0,
        ),
        // BIcraft's look (docs/formats/visual-look.md): presets, switches and the theme
        (
            "look_hist_bold",
            "histogram",
            hist.clone(),
            json!({"bins": {"count": 8}, "labels": {"show": true}, "style": {"preset": "bold"}}),
            360.0,
            220.0,
        ),
        (
            "look_bullet_theme",
            "bullet",
            bullets.clone(),
            json!({"$theme": {"colors": ["#3B5BA9", "#C0504D"]}}),
            360.0,
            180.0,
        ),
        (
            "look_bullet_minimal_rounded",
            "bullet",
            banded.clone(),
            json!({"style": {"preset": "minimal", "rounded": true, "gradient": true}}),
            300.0,
            100.0,
        ),
        (
            "look_box_flat",
            "box_plot",
            boxes.clone(),
            json!({"style": {"rounded": false, "gradient": false}}),
            360.0,
            220.0,
        ),
        (
            "look_dumbbell_bicraft",
            "dumbbell",
            pairs.clone(),
            json!({"style": {"palette": "bicraft"}, "$theme": {"colors": ["#111111", "#222222"]}}),
            360.0,
            200.0,
        ),
        (
            "look_slope_bold",
            "dumbbell",
            pairs.clone(),
            json!({"style": {"slope": true, "preset": "bold"}}),
            300.0,
            220.0,
        ),
        // the look's second pass: a dark report background, value chips, no card
        (
            "look2_hist_dark",
            "histogram",
            hist.clone(),
            json!({"bins": {"count": 8}, "labels": {"show": true}, "$theme": {"colors": [], "background": "#1F1F1F"}}),
            360.0,
            220.0,
        ),
        (
            "look2_bullet_pills",
            "bullet",
            bullets.clone(),
            json!({"style": {"pills": true}}),
            360.0,
            180.0,
        ),
        (
            "look2_dumbbell_bold_no_card",
            "dumbbell",
            pairs.clone(),
            json!({"style": {"preset": "bold", "card": false}}),
            360.0,
            200.0,
        ),
        (
            "bullet_dates_fr",
            "bullet",
            dated("dd/mm/yyyy", "fr-FR"),
            json!({}),
            360.0,
            200.0,
        ),
        (
            "bullet_dates_months",
            "bullet",
            dated("mmm yy", "en-US"),
            json!({"style": {"vertical": true}}),
            300.0,
            220.0,
        ),
        (
            "bullet_dates_time",
            "bullet",
            dated("General Date", "fr-FR"),
            json!({}),
            420.0,
            200.0,
        ),
        ("dumbbell_dates", "dumbbell", dated_pairs, json!({}), 420.0, 160.0),
        ("box_dates", "box_plot", dated_boxes, json!({}), 360.0, 220.0),
        (
            "hist_basic",
            "histogram",
            hist.clone(),
            json!({"bins": {"count": 8}}),
            360.0,
            220.0,
        ),
        (
            "hist_size",
            "histogram",
            hist.clone(),
            json!({"bins": {"size": 25}, "labels": {"show": true}, "colors": {"fill": "#aa3300"}}),
            300.0,
            200.0,
        ),
        (
            "hist_distinct",
            "histogram",
            distinct.clone(),
            json!({}),
            320.0,
            180.0,
        ),
        (
            "hist_single",
            "histogram",
            json!({"columns": [col("values", "X", json!([5]))]}),
            json!({}),
            200.0,
            150.0,
        ),
        ("hist_too_small", "histogram", distinct, json!({}), 40.0, 30.0),
        (
            "hist_cumulative",
            "histogram",
            hist.clone(),
            json!({"bins": {"count": 6, "cumulative": true}, "labels": {"show": true}}),
            320.0,
            200.0,
        ),
        (
            "bullet_rows",
            "bullet",
            bullets.clone(),
            json!({"labels": {"show": true}}),
            400.0,
            200.0,
        ),
        (
            "bullet_bands",
            "bullet",
            banded,
            json!({"colors": {"value": "#000000", "target": "#C23B5A"}}),
            300.0,
            80.0,
        ),
        (
            "bullet_negative",
            "bullet",
            negative,
            json!({"bands": {"first": 50, "second": 0, "third": 120}}),
            300.0,
            120.0,
        ),
        ("bullet_narrow", "bullet", bullets.clone(), json!({}), 120.0, 90.0),
        (
            "bullet_vertical",
            "bullet",
            bullets,
            json!({"style": {"vertical": true}, "labels": {"show": true}}),
            300.0,
            240.0,
        ),
        (
            "bullet_vertical_negative",
            "bullet",
            json!({"columns": [
                col("category", "K", json!(["a", "b"])),
                col("value", "Margin", json!([-30, 45])),
                col("target", "Goal", json!([10, 40]))
            ]}),
            json!({"style": {"vertical": true}, "labels": {"show": true}}),
            160.0,
            220.0,
        ),
        ("box_groups", "box_plot", boxes.clone(), json!({}), 360.0, 240.0),
        (
            "box_horizontal",
            "box_plot",
            boxes.clone(),
            json!({"box": {"horizontal": true}}),
            360.0,
            220.0,
        ),
        (
            "box_options",
            "box_plot",
            boxes,
            json!({"box": {"mean": false, "outliers": false}, "colors": {"fill": "#5A9E3A"}}),
            300.0,
            200.0,
        ),
        ("box_one", "box_plot", one_box.clone(), json!({}), 160.0, 200.0),
        (
            "box_horizontal_one",
            "box_plot",
            one_box,
            json!({"box": {"horizontal": true, "mean": false}}),
            240.0,
            100.0,
        ),
        ("dumbbell", "dumbbell", pairs.clone(), json!({}), 400.0, 200.0),
        (
            "dumbbell_plain",
            "dumbbell",
            pairs.clone(),
            json!({"labels": {"show": false}, "colors": {"line": "#111111"}}),
            300.0,
            160.0,
        ),
        (
            "slope",
            "dumbbell",
            pairs.clone(),
            json!({"style": {"slope": true}}),
            360.0,
            240.0,
        ),
        (
            "slope_crowded",
            "dumbbell",
            crowded,
            json!({"style": {"slope": true}}),
            300.0,
            200.0,
        ),
        (
            "slope_plain",
            "dumbbell",
            pairs,
            json!({"style": {"slope": true}, "labels": {"show": false}}),
            200.0,
            160.0,
        ),
        (
            "dumbbell_crowded",
            "dumbbell",
            dense.clone(),
            json!({}),
            360.0,
            200.0,
        ),
        (
            "dumbbell_crowded_pills",
            "dumbbell",
            dense,
            json!({"style": {"pills": true, "preset": "minimal"}}),
            360.0,
            260.0,
        ),
        (
            "dumbbell_overfull",
            "dumbbell",
            overfull.clone(),
            json!({}),
            360.0,
            180.0,
        ),
        (
            "slope_overfull",
            "dumbbell",
            overfull,
            json!({"style": {"slope": true}}),
            300.0,
            150.0,
        ),
        // revenue in millions, the larger value at the right end of the scale: its label is
        // wider than a fixed margin and must stay inside the visual (README dashboard, 2026-11-26)
        (
            "dumbbell_wide_values",
            "dumbbell",
            wide.clone(),
            json!({}),
            400.0,
            216.0,
        ),
        (
            "dumbbell_wide_values_pills",
            "dumbbell",
            wide,
            json!({"style": {"pills": true, "preset": "minimal"}}),
            300.0,
            160.0,
        ),
        ("dumbbell_empty", "dumbbell", json!({}), json!({}), 300.0, 160.0),
    ]
}

fn golden_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/report/bvis_golden.json")
}

fn rust_results() -> serde_json::Map<String, Value> {
    let mut out = serde_json::Map::new();
    for (name, kind, data, opts, w, h) in cases() {
        out.insert(
            name.into(),
            bvis::layout_json(kind, &bvis::Data::from_json(&data), &opts, w, h),
        );
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
        let mut text = serde_json::to_string_pretty(&Value::Object(got.clone())).unwrap();
        text.push('\n');
        std::fs::write(&path, text).unwrap();
    }
    let want: Value = serde_json::from_str(
        &std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{}: {e} (BICRAFT_BLESS=1 writes it)", path.display())),
    )
    .unwrap();
    same(&Value::Object(got.clone()), &want, "golden").unwrap();
    for (name, v) in got {
        let a = v.as_array().unwrap_or_else(|| panic!("{name}: {v}"));
        let empty = name.ends_with("too_small") || name.ends_with("empty");
        assert_eq!(a.is_empty(), empty, "{name}");
        // every primitive lies within the visual (to a pixel), every number is finite
        let (_, _, _, _, w, h) = cases().into_iter().find(|c| c.0 == name).unwrap();
        for p in a {
            for (k, lim) in [
                ("x", w),
                ("x1", w),
                ("x2", w),
                ("cx", w),
                ("y", h),
                ("y1", h),
                ("y2", h),
                ("cy", h),
            ] {
                if let Some(x) = p.get(k).and_then(Value::as_f64) {
                    assert!(
                        (-1.0..=lim + 1.0).contains(&x),
                        "{name}: {k} = {x} outside 0..{lim}: {p}"
                    );
                }
            }
        }
    }
}

/// What the cases must show, read off the primitives.
#[test]
fn the_layouts_draw_what_the_data_says() {
    let r = rust_results();
    let count = |name: &str, t: &str, f: &dyn Fn(&Value) -> bool| {
        r[name]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["t"] == t && f(p))
            .count()
    };
    // 8 bins of the 30 weighted amounts: one bar per non-empty bin
    assert!(count("hist_basic", "rect", &|p| p["role"] != "card") <= 8);
    assert_eq!(
        count("hist_size", "rect", &|p| p["fill"] == "#AA3300"),
        4,
        "bins of 25 from 0 to 100"
    );
    assert_eq!(count("hist_single", "rect", &|p| p["role"] != "card"), 1);
    // the bullets' labels, bands and targets
    assert_eq!(count("bullet_rows", "text", &|p| p["text"] == "East and West"), 1);
    assert_eq!(
        count("bullet_rows", "line", &|p| p["width"] == 3.0),
        2,
        "two targets (one blank)"
    );
    // two band fields and the maximum: three bands, the darkest of the last three shades first
    assert_eq!(
        count("bullet_bands", "rect", &|p| ["#E2E1DC", "#ECEBE7", "#F5F4F1"]
            .contains(&p["fill"].as_str().unwrap())),
        3
    );
    // boxes: Alpha's 250 is an outlier, Gamma has one value
    assert_eq!(count("box_groups", "circle", &|p| p["opacity"] == 0.75), 1);
    assert_eq!(count("box_options", "circle", &|_| true), 0);
    assert_eq!(count("box_groups", "text", &|p| p["text"] == "Gamma"), 1);
    // dumbbell: three joined rows (Italy has no end), the legend names the two fields
    assert_eq!(count("dumbbell", "path", &|_| true), 3);
    assert_eq!(
        count("dumbbell", "text", &|p| p["text"] == "2015"
            || p["text"] == "2025"),
        2
    );
    assert_eq!(count("slope", "text", &|p| p["text"] == "France 62.50"), 1);
    assert_eq!(count("slope_plain", "text", &|p| p["size"] == 11.0), 0);
    // cumulative: the last bar's label is every weight, each bar at least the one before
    let total: f64 = (0..30).filter(|i| i % 7 != 3).map(|i| f64::from(1 + i % 4)).sum();
    assert_eq!(
        count("hist_cumulative", "text", &|p| p["text"]
            == bicraft_render::spec::fmt_auto(total)),
        1
    );
    let tops: Vec<f64> = r["hist_cumulative"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["t"] == "rect" && p["role"] != "card")
        .map(|p| p["y"].as_f64().unwrap())
        .collect();
    assert!(tops.windows(2).all(|w| w[1] <= w[0] + 0.01), "{tops:?}");
    // vertical bullets: the names below, the value bars standing; horizontal boxes: names on the left
    assert_eq!(
        count("bullet_vertical", "text", &|p| p["text"] == "North"
            && p["anchor"] == "middle"),
        1
    );
    assert_eq!(
        count("box_horizontal", "text", &|p| p["text"] == "Gamma"
            && p["anchor"] == "end"),
        1
    );
    assert_eq!(count("box_horizontal", "circle", &|p| p["opacity"] == 0.75), 1);
    // the slope chart's labels (Modern's 11 px tick text) on each side are at least 12 px apart
    for anchor in ["end", "start"] {
        let mut ys: Vec<f64> = r["slope_crowded"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["t"] == "text" && p["size"] == 11.0 && p["anchor"] == anchor)
            .map(|p| p["y"].as_f64().unwrap())
            .collect();
        ys.sort_by(f64::total_cmp);
        assert_eq!(ys.len(), 6, "{anchor}");
        assert!(ys.windows(2).all(|w| w[1] - w[0] >= 11.99), "{anchor}: {ys:?}");
    }
    // a label moved away from its dot has a thin leader line; one in place has none
    let leaders = |name: &str| count(name, "line", &|p| p["width"] == 0.75);
    assert!(leaders("slope_crowded") >= 4, "{}", leaders("slope_crowded"));
    assert_eq!(leaders("slope"), 0);
    assert_eq!(leaders("dumbbell"), 0);
    // the crowded dumbbell: the start labels (anchored at their end, Modern's 11 px value text)
    // at least 12 px apart, the end labels kept on their rows where they clear each other, one
    // leader per moved label
    let pick = |anchor: &str| -> Vec<f64> {
        let mut ys: Vec<f64> = r["dumbbell_crowded"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["t"] == "text" && p["size"] == 11.0 && p["anchor"] == anchor)
            .map(|p| p["y"].as_f64().unwrap())
            .collect();
        ys.sort_by(f64::total_cmp);
        ys
    };
    let (starts, ends) = (pick("end"), pick("start"));
    // the category names are anchored at their end too, at the rows' heights (12 px names)
    assert_eq!(ends.len(), 13);
    assert!(starts.windows(2).all(|w| w[1] - w[0] >= 11.99), "{starts:?}");
    // the end labels 60 … 95 are narrow enough to clear each other and stay on their rows; from
    // 100 on, three digits wide, each meets the next and they move 12 px apart
    let rh = (200.0 - 16.0 - 36.0) / 13.0;
    for (i, y) in ends.iter().take(8).enumerate() {
        assert!((y - (26.0 + rh * (i as f64 + 0.5))).abs() < 0.011, "{ends:?}");
    }
    assert!(ends[8..].windows(2).all(|w| w[1] - w[0] >= 11.99), "{ends:?}");
    assert!(
        leaders("dumbbell_crowded") >= 6,
        "{}",
        leaders("dumbbell_crowded")
    );
    assert!(leaders("dumbbell_crowded_pills") >= 6);
    // labels stay inside the plot (G110): a dumbbell's value labels never above the first row's
    // centre or half a text below the plot's top (26 px), whichever is higher; the legend's names
    // (17 px) and the axis below the plot aside. A slope chart's labels never above its axes' top
    // (32 px), its field names (15 px) aside. The ones that cannot fit are left out.
    let lowest = |name: &str, skip: f64, below: f64| {
        r[name]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["t"] == "text" && p["y"] != skip && p["y"].as_f64().unwrap() <= below)
            .map(|p| p["y"].as_f64().unwrap())
            .fold(f64::INFINITY, f64::min)
    };
    // (Modern's 11 px value text, Minimal's 10 px; the overfull case's rows 6.4 px high)
    for (name, h, top) in [
        ("dumbbell_crowded", 200.0, 26.0 + 5.5),
        ("dumbbell_crowded_pills", 260.0, 26.0 + 5.0),
        ("dumbbell_overfull", 180.0, 26.0 + 3.2),
    ] {
        let low = lowest(name, 17.0, h - 26.0);
        assert!(low >= top - 0.011, "{name}: a label at {low}");
    }
    let values = count("dumbbell_overfull", "text", &|p| p["weight"] == 600);
    assert!(
        (16..40).contains(&values),
        "{values} of 40 value labels in 128 px"
    );
    let low = lowest("slope_overfull", 15.0, 150.0);
    assert!(low >= 32.0 - 0.011, "slope_overfull: a label at {low}");
}

/// A dumbbell's texts stay inside the visual across its width: each one's estimated box (0.55
/// of its size per character, from its anchor, a pill 5 px wider) within 0..w and clear of the
/// other texts on its row, whatever the values' width.
#[test]
fn dumbbell_texts_stay_inside_the_visual() {
    let r = rust_results();
    let mut errors = Vec::new();
    for (name, kind, _, _, w, _) in cases() {
        if kind != "dumbbell" {
            continue;
        }
        // (text, y, from, to); a value label's pill reaches 5 px past its text
        let mut boxes: Vec<(String, f64, f64, f64)> = Vec::new();
        let prims = r[name].as_array().cloned().unwrap_or_default();
        for (i, p) in prims.iter().enumerate() {
            if p["t"] != "text" {
                continue;
            }
            let s = p["text"].as_str().unwrap_or("");
            let pill = i
                .checked_sub(1)
                .and_then(|j| prims.get(j))
                .is_some_and(|q| q["role"] == "pill");
            let pad = if pill { 5.0 } else { 0.0 };
            let wd = s.chars().count() as f64 * p["size"].as_f64().unwrap_or(0.0) * 0.55;
            let x = p["x"].as_f64().unwrap_or(f64::NAN);
            let (a, b) = match p["anchor"].as_str() {
                Some("start") => (x - pad, x + wd + pad),
                Some("end") => (x - wd - pad, x + pad),
                _ => (x - wd / 2.0 - pad, x + wd / 2.0 + pad),
            };
            if a < -0.5 || b > w + 0.5 {
                errors.push(format!("{name}: `{s}` spans {a:.1}..{b:.1}, outside 0..{w}"));
            }
            let y = p["y"].as_f64().unwrap_or(f64::NAN);
            for (t, ty, ta, tb) in &boxes {
                if (ty - y).abs() < 1.0 && a < tb - 0.5 && *ta < b - 0.5 {
                    errors.push(format!(
                        "{name}: `{s}` ({a:.1}..{b:.1}) overlaps `{t}` ({ta:.1}..{tb:.1})"
                    ));
                }
            }
            boxes.push((s.to_string(), y, a, b));
        }
    }
    // the wide values are all drawn: there is room for them
    let values = |name: &str| {
        r[name]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|p| {
                p["t"] == "text"
                    && p["anchor"] != "middle"
                    && p["text"].as_str().is_some_and(|s| s.contains(','))
            })
            .count()
    };
    for name in ["dumbbell_wide_values", "dumbbell_wide_values_pills"] {
        if values(name) != 8 {
            errors.push(format!("{name}: {} of 8 value labels drawn", values(name)));
        }
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

#[test]
fn the_typescript_layouts_match_the_rust_ones() {
    let node = std::process::Command::new("node").arg("--version").output();
    if !node.is_ok_and(|o| o.status.success()) {
        eprintln!(
            "note: node is not on the PATH; the TypeScript layouts are not compared (development check)"
        );
        return;
    }
    let dir = TempDir::new("bvis-golden");
    let mut harness = String::from("var fs = require('fs');\nvar L = {};\n");
    for d in DEFS {
        harness.push_str(&format!(
            "L[{:?}] = {};\n",
            d.kind,
            layout_script(d.kind).unwrap()
        ));
    }
    harness.push_str(
        "var cases = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));\n\
         var out = {}; var faded = {};\n\
         for (var i = 0; i < cases.length; i++) {\n\
           var c = cases[i]; var M = L[c.kind];\n\
           var o = {units: [], hits: []};\n\
           try { out[c.name] = M.layout(M.dataFromJson(c.data), c.opts, c.w, c.h, o);\n\
                 faded[c.name] = o.faded || [];\n\
                 var bad = o.hits.filter(function (h) { return h.prim >= out[c.name].length || h.unit >= o.units.length; });\n\
                 if (bad.length) { out[c.name] = {error: 'bad hits ' + JSON.stringify(bad)}; } }\n\
           catch (e) { out[c.name] = {error: String(e && e.stack ? e.stack : e)}; }\n\
         }\n\
         fs.writeFileSync(process.argv[3], JSON.stringify({out: out, faded: faded}));\n",
    );
    let script = dir.join("harness.js");
    std::fs::write(&script, harness).unwrap();
    let input: Vec<Value> = cases()
        .into_iter()
        .map(|(name, kind, data, opts, w, h)| json!({"name": name, "kind": kind, "data": data, "opts": opts, "w": w, "h": h}))
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
    let got: Value = serde_json::from_str(&std::fs::read_to_string(&out_path).unwrap()).unwrap();
    let ts = &got["out"];
    let mut errors = Vec::new();
    // the units dimmed under a highlight, the same in both
    for (name, kind, data, opts, w, h) in cases() {
        let (_, u) = bvis::layout_units(kind, &bvis::Data::from_json(&data), &opts, w, h).unwrap();
        if let Err(e) = same(&got["faded"][name], &json!(u.faded), name) {
            errors.push(format!("faded units: {e}"));
        }
    }
    for (name, want) in &rust_results() {
        if let Err(e) = same(&ts[name], want, name) {
            let clip = |v: &Value| v.to_string().chars().take(600).collect::<String>();
            errors.push(format!(
                "{e}\n  ts:   {}\n  rust: {}",
                clip(&ts[name]),
                clip(want)
            ));
        }
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

#[test]
fn the_packages_capabilities_follow_the_api_schema() {
    let schema_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/report/powerbi-visuals-api/schema.capabilities.json");
    let schema: Value = serde_json::from_str(&std::fs::read_to_string(schema_path).unwrap()).unwrap();
    for d in DEFS {
        let read = bicraft_pbix::pbiviz::read_pbiviz(&package_bytes(d.kind).unwrap()).unwrap();
        let name = format!("resources/{}.pbiviz.json", d.guid);
        let (_, body) = read.files.iter().find(|(n, _)| *n == name).unwrap();
        let v: Value = serde_json::from_slice(body).unwrap();
        let caps = &v["capabilities"];
        let issues = bicraft_pbir::JsonSchema::new(schema.clone()).validate(caps);
        assert!(issues.is_empty(), "{}: {issues:?}\n{caps}", d.kind);
        assert_eq!(caps["privileges"], json!([]), "{}", d.kind);
        assert_eq!(caps["supportsKeyboardFocus"], json!(true), "{}", d.kind);
        assert_eq!(v["visual"]["displayName"], d.display_name);
        // every option the engine accepts is declared, with its role names
        for p in bicraft_engine::bvis::props(d) {
            assert!(
                caps["objects"][p.object]["properties"][p.property].is_object(),
                "{}: {}.{}",
                d.kind,
                p.object,
                p.property
            );
        }
        let roles: Vec<&str> = caps["dataRoles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["name"].as_str().unwrap())
            .collect();
        assert_eq!(roles, d.roles.iter().map(|r| r.name).collect::<Vec<_>>());
        assert!(
            v["content"]["js"]
                .as_str()
                .unwrap()
                .starts_with(&format!("var {} =", d.guid))
        );
    }
}

// ---------------------------------------------------------------------------------------------
// The command
// ---------------------------------------------------------------------------------------------

fn session() -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Shop"}));
    run(
        &mut s,
        "table.import",
        json!({"name": "T", "m": "let Source = #table(type table [K = text, G = text, V = Int64.Type], {{\"a\", \"x\", 10}, {\"b\", \"x\", 20}, {\"c\", \"y\", 35}, {\"d\", \"y\", 40}, {\"e\", \"y\", 400}}) in Source"}),
    );
    for (name, e) in [
        ("Total", "SUM(T[V])"),
        ("Rows", "COUNTROWS(T)"),
        ("Target", "100"),
    ] {
        run(
            &mut s,
            "measure.add",
            json!({"table": "T", "name": name, "expression": e}),
        );
    }
    run(&mut s, "report.new", json!({}));
    s
}

fn page(s: &Session) -> String {
    s.report.as_ref().unwrap().pages[0].name.clone()
}

fn add(s: &mut Session, visual_type: &str, fields: Value, y: i64) -> String {
    let p = page(s);
    run(
        s,
        "visual.add",
        json!({"page": p, "visual_type": visual_type, "x": 10, "y": y, "width": 400, "height": 160, "fields": fields}),
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

#[test]
fn set_bicraft_visual_embeds_converts_and_the_renderer_draws_it() {
    let mut s = session();
    let p = page(&s);
    let hist = add(
        &mut s,
        "clusteredColumnChart",
        json!([{"role": "Category", "field": "T[V]"}, {"role": "Y", "field": "[Rows]"}]),
        10,
    );
    let bullet = add(
        &mut s,
        "clusteredColumnChart",
        json!([{"role": "Category", "field": "T[K]"}, {"role": "Y", "field": "[Total]"}, {"role": "Y", "field": "[Target]"}]),
        180,
    );
    let boxes = add(
        &mut s,
        "tableEx",
        json!([{"role": "Values", "field": "T[G]"}, {"role": "Values", "field": "T[K]"}, {"role": "Values", "field": "[Total]"}]),
        350,
    );
    let pairs = add(
        &mut s,
        "clusteredColumnChart",
        json!([{"role": "Category", "field": "T[K]"}, {"role": "Y", "field": "[Total]"}, {"role": "Y", "field": "[Target]"}, {"role": "Y", "field": "[Rows]"}]),
        520,
    );
    let out = run(
        &mut s,
        "visual.set_bicraft_visual",
        json!({"page": p, "visual": hist, "kind": "histogram", "options": {"bins": {"count": 4}, "colors": {"fill": "#aa3300"}}}),
    );
    assert_eq!(out["installed"], true, "{out}");
    assert_eq!(out["converted_from"], "clusteredColumnChart");
    assert_eq!(
        out["fields"],
        json!([{"role": "values", "field": "T[V]"}, {"role": "frequency", "field": "[Rows]"}])
    );
    assert_eq!(
        out["options"],
        json!({"bins": {"count": 4.0}, "colors": {"fill": "#AA3300"}})
    );
    assert_eq!(out["warnings"], json!([]));
    let out = run(
        &mut s,
        "visual.set_bicraft_visual",
        json!({"page": p, "visual": bullet, "kind": "bullet"}),
    );
    assert_eq!(
        out["fields"],
        json!([{"role": "category", "field": "T[K]"}, {"role": "value", "field": "[Total]"}, {"role": "target", "field": "[Target]"}])
    );
    let out = run(
        &mut s,
        "visual.set_bicraft_visual",
        json!({"page": p, "visual": boxes, "kind": "box_plot"}),
    );
    assert_eq!(
        out["fields"],
        json!([{"role": "category", "field": "T[G]"}, {"role": "sample", "field": "T[K]"}, {"role": "value", "field": "[Total]"}])
    );
    let out = run(
        &mut s,
        "visual.set_bicraft_visual",
        json!({"page": p, "visual": pairs, "kind": "dumbbell", "options": {"style": {"slope": true}}}),
    );
    assert_eq!(
        out["fields"],
        json!([{"role": "category", "field": "T[K]"}, {"role": "start", "field": "[Total]"}, {"role": "end", "field": "[Target]"}])
    );
    assert_eq!(out["dropped"], json!([{"role": "Y", "field": "[Rows]"}]));
    let listed = run(&mut s, "report.custom_visuals", json!({}));
    assert_eq!(listed["embedded"].as_array().unwrap().len(), 4, "{listed}");
    for e in listed["embedded"].as_array().unwrap() {
        assert_eq!(e["registered"], true, "{listed}");
        assert_eq!(e["issues"], json!([]), "{listed}");
    }
    // the snapshot carries the kind and options; the data is the model's
    for (v, kind) in [
        (&hist, "histogram"),
        (&bullet, "bullet"),
        (&boxes, "box_plot"),
        (&pairs, "dumbbell"),
    ] {
        let sv = snap_visual(&s, v);
        assert_eq!(sv["bicraft_visual"]["kind"], kind, "{sv}");
        assert_eq!(sv["data"]["status"], "ok", "{kind}: {}", sv["data"]);
    }
    assert_eq!(
        snap_visual(&s, &pairs)["bicraft_visual"]["options"],
        json!({"style": {"slope": true}})
    );
    let r = run(&mut s, "report.render", json!({"page": p, "system_fonts": false}));
    for v in r["pages"][0]["visuals"].as_array().unwrap() {
        assert_eq!(v["status"], "drawn", "{v}");
    }
    // the renderer lays the histogram out from the model's data: 5 values, 4 bins
    let sv = snap_visual(&s, &hist);
    assert_eq!(sv["data"]["rows"].as_array().unwrap().len(), 5, "{}", sv["data"]);
    // a second call: the package is there, an option removed
    let again = run(
        &mut s,
        "visual.set_bicraft_visual",
        json!({"page": p, "visual": hist, "kind": "histogram", "options": {"colors": {"fill": null}}}),
    );
    assert_eq!(again["installed"], false);
    assert_eq!(again["converted_from"], Value::Null);
    assert_eq!(again["options"], json!({"bins": {"count": 4.0}}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(
        snap_visual(&s, &hist)["bicraft_visual"]["options"]["colors"]["fill"],
        "#AA3300"
    );
    // PDF export draws the same
    let dir = TempDir::new("bvis");
    let pdf = dir.join("r.pdf");
    let r = run(
        &mut s,
        "report.export_pdf",
        json!({"out": pdf, "system_fonts": false}),
    );
    for v in r["pages"][0]["visuals"].as_array().unwrap() {
        assert_eq!(v["status"], "drawn", "{v}");
    }
    // saved and reopened: the packages, the types and the options
    for file in ["Shop.pbip", "Shop.pbix"] {
        run(&mut s, "file.save_as", json!({"path": dir.join(file)}));
        let mut back = Session::new();
        run(&mut back, "file.open", json!({"path": dir.join(file)}));
        assert_eq!(
            snap_visual(&back, &hist)["bicraft_visual"]["options"]["bins"]["count"],
            4.0,
            "{file}"
        );
        assert_eq!(
            snap_visual(&back, &boxes)["bicraft_visual"]["kind"],
            "box_plot",
            "{file}"
        );
        let again = run(
            &mut back,
            "visual.set_bicraft_visual",
            json!({"page": p, "visual": bullet, "kind": "bullet"}),
        );
        assert_eq!(
            again["installed"], false,
            "{file}: the package survives byte for byte"
        );
    }
}

#[test]
fn set_bicraft_visual_checks_everything_before_changing_the_report() {
    let mut s = session();
    let p = page(&s);
    let v = add(
        &mut s,
        "clusteredColumnChart",
        json!([{"role": "Category", "field": "T[K]"}, {"role": "Y", "field": "[Total]"}]),
        10,
    );
    let bad = [
        (json!({"page": p, "visual": v}), "kind"),
        (json!({"page": p, "visual": v, "kind": "pie"}), "histogram"),
        (
            json!({"page": p, "visual": v, "kind": "histogram", "options": 3}),
            "options",
        ),
        (
            json!({"page": p, "visual": v, "kind": "histogram", "options": {"bins": {"colour": 1}}}),
            "no such option",
        ),
        (
            json!({"page": p, "visual": v, "kind": "histogram", "options": {"bins": {"count": 0}}}),
            "from 1 to 200",
        ),
        (
            json!({"page": p, "visual": v, "kind": "histogram", "options": {"bins": {"count": 2.5}}}),
            "whole number",
        ),
        (
            json!({"page": p, "visual": v, "kind": "bullet", "options": {"colors": {"value": "blue"}}}),
            "#RRGGBB",
        ),
        (
            json!({"page": p, "visual": v, "kind": "dumbbell", "options": {"style": {"slope": "yes"}}}),
            "true or false",
        ),
        (
            json!({"page": p, "visual": "nope", "kind": "bullet"}),
            "no visual named",
        ),
        (json!({"page": p, "visual": v, "kind": "bullet", "x": 1}), "x"),
    ];
    for (params, want) in bad {
        let e = fails(&mut s, "visual.set_bicraft_visual", params.clone());
        assert!(e.contains(want), "{params}: {e}");
    }
    assert!(
        s.report.as_ref().unwrap().resources.is_empty(),
        "nothing was embedded"
    );
    // a bullet without a measure warns
    let col_only = add(
        &mut s,
        "clusteredColumnChart",
        json!([{"role": "Category", "field": "T[K]"}]),
        200,
    );
    let out = run(
        &mut s,
        "visual.set_bicraft_visual",
        json!({"page": p, "visual": col_only, "kind": "bullet"}),
    );
    assert!(out["warnings"][0].as_str().unwrap().contains("`value`"), "{out}");
    assert!(def("histogram").is_some() && def("pie").is_none());
}

/// Power BI Desktop 2.159.1231.0 (2026-11-24) gives one identity per row of the categories (category
/// and sample): a click on a box selects all of its rows, so the page filters to the box's
/// category (the first row's identity alone selected one sample there). The packaged box plot run
/// under Node with the chart pack's stand-in host and document.
#[test]
fn a_box_selects_all_its_rows() {
    if !crate::chart_pack_tests::has_node() {
        eprintln!("note: node is not on the PATH; the box plot is not run (development check)");
        return;
    }
    let d = def("box_plot").unwrap();
    let pkg = crate::chart_pack_tests::package_json(d.guid, &package_bytes("box_plot").unwrap());
    let mut script = String::from(crate::chart_pack_tests::DOM);
    script.push_str(pkg["content"]["js"].as_str().unwrap());
    script.push_str(&format!("\nvar Box = {}.default;\n", d.guid));
    script.push_str(
        r#"
var cats = [col("Dept", "category", ["E", "E", "E", "N"]), col("Id", "sample", ["a", "b", "c", "d"])];
var vals = [col("Score", "value", [1, 2, 9, 4], "0")];
var element = new El("div"); var host = makeHost("en-US"); host.cats = cats;
var v = Box.create({ element: element, host: host });
v.update({ dataViews: [dataView(cats, vals)], viewport: { width: 400, height: 300 } });
function walk(e, out) { e.children.forEach(function (c) { if (c.attrs.tabindex === "0") { out.push(c); } walk(c, out); }); return out; }
var points = walk(element.all("svg")[0], []);
fire(points[0], "click");
var out = { boxes: points.length, selected: host.sel.map(function (s) { return s.k; }), on: points.map(function (p) { return p.attrs["aria-selected"]; }) };
require("fs").writeFileSync(process.argv[2], JSON.stringify(out));
"#,
    );
    let dir = TempDir::new("bvis-box-select");
    let path = dir.join("box.js");
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
    assert_eq!(
        got,
        json!({"boxes": 2, "selected": ["E|a", "E|b", "E|c"], "on": ["true", "false"]})
    );
}
