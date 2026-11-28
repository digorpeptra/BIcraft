//! Analytics drawn by the renderer (docs/gaps.md G56): error bars on clustered columns, bars and
//! lines (whiskers and a line's band), per legend series, on stacked columns and on both parts of
//! a combo chart, a scatter chart's constant and statistical lines, ratio
//! line and symmetry shading; hostile values do not crash.

use std::collections::HashMap;

use serde_json::{Value, json};

use super::*;

fn snap(visuals: Value) -> Value {
    json!({
        "version": 1,
        "model": {"culture": "en-US", "tables": [{"name": "Sales", "columns": [{"name": "Region", "data_type": "string"}],
                  "measures": [{"name": "Total", "format_string": "#,0"}]}]},
        "report": {"theme": null, "pages": [{"name": "p1", "display_name": "Page 1", "width": 640, "height": 360, "visuals": visuals}]}
    })
}

fn data(cols: Value, rows: Value) -> Value {
    json!({"status": "ok", "columns": cols, "rows": rows})
}

fn render(s: &Value) -> PageImage {
    let fonts = FontBook::bundled_only();
    let opts = RenderOptions {
        system_fonts: false,
        ..RenderOptions::default()
    };
    render_page(s, 0, &HashMap::<String, Vec<u8>>::new(), &fonts, &opts).unwrap()
}

/// Pixels within 40 of `c` on every channel.
fn pixels_of(img: &PageImage, c: (u8, u8, u8)) -> usize {
    let pm = tiny_skia::Pixmap::decode_png(&img.png).unwrap();
    let near = |a: u8, b: u8| (i16::from(a) - i16::from(b)).abs() <= 40;
    pm.pixels()
        .iter()
        .filter(|p| near(p.red(), c.0) && near(p.green(), c.1) && near(p.blue(), c.2))
        .count()
}

/// A chart of `ty` over four regions, its analytics `lines`, `data.error_bars` `errs`.
fn chart(ty: &str, lines: Value, errs: Option<Value>) -> Value {
    let mut d = data(
        json!([{"name": "Region", "role": "Category", "kind": "column", "data_type": "string"},
               {"name": "Total", "role": "Y", "kind": "measure"}]),
        json!([["North", 120.0], ["South", 80.0], ["East", 45.5], ["West", 10.0]]),
    );
    if let (Some(e), Some(o)) = (errs, d.as_object_mut()) {
        o.insert("error_bars".into(), e);
    }
    snap(
        json!([{"name": "c", "type": ty, "x": 0, "y": 0, "width": 600, "height": 330,
        "fields": [{"role": "Category", "kind": "column", "table": "Sales", "name": "Region"},
                   {"role": "Y", "kind": "measure", "table": "Sales", "name": "Total"}],
        "data": d, "format": {"reference_lines": lines}}]),
    )
}

#[test]
fn error_bars_are_drawn_on_columns_bars_and_lines() {
    let errs = json!([{"column": 1, "upper": [140.0, 90.0, 60.0, 20.0], "lower": [100.0, 60.0, 30.0, 0.0]}]);
    let magenta = (255, 0, 255);
    for ty in ["clusteredColumnChart", "clusteredBarChart", "lineChart"] {
        let lines = json!([{"kind": "error_bars", "axis": "y", "column": 1, "color": "#FF00FF", "width": 3}]);
        let with = render(&chart(ty, lines.clone(), Some(errs.clone())));
        let without = render(&chart(ty, lines, None));
        assert!(pixels_of(&with, magenta) > 100, "{ty}: error bars drawn");
        assert_eq!(pixels_of(&without, magenta), 0, "{ty}: no bounds, nothing drawn");
    }
    // a line's band: the area between the bounds
    let band = json!([{"kind": "error_bars", "axis": "y", "column": 1, "bar": false, "marker": false, "band": true,
                       "band_color": "#00FF00", "transparency": 0}]);
    let img = render(&chart("lineChart", band.clone(), Some(errs.clone())));
    assert!(pixels_of(&img, (0, 255, 0)) > 1000, "the band is filled");
    let none = render(&chart("lineChart", band, None));
    assert_eq!(pixels_of(&none, (0, 255, 0)), 0);
    // hostile values do not crash
    let hostile =
        json!([{"column": 1e30, "upper": "x"}, {"column": 1, "upper": [1e308, null, "a"], "lower": 7}, 3]);
    let lines = json!([{"kind": "error_bars", "column": 1, "width": -5, "transparency": "x", "band": true}]);
    for ty in ["lineChart", "clusteredBarChart", "clusteredColumnChart"] {
        render(&chart(ty, lines.clone(), Some(hostile.clone())));
    }
}

#[test]
fn scatter_draws_its_lines_ratio_line_and_symmetry_shading() {
    let rows = json!([["a", 10, 30], ["b", 20, 15], ["c", 35, 40]]);
    let cols = json!([{"name": "Region", "role": "Category", "kind": "column", "data_type": "string"},
                      {"name": "Units", "role": "X", "kind": "measure"},
                      {"name": "Total", "role": "Y", "kind": "measure"}]);
    let sc = |lines: Value| {
        snap(
            json!([{"name": "s", "type": "scatterChart", "x": 0, "y": 0, "width": 600, "height": 330,
            "data": data(cols.clone(), rows.clone()), "format": {"reference_lines": lines}}]),
        )
    };
    let base = render(&sc(json!([])));
    let img = render(&sc(json!([
        {"kind": "symmetry_shading", "axis": "y", "upper_color": "#FF0000", "lower_color": "#0000FF", "transparency": 0},
        {"kind": "ratio_line", "axis": "y", "color": "#FF00FF", "width": 3, "style": "solid"},
        {"kind": "constant", "axis": "x", "value": 25, "color": "#00FFFF", "width": 3, "style": "solid"},
        {"kind": "average", "axis": "y", "column": 2, "color": "#00FF00", "width": 3, "style": "solid"},
    ])));
    for (name, c) in [
        ("upper shading", (255, 0, 0)),
        ("lower shading", (0, 0, 255)),
        ("ratio line", (255, 0, 255)),
        ("x constant line", (0, 255, 255)),
        ("average line", (0, 255, 0)),
    ] {
        assert!(pixels_of(&img, c) > 100, "{name} drawn");
        assert!(pixels_of(&base, c) < 100, "{name} not in the plain chart");
    }
}

/// A line chart over twelve days (a spike on day 7), its analytics `lines`, a legend or not.
fn days(lines: Value, legend: bool) -> Value {
    let mut cols = vec![json!({"name": "Day", "role": "Category", "kind": "column", "data_type": "int64"})];
    if legend {
        cols.push(json!({"name": "Region", "role": "Series", "kind": "column", "data_type": "string"}));
    }
    cols.push(json!({"name": "Total", "role": "Y", "kind": "measure"}));
    let vals = [
        100.0, 102.0, 99.0, 101.0, 103.0, 100.0, 190.0, 98.0, 101.0, 102.0, 100.0, 99.0,
    ];
    let rows: Vec<Value> = vals
        .iter()
        .enumerate()
        .map(|(i, v)| {
            if legend {
                json!([i + 1, "a", v])
            } else {
                json!([i + 1, v])
            }
        })
        .collect();
    snap(
        json!([{"name": "l", "type": "lineChart", "x": 0, "y": 0, "width": 600, "height": 330,
        "data": data(Value::Array(cols), Value::Array(rows)), "format": {"reference_lines": lines}}]),
    )
}

#[test]
fn anomalies_draw_their_range_and_markers_on_a_plain_line_chart() {
    let lines = json!([{"kind": "anomaly_detection", "axis": "y", "column": 1, "band_color": "#00FF00",
                        "transparency": 0, "marker_color": "#FF00FF", "marker_size": 6}]);
    let img = render(&days(lines.clone(), false));
    assert!(
        pixels_of(&img, (0, 255, 0)) > 1000,
        "the expected range is filled"
    );
    let marker = pixels_of(&img, (255, 0, 255));
    assert!(marker > 40, "the spike is marked ({marker})");
    let note = img
        .visuals
        .first()
        .and_then(|v| v.note.clone())
        .unwrap_or_default();
    assert!(note.contains("BIcraft's own estimate"), "{note}");
    // no markers when off, no range when its style is none
    let quiet = json!([{"kind": "anomaly_detection", "column": 1, "band_style": "none", "band_color": "#00FF00",
                        "marker": false, "marker_color": "#FF00FF"}]);
    let img = render(&days(quiet, false));
    assert_eq!(pixels_of(&img, (0, 255, 0)), 0);
    assert_eq!(pixels_of(&img, (255, 0, 255)), 0);
    // nothing with a legend, nor on a plain chart
    let with_legend = render(&days(lines, true));
    assert_eq!(pixels_of(&with_legend, (0, 255, 0)), 0);
    assert_eq!(pixels_of(&render(&days(json!([]), false)), (255, 0, 255)), 0);
    // hostile options do not crash
    for shape in ["square", "diamond", "triangle", "droplet", "x", ""] {
        let l = json!([{"kind": "anomaly_detection", "column": 1e30, "marker_shape": shape, "marker_size": -3,
                        "sensitivity": "x", "transparency": 1e9, "band_style": 3}]);
        render(&days(l, false));
    }
}

/// The `(x, y)` of every pixel within 40 of `c`.
fn where_is(img: &PageImage, c: (u8, u8, u8)) -> Vec<(u32, u32)> {
    let pm = tiny_skia::Pixmap::decode_png(&img.png).unwrap();
    let near = |a: u8, b: u8| (i16::from(a) - i16::from(b)).abs() <= 40;
    let w = pm.width();
    pm.pixels()
        .iter()
        .enumerate()
        .filter(|(_, p)| near(p.red(), c.0) && near(p.green(), c.1) && near(p.blue(), c.2))
        .map(|(i, _)| (i as u32 % w, i as u32 / w))
        .collect()
}

const RED: (u8, u8, u8) = (255, 0, 0);
const BLUE: (u8, u8, u8) = (0, 0, 255);
const MAGENTA: (u8, u8, u8) = (255, 0, 255);
const GREEN: (u8, u8, u8) = (0, 255, 0);

/// A chart of `ty` over regions and a legend (`a` red, `b` blue), its analytics `lines`,
/// `data.error_bars` `errs`, the rows `[region, legend, value]`.
fn legend_chart(ty: &str, rows: Value, lines: Value, errs: Option<Value>) -> Value {
    let mut d = data(
        json!([{"name": "Region", "role": "Category", "kind": "column", "data_type": "string"},
               {"name": "Kind", "role": "Series", "kind": "column", "data_type": "string"},
               {"name": "Total", "role": "Y", "kind": "measure"}]),
        rows,
    );
    if let (Some(e), Some(o)) = (errs, d.as_object_mut()) {
        o.insert("error_bars".into(), e);
    }
    snap(
        json!([{"name": "c", "type": ty, "x": 0, "y": 0, "width": 600, "height": 330,
        "data": d, "format": {"reference_lines": lines, "legend": {"show": false},
            "series_colors": [{"value": "a", "color": "#FF0000"}, {"value": "b", "color": "#0000FF"}]}}]),
    )
}

fn mean(xs: impl Iterator<Item = u32>) -> f64 {
    let v: Vec<u32> = xs.collect();
    v.iter().map(|x| f64::from(*x)).sum::<f64>() / v.len().max(1) as f64
}

#[test]
fn each_legend_series_gets_its_error_bars_at_its_points_in_its_colour() {
    // small values, caps only (no whisker): the caps float high above the bars and lines
    let rows = json!([["N", "a", 5.0], ["N", "b", 5.0], ["S", "a", 5.0], ["S", "b", 5.0]]);
    let errs = json!([{"column": 2, "upper": [100.0, 60.0, 100.0, 60.0], "lower": [80.0, 40.0, 80.0, 40.0]}]);
    let lines = json!([{"kind": "error_bars", "axis": "y", "column": 2, "match_series": true, "bar": false, "width": 3}]);
    for ty in ["clusteredColumnChart", "lineChart"] {
        let img = render(&legend_chart(ty, rows.clone(), lines.clone(), Some(errs.clone())));
        // the caps: the series' colour, well above the values (y < 200 of 360)
        let high = |img: &PageImage, c| -> Vec<(u32, u32)> {
            where_is(img, c).into_iter().filter(|p| p.1 < 200).collect()
        };
        let (r, b) = (high(&img, RED), high(&img, BLUE));
        assert!(
            r.len() > 20 && b.len() > 20,
            "{ty}: both series have caps ({} red, {} blue)",
            r.len(),
            b.len()
        );
        // each series at its own bounds: a's (80-100) above b's (40-60)
        let (ry, by) = (mean(r.iter().map(|p| p.1)), mean(b.iter().map(|p| p.1)));
        assert!(ry < by, "{ty}: a's caps above b's ({ry} vs {by})");
        if ty == "clusteredColumnChart" {
            // at its own bar: in each category a's caps left of b's
            for (lo, hi) in [(0, 300), (300, 600)] {
                let rx = mean(r.iter().filter(|p| p.0 >= lo && p.0 < hi).map(|p| p.0));
                let bx = mean(b.iter().filter(|p| p.0 >= lo && p.0 < hi).map(|p| p.0));
                assert!(rx < bx, "a's caps left of b's in {lo}..{hi} ({rx} vs {bx})");
            }
        }
    }
}

#[test]
fn stacked_columns_draw_error_bars_at_the_top_of_each_segment() {
    // a: 0-30, b stacked on it: 30-80; b's bounds are 45 and 55 around its value 50: drawn at
    // the top of its segment (75 and 85), not at 45 and 55 (inside the segment)
    let rows = json!([["N", "a", 30.0], ["N", "b", 50.0]]);
    let errs = json!([{"column": 2, "upper": [35.0, 55.0], "lower": [25.0, 45.0]}]);
    let lines = json!([{"kind": "error_bars", "axis": "y", "column": 2, "color": "#FF00FF", "bar": false, "width": 2}]);
    for ty in ["columnChart", "hundredPercentStackedColumnChart"] {
        let img = render(&legend_chart(ty, rows.clone(), lines.clone(), Some(errs.clone())));
        let blue = where_is(&img, BLUE);
        let top = blue.iter().map(|p| p.1).min().unwrap();
        let bottom = blue.iter().map(|p| p.1).max().unwrap();
        let caps = where_is(&img, MAGENTA);
        assert!(!caps.is_empty(), "{ty}: error bars drawn");
        let at = |frac: f64| f64::from(bottom) - frac * f64::from(bottom - top);
        // the middle of b's segment (where b's own bounds would sit) holds no cap
        let (lo, hi) = (at(0.6), at(0.2));
        assert!(
            !caps.iter().any(|p| f64::from(p.1) > lo && f64::from(p.1) < hi),
            "{ty}: no cap in the middle of the segment ({top}..{bottom})"
        );
        // b's upper cap at or above the stack's top
        assert!(
            caps.iter().any(|p| p.1 <= top + 1),
            "{ty}: a cap at the top of the stack"
        );
    }
}

#[test]
fn combo_charts_draw_error_bars_on_their_columns_and_their_line() {
    let cols = json!([{"name": "Region", "role": "Category", "kind": "column", "data_type": "string"},
                      {"name": "Total", "role": "Y", "kind": "measure"},
                      {"name": "Other", "role": "Y2", "kind": "measure"}]);
    let rows = json!([["N", 50.0, 20.0], ["S", 40.0, 25.0], ["E", 30.0, 15.0]]);
    let errs = json!([{"column": 1, "upper": [60.0, 50.0, 40.0], "lower": [45.0, 35.0, 25.0]},
                      {"column": 2, "upper": [25.0, 30.0, 20.0], "lower": [15.0, 20.0, 10.0]}]);
    let lines = json!([{"kind": "error_bars", "column": 1, "color": "#FF00FF", "width": 3},
                       {"kind": "error_bars", "column": 2, "color": "#00FF00", "width": 3}]);
    let combo = |ty: &str, e: Option<Value>| {
        let mut d = data(cols.clone(), rows.clone());
        if let (Some(e), Some(o)) = (e, d.as_object_mut()) {
            o.insert("error_bars".into(), e);
        }
        snap(
            json!([{"name": "c", "type": ty, "x": 0, "y": 0, "width": 600, "height": 330,
                     "data": d, "format": {"reference_lines": lines.clone()}}]),
        )
    };
    for ty in ["lineClusteredColumnComboChart", "lineStackedColumnComboChart"] {
        let img = render(&combo(ty, Some(errs.clone())));
        let plain = render(&combo(ty, None));
        assert!(pixels_of(&img, MAGENTA) > 60, "{ty}: the columns' error bars");
        assert!(pixels_of(&img, GREEN) > 60, "{ty}: the line's error bars");
        assert_eq!(
            pixels_of(&plain, MAGENTA) + pixels_of(&plain, GREEN),
            0,
            "{ty}: none without bounds"
        );
    }
}
