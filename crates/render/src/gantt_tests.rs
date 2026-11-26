//! The Gantt visual (docs/formats/gantt.md): the lines of the list, the time line's units and
//! bounds, bars, milestones and progress where the layout puts them, collapsed categories,
//! shaded week days, the hit map, and hostile snapshots. Synthetic data only.

use std::collections::HashMap;

use serde_json::{Value, json};

use super::*;

fn data() -> Value {
    json!({"status": "ok",
        "columns": [
            {"name": "Phase", "role": "Category", "kind": "column", "data_type": "string"},
            {"name": "Task", "role": "Task", "kind": "column", "data_type": "string"},
            {"name": "Start", "role": "StartDate", "kind": "aggregation", "function": "Min", "data_type": "dateTime"},
            {"name": "End", "role": "EndDate", "kind": "aggregation", "function": "Max", "data_type": "dateTime"},
            {"name": "Done", "role": "Progress", "kind": "measure", "data_type": "double"},
            {"name": "Owner", "role": "Columns", "kind": "aggregation", "function": "Min", "data_type": "string"}],
        "rows": [
            ["Build", "Design", "2025-01-06", "2025-01-31", 1.0, "Ann"],
            ["Build", "Code", "2025-02-03", "2025-03-28", 0.4, "Bob"],
            ["Launch", "Review", "2025-04-01", null, null, "Ann"],
            ["Launch", "Ship", "2025-04-07", "2025-04-30", 0.0, "Cy"]]})
}

fn visual(gantt: Value, extra: Value) -> Value {
    let mut format = json!({"title": {"show": false}, "gantt": gantt});
    if let (Some(f), Value::Object(e)) = (format.as_object_mut(), extra) {
        f.extend(e);
    }
    json!({"name": "g", "type": "ganttChart", "x": 0, "y": 0, "width": 910, "height": 310,
           "format": format, "data": data(),
           "fields": [{"role": "Category", "kind": "column", "table": "T", "name": "Phase"},
                      {"role": "Task", "kind": "column", "table": "T", "name": "Task"},
                      {"role": "StartDate", "kind": "aggregation", "table": "T", "name": "Start", "function": "Min"},
                      {"role": "EndDate", "kind": "aggregation", "table": "T", "name": "End", "function": "Max"},
                      {"role": "Progress", "kind": "measure", "table": "T", "name": "Done"},
                      {"role": "Columns", "kind": "aggregation", "table": "T", "name": "Owner", "function": "Min"}]})
}

fn page(v: Value) -> Value {
    json!({"version": 1, "model": {"culture": "en-US", "tables": []},
           "report": {"theme": null, "pages": [{"name": "p1", "display_name": "Page 1", "width": 920, "height": 320, "visuals": [v]}]}})
}

fn run(s: &Value) -> PageImage {
    let fonts = FontBook::bundled_only();
    let o = RenderOptions {
        system_fonts: false,
        text_dump: true,
        hits: true,
        raw: true,
        ..RenderOptions::default()
    };
    render_page(s, 0, &HashMap::<String, Vec<u8>>::new(), &fonts, &o).unwrap()
}

/// The body's layout as the renderer computes it (the body is the visual less 5 px of padding).
fn lay(v: &Value) -> gantt::Layout {
    let fonts = FontBook::bundled_only();
    let s = page(v.clone());
    let ctx = data::Ctx::new(&s, &fonts, None);
    let ds = data::dataset(v, "gantt", &ctx);
    gantt::layout(v, &ds, 900.0, 300.0)
}

fn serial(iso: &str) -> f64 {
    gantt::serial(&json!(iso)).unwrap()
}

/// The (r, g, b) of page pixel (x, y), un-premultiplied over white.
fn px(img: &PageImage, x: f32, y: f32) -> (u8, u8, u8) {
    let i = ((y.round() as usize) * img.width as usize + x.round() as usize) * 4;
    (img.rgba[i], img.rgba[i + 1], img.rgba[i + 2])
}

fn lum((r, g, b): (u8, u8, u8)) -> u32 {
    u32::from(r) + u32::from(g) + u32::from(b)
}

#[test]
fn lines_follow_the_categories_and_the_time_line_is_scaled() {
    let l = lay(&visual(json!({}), json!({})));
    let kinds: Vec<(usize, Value)> = l
        .lines
        .iter()
        .map(|x| (x.level, x.path.last().cloned().unwrap_or(Value::Null)))
        .collect();
    assert_eq!(
        kinds,
        vec![
            (0, json!("Build")),
            (1, json!("Design")),
            (1, json!("Code")),
            (0, json!("Launch")),
            (1, json!("Review")),
            (1, json!("Ship"))
        ]
    );
    assert!(matches!(
        l.lines[0].kind,
        gantt::Kind::Category {
            level: 0,
            collapsed: false
        }
    ));
    assert!(matches!(l.lines[2].kind, gantt::Kind::Task { row: 1 }));
    // a category spans its tasks
    assert_eq!(
        (l.lines[0].start, l.lines[0].end),
        (Some(serial("2025-01-06")), Some(serial("2025-03-28")))
    );
    // about 120 days on ~670 px: one month level, from 1 January to 1 May
    assert_eq!(l.units, vec![gantt::Unit::Month]);
    assert_eq!(l.domain, Some((serial("2025-01-01"), serial("2025-05-01"))));
    assert_eq!(l.task_w, 150.0);
    assert_eq!(l.detail_w, vec![80.0]);
    // bars from start to end, a milestone for the task without an end
    let (x, w) = l.bar(&l.lines[1]).unwrap();
    assert!((x - l.x_of(serial("2025-01-06"))).abs() < 0.01);
    assert!((x + w - l.x_of(serial("2025-01-31"))).abs() < 0.01);
    assert!(l.bar(&l.lines[4]).is_none());
    assert!((l.milestone(&l.lines[4]).unwrap() - l.x_of(serial("2025-04-01"))).abs() < 0.01);
    // rows one under the other below the header
    assert_eq!(l.lines[0].y, l.header_h);
    assert_eq!(l.lines[1].y, l.header_h + l.row_h);
    assert_eq!(l.cut, 0);
}

#[test]
fn units_bounds_and_inversion_from_the_axis_object() {
    let fixed = lay(&visual(
        json!({"axis": {"auto_scale": false, "primary_unit": "year", "secondary_unit": "month"}}),
        json!({}),
    ));
    assert_eq!(fixed.units, vec![gantt::Unit::Year, gantt::Unit::Month]);
    assert_eq!(fixed.header_h, 40.0);
    let days = lay(&visual(
        json!({"axis": {"start": "2025-02-01", "end": "2025-02-21"}}),
        json!({}),
    ));
    assert_eq!(days.units, vec![gantt::Unit::Day]);
    assert_eq!(days.domain, Some((serial("2025-02-01"), serial("2025-02-21"))));
    let inv = lay(&visual(json!({"axis": {"invert": true}}), json!({})));
    assert!(inv.x_of(serial("2025-01-01")) > inv.x_of(serial("2025-04-01")));
    // labels: the year with the first month, then month names; the fixed levels' own words
    let img = run(&page(visual(json!({}), json!({}))));
    let t: Vec<String> = img.texts.iter().map(|r| r.text.clone()).collect();
    for want in [
        "Jan 2025", "Feb", "Apr", "Task", "Owner", "Design", "Bob", "Build",
    ] {
        assert!(t.iter().any(|x| x == want), "{want} in {t:?}");
    }
    let img = run(&page(visual(
        json!({"axis": {"auto_scale": false, "primary_unit": "year", "secondary_unit": "month", "format_month": "mmmm"}}),
        json!({}),
    )));
    let t: Vec<String> = img.texts.iter().map(|r| r.text.clone()).collect();
    for want in ["2025", "January", "March"] {
        assert!(t.iter().any(|x| x == want), "{want} in {t:?}");
    }
}

#[test]
fn collapsed_categories_hide_their_tasks() {
    let one = lay(&visual(
        json!({}),
        json!({"gantt_rows": [{"path": [{"c": 0, "v": "build"}], "collapsed": true}]}),
    ));
    let names: Vec<Value> = one.lines.iter().filter_map(|l| l.path.last().cloned()).collect();
    assert_eq!(
        names,
        vec![json!("Build"), json!("Launch"), json!("Review"), json!("Ship")]
    );
    assert!(matches!(
        one.lines[0].kind,
        gantt::Kind::Category { collapsed: true, .. }
    ));
    // all collapsed, one opened again
    let all = lay(&visual(
        json!({"row": {"collapsed": true}}),
        json!({"gantt_rows": [{"path": [{"c": 0, "v": "Launch"}], "collapsed": false}]}),
    ));
    assert_eq!(all.lines.len(), 4);
    // without categories: tasks only, no indentation
    let mut flat = visual(json!({}), json!({}));
    flat["data"]["columns"][0]["role"] = json!("Tooltips");
    let l = lay(&flat);
    assert_eq!(l.lines.len(), 4);
    assert!(
        l.lines
            .iter()
            .all(|x| x.level == 0 && matches!(x.kind, gantt::Kind::Task { .. }))
    );
}

#[test]
fn bars_progress_milestones_and_shading_are_drawn_where_laid_out() {
    let v = visual(
        json!({"axis": {"shading_show": true, "gridline_show": false}}),
        json!({}),
    );
    let l = lay(&v);
    let img = run(&page(v));
    assert_eq!(img.visuals[0].status, "drawn", "{:?}", img.visuals[0].note);
    let (ox, oy) = (5.0, 5.0);
    let code = &l.lines[2];
    let cy = oy + code.y + l.row_h / 2.0;
    let (x, w) = l.bar(code).unwrap();
    // 40 % done: the left part opaque, the rest lighter
    let done = px(&img, ox + x + w * 0.2, cy);
    let left = px(&img, ox + x + w * 0.8, cy);
    assert!(lum(done) + 60 < lum(left), "{done:?} {left:?}");
    assert!(lum(left) < 3 * 250, "{left:?}");
    // the milestone's centre is filled
    let review = &l.lines[4];
    let m = px(
        &img,
        ox + l.milestone(review).unwrap(),
        oy + review.y + l.row_h / 2.0,
    );
    assert!(lum(m) < 600, "{m:?}");
    // Saturday 8 February shaded, Wednesday 5 February not (Design's row: no bar there)
    let design = &l.lines[1];
    let y = oy + design.y + 3.0;
    let sat = px(&img, ox + l.x_of(serial("2025-02-08") + 0.5), y);
    let wed = px(&img, ox + l.x_of(serial("2025-02-05") + 0.5), y);
    assert!(lum(sat) < lum(wed), "{sat:?} {wed:?}");
    // the category row is tinted
    let cat = px(&img, ox + 60.0, oy + l.lines[0].y + 2.0);
    assert!(lum(cat) < 3 * 252, "{cat:?}");
}

#[test]
fn a_task_click_selects_it_with_its_category_and_shows_its_fields() {
    let v = visual(json!({}), json!({}));
    let l = lay(&v);
    let img = run(&page(v));
    let code = &l.lines[2];
    let (x, w) = l.bar(code).unwrap();
    let h = hits::hit_at(&img.hits, 5.0 + x + w / 2.0, 5.0 + code.y + l.row_h / 2.0).unwrap();
    let a = h.action_for(false).unwrap();
    assert_eq!(a["action"], "select");
    assert_eq!(a["point"], json!([{"c": 0, "v": "Build"}, {"c": 1, "v": "Code"}]));
    let labels: Vec<&str> = h.tip.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(labels, vec!["Phase", "Task", "Start", "End", "Done", "Owner"]);
    // a category row selects its member
    let cat = &l.lines[3];
    let a = hits::hit_at(&img.hits, 20.0, 5.0 + cat.y + 4.0)
        .and_then(|h| h.action_for(false))
        .unwrap();
    assert_eq!(a["point"], json!([{"c": 0, "v": "Launch"}]));
    // a selected task dims the others
    let mut sel = visual(json!({}), json!({}));
    sel["interaction"] = json!({"selected": {"points": [[{"c": 1, "v": "Code"}]]}});
    let img2 = run(&page(sel));
    let design = &l.lines[1];
    let (dx, dw) = l.bar(design).unwrap();
    let at = (5.0 + dx + dw / 2.0, 5.0 + design.y + l.row_h / 2.0);
    assert!(lum(px(&img2, at.0, at.1)) > lum(px(&img, at.0, at.1)) + 60);
}

#[test]
fn rows_below_the_body_are_counted_and_hostile_snapshots_do_not_crash() {
    let mut many = visual(json!({}), json!({}));
    let rows: Vec<Value> = (0..1500)
        .map(|i| json!(["P", format!("T{i}"), "2025-01-01", "2025-01-09", 0.5, "x"]))
        .collect();
    many["data"]["rows"] = Value::Array(rows);
    let img = run(&page(many.clone()));
    assert!(
        img.visuals[0]
            .note
            .as_deref()
            .is_some_and(|n| n.contains("not drawn")),
        "{:?}",
        img.visuals[0].note
    );
    let l = lay(&many);
    assert!(l.lines.len() + l.cut <= gantt::MAX_ROWS + 1);
    // no task field: the labelled placeholder
    let mut no_task = visual(json!({}), json!({}));
    no_task["data"]["columns"][1]["role"] = json!("Tooltips");
    assert_eq!(run(&page(no_task)).visuals[0].status, "placeholder");
    // hostile values
    for (gantt, extra, rows) in [
        (
            json!({"axis": {"start": "x", "end": 1e300, "auto_scale": "no", "primary_unit": 5, "shading_show": true, "shading_days": [99, -1, "a"]},
                   "bar": {"height": -5, "corner_radius": 1e9}, "row": {"indent": "x", "toggle_type": 99}}),
            json!({"gantt_rows": [{"path": [{"c": 1e30, "v": {}}]}, 5], "gantt_widths": [{"column": 0, "width": -1}]}),
            json!([
                [null, null, "9999-99-99", "0000-01-01", "x", null],
                [],
                7,
                ["a", "b", -1e308, 1e308, 1e9, 1]
            ]),
        ),
        (
            json!({"axis": {"auto_scale": false, "primary_unit": "day", "start": "0100-01-01", "end": "9999-12-31", "position": "both"}}),
            json!({}),
            json!([["A", "B", "2025-01-01", "2025-01-02", 0.1, "x"]]),
        ),
    ] {
        let mut v = visual(gantt, extra);
        v["data"]["rows"] = rows;
        let _ = run(&page(v));
    }
}
