//! Continuous date axes (timeaxis.rs): column, bar, line, area and combo charts on a date
//! category place their points at their dates, label round dates, keep their hit regions, and
//! fall back to one band per value when the axis is set categorical or expanded down a level.

use std::collections::HashMap;

use serde_json::{Value, json};

use super::*;

fn page(culture: &str, visuals: Value) -> Value {
    json!({
        "version": 1,
        "model": {"culture": culture, "tables": []},
        "report": {"theme": null, "pages": [{"name": "p1", "display_name": "Page 1", "width": 800, "height": 400, "visuals": visuals}]}
    })
}

fn chart(ty: &str, fmt: &str, rows: Value, format: Value) -> Value {
    json!({"name": "c", "type": ty, "x": 0, "y": 0, "width": 600, "height": 300, "title": "", "format": format,
        "fields": [{"role": "Category", "kind": "column", "table": "Sales", "name": "Day"}],
        "data": {"status": "ok", "columns": [
            {"name": "Day", "role": "Category", "kind": "column", "data_type": "dateTime", "format_string": fmt},
            {"name": "Total", "role": "Y", "kind": "measure"}], "rows": rows}})
}

fn render_with(s: &Value, hits: bool) -> PageImage {
    let fonts = FontBook::bundled_only();
    let o = RenderOptions {
        system_fonts: false,
        hits,
        raw: hits,
        text_dump: true,
        ..RenderOptions::default()
    };
    render_page(s, 0, &HashMap::<String, Vec<u8>>::new(), &fonts, &o).unwrap()
}

/// The data points' hit boxes `(x, y, w, h)` and their category values, in drawing order.
fn points(img: &PageImage) -> Vec<((f32, f32, f32, f32), Value)> {
    img.hits
        .iter()
        .filter_map(|h| {
            let a = h.action_for(false)?;
            let v = a["point"][0]["v"].clone();
            (!v.is_null()).then(|| (h.shape.bounds(), v))
        })
        .collect()
}

fn days_rows() -> Value {
    json!([
        ["2026-01-05T00:00:00", 120.0],
        ["2026-01-06T00:00:00", 90.0],
        ["2026-01-07T00:00:00", 140.0],
        ["2026-01-12T00:00:00", 60.0]
    ])
}

fn centres_x(img: &PageImage) -> Vec<f32> {
    points(img).iter().map(|((x, _, w, _), _)| x + w / 2.0).collect()
}

#[test]
fn date_columns_sit_at_their_dates_and_keep_their_hits() {
    let s = page(
        "en-US",
        json!([chart(
            "clusteredColumnChart",
            "General Date",
            days_rows(),
            json!({})
        )]),
    );
    let img = render_with(&s, true);
    let pts = points(&img);
    assert_eq!(pts.len(), 4, "{pts:?}");
    let c = centres_x(&img);
    let (d1, d3) = (c[1] - c[0], c[3] - c[2]);
    // 1 day, then 5 days: positions proportional to time
    assert!((d3 / d1 - 5.0).abs() < 0.05, "{c:?}");
    // columns as wide as the smallest gap less the 20 % inner padding, all alike
    let w: Vec<f32> = pts.iter().map(|((_, _, w, _), _)| *w).collect();
    assert!(w.iter().all(|x| (x - w[0]).abs() < 0.01), "{w:?}");
    assert!((w[0] / d1 - 0.8).abs() < 0.01, "{w:?} {d1}");
    // a click on the last column selects its date
    let ((x, y, ww, h), v) = &pts[3];
    assert_eq!(v, "2026-01-12T00:00:00");
    let hit = hits::hit_at(&img.hits, x + ww / 2.0, y + h / 2.0).and_then(|h| h.action_for(false));
    assert_eq!(hit.unwrap()["point"][0]["v"], "2026-01-12T00:00:00");
    // a tick every day: the column's General Date would not fit, short days do
    let texts: Vec<&str> = img.texts.iter().map(|t| t.text.as_str()).collect();
    assert!(texts.contains(&"Jan 5") && texts.contains(&"Jan 12"), "{texts:?}");
    assert!(
        !texts
            .iter()
            .any(|t| t.contains("12:00:00") || t.contains("00:00")),
        "{texts:?}"
    );
}

#[test]
fn a_categorical_axis_type_keeps_one_band_per_value() {
    for at in ["Categorical", "'Categorical'"] {
        let f = json!({"x_axis": {"axis_type": at}});
        let s = page(
            "en-US",
            json!([chart("clusteredColumnChart", "General Date", days_rows(), f)]),
        );
        let c = centres_x(&render_with(&s, true));
        let (d1, d3) = (c[1] - c[0], c[3] - c[2]);
        assert!((d3 - d1).abs() < 0.01, "{c:?}");
    }
    // Scalar is the default made explicit
    let f = json!({"x_axis": {"axis_type": "Scalar"}});
    let s = page(
        "en-US",
        json!([chart("clusteredColumnChart", "General Date", days_rows(), f)]),
    );
    let c = centres_x(&render_with(&s, true));
    assert!(((c[3] - c[2]) / (c[1] - c[0]) - 5.0).abs() < 0.05, "{c:?}");
}

#[test]
fn line_points_follow_time_and_join_in_time_order() {
    let rows = json!([
        ["2025-03-01", 30.0],
        ["2025-01-01", 10.0],
        ["2025-01-15", 25.0],
        ["2025-06-10", 18.0],
        ["2025-02-01", 40.0]
    ]);
    let s = page(
        "en-US",
        json!([chart("lineChart", "Short Date", rows, json!({}))]),
    );
    let img = render_with(&s, true);
    let mut pts: Vec<(f32, f32, String)> = points(&img)
        .into_iter()
        .map(|((x, y, w, h), v)| (x + w / 2.0, y + h / 2.0, v.as_str().unwrap_or("").to_string()))
        .collect();
    assert_eq!(pts.len(), 5);
    pts.sort_by(|a, b| a.0.total_cmp(&b.0));
    let order: Vec<&str> = pts.iter().map(|p| p.2.as_str()).collect();
    assert_eq!(
        order,
        [
            "2025-01-01",
            "2025-01-15",
            "2025-02-01",
            "2025-03-01",
            "2025-06-10"
        ]
    );
    // 14 days, then 17: proportional
    let r = (pts[2].0 - pts[1].0) / (pts[1].0 - pts[0].0);
    assert!((r - 17.0 / 14.0).abs() < 0.02, "{pts:?}");
    // the line runs from 15 January to 1 February (time order), not from 15 January to June
    // (row order)
    let pm = tiny_skia::Pixmap::from_vec(
        img.rgba.clone(),
        tiny_skia::IntSize::from_wh(img.width, img.height).unwrap(),
    )
    .unwrap();
    let inked = |x: f32, y: f32| {
        (-1i32..=1).any(|dy| {
            pm.pixel(x.round() as u32, (y.round() as i32 + dy) as u32)
                .is_some_and(|p| p.red() < 200 || p.blue() < 200)
        })
    };
    let mid = |a: &(f32, f32, String), b: &(f32, f32, String)| ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    let (x, y) = mid(&pts[1], &pts[2]);
    assert!(
        inked(x, y),
        "no line between 15 January and 1 February at ({x}, {y})"
    );
    // month ticks
    let texts: Vec<&str> = img.texts.iter().map(|t| t.text.as_str()).collect();
    assert!(
        texts.contains(&"Jan 2025") && texts.contains(&"Jun 2025"),
        "{texts:?}"
    );
}

#[test]
fn hours_tick_at_round_hours_and_bars_run_down_in_time() {
    let rows = json!([
        ["2026-01-05T00:00:00", 5.0],
        ["2026-01-05T03:00:00", 8.0],
        ["2026-01-05T15:00:00", 9.0],
        ["2026-01-06T06:00:00", 6.0]
    ]);
    let s = page(
        "fr-FR",
        json!([chart("areaChart", "General Date", rows, json!({}))]),
    );
    let img = render_with(&s, false);
    let texts: Vec<&str> = img.texts.iter().map(|t| t.text.as_str()).collect();
    // the day and the hour on every tick, midnight included
    assert!(texts.contains(&"5 janv. 00:00"), "{texts:?}");
    assert!(texts.contains(&"5 janv. 12:00"), "{texts:?}");
    // horizontal bars: the earliest date on top, gaps proportional
    let rows = json!([["2024-01-01", 50.0], ["2024-04-01", 70.0], ["2025-01-01", 90.0]]);
    let s = page(
        "en-US",
        json!([chart("clusteredBarChart", "Short Date", rows, json!({}))]),
    );
    let img = render_with(&s, true);
    let ys: Vec<f32> = points(&img).iter().map(|((_, y, _, h), _)| y + h / 2.0).collect();
    assert_eq!(ys.len(), 3);
    let r = (ys[2] - ys[1]) / (ys[1] - ys[0]);
    assert!((r - 275.0 / 91.0).abs() < 0.05, "{ys:?}");
}

/// The x-axis tick texts of a chart drawn alone, left to right (texts under the plot).
fn x_ticks(img: &PageImage, below: f32) -> Vec<String> {
    let mut t: Vec<(f32, String)> = img
        .texts
        .iter()
        .filter(|t| t.y > below)
        .map(|t| (t.x, t.text.clone()))
        .collect();
    t.sort_by(|a, b| a.0.total_cmp(&b.0));
    t.into_iter().map(|(_, s)| s).collect()
}

/// Daily serial-based ISO dates: `n` dates from `first` (a `YYYY-MM-DD` date) stepping `step`.
fn series(dates: Vec<String>) -> Vec<Value> {
    dates
        .into_iter()
        .enumerate()
        .map(|(i, d)| json!([d, 10.0 + ((i * 37) % 50) as f64]))
        .collect()
}

fn months_from(y: i32, n: i32, step: i32) -> Vec<String> {
    (0..n)
        .map(|i| {
            let m = i * step;
            format!("{}-{:02}-01T00:00:00", y + m / 12, m % 12 + 1)
        })
        .collect()
}

fn days_from_jan_1(n: u32) -> Vec<String> {
    (0..n)
        .map(|i| {
            let (m, d) = if i < 31 {
                (1, i + 1)
            } else if i < 59 {
                (2, i - 30)
            } else {
                (3, i - 58)
            };
            format!("2026-{m:02}-{d:02}T00:00:00")
        })
        .collect()
}

#[test]
fn date_ticks_follow_desktop() {
    // Ticks Power BI Desktop 2.159.1231.0 drew on synthetic data (French UI, 2026-11-24,
    // docs/render.md "Continuous date axes"): every tick of an axis whose dates carry times
    // writes the day and the time ("1 mars 00:00"), midnight included; day ticks are "4 janv."
    // whatever the column's format; at most eight labels.
    let hours: Vec<String> = (0..48)
        .map(|i| format!("2026-03-{:02}T{:02}:00:00", 1 + i / 24, i % 24))
        .collect();
    let six_hours = vec![
        "1 mars 00:00",
        "1 mars 06:00",
        "1 mars 12:00",
        "1 mars 18:00",
        "2 mars 00:00",
        "2 mars 06:00",
        "2 mars 12:00",
        "2 mars 18:00",
    ];
    let half_days = vec!["1 mars 00:00", "1 mars 12:00", "2 mars 00:00", "2 mars 12:00"];
    let weeks = vec!["4 janv.", "11 janv.", "18 janv."];
    let half_years = vec![
        "janv. 2023",
        "juil. 2023",
        "janv. 2024",
        "juil. 2024",
        "janv. 2025",
        "juil. 2025",
    ];
    let cases: Vec<(&str, &str, i32, i32, Vec<String>, Vec<&str>)> = vec![
        (
            "lineChart",
            "General Date",
            380,
            280,
            hours.clone(),
            half_days.clone(),
        ),
        (
            "clusteredColumnChart",
            "General Date",
            380,
            280,
            hours.clone(),
            half_days,
        ),
        (
            "clusteredColumnChart",
            "General Date",
            240,
            200,
            hours.clone(),
            vec!["1 mars 00:00", "2 mars 00:00"],
        ),
        (
            "lineChart",
            "General Date",
            600,
            280,
            hours.clone(),
            six_hours.clone(),
        ),
        ("lineChart", "General Date", 1200, 280, hours, six_hours),
        (
            "clusteredColumnChart",
            "General Date",
            380,
            280,
            days_from_jan_1(20),
            weeks.clone(),
        ),
        (
            "clusteredColumnChart",
            "General Date",
            1200,
            280,
            days_from_jan_1(20),
            weeks.clone(),
        ),
        ("lineChart", "Long Date", 380, 280, days_from_jan_1(20), weeks),
        (
            "clusteredColumnChart",
            "Long Date",
            1200,
            280,
            days_from_jan_1(60),
            vec!["janv. 2026", "févr. 2026", "mars 2026"],
        ),
        (
            "clusteredColumnChart",
            "Long Date",
            380,
            280,
            months_from(2025, 12, 1),
            vec!["janv. 2025", "avr. 2025", "juil. 2025", "oct. 2025"],
        ),
        (
            "clusteredColumnChart",
            "Long Date",
            600,
            280,
            months_from(2023, 36, 1),
            half_years.clone(),
        ),
        (
            "clusteredColumnChart",
            "Long Date",
            1200,
            280,
            months_from(2023, 36, 1),
            half_years,
        ),
        (
            "lineChart",
            "Long Date",
            380,
            280,
            months_from(2023, 12, 3),
            vec!["2023", "2024", "2025"],
        ),
    ];
    for (ty, fmt, w, h, dates, want) in cases {
        let mut c = chart(ty, fmt, json!(series(dates)), json!({}));
        c["width"] = json!(w);
        c["height"] = json!(h);
        let mut s = page("fr-FR", json!([c]));
        s["report"]["pages"][0]["width"] = json!(1300);
        let img = render_with(&s, false);
        assert_eq!(x_ticks(&img, h as f32 - 30.0), want, "{ty} {fmt} {w}");
    }
    // one round date at most: the first and last dates (Desktop drew twenty days on 240 px so;
    // there BIcraft still draws weekly ticks, which Desktop did not)
    let mut c = chart(
        "clusteredColumnChart",
        "Long Date",
        json!(series(days_from_jan_1(20))),
        json!({}),
    );
    c["width"] = json!(170);
    c["height"] = json!(200);
    let img = render_with(&page("fr-FR", json!([c])), false);
    assert_eq!(x_ticks(&img, 170.0), vec!["1 janv.", "20 janv."]);
}
#[test]
fn an_expanded_date_hierarchy_stays_categorical() {
    // year and date both on the axis (expanded down a level): concatenated labels, one band each
    let v = json!({"name": "c", "type": "clusteredColumnChart", "x": 0, "y": 0, "width": 600, "height": 300, "title": "",
        "fields": [{"role": "Category", "kind": "column", "table": "Sales", "name": "Day"}],
        "data": {"status": "ok", "columns": [
            {"name": "Year", "role": "Category", "kind": "column", "data_type": "int64"},
            {"name": "Day", "role": "Category", "kind": "column", "data_type": "dateTime"},
            {"name": "Total", "role": "Y", "kind": "measure"}],
            "rows": [[2026, "2026-01-05", 1.0], [2026, "2026-01-06", 2.0], [2026, "2026-01-07", 3.0], [2026, "2026-01-12", 4.0]]}});
    let c = centres_x(&render_with(&page("en-US", json!([v])), true));
    assert_eq!(c.len(), 4);
    assert!(((c[3] - c[2]) - (c[1] - c[0])).abs() < 0.01, "{c:?}");
}

#[test]
fn a_combo_puts_columns_and_line_points_at_the_same_dates() {
    let v = json!({"name": "c", "type": "lineClusteredColumnComboChart", "x": 0, "y": 0, "width": 600, "height": 300, "title": "",
        "fields": [{"role": "Category", "kind": "column", "table": "Sales", "name": "Month"}],
        "data": {"status": "ok", "columns": [
            {"name": "Month", "role": "Category", "kind": "column", "data_type": "dateTime"},
            {"name": "Total", "role": "Y", "kind": "measure"},
            {"name": "Cost", "role": "Y2", "kind": "measure"}],
            "rows": [["2025-01-01", 60.0, 30.0], ["2025-02-01", 75.0, 35.0], ["2025-05-01", 95.0, 40.0]]}});
    let img = render_with(&page("en-US", json!([v])), true);
    let pts = points(&img);
    let at = |date: &str, round: bool| {
        pts.iter()
            .filter(|(_, v)| v == date)
            .map(|((x, _, w, h), _)| (x + w / 2.0, (w - h).abs() < 0.01))
            .find(|(_, r)| *r == round)
            .map(|p| p.0)
    };
    for d in ["2025-01-01", "2025-02-01", "2025-05-01"] {
        let (col, dot) = (at(d, false).unwrap(), at(d, true).unwrap());
        assert!((col - dot).abs() < 0.6, "{d}: {col} {dot}");
    }
    let (a, b, c) = (
        at("2025-01-01", false).unwrap(),
        at("2025-02-01", false).unwrap(),
        at("2025-05-01", false).unwrap(),
    );
    assert!(((c - b) / (b - a) - 89.0 / 31.0).abs() < 0.05, "{a} {b} {c}");
}

#[test]
fn hostile_dates_never_crash() {
    let cases = [
        json!([["2026-01-05", 1.0], ["2026-01-05T00:00:00", 2.0]]),
        json!([["0001-01-01", 1.0], ["9999-12-31T23:59:59", 2.0]]),
        json!([
            ["2026-01-05T00:00:00", 1.0],
            ["2026-01-05T00:00:01", 2.0],
            ["2027-01-05", 3.0]
        ]),
        json!([["not a date", 1.0], ["2026-01-05", 2.0]]),
        json!([["2026-13-45", 1.0], ["2026-01-05", null], [null, 3.0]]),
        json!([["2026-01-05", 1.0]]),
    ];
    for rows in cases {
        for ty in [
            "clusteredColumnChart",
            "stackedColumnChart",
            "clusteredBarChart",
            "lineChart",
            "areaChart",
            "lineClusteredColumnComboChart",
        ] {
            for w in [600, 40] {
                let mut v = chart(ty, "General Date", rows.clone(), json!({}));
                v["width"] = json!(w);
                let _ = render_with(&page("fr-FR", json!([v])), true);
            }
        }
    }
}
