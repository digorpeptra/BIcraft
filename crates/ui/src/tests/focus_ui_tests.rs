//! The visual header's reader tools (egui_kittest, no GPU), over an engine session run in the
//! test: focus mode from the header's icon and back (button and Escape), More options › Show as
//! a table (the visual over its data table), and More options › Sort axis sending
//! `visual.set_sort`, with the menu checking the sort the snapshot gives back; a reader's Sort
//! axis sends `view.interact` `sort` instead; More options › Spotlight dims the other visuals,
//! Escape or a click outside the visual ends it.

use std::path::Path;

use bicraft_engine::Session;
use bicraft_render::hits::HitShape;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use super::backstage_tests::{engine_shell, steps};
use super::{click, harness};
use crate::App;
use crate::focus::{Focus, TABLE_VISUAL};
use crate::ribbon::VisualSel;
use crate::shell::Caps;

/// Sales(Region, Amount) with a Total measure; on the first page a column chart by region
/// (`chart`) and a card (`card`).
fn session() -> Session {
    session_at(120)
}

/// [`session`] with both visuals at `y` (0: at the page's top, no room above for the header).
fn session_at(y: i64) -> Session {
    let mut s = Session::new();
    let sales = r#"#table({"Region", "Amount"}, {{"North", 100}, {"South", 50}, {"East", 75}})"#;
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Region", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        ("report.new", json!({})),
    ] {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    for (name, ty, x, fields) in [
        (
            "chart",
            "clusteredColumnChart",
            20,
            json!([{"role": "Category", "field": "Sales[Region]"}, {"role": "Y", "field": "[Total]"}]),
        ),
        (
            "card",
            "card",
            700,
            json!([{"role": "Values", "field": "[Total]"}]),
        ),
    ] {
        let r = s
            .execute(
                "visual.add",
                &json!({"page": page, "visual_type": ty, "x": x, "y": y, "width": 400,
                        "height": 250, "fields": fields}),
            )
            .unwrap_or_else(|e| panic!("visual.add: {e}"));
        let made = r["visual"].as_str().unwrap_or_default().to_string();
        if let Some(v) = s
            .report
            .as_mut()
            .and_then(|r| r.pages.first_mut())
            .and_then(|p| p.visuals.iter_mut().find(|v| v.name == made))
        {
            v.name = name.to_string();
        }
    }
    s.dirty = false;
    s.report_dirty = false;
    s
}

fn app() -> Harness<'static, App> {
    app_with(session)
}

fn app_with(make: fn() -> Session) -> Harness<'static, App> {
    let mut app = App::new("en-US", engine_shell(Caps::DESKTOP, make)).with_sync_render();
    app.open_path(Path::new("r.pbip"));
    let mut h = harness(app, (1280.0, 860.0));
    steps(&mut h, 6);
    h
}

fn page(h: &Harness<'_, App>) -> String {
    h.state()
        .loaded
        .as_ref()
        .and_then(|l| l.snap.pointer("/report/pages/0/name"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn select(h: &mut Harness<'_, App>, visual: &str) {
    let page = page(h);
    h.state_mut().cmd.visual = Some(VisualSel {
        page,
        visual: visual.to_string(),
    });
    steps(h, 3);
}

/// The visuals the page drawn holds (its hit map's visuals).
fn drawn(h: &Harness<'_, App>) -> Vec<String> {
    let mut v: Vec<String> = match h.state().shown.as_ref() {
        Some((_, Ok(img))) => img.visuals.iter().map(|n| n.name.clone()).collect(),
        _ => Vec::new(),
    };
    v.sort();
    v
}

/// The right edge of visual `name`'s rectangular hits, in report pixels.
fn right_edge(h: &Harness<'_, App>, name: &str) -> f32 {
    match h.state().shown.as_ref() {
        Some((_, Ok(img))) => img
            .hits
            .iter()
            .filter(|x| x.visual == name)
            .filter_map(|x| match x.shape {
                HitShape::Rect { x, w, .. } => Some(x + w),
                _ => None,
            })
            .fold(0.0, f32::max),
        _ => 0.0,
    }
}

/// The focus the snapshot shows.
fn focus(h: &Harness<'_, App>) -> Option<Focus> {
    crate::focus::current(h.state())
}

/// The last `view.interact` action sent.
fn last_action(h: &Harness<'_, App>) -> Value {
    h.state().last_action.clone().unwrap_or(Value::Null)
}

/// Whether the control labelled `name` is marked (selected or toggled on).
fn marked(h: &Harness<'_, App>, name: &str) -> bool {
    let n = h.get_by_label(name).accesskit_node();
    n.is_selected() == Some(true) || n.toggled() == Some(egui::accesskit::Toggled::True)
}

fn sent(h: &Harness<'_, App>) -> Vec<&'static str> {
    h.state().sent.iter().map(|c| c.id).collect()
}

#[test]
fn the_focus_layout_fills_the_page() {
    let h = app();
    let snap = h.state().loaded.as_ref().map(|l| l.snap.clone()).unwrap();
    let f = Focus {
        page: page(&h),
        visual: "chart".into(),
        table: false,
    };
    let s = crate::focus::snapshot(&snap, 0, &f).unwrap();
    let vs = s["report"]["pages"][0]["visuals"].as_array().unwrap();
    assert_eq!(vs.len(), 1);
    let v = &vs[0];
    let pw = s["report"]["pages"][0]["width"].as_f64().unwrap();
    assert_eq!((v["name"].as_str(), v["x"].as_f64()), (Some("chart"), Some(8.0)));
    assert_eq!(v["width"].as_f64(), Some(pw - 16.0));
    // as a table: the visual on top, its data under it in the visual's columns and rows
    let t = crate::focus::snapshot(
        &snap,
        0,
        &Focus {
            table: true,
            ..f.clone()
        },
    )
    .unwrap();
    let vs = t["report"]["pages"][0]["visuals"].as_array().unwrap();
    assert_eq!(vs.len(), 2);
    let (top, table) = (&vs[0], &vs[1]);
    assert_eq!(
        (table["name"].as_str(), table["type"].as_str()),
        (Some(TABLE_VISUAL), Some("tableEx"))
    );
    assert!(table["y"].as_f64() > Some(top["y"].as_f64().unwrap() + top["height"].as_f64().unwrap()));
    assert_eq!(table["data"]["rows"], top["data"]["rows"]);
    assert_eq!(table["data"]["columns"][0]["role"], "Values");
    // another page, or a visual that is not there: no focus
    assert!(
        crate::focus::snapshot(
            &snap,
            0,
            &Focus {
                page: "x".into(),
                ..f.clone()
            }
        )
        .is_none()
    );
    assert!(
        crate::focus::snapshot(
            &snap,
            0,
            &Focus {
                visual: "nope".into(),
                ..f
            }
        )
        .is_none()
    );
    // the snapshot itself is not changed
    assert_eq!(
        snap["report"]["pages"][0]["visuals"].as_array().map(Vec::len),
        Some(2)
    );
}

#[test]
fn focus_mode_from_the_header_and_back() {
    let mut h = app();
    assert_eq!(drawn(&h), ["card", "chart"]);
    select(&mut h, "chart");
    let before = h.state().sent.len();
    let pg = page(&h);
    h.get_by_label("Focus mode").click();
    steps(&mut h, 4);
    // one view.interact action, the state read back from the snapshot
    assert_eq!(
        last_action(&h),
        json!({"action": "focus", "page": pg, "visual": "chart"})
    );
    assert_eq!(
        focus(&h).map(|f| (f.visual, f.table)),
        Some(("chart".to_string(), false))
    );
    // the visual alone, filling the page; no command (the report does not change)
    assert_eq!(drawn(&h), ["chart"]);
    assert_eq!(h.state().sent.len(), before, "{:?}", sent(&h));
    // its columns spread over the page (the visual was 400 px wide at x = 20)
    assert!(right_edge(&h, "chart") > 900.0, "{}", right_edge(&h, "chart"));
    // the header now leaves focus mode; the bar's button too
    h.get_by_label("Exit focus mode");
    h.get_by_label("Back to report").click();
    steps(&mut h, 4);
    assert_eq!(
        last_action(&h),
        json!({"action": "focus", "page": pg, "visual": null})
    );
    assert!(focus(&h).is_none());
    assert_eq!(drawn(&h), ["card", "chart"]);
    // Escape leaves it too, the visual still selected
    h.get_by_label("Focus mode").click();
    steps(&mut h, 4);
    assert!(focus(&h).is_some());
    h.key_press(egui::Key::Escape);
    steps(&mut h, 4);
    assert_eq!(last_action(&h)["visual"], Value::Null);
    assert!(focus(&h).is_none());
    assert!(h.state().cmd.visual.is_some());
    assert_eq!(h.state().sent.len(), before, "{:?}", sent(&h));
}

#[test]
fn show_as_a_table_from_more_options() {
    let mut h = app();
    select(&mut h, "chart");
    let before = h.state().sent.len();
    h.get_by_label("More options").click();
    steps(&mut h, 2);
    h.get_by_label("Show as a table").click();
    steps(&mut h, 4);
    assert_eq!(
        last_action(&h),
        json!({"action": "show_table", "page": page(&h), "visual": "chart"})
    );
    assert_eq!(
        focus(&h).map(|f| (f.visual, f.table)),
        Some(("chart".to_string(), true))
    );
    assert_eq!(drawn(&h), [TABLE_VISUAL, "chart"]);
    // the table's rows are not clickable, and nothing was sent
    let (cell, placed) = {
        let app = h.state();
        let img = app.shown.as_ref().and_then(|(_, r)| r.as_ref().ok()).unwrap();
        let cell = img.hits.iter().find_map(|x| match x.shape {
            HitShape::Rect { x: hx, y, w, h } if x.visual == TABLE_VISUAL => {
                Some((hx + w / 2.0, y + h / 2.0))
            }
            _ => None,
        });
        (cell, app.placed.unwrap())
    };
    let (cx, cy) = cell.unwrap_or_else(|| panic!("the data table has no rows"));
    let p = placed.rect.min + egui::vec2(cx, cy) * placed.zoom;
    assert!(h.state().hit_at_screen(p).is_none());
    click(&mut h, p);
    assert_eq!(h.state().sent.len(), before, "{:?}", sent(&h));
    // the item is checked; unchecking it keeps the focus, without the table
    h.get_by_label("More options").click();
    steps(&mut h, 2);
    assert!(marked(&h, "Show as a table"));
    h.get_by_label("Show as a table").click();
    steps(&mut h, 4);
    assert_eq!(
        last_action(&h),
        json!({"action": "show_table", "page": page(&h), "visual": "chart", "on": false})
    );
    assert_eq!(drawn(&h), ["chart"]);
    assert_eq!(focus(&h).map(|f| f.table), Some(false));
}

#[test]
fn sort_axis_from_more_options_sends_set_sort() {
    let mut h = app();
    select(&mut h, "chart");
    let open_sort = |h: &mut Harness<'_, App>| {
        h.get_by_label("More options").click();
        steps(h, 2);
        // a submenu's button carries its arrow after the text
        let r = h
            .query_by(|n| n.label().is_some_and(|l| l.starts_with("Sort axis")))
            .unwrap_or_else(|| panic!("no Sort axis entry"))
            .rect();
        h.hover_at(r.center());
        steps(h, 4);
    };
    open_sort(&mut h);
    // the visual's fields, then the directions
    h.get_by_label("Region");
    h.get_by_label("Sort ascending");
    h.get_by_label("Total").click();
    steps(&mut h, 4);
    assert!(h.state().form.is_none(), "a form opened");
    let last = h.state().sent.last().map(|c| (c.id, c.params.clone()));
    assert_eq!(
        last,
        Some((
            "visual.set_sort",
            json!({"page": page(&h), "visual": "chart", "field": "[Total]", "direction": "desc"})
        ))
    );
    let sort = |h: &Harness<'_, App>| {
        h.state()
            .loaded
            .as_ref()
            .and_then(|l| {
                l.snap["report"]["pages"][0]["visuals"]
                    .as_array()?
                    .iter()
                    .find(|v| v["name"] == "chart")
                    .map(|v| v["sort"].clone())
            })
            .unwrap_or(Value::Null)
    };
    assert_eq!(sort(&h), json!([{"field": "[Total]", "descending": true}]));
    // the menu checks the field and the direction; ascending keeps the field
    open_sort(&mut h);
    assert!(marked(&h, "Total"));
    assert!(marked(&h, "Sort descending"));
    assert!(!marked(&h, "Sort ascending"));
    h.get_by_label("Sort ascending").click();
    steps(&mut h, 4);
    assert_eq!(sort(&h), json!([{"field": "[Total]", "descending": false}]));
    let p = h
        .state()
        .sent
        .last()
        .map(|c| c.params.clone())
        .unwrap_or(Value::Null);
    assert_eq!(
        (p["field"].as_str(), p["direction"].as_str()),
        (Some("[Total]"), Some("asc"))
    );
}

/// The mean brightness (0–255) of the page drawn inside report rectangle (x, y, w, h).
fn brightness(h: &Harness<'_, App>, x: f32, y: f32, w: f32, hh: f32) -> f32 {
    let app = h.state();
    let Some((_, Ok(img))) = app.shown.as_ref() else {
        return 0.0;
    };
    let pw = app
        .loaded
        .as_ref()
        .and_then(|l| l.snap.pointer("/report/pages/0/width"))
        .and_then(Value::as_f64)
        .unwrap_or(1280.0) as f32;
    let k = img.width as f32 / pw;
    let (mut sum, mut n) = (0.0f32, 0.0f32);
    for py in (y * k) as u32..((y + hh) * k) as u32 {
        for px in (x * k) as u32..((x + w) * k) as u32 {
            let i = ((py * img.width + px) * 4) as usize;
            if let Some(p) = img.rgba.get(i..i + 3) {
                sum += (p[0] as f32 + p[1] as f32 + p[2] as f32) / 3.0;
                n += 1.0;
            }
        }
    }
    sum / n.max(1.0)
}

/// The spotlight the snapshot shows: (page, visual).
fn spotlight(h: &Harness<'_, App>) -> Option<(String, String)> {
    crate::focus::spotlight(h.state())
}

/// A screen point at report position (x, y).
fn at(h: &Harness<'_, App>, x: f32, y: f32) -> egui::Pos2 {
    let placed = h.state().placed.unwrap();
    placed.rect.min + egui::vec2(x, y) * placed.zoom
}

#[test]
fn spotlight_from_more_options_dims_the_others_and_ends() {
    let mut h = app();
    select(&mut h, "chart");
    let pg = page(&h);
    let before = h.state().sent.len();
    // the card (x 700, y 120, 400 × 250) as drawn
    let card = |h: &Harness<'_, App>| brightness(h, 700.0, 120.0, 400.0, 250.0);
    let card_before = card(&h);
    let chart_before = brightness(&h, 20.0, 120.0, 400.0, 250.0);
    let spot_on = |h: &mut Harness<'_, App>| {
        h.get_by_label("More options").click();
        steps(h, 2);
        h.get_by_label("Spotlight").click();
        steps(h, 4);
    };
    spot_on(&mut h);
    assert_eq!(
        last_action(&h),
        json!({"action": "spotlight", "page": pg, "visual": "chart"})
    );
    assert_eq!(spotlight(&h), Some((pg.clone(), "chart".to_string())));
    // the card fades towards the page's white, the chart is drawn as before
    assert!(card(&h) > card_before + 1.0, "{} {card_before}", card(&h));
    assert!((brightness(&h, 20.0, 120.0, 400.0, 250.0) - chart_before).abs() < 0.5);
    // the menu checks it; nothing was sent (the report does not change)
    h.get_by_label("More options").click();
    steps(&mut h, 2);
    assert!(marked(&h, "Spotlight"));
    h.get_by_label("More options").click();
    steps(&mut h, 2);
    assert!(h.query_by_label("Spotlight").is_none(), "the menu stayed open");
    assert_eq!(h.state().sent.len(), before, "{:?}", sent(&h));
    // Escape ends it
    h.key_press(egui::Key::Escape);
    steps(&mut h, 4);
    assert_eq!(spotlight(&h), None);
    assert_eq!(last_action(&h)["visual"], Value::Null);
    assert!((card(&h) - card_before).abs() < 0.5);
    // a click inside the visual keeps it, one outside ends it (and selects nothing)
    select(&mut h, "chart");
    spot_on(&mut h);
    assert!(spotlight(&h).is_some());
    let inside = at(&h, 30.0, 130.0);
    click(&mut h, inside);
    steps(&mut h, 3);
    assert!(spotlight(&h).is_some());
    let outside = at(&h, 900.0, 245.0);
    click(&mut h, outside);
    steps(&mut h, 4);
    assert_eq!(spotlight(&h), None);
    assert_eq!(
        last_action(&h),
        json!({"action": "spotlight", "page": pg, "visual": null})
    );
    assert_eq!(h.state().sent.len(), before, "{:?}", sent(&h));
}

#[test]
fn a_reader_sorts_the_view_from_sort_axis() {
    let mut h = app();
    h.state_mut().access = Some("reader".into());
    select(&mut h, "chart");
    let before = h.state().sent.len();
    h.get_by_label("More options").click();
    steps(&mut h, 2);
    let r = h
        .query_by(|n| n.label().is_some_and(|l| l.starts_with("Sort axis")))
        .unwrap_or_else(|| panic!("no Sort axis entry"))
        .rect();
    h.hover_at(r.center());
    steps(&mut h, 4);
    h.get_by_label("Total").click();
    steps(&mut h, 4);
    // the reader's sort: a view.interact action, no report command
    assert_eq!(
        last_action(&h),
        json!({"action": "sort", "page": page(&h), "visual": "chart", "field": "[Total]",
               "direction": "desc"})
    );
    assert_eq!(h.state().sent.len(), before, "{:?}", sent(&h));
    let chart = h
        .state()
        .loaded
        .as_ref()
        .and_then(|l| {
            l.snap["report"]["pages"][0]["visuals"]
                .as_array()?
                .iter()
                .find(|v| v["name"] == "chart")
                .cloned()
        })
        .unwrap_or(Value::Null);
    assert_eq!(chart["sort"], json!([{"field": "[Total]", "descending": true}]));
    let regions: Vec<&str> = chart["data"]["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| r[0].as_str())
        .collect();
    assert_eq!(regions, ["North", "East", "South"]);
    assert_eq!(
        h.state()
            .loaded
            .as_ref()
            .and_then(|l| l.snap.pointer("/dirty/report").cloned()),
        Some(json!(false))
    );
}

/// The screen rectangle of the dark pixels (text) drawn in the top `band` report pixels of the
/// visual `name`: its title, on a visual at the page's top.
fn title_ink(h: &Harness<'_, App>, name: &str, band: f32) -> Option<egui::Rect> {
    let app = h.state();
    let v = app.loaded.as_ref().and_then(|l| {
        l.snap["report"]["pages"][0]["visuals"]
            .as_array()?
            .iter()
            .find(|v| v["name"] == name)
            .cloned()
    })?;
    let n = |k: &str| v[k].as_f64().unwrap_or_default() as f32;
    let ((key, Ok(img)), placed) = (app.shown.as_ref()?, app.placed?) else {
        return None;
    };
    let s = key.scale();
    let (w, hgt) = (img.width as usize, img.height as usize);
    let mut ink: Option<egui::Rect> = None;
    let (x0, x1) = ((n("x") * s) as usize, ((n("x") + n("width")) * s) as usize);
    let (y0, y1) = ((n("y") * s) as usize, ((n("y") + band) * s) as usize);
    for y in y0..y1.min(hgt) {
        for x in x0..x1.min(w) {
            let i = (y * w + x) * 4;
            let px = &img.rgba[i..i + 3];
            // text, even antialiased at a small scale, on the visual's white background
            if px.iter().map(|c| u32::from(*c)).sum::<u32>() < 3 * 240 {
                let p = egui::pos2(x as f32 / s, y as f32 / s);
                let r = egui::Rect::from_min_size(p, egui::vec2(1.0 / s, 1.0 / s));
                ink = Some(ink.map_or(r, |i| i.union(r)));
            }
        }
    }
    ink.map(|r| {
        egui::Rect::from_min_max(
            placed.rect.min + r.min.to_vec2() * placed.zoom,
            placed.rect.min + r.max.to_vec2() * placed.zoom,
        )
    })
}

/// A visual at the page's top has no room above it for its header: More options goes inside the
/// visual at its top right, as Power BI's header does, clear of the title drawn at the top left
/// and of the drill icons (drill.rs) right-aligned at its top; it stays a button the keyboard
/// reaches.
#[test]
fn the_header_inside_a_visual_leaves_its_title_clear() {
    let mut h = app_with(|| session_at(0));
    select(&mut h, "chart");
    steps(&mut h, 2);
    let title = title_ink(&h, "chart", 28.0).expect("the chart's title is drawn");
    let more = h.get_by_label("More options").rect();
    assert!(
        !more.intersects(title),
        "More options {more:?} covers the title {title:?}"
    );
    let placed = h.state().placed.unwrap();
    let chart = egui::Rect::from_min_size(placed.rect.min, egui::vec2(400.0, 250.0) * placed.zoom)
        .translate(egui::vec2(20.0, 0.0) * placed.zoom);
    // inside the visual, at its top right, clear of the drill icons
    assert!(chart.contains_rect(more), "{more:?} outside {chart:?}");
    assert!(more.center().x > chart.center().x, "{more:?} not at the right");
    // (the drill icons show on a visual with a drill hierarchy only)
    let drillable =
        crate::rich::snap_visual(h.state(), &page(&h), "chart").is_some_and(|v| v["drill"].is_object());
    for (_, r) in crate::drill::icon_rects(chart).into_iter().filter(|_| drillable) {
        assert!(
            !more.intersects(r),
            "More options {more:?} over a drill icon {r:?}"
        );
    }
    // still a button in the accessibility tree (egui's buttons take the keyboard's focus)
    let node = h.get_by_label("More options");
    assert_eq!(node.accesskit_node().role(), egui::accesskit::Role::Button);
}
