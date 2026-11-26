//! Continuous numeric axes and the `start` / `end` of continuous category axes (timeaxis.rs):
//! column, bar, line, area and combo charts on a number column place their categories at their
//! values, label round numbers in the column's format, keep their hit regions and fall back to
//! one band per value when the axis is set categorical or a value is no number; `start` / `end`
//! widen the axis and drop the categories outside it, on number and date axes alike.

use std::collections::HashMap;

use serde_json::{Value, json};

use super::*;

fn page(visuals: Value) -> Value {
    json!({
        "version": 1,
        "model": {"culture": "en-US", "tables": []},
        "report": {"theme": null, "pages": [{"name": "p1", "display_name": "Page 1", "width": 800, "height": 400, "visuals": visuals}]}
    })
}

fn chart(ty: &str, typ: &str, fmt: &str, rows: Value, format: Value) -> Value {
    json!({"name": "c", "type": ty, "x": 0, "y": 0, "width": 600, "height": 300, "title": "", "format": format,
        "fields": [{"role": "Category", "kind": "column", "table": "Sales", "name": "Size"}],
        "data": {"status": "ok", "columns": [
            {"name": "Size", "role": "Category", "kind": "column", "data_type": typ, "format_string": fmt},
            {"name": "Total", "role": "Y", "kind": "measure"}], "rows": rows}})
}

fn render(s: &Value) -> PageImage {
    let fonts = FontBook::bundled_only();
    let o = RenderOptions {
        system_fonts: false,
        hits: true,
        text_dump: true,
        ..RenderOptions::default()
    };
    render_page(s, 0, &HashMap::<String, Vec<u8>>::new(), &fonts, &o).unwrap()
}

/// The data points' hit-box centres along x and their category values, in drawing order.
fn points(img: &PageImage) -> Vec<(f32, Value)> {
    img.hits
        .iter()
        .filter_map(|h| {
            let a = h.action_for(false)?;
            let v = a["point"][0]["v"].clone();
            let (x, _, w, _) = h.shape.bounds();
            (!v.is_null()).then(|| (x + w / 2.0, v))
        })
        .collect()
}

fn texts(img: &PageImage) -> Vec<&str> {
    img.texts.iter().map(|t| t.text.as_str()).collect()
}

fn sizes() -> Value {
    json!([[1, 120.0], [2, 90.0], [3, 140.0], [8, 60.0]])
}

#[test]
fn number_columns_sit_at_their_values_and_keep_their_hits() {
    let s = page(json!([chart(
        "clusteredColumnChart",
        "int64",
        "",
        sizes(),
        json!({})
    )]));
    let img = render(&s);
    let pts = points(&img);
    assert_eq!(pts.len(), 4, "{pts:?}");
    let c: Vec<f32> = pts.iter().map(|p| p.0).collect();
    // 1, then 5: positions proportional to the values
    assert!(((c[3] - c[2]) / (c[1] - c[0]) - 5.0).abs() < 0.05, "{c:?}");
    // a click on the last column selects its value
    let hit = img
        .hits
        .iter()
        .rev()
        .find(|h| h.action_for(false).is_some_and(|a| a["point"][0]["v"] == 8))
        .map(|h| h.shape.bounds())
        .unwrap();
    let a = hits::hit_at(&img.hits, hit.0 + hit.2 / 2.0, hit.1 + hit.3 / 2.0)
        .and_then(|h| h.action_for(false))
        .unwrap();
    assert_eq!(a["point"][0]["v"], 8);
    // whole-number ticks only, at the round values in between too
    let t = texts(&img);
    assert!(t.contains(&"4") && t.contains(&"6"), "{t:?}");
    assert!(!t.iter().any(|x| x.contains('.')), "{t:?}");
}

#[test]
fn a_categorical_axis_type_or_a_value_that_is_no_number_keeps_one_band_per_value() {
    let even = |s: &Value| {
        let c: Vec<f32> = points(&render(s)).iter().map(|p| p.0).collect();
        // a blank category has no value to select: three points then
        assert!(c.len() >= 3, "{c:?}");
        assert!(
            c.windows(3).all(|w| ((w[2] - w[1]) - (w[1] - w[0])).abs() < 0.01),
            "{c:?}"
        );
    };
    let f = json!({"x_axis": {"axis_type": "Categorical"}});
    even(&page(json!([chart(
        "clusteredColumnChart",
        "int64",
        "",
        sizes(),
        f
    )])));
    // a text column, and a number column with a blank value
    even(&page(json!([chart(
        "clusteredColumnChart",
        "string",
        "",
        json!([["1", 1.0], ["2", 2.0], ["8", 3.0], ["9", 4.0]]),
        json!({})
    )])));
    even(&page(json!([chart(
        "clusteredColumnChart",
        "int64",
        "",
        json!([[1, 1.0], [2, 2.0], [8, 3.0], [null, 4.0]]),
        json!({})
    )])));
    // a number level of a drill hierarchy (an auto date/time Year at the top level)
    let mut v = chart("clusteredColumnChart", "int64", "", sizes(), json!({}));
    v["drill"] = json!({"role": "Category", "levels": ["Year", "Quarter"], "top": 0, "bottom": 0,
                        "mode": false, "can_up": false, "can_down": true, "path": []});
    even(&page(json!([v])));
}

#[test]
fn number_ticks_follow_the_column_format_and_lines_join_in_value_order() {
    let rows = json!([[1500.0, 3.0], [1000.0, 1.0], [4000.0, 2.0], [2000.0, 5.0]]);
    let s = page(json!([chart("lineChart", "double", "#,0.0", rows, json!({}))]));
    let img = render(&s);
    let t = texts(&img);
    assert!(t.contains(&"2,000.0") && t.contains(&"3,000.0"), "{t:?}");
    let mut pts = points(&img);
    assert_eq!(pts.len(), 4);
    pts.sort_by(|a, b| a.0.total_cmp(&b.0));
    let order: Vec<f64> = pts.iter().filter_map(|p| p.1.as_f64()).collect();
    assert_eq!(order, [1000.0, 1500.0, 2000.0, 4000.0]);
    // 500, then 2000
    assert!(
        ((pts[3].0 - pts[2].0) / (pts[1].0 - pts[0].0) - 4.0).abs() < 0.05,
        "{pts:?}"
    );
}

/// The x centres of a line chart's points and their values.
fn line_points(typ: &str, fmt: &str, rows: Value, format: Value) -> Vec<(f32, Value)> {
    points(&render(&page(json!([chart(
        "lineChart",
        typ,
        fmt,
        rows,
        format
    )]))))
}

/// With bounds `[a, b]` around data `[d0, d1]`, the points sit as on an axis from `a` to `b`:
/// compared with the same points without bounds, which run from the plot's first to its last
/// point position (the value axis, so the plot, is the same).
fn check_widened(with: &[(f32, Value)], without: &[(f32, Value)], t: &[f64], a: f64, b: f64) {
    let (Some(l), Some(r)) = (without.first(), without.last()) else {
        panic!("no points");
    };
    let at = |x: f64| l.0 + (r.0 - l.0) * ((x - a) / (b - a)) as f32;
    assert_eq!(with.len(), t.len(), "{with:?}");
    for ((x, v), tv) in with.iter().zip(t) {
        assert!((x - at(*tv)).abs() < 0.6, "{v} at {x}, expected {}", at(*tv));
    }
}

#[test]
fn start_and_end_widen_a_number_axis_and_drop_what_lies_outside() {
    let rows = json!([[2, 1.0], [3, 2.0], [8, 3.0], [12, 3.0]]);
    let f = json!({"x_axis": {"min": 0, "max": 10}});
    let with = line_points("int64", "", rows, f);
    let vals: Vec<i64> = with.iter().filter_map(|p| p.1.as_i64()).collect();
    assert_eq!(vals, [2, 3, 8], "12 lies after the end");
    // the same plot from 0 to 10, without bounds
    let without = line_points(
        "int64",
        "",
        json!([[0, 1.0], [2, 1.0], [3, 2.0], [8, 3.0], [10, 3.0]]),
        json!({}),
    );
    check_widened(&with, &without, &[2.0, 3.0, 8.0], 0.0, 10.0);
    // start not before end: both ignored
    let rows = json!([[2, 1.0], [3, 2.0], [8, 3.0], [12, 4.0]]);
    let f = json!({"x_axis": {"min": 10, "max": 0}});
    assert_eq!(line_points("int64", "", rows, f).len(), 4);
}

#[test]
fn start_and_end_of_a_date_axis() {
    let rows = json!([
        ["2026-01-05T00:00:00", 1.0],
        ["2026-01-12T00:00:00", 2.0],
        ["2026-01-19T00:00:00", 3.0],
        ["2026-03-02T00:00:00", 3.0]
    ]);
    let without = line_points(
        "dateTime",
        "Short Date",
        json!([
            ["2026-01-01T00:00:00", 1.0],
            ["2026-01-05T00:00:00", 1.0],
            ["2026-01-12T00:00:00", 2.0],
            ["2026-01-19T00:00:00", 3.0],
            ["2026-02-01T00:00:00", 3.0]
        ]),
        json!({}),
    );
    // an ISO text (the engine's reading of a datetime'…' literal), and milliseconds since 1970
    for (a, b) in [
        (json!("2026-01-01T00:00:00"), json!("2026-02-01T00:00:00")),
        (json!(1_767_225_600_000.0f64), json!(1_769_904_000_000.0f64)),
    ] {
        let f = json!({"x_axis": {"min": a, "max": b}});
        let with = line_points("dateTime", "Short Date", rows.clone(), f);
        let vals: Vec<&str> = with.iter().filter_map(|p| p.1.as_str()).collect();
        assert_eq!(
            vals,
            [
                "2026-01-05T00:00:00",
                "2026-01-12T00:00:00",
                "2026-01-19T00:00:00"
            ],
            "2 March lies after the end"
        );
        // 1 January is day 0, 1 February day 31
        check_widened(&with, &without, &[4.0, 11.0, 18.0], 0.0, 31.0);
    }
}

#[test]
fn hostile_numbers_never_crash() {
    let cases = [
        json!([[1, 1.0], [1, 2.0]]),
        json!([[-1e300, 1.0], [1e300, 2.0]]),
        json!([["1e400", 1.0], ["2", 2.0]]),
        json!([[0.0, 1.0], [1e-300, 2.0], [2e-300, 3.0]]),
        json!([["x", 1.0], [2, 2.0]]),
        json!([[1, 1.0]]),
        json!([[1e14, 1.0], [1e14 + 1.0, 2.0], [-1e14, 3.0]]),
    ];
    let formats = [
        json!({}),
        json!({"x_axis": {"min": 5, "max": 4}}),
        json!({"x_axis": {"min": 1e300, "max": -1e300}}),
        json!({"x_axis": {"min": 100, "max": 200}}),
        json!({"x_axis": {"min": "2026-01-01", "max": "x", "display_units": 1000}}),
    ];
    for rows in cases {
        for f in &formats {
            for (ty, typ) in [
                ("clusteredColumnChart", "int64"),
                ("stackedColumnChart", "double"),
                ("clusteredBarChart", "decimal"),
                ("lineChart", "double"),
                ("areaChart", "int64"),
                ("lineClusteredColumnComboChart", "double"),
                ("lineChart", "dateTime"),
            ] {
                for w in [600, 40] {
                    let mut v = chart(ty, typ, "", rows.clone(), f.clone());
                    v["width"] = json!(w);
                    let _ = render(&page(json!([v])));
                }
            }
        }
    }
}
