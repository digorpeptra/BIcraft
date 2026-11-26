//! Tests of grid and shelf placement, `visual.place`, `page.arrange` and the layout checks of
//! `report.validate`.

use serde_json::{Value, json};

use super::{Rect, place_all};
use crate::Session;

fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
    Rect { x, y, width, height }
}

/// `place_all` on a 1280 x 720 page.
fn place(layout: Value, items: &[Value]) -> Result<Vec<Rect>, String> {
    let refs: Vec<Option<&Value>> = items.iter().map(Some).collect();
    place_all(1280.0, 720.0, Some(&layout), &refs)
}

fn card() -> Value {
    json!({"visual_type": "card"})
}

fn chart() -> Value {
    json!({"visual_type": "clusteredColumnChart"})
}

fn assert_whole(rects: &[Rect]) {
    for r in rects {
        for n in [r.x, r.y, r.width, r.height] {
            assert!(n.is_finite() && n.fract() == 0.0, "{r:?}");
        }
        assert!(r.width >= 1.0 && r.height >= 1.0, "{r:?}");
    }
}

fn overlap(a: &Rect, b: &Rect) -> bool {
    a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
}

// ---------------------------------------------------------------------------------------------
// Shelf packing
// ---------------------------------------------------------------------------------------------

#[test]
fn shelf_packs_cards_then_charts() {
    let mut items = vec![card(), card(), card(), card()];
    items.extend([chart(), chart(), chart(), chart()]);
    let r = place(json!("shelf"), &items).unwrap();
    // Content area 1248 x 688; cards 25% x 20% (300 x 124), charts 50% x 40% (616 x 265).
    assert_eq!(r[0], rect(16.0, 16.0, 300.0, 124.0));
    assert_eq!(r[1], rect(332.0, 16.0, 300.0, 124.0));
    assert_eq!(r[3], rect(964.0, 16.0, 300.0, 124.0));
    assert_eq!(r[4], rect(16.0, 156.0, 616.0, 265.0));
    assert_eq!(r[5], rect(648.0, 156.0, 616.0, 265.0));
    assert_eq!(r[6], rect(16.0, 437.0, 616.0, 265.0));
    assert_eq!(r[7], rect(648.0, 437.0, 616.0, 265.0));
    assert_whole(&r);
    // The default layout is the same shelf packing.
    let refs: Vec<Option<&Value>> = items.iter().map(Some).collect();
    assert_eq!(place_all(1280.0, 720.0, None, &refs).unwrap(), r);
    // No type: chart size.
    assert_eq!(place_all(1280.0, 720.0, None, &[None]).unwrap()[0].width, 616.0);
}

#[test]
fn shelf_overflow_names_the_item_and_the_missing_room() {
    let items = vec![chart(); 7];
    let e = place(json!("shelf"), &items).unwrap_err();
    assert!(e.contains("item 4"), "{e}");
    assert!(e.contains("139 px missing"), "{e}");

    let e = place(json!("shelf"), &[json!({"width": 2000, "height": 100})]).unwrap_err();
    assert!(e.contains("item 0") && e.contains("752 px missing"), "{e}");
}

#[test]
fn shelf_sizes_in_pixels_and_percent() {
    let r = place(
        json!({"shelf": {"margin": 0, "gutter": 10}}),
        &[
            json!({"width": "50%", "height": 100}),
            json!({"width": "50%", "height": "10%"}),
            json!({"width": 99.6, "height": 50.4}),
        ],
    )
    .unwrap();
    // 50% + gutter + 50% fills the 1280 px shelf exactly.
    assert_eq!(r[0], rect(0.0, 0.0, 635.0, 100.0));
    assert_eq!(r[1], rect(645.0, 0.0, 635.0, 63.0));
    assert_eq!(r[2], rect(0.0, 110.0, 100.0, 50.0));
    // Four 25% items share one shelf.
    let r = place(json!("shelf"), &vec![json!({"width": "25%", "height": 50}); 4]).unwrap();
    assert!(r.iter().all(|x| x.y == 16.0), "{r:?}");
    assert!(r[3].x + r[3].width <= 1264.0);
}

#[test]
fn fixed_rectangles_pass_through_and_must_be_on_the_page() {
    let r = place(
        json!("shelf"),
        &[json!({"x": 10.4, "y": 20.6, "width": 100, "height": 50}), chart()],
    )
    .unwrap();
    assert_eq!(r[0], rect(10.0, 21.0, 100.0, 50.0));
    assert_eq!(r[1], rect(16.0, 16.0, 616.0, 265.0));
    let e = place(
        json!("shelf"),
        &[json!({"x": 1200, "y": 0, "width": 100, "height": 50})],
    )
    .unwrap_err();
    assert!(e.contains("does not lie on"), "{e}");
    for bad in [
        json!({"x": 0, "width": 10, "height": 10}),
        json!({"x": 0, "y": 0, "width": "10%", "height": 10}),
        json!({"x": 0, "y": 0, "width": 10, "height": 10, "col": 1}),
    ] {
        assert!(place(json!("grid"), std::slice::from_ref(&bad)).is_err(), "{bad}");
    }
}

// ---------------------------------------------------------------------------------------------
// Grid
// ---------------------------------------------------------------------------------------------

#[test]
fn grid_cells_have_exact_gutters() {
    let items: Vec<Value> = (0..12).map(|c| json!({"col": c, "row": c % 8})).collect();
    let r = place(json!("grid"), &items).unwrap();
    assert_whole(&r);
    for pair in r.windows(2) {
        assert_eq!(pair[0].x + pair[0].width + 16.0, pair[1].x, "{pair:?}");
    }
    assert_eq!(r[0].x, 16.0);
    assert_eq!(r[11].x + r[11].width, 1264.0);
    // Full width, three rows: rows are 72 px plus 16 px gutters.
    let r = place(
        json!("grid"),
        &[json!({"col": 0, "row": 0, "col_span": 12, "row_span": 3})],
    )
    .unwrap();
    assert_eq!(r[0], rect(16.0, 16.0, 1248.0, 248.0));
    // Odd page sizes still give whole, non-overlapping cells.
    let refs: Vec<Value> = (0..7).map(|c| json!({"col": c, "row": 0})).collect();
    let refs: Vec<Option<&Value>> = refs.iter().map(Some).collect();
    let layout = json!({"grid": {"columns": 7, "rows": 3, "margin": 5.4, "gutter": 0}});
    let r = place_all(1279.7, 719.3, Some(&layout), &refs).unwrap();
    assert_whole(&r);
    for pair in r.windows(2) {
        assert_eq!(pair[0].x + pair[0].width, pair[1].x, "{pair:?}");
    }
}

#[test]
fn grid_auto_flow_fills_rows_around_explicit_cells() {
    let mut items = vec![card(), card(), card(), card()];
    items.extend([chart(), chart(), chart(), chart()]);
    let r = place(json!("grid"), &items).unwrap();
    // Cards: 3 x 2 cells on row 0; charts: 6 x 3 cells on rows 2 and 5.
    let cell = |c: f64| (16.0 + c * (1072.0 / 12.0 + 16.0)).round();
    assert_eq!((r[0].x, r[0].y), (16.0, 16.0));
    assert_eq!((r[1].x, r[3].x), (cell(3.0), cell(9.0)));
    assert_eq!((r[4].x, r[4].y), (16.0, 192.0));
    assert_eq!((r[5].x, r[5].y), (cell(6.0), 192.0));
    assert_eq!((r[7].x, r[7].y), (cell(6.0), 456.0));
    assert_eq!(r[7].y + r[7].height, 704.0);
    for (i, a) in r.iter().enumerate() {
        for b in r.iter().skip(i + 1) {
            assert!(!overlap(a, b), "{a:?} {b:?}");
        }
    }
    items.push(chart());
    let e = place(json!("grid"), &items).unwrap_err();
    assert!(e.contains("item 8") && e.contains("no free area"), "{e}");

    // An explicit cell reserves its area first, even when listed after an automatic item.
    let r = place(
        json!("grid"),
        &[chart(), json!({"col": 0, "row": 0, "col_span": 6, "row_span": 3})],
    )
    .unwrap();
    assert_eq!((r[1].x, r[1].y), (16.0, 16.0));
    assert_eq!((r[0].x, r[0].y), (cell(6.0), 16.0));
    // Spans scale with the grid.
    let r = place(json!({"grid": {"columns": 24, "rows": 16}}), &[card()]).unwrap();
    let r12 = place(json!("grid"), &[card()]).unwrap();
    assert!((r[0].width - r12[0].width).abs() <= 1.0, "{r:?} {r12:?}");
}

#[test]
fn grid_rejects_out_of_grid_and_overlapping_cells() {
    let e = place(json!("grid"), &[json!({"col": 10, "row": 0, "col_span": 3})]).unwrap_err();
    assert!(e.contains("columns 10 to 12") && e.contains("0 to 11"), "{e}");
    let e = place(json!("grid"), &[json!({"col": 0, "row": 7, "row_span": 2})]).unwrap_err();
    assert!(e.contains("rows 7 to 8"), "{e}");
    let e = place(
        json!("grid"),
        &[
            json!({"col": 0, "row": 0, "col_span": 4, "row_span": 2}),
            json!({"col": 3, "row": 1}),
        ],
    )
    .unwrap_err();
    assert!(
        e.contains("item 1") && e.contains("already taken by item 0"),
        "{e}"
    );
    let e = place(json!("grid"), &[json!({"col_span": 13})]).unwrap_err();
    assert!(e.contains("13"), "{e}");
    let e = place(json!("grid"), &[json!({"width": 100})]).unwrap_err();
    assert!(e.contains("cannot be used in a grid"), "{e}");
    let e = place(json!("shelf"), &[json!({"col": 0, "row": 0})]).unwrap_err();
    assert!(e.contains("need a grid layout"), "{e}");
}

// ---------------------------------------------------------------------------------------------
// Hostile inputs
// ---------------------------------------------------------------------------------------------

#[test]
fn hostile_pages_and_layouts_are_errors() {
    for (w, h) in [
        (f64::NAN, 720.0),
        (1280.0, f64::INFINITY),
        (-1.0, 720.0),
        (0.0, 720.0),
        (1e308, 720.0),
    ] {
        assert!(place_all(w, h, None, &[None]).is_err(), "{w} x {h}");
    }
    // An empty page list of items is fine, even on a tiny page.
    assert_eq!(place_all(40.0, 40.0, None, &[]).unwrap(), vec![]);
    for layout in [
        json!("diagonal"),
        json!(3),
        json!([]),
        json!({"grid": {}, "shelf": {}}),
        json!({"grid": 4}),
        json!({"grid": {"columns": 0}}),
        json!({"grid": {"columns": 1e9}}),
        json!({"grid": {"columns": 2.5}}),
        json!({"grid": {"rows": -1}}),
        json!({"grid": {"columns": 100, "rows": 100}}),
        json!({"grid": {"cols": 4}}),
        json!({"shelf": {"margin": -1}}),
        json!({"shelf": {"margin": 1e12}}),
        json!({"shelf": {"margin": 640}}),
        json!({"shelf": {"gutter": "16"}}),
    ] {
        assert!(place(layout.clone(), &[chart()]).is_err(), "{layout}");
    }
    // `{}` options and float-valued counts are accepted.
    assert!(place(json!({"grid": {"columns": 4.0, "rows": null}}), &[card()]).is_ok());
    assert!(place(json!({"shelf": null}), &[card()]).is_ok());
}

#[test]
fn hostile_items_are_errors() {
    for item in [
        json!("card"),
        json!(5),
        json!({"width": "abc%"}),
        json!({"width": "150%"}),
        json!({"width": "0%"}),
        json!({"width": "-5%"}),
        json!({"width": -5}),
        json!({"width": 0}),
        json!({"height": 1e9}),
        json!({"col": 1}),
        json!({"col": -1, "row": 0}),
        json!({"col": 0, "row": 0, "col_span": 0}),
        json!({"col": 0, "row": 0, "col_span": 1e12}),
        json!({"col": 0, "row": 0, "width": 10}),
        json!({"colour": "red"}),
        json!({"visual_type": 3}),
        json!({"x": 1e9, "y": 0, "width": 10, "height": 10}),
    ] {
        assert!(
            place(json!("grid"), std::slice::from_ref(&item)).is_err(),
            "{item}"
        );
        assert!(
            place(json!("shelf"), std::slice::from_ref(&item)).is_err(),
            "{item}"
        );
    }
    let too_many: Vec<Option<&Value>> = vec![None; 10_001];
    assert!(place_all(1280.0, 720.0, None, &too_many).is_err());
}

#[test]
fn a_thousand_items() {
    // Shelf: 1000 small tiles on a big page.
    let items = vec![json!({"width": 40, "height": 40}); 1000];
    let refs: Vec<Option<&Value>> = items.iter().map(Some).collect();
    let r = place_all(
        2000.0,
        2000.0,
        Some(&json!({"shelf": {"margin": 0, "gutter": 0}})),
        &refs,
    )
    .unwrap();
    assert_eq!(r.len(), 1000);
    assert_whole(&r);
    assert_eq!(r[999], rect(1960.0, 760.0, 40.0, 40.0));
    // On a normal page they do not fit: the error says how much room is missing.
    let e = place_all(1280.0, 720.0, None, &refs).unwrap_err();
    assert!(e.contains("px missing"), "{e}");
    // Grid: 10 000 single cells fill a 100 x 100 grid exactly.
    let items = vec![json!({"col_span": 1, "row_span": 1}); 10_000];
    let refs: Vec<Option<&Value>> = items.iter().map(Some).collect();
    let layout = json!({"grid": {"columns": 100, "rows": 100, "margin": 0, "gutter": 0}});
    let r = place_all(10_000.0, 10_000.0, Some(&layout), &refs).unwrap();
    assert_eq!(r[9_999], rect(9_900.0, 9_900.0, 100.0, 100.0));
}

// ---------------------------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------------------------

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// A command that must fail and leave the report untouched.
fn fails(s: &mut Session, id: &str, params: Value) -> String {
    let before = s.report.clone();
    let err = s
        .execute(id, &params)
        .expect_err(&format!("{id} {params} should fail"));
    assert_eq!(s.report, before, "{id} changed the report although it failed");
    err
}

/// A blank report and its page name.
fn report() -> (Session, String) {
    let mut s = Session::new();
    run(&mut s, "report.new", json!({}));
    s.report_dirty = false;
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    (s, page)
}

fn add(s: &mut Session, page: &str, visual_type: &str, x: f64, y: f64) -> String {
    let v = run(
        s,
        "visual.add",
        json!({"page": page, "visual_type": visual_type, "x": x, "y": y, "width": 300, "height": 200}),
    );
    v["visual"].as_str().unwrap().to_string()
}

fn position(s: &Session, page: &str, visual: &str) -> bicraft_report::Position {
    s.report
        .as_ref()
        .unwrap()
        .page(page)
        .unwrap()
        .visual(visual)
        .unwrap()
        .position
}

fn validate(s: &mut Session) -> Value {
    run(s, "report.validate", json!({}))
}

fn messages(v: &Value) -> Vec<String> {
    v["issues"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["message"].as_str().unwrap_or("").to_string())
        .collect()
}

fn overlaps(v: &Value) -> usize {
    messages(v).iter().filter(|m| m.contains("overlaps")).count()
}

#[test]
fn visual_place_puts_a_visual_on_a_cell() {
    let (mut s, page) = report();
    let v = add(&mut s, &page, "card", 5.0, 5.0);
    let out = run(
        &mut s,
        "visual.place",
        json!({"page": page, "visual": v, "col": 0, "row": 0, "col_span": 6, "row_span": 3}),
    );
    assert!(s.report_dirty);
    let p = position(&s, &page, &v);
    assert_eq!((p.x, p.y, p.width, p.height), (16.0, 16.0, 616.0, 248.0));
    assert_eq!(out["position"]["width"], 616.0);
    assert_eq!(out["cell"]["col_span"], 6);
    assert_eq!(out["layout"]["grid"]["columns"], 12);
    // A custom grid, and the z order is kept.
    let z = p.z;
    run(
        &mut s,
        "visual.place",
        json!({"page": page, "visual": v, "col": 1, "row": 1, "grid": {"columns": 2, "rows": 2, "margin": 0, "gutter": 0}}),
    );
    let p = position(&s, &page, &v);
    assert_eq!(
        (p.x, p.y, p.width, p.height, p.z),
        (640.0, 360.0, 640.0, 360.0, z)
    );
    // Overlaps with other visuals come back as warnings.
    let other = add(&mut s, &page, "card", 0.0, 0.0);
    let out = run(
        &mut s,
        "visual.place",
        json!({"page": page, "visual": other, "col": 0, "row": 0, "col_span": 12, "row_span": 8}),
    );
    assert_eq!(out["warnings"].as_array().unwrap().len(), 1, "{out}");

    for bad in [
        json!({"page": page, "visual": v, "col": 12, "row": 0}),
        json!({"page": page, "visual": v, "col": 0}),
        json!({"page": page, "visual": v, "col": -1, "row": 0}),
        json!({"page": page, "visual": v, "col": 0, "row": 0, "col_span": 0}),
        json!({"page": page, "visual": v, "col": 0, "row": 0, "grid": {"columns": 0}}),
        json!({"page": page, "visual": v, "col": 0, "row": 0, "grid": "big"}),
        json!({"page": page, "visual": "nope", "col": 0, "row": 0}),
        json!({"page": "nope", "visual": v, "col": 0, "row": 0}),
        json!({"page": page, "visual": v, "col": 0, "row": 0, "x": 4}),
    ] {
        fails(&mut s, "visual.place", bad);
    }
}

#[test]
fn page_arrange_lays_out_every_visual_and_clears_overlaps() {
    let (mut s, page) = report();
    // Reading order: b (top), then a and c on the same row, left to right.
    let a = add(&mut s, &page, "clusteredColumnChart", 50.0, 300.0);
    let b = add(&mut s, &page, "card", 400.0, 100.0);
    let c = add(&mut s, &page, "tableEx", 100.0, 300.0);
    let before = validate(&mut s);
    assert!(overlaps(&before) >= 1, "{before}");
    s.report_dirty = false;

    // Dry run: positions come back, nothing changes.
    let saved = s.report.clone();
    let dry = run(&mut s, "page.arrange", json!({"page": page, "dry_run": true}));
    assert_eq!(s.report, saved);
    assert!(!s.report_dirty);
    assert_eq!(dry["dry_run"], true);
    let order: Vec<&str> = dry["arranged"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["visual"].as_str().unwrap())
        .collect();
    assert_eq!(order, [b.as_str(), a.as_str(), c.as_str()]);

    let out = run(&mut s, "page.arrange", json!({"page": page}));
    assert!(s.report_dirty);
    assert_eq!(out["arranged"], dry["arranged"]);
    assert_eq!(out["warnings"], json!([]));
    let pb = position(&s, &page, &b);
    assert_eq!((pb.x, pb.y, pb.width, pb.height), (16.0, 16.0, 300.0, 124.0));
    let pa = position(&s, &page, &a);
    assert_eq!((pa.x, pa.y), (332.0, 16.0));
    let pc = position(&s, &page, &c);
    assert_eq!((pc.x, pc.y), (16.0, 297.0));
    let after = validate(&mut s);
    assert_eq!(overlaps(&after), 0, "{after}");
    assert!(
        after["warning_count"].as_u64().unwrap() < before["warning_count"].as_u64().unwrap(),
        "{before} {after}"
    );

    // A grid, with listed visuals and explicit cells; unlisted visuals stay where they are.
    let out = run(
        &mut s,
        "page.arrange",
        json!({"page": page, "layout": {"grid": {"columns": 4, "rows": 2}},
               "visuals": [{"visual": a, "place": {"col": 2, "row": 0, "col_span": 2, "row_span": 2}}, c]}),
    );
    assert_eq!(out["arranged"].as_array().unwrap().len(), 2);
    assert_eq!(position(&s, &page, &b), pb);
    assert_eq!(position(&s, &page, &a).x, 648.0);
    // c flowed into the first free cells (a chart-sized span scaled to 4 x 2: 2 x 1).
    let pc = position(&s, &page, &c);
    assert_eq!((pc.x, pc.y), (16.0, 16.0));
    // b now sits under c: the overlap is reported.
    assert_eq!(out["warnings"].as_array().unwrap().len(), 1, "{out}");
}

#[test]
fn page_arrange_errors_change_nothing() {
    let (mut s, page) = report();
    let a = add(&mut s, &page, "card", 0.0, 0.0);
    let charts: Vec<String> = (0..7)
        .map(|i| add(&mut s, &page, "lineChart", 0.0, f64::from(i) + 1.0))
        .collect();
    let e = fails(&mut s, "page.arrange", json!({"page": page}));
    assert!(e.contains("px missing"), "{e}");
    assert!(e.contains(&charts[3]), "{e}");
    for bad in [
        json!({"page": page, "visuals": [a, a]}),
        json!({"page": page, "visuals": ["nope"]}),
        json!({"page": page, "visuals": [3]}),
        json!({"page": page, "visuals": "all"}),
        json!({"page": page, "visuals": [{"visual": a, "place": {"col": 0}}]}),
        json!({"page": page, "visuals": [{"visual": a, "where": {}}]}),
        json!({"page": page, "layout": "spiral"}),
        json!({"page": page, "dry_run": "yes"}),
        json!({"page": page, "extra": 1}),
    ] {
        fails(&mut s, "page.arrange", bad);
    }
    // An empty list arranges nothing.
    let out = run(&mut s, "page.arrange", json!({"page": page, "visuals": []}));
    assert_eq!(out["arranged"], json!([]));
}

#[test]
fn page_arrange_moves_group_members_and_refuses_them_alone() {
    let (mut s, page) = report();
    let a = add(&mut s, &page, "card", 100.0, 100.0);
    let b = add(&mut s, &page, "card", 400.0, 100.0);
    {
        let p = s.report.as_mut().unwrap().page_mut(&page).unwrap();
        p.visuals.push(bicraft_report::Visual {
            name: "grp".into(),
            position: bicraft_report::Position {
                x: 100.0,
                y: 100.0,
                width: 600.0,
                height: 200.0,
                ..Default::default()
            },
            ..Default::default()
        });
        p.visual_mut(&a).unwrap().parent_group = Some("grp".into());
        p.visual_mut(&b).unwrap().parent_group = Some("grp".into());
    }
    // Group members do not overlap their group.
    assert_eq!(overlaps(&validate(&mut s)), 0);
    let e = fails(&mut s, "page.arrange", json!({"page": page, "visuals": [a]}));
    assert!(e.contains("group"), "{e}");
    let out = run(
        &mut s,
        "page.arrange",
        json!({"page": page, "layout": {"grid": {"columns": 2, "rows": 2, "margin": 0, "gutter": 0}},
               "visuals": [{"visual": "grp", "place": {"col": 1, "row": 1}}]}),
    );
    assert_eq!(out["members_moved"], 2);
    // The group went from (100, 100, 600 x 200) to (640, 360, 640 x 360): members scale along.
    let pa = position(&s, &page, &a);
    assert_eq!((pa.x, pa.y, pa.width, pa.height), (640.0, 360.0, 320.0, 360.0));
    let pb = position(&s, &page, &b);
    assert_eq!((pb.x, pb.y), (960.0, 360.0));
}

#[test]
fn validate_reports_visuals_off_the_page() {
    let (mut s, page) = report();
    let base = validate(&mut s)["warning_count"].as_u64().unwrap();
    add(&mut s, &page, "card", 1100.0, 600.0);
    let v = validate(&mut s);
    let m = messages(&v);
    let off: Vec<&String> = m.iter().filter(|m| m.contains("does not fit")).collect();
    assert_eq!(off.len(), 1, "{v}");
    assert!(off[0].contains("120 px past the right edge"), "{v}");
    assert!(off[0].contains("80 px past the bottom edge"), "{v}");
    assert_eq!(v["warning_count"].as_u64().unwrap(), base + 1);
    assert_eq!(v["valid"], true);
    // Within a pixel is fine.
    let (mut s, page) = report();
    add(&mut s, &page, "card", 980.5, 520.0);
    let v = validate(&mut s);
    assert!(!messages(&v).iter().any(|m| m.contains("does not fit")), "{v}");
}

#[test]
fn validate_reports_overlaps_but_not_noise() {
    let (mut s, page) = report();
    let a = add(&mut s, &page, "card", 0.0, 0.0);
    // Touching, and overlapping by one pixel: not reported.
    add(&mut s, &page, "card", 300.0, 0.0);
    add(&mut s, &page, "card", 0.0, 199.0);
    let v = validate(&mut s);
    assert_eq!(overlaps(&v), 0, "{v}");
    // A real overlap: one warning, on the first visual, naming the other.
    let d = add(&mut s, &page, "card", 100.0, 50.0);
    let v = validate(&mut s);
    let issues: Vec<&Value> = v["issues"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["message"].as_str().unwrap().contains("overlaps"))
        .collect();
    assert_eq!(issues.len(), 3, "{v}");
    assert!(issues.iter().all(|i| i["severity"] == "warning"));
    assert!(
        issues
            .iter()
            .any(|i| i["visual"] == a.as_str() && i["other"] == d.as_str()),
        "{v}"
    );
    assert!(issues[0]["message"].as_str().unwrap().contains(" x "), "{v}");
    // Hidden visuals are ignored.
    s.report
        .as_mut()
        .unwrap()
        .page_mut(&page)
        .unwrap()
        .visual_mut(&d)
        .unwrap()
        .is_hidden = true;
    assert_eq!(overlaps(&validate(&mut s)), 0);
    // A background shape that holds a visual is ignored; a shape over part of one is not.
    run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "shape", "x": 0, "y": 0, "width": 1280, "height": 720}),
    );
    assert_eq!(overlaps(&validate(&mut s)), 0);
    run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "shape", "x": 250, "y": 150, "width": 100, "height": 100}),
    );
    assert!(overlaps(&validate(&mut s)) >= 1);
}

#[test]
fn validate_a_thousand_stacked_visuals_stays_bounded() {
    let (mut s, page) = report();
    let base = s.report_validation_for_tests()["warning_count"].as_u64().unwrap();
    {
        let p = s.report.as_mut().unwrap().page_mut(&page).unwrap();
        for i in 0..1000 {
            p.visuals.push(bicraft_report::Visual {
                name: format!("v{i}"),
                visual_type: Some("card".into()),
                position: bicraft_report::Position {
                    x: 10.0,
                    y: 10.0,
                    width: 100.0,
                    height: 100.0,
                    ..Default::default()
                },
                ..Default::default()
            });
        }
    }
    let started = std::time::Instant::now();
    let v = s.report_validation_for_tests();
    assert!(started.elapsed().as_secs() < 30);
    // 1000 overlaps plus a summary (past the capped issue list, but counted).
    assert_eq!(v["warning_count"].as_u64().unwrap(), base + 1001);
    assert!(v["issues"].as_array().unwrap().len() <= 500);

    // page.arrange on a grid big enough puts them side by side.
    let visuals: Vec<Value> = (0..1000)
        .map(|i| json!({"visual": format!("v{i}"), "place": {"col_span": 1, "row_span": 1}}))
        .collect();
    let out = run(
        &mut s,
        "page.arrange",
        json!({"page": page, "visuals": visuals,
               "layout": {"grid": {"columns": 40, "rows": 25, "margin": 0, "gutter": 0}}}),
    );
    assert_eq!(out["arranged"].as_array().unwrap().len(), 1000);
    assert_eq!(out["warnings"], json!([]));
    assert_eq!(overlaps(&s.report_validation_for_tests()), 0);
    // The default shelf packing cannot fit them and says so.
    let e = fails(&mut s, "page.arrange", json!({"page": page}));
    assert!(e.contains("px missing"), "{e}");
}

impl Session {
    /// `report.validate` without the PBIR schema pass (fast for big reports).
    fn report_validation_for_tests(&self) -> Value {
        crate::report::report_validation(self, false).unwrap()
    }
}
