//! The spec parser and layout (docs/formats/spec-visual.md); the TypeScript drawer is compared
//! with this layout by the engine's golden tests.

use super::*;

fn data() -> SpecData {
    SpecData::from_json(&json!({"categories": ["a", "b", "c"], "values": [[100, 200, 442], [3, null, 1]]}))
}

#[test]
fn every_template_parses_and_lays_out() {
    for k in TEMPLATES {
        let v = template(k).unwrap();
        let s = parse(&v).unwrap_or_else(|e| panic!("{k}: {e}"));
        let prims = layout(&s, &data(), 300.0, 200.0);
        assert!(!prims.is_empty(), "{k}");
    }
    assert!(template("pie").is_none());
}

#[test]
fn bars_stand_on_the_baseline_in_their_band() {
    let s = parse(&json!({"marks": [{"type": "bar"}], "y": {"grid": false}})).unwrap();
    // one measure bound (two would be drawn as two series side by side)
    let one = SpecData::from_json(&json!({"categories": ["a", "b", "c"], "values": [[100, 200, 442]]}));
    let p = layout(&s, &one, 300.0, 200.0);
    let bars: Vec<&Prim> = p
        .iter()
        .filter(|x| matches!(x, Prim::Rect { .. }) && x.json()["role"] != "card")
        .collect();
    assert_eq!(bars.len(), 3);
    // plot: left 8 + 36, right 292, bottom 200 - 8 - 18; y domain 0..500 (step 100)
    let Prim::Rect { x, y, w, h, .. } = bars[2] else {
        panic!("not a rect")
    };
    let step = (292.0 - 44.0) / 3.0;
    assert_eq!(*x, r2(44.0 + step * 2.0 + step * 0.1));
    assert_eq!(*w, r2(step * 0.8));
    assert_eq!(r2(y + h), 174.0);
    assert_eq!(*y, r2(174.0 - 442.0 / 500.0 * (174.0 - 8.0)));
    // the y axis: 0, 100 … 500
    let labels: Vec<String> = p
        .iter()
        .filter_map(|x| match x {
            Prim::Text {
                text, anchor: "end", ..
            } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(labels, ["0", "100", "200", "300", "400", "500"]);
}

#[test]
fn a_missing_value_breaks_a_line() {
    let s = parse(&json!({"marks": [{"type": "line", "y": "value.1"}]})).unwrap();
    let paths = layout(&s, &data(), 300.0, 200.0)
        .into_iter()
        .filter(|p| matches!(p, Prim::Path { .. }))
        .count();
    // a, (b missing), c: two runs of one point each, so no path
    assert_eq!(paths, 0);
}

#[test]
fn numbers_and_dates_format_alike_everywhere() {
    assert_eq!(fmt_fixed(1234567.891, 2), "1,234,567.89");
    assert_eq!(fmt_fixed(-0.004, 2), "0.00");
    assert_eq!(fmt_fixed(2.5, 0), "3");
    assert_eq!(fmt_fixed(-2.5, 0), "-3");
    assert_eq!(fmt_fixed(f64::NAN, 2), "");
    assert_eq!(fmt_fixed(1e300, 2), "#");
    assert_eq!(fmt_auto(12.0), "12");
    assert_eq!(fmt_auto(0.125), "0.13");
    let ms = parse_iso("2024-02-29T13:45:10.5Z").unwrap();
    assert_eq!(date_text(ms, false), "2024-02-29");
    assert_eq!(date_text(ms, true), "2024-02-29 13:45");
    assert_eq!(parse_iso("1970-01-01"), Some(0.0));
    assert_eq!(date_text(parse_iso("1969-12-31").unwrap(), false), "1969-12-31");
    for bad in [
        "2024-13-01",
        "24-01-01",
        "2024-01-01T25:00",
        "x",
        "",
        "2024-01-01T10:00:00.abc",
    ] {
        assert_eq!(parse_iso(bad), None, "{bad}");
    }
    assert_eq!(date_text(f64::MAX, false), "275760-09-13");
}

#[test]
fn bad_specs_are_refused_with_their_place() {
    let cases = [
        (json!([]), "JSON object"),
        (json!({"marks": []}), "spec.marks"),
        (json!({"marks": [{"type": "pie"}]}), "marks[0].type"),
        (
            json!({"marks": [{"type": "bar", "colour": "#000000"}]}),
            "unknown key `colour`",
        ),
        (json!({"marks": [{"type": "bar", "color": "red"}]}), "#RRGGBB"),
        (json!({"marks": [{"type": "bar", "y": "category"}]}), "marks[0].y"),
        (json!({"marks": [{"type": "bar", "y": "value.16"}]}), "value.N"),
        (
            json!({"marks": [{"type": "bar"}], "x": {"scale": "linear"}}),
            "band x scale",
        ),
        (json!({"marks": [{"type": "point", "x": "value"}]}), "linear"),
        (json!({"marks": [{"type": "rule"}]}), "a rule takes"),
        (json!({"marks": [{"type": "text"}]}), "label"),
        (
            json!({"marks": [{"type": "rect", "x": "value"}], "x": {"scale": "linear"}}),
            "x2",
        ),
        (json!({"version": 2, "marks": [{"type": "bar"}]}), "spec.version"),
        (
            json!({"padding": 1e9, "marks": [{"type": "bar"}]}),
            "spec.padding",
        ),
        (
            json!({"marks": [{"type": "bar", "name": "x".repeat(300)}]}),
            "200 characters",
        ),
        (
            json!({"marks": [{"type": "point", "x": {"value": "2024-99-01"}}], "x": {"scale": "time"}}),
            "ISO",
        ),
    ];
    for (v, want) in cases {
        let e = parse(&v).expect_err(&v.to_string());
        assert!(e.contains(want), "{v}: {e}");
    }
    let marks: Vec<Value> = (0..17).map(|_| json!({"type": "bar"})).collect();
    assert!(parse(&json!({"marks": marks})).is_err());
    let big = format!(
        "{{\"marks\": [{{\"type\": \"bar\", \"name\": \"{}\"}}]}}",
        " ".repeat(MAX_SPEC_BYTES)
    );
    assert!(parse_text(&big).unwrap_err().contains("bytes"));
    assert!(parse_text("{").unwrap_err().contains("JSON"));
}

/// Hostile sizes and values never panic and keep the output bounded.
#[test]
fn hostile_layouts_are_bounded() {
    let s = parse(&json!({"y": {"min": -1e15, "max": 1e15}, "x": {"scale": "linear"},
        "marks": [{"type": "point", "x": "value", "y": "value.1", "size": {"field": "value", "range": [0, 200]}}]}))
    .unwrap();
    let d = SpecData {
        cat_format: String::new(),
        lang: String::new(),
        cats: Vec::new(),
        values: vec![
            (0..5000).map(|i| Some(i as f64 * 1e12)).collect(),
            vec![Some(f64::NAN), None],
        ],
        highlights: Vec::new(),
        ..SpecData::default()
    };
    for (w, h) in [
        (0.0, 0.0),
        (1e9, 1e9),
        (f64::NAN, 10.0),
        (300.0, 200.0),
        (-5.0, 100.0),
    ] {
        let p = layout(&s, &d, w, h);
        assert!(p.len() <= MAX_ROWS * 2 + 4 * MAX_TICKS, "{w}x{h}: {}", p.len());
    }
    let one = parse(&json!({"marks": [{"type": "bar"}], "y": {"min": 5, "max": 5}})).unwrap();
    assert!(!layout(&one, &data(), 300.0, 200.0).is_empty());
}

/// A dimmed mark's edge fades with it, as the element opacity fades the SVG stroke in Power BI
/// (a Scatter+ point left out by a highlight looked hollow in BIcraft, faded in Desktop
/// 2.159.1231.0, 2026-11-25).
#[test]
fn a_dimmed_mark_fades_its_edge_too() {
    let deco = Deco {
        stroke: "#000000".into(),
        sw: 2.0,
        ..Deco::default()
    };
    let darkest = |alpha: f64| {
        let mut cv = crate::canvas::Canvas::new(40.0, 40.0, 1.0).expect("canvas");
        cv.fill(crate::canvas::Rgb::WHITE, 1.0);
        let prims = [Prim::Circle {
            cx: 20.0,
            cy: 20.0,
            r: 10.0,
            fill: "#FFFFFF".into(),
            opacity: 0.0,
            deco: deco.clone(),
        }];
        let fonts = crate::fonts::FontBook::bundled_only();
        let snap = json!({"version": 1, "model": {"culture": "en-US", "tables": []}, "report": {"theme": null, "pages": []}});
        let ctx = crate::data::Ctx::new(&snap, &fonts, None);
        draw_with(&mut cv, &prims, &ctx, &|_| alpha);
        cv.pm.pixels().iter().map(|p| p.red()).min().unwrap_or(255)
    };
    assert!(darkest(1.0) < 40, "{}", darkest(1.0));
    // at 0.4 the black edge is at most 40 % dark over white
    assert!(darkest(0.4) > 140, "{}", darkest(0.4));
}
