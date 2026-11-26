//! Golden images: synthetic snapshots (one page per visual family) rendered with the bundled
//! fonts only, compared with the PNGs under `tests/golden/` with a small per-pixel tolerance (so
//! the test passes on Windows and Linux alike). `BICRAFT_BLESS=1` rewrites the goldens.

use std::collections::HashMap;
use std::path::PathBuf;

use bicraft_render::{FontBook, RenderOptions, render_page};
use serde_json::{Value, json};

fn cols(c: &[(&str, &str, &str)]) -> Value {
    Value::Array(
        c.iter()
            .map(|(n, r, k)| json!({"name": n, "role": r, "kind": k}))
            .collect(),
    )
}

fn ok(columns: Value, rows: Value) -> Value {
    json!({"status": "ok", "columns": columns, "rows": rows})
}

fn v(name: &str, t: &str, x: i32, y: i32, w: i32, h: i32, data: Value) -> Value {
    json!({"name": name, "type": t, "x": x, "y": y, "z": 0, "width": w, "height": h,
           "fields": [{"role": "Y", "kind": "measure", "table": "Sales", "name": "Total"}], "data": data})
}

fn snapshot() -> Value {
    let regions = json!([
        ["North", 300.0],
        ["South", 240.0],
        ["East", 180.0],
        ["West", 120.0],
        ["Central", 65.0]
    ]);
    let by_cat = ok(
        cols(&[("Region", "Category", "column"), ("Total", "Y", "measure")]),
        regions.clone(),
    );
    let stacked_rows: Vec<Value> = ["North", "South", "East"]
        .iter()
        .enumerate()
        .flat_map(|(i, r)| {
            ["Bikes", "Clothing"]
                .iter()
                .enumerate()
                .map(move |(j, c)| json!([r, c, 100.0 - 20.0 * i as f64 + 30.0 * j as f64]))
        })
        .collect();
    let stacked = ok(
        cols(&[
            ("Region", "Category", "column"),
            ("Category", "Series", "column"),
            ("Total", "Y", "measure"),
        ]),
        Value::Array(stacked_rows),
    );
    let months: Vec<Value> = (1..=12)
        .map(|m| {
            json!([
                format!("2025-{m:02}-01"),
                50.0 + 10.0 * m as f64,
                30.0 + 5.0 * m as f64
            ])
        })
        .collect();
    let mut line_cols = cols(&[
        ("Month", "Category", "column"),
        ("Total", "Y", "measure"),
        ("Cost", "Y", "measure"),
    ]);
    line_cols[0]["data_type"] = json!("dateTime");
    let lines = ok(line_cols, Value::Array(months));
    let card = ok(cols(&[("Total", "Values", "measure")]), json!([[910047.0]]));
    let mut table = ok(
        cols(&[("Region", "Values", "column"), ("Total", "Values", "measure")]),
        regions.clone(),
    );
    table["cell_styles"] = json!([{"r": 0, "c": 1, "bar": {"from": 0, "to": 1, "color": "#5F6B6D"}},
                                  {"r": 1, "c": 1, "bar": {"from": 0, "to": 0.8, "color": "#5F6B6D"}, "icon": {"name": "arrowUp", "position": "before"}},
                                  {"r": 2, "c": 0, "bg": "#F8696B"}]);
    let matrix = ok(
        cols(&[
            ("Region", "Rows", "column"),
            ("Category", "Columns", "column"),
            ("Total", "Values", "measure"),
        ]),
        json!([
            ["North", "Bikes", 10],
            ["North", "Clothing", 20],
            ["South", "Bikes", 5]
        ]),
    );
    let slicer = ok(
        cols(&[("Category", "Values", "column")]),
        json!([["Bikes"], ["Clothing"], ["Accessories"]]),
    );
    let scatter = ok(
        cols(&[
            ("Region", "Category", "column"),
            ("Units", "X", "measure"),
            ("Total", "Y", "measure"),
        ]),
        json!([["a", 1, 5], ["b", 3, 2], ["c", 6, 8]]),
    );
    let gauge = ok(
        cols(&[("Margin", "Y", "measure"), ("Goal", "TargetValue", "measure")]),
        json!([[0.38, 0.35]]),
    );
    let pages = json!([
        {"name": "charts", "display_name": "Charts", "width": 960, "height": 540, "visuals": [
            v("c1", "clusteredColumnChart", 10, 10, 300, 250, by_cat.clone()),
            v("c2", "stackedBarChart", 320, 10, 300, 250, stacked.clone()),
            v("c3", "lineChart", 630, 10, 320, 250, lines.clone()),
            v("c4", "pieChart", 10, 280, 300, 250, by_cat.clone()),
            v("c5", "waterfallChart", 320, 280, 300, 250, by_cat.clone()),
            v("c6", "scatterChart", 630, 280, 320, 250, scatter),
        ]},
        {"name": "others", "display_name": "Others", "width": 960, "height": 540, "background": "#F3F2F1", "visuals": [
            v("o1", "card", 10, 10, 200, 120, card),
            v("o2", "gauge", 220, 10, 300, 200, gauge),
            v("o3", "tableEx", 530, 10, 420, 200, table),
            v("o4", "pivotTable", 10, 230, 400, 150, matrix),
            v("o5", "slicer", 420, 230, 200, 150, slicer),
            v("o6", "treemap", 630, 230, 320, 300, by_cat),
            json!({"name": "o7", "type": "textbox", "x": 10, "y": 400, "width": 400, "height": 120,
                   "text": {"paragraphs": [{"runs": [{"text": "Bold title", "bold": true, "font_size": 16}]},
                                           {"runs": [{"text": "A longer paragraph that wraps over more than one line in the box.", "color": "#118DFF"}]}]}}),
            json!({"name": "o8", "type": "keyDriversVisual", "x": 420, "y": 400, "width": 200, "height": 120, "title": "Not drawn"}),
        ]},
        {"name": "drill", "display_name": "Drill", "width": 960, "height": 540, "visuals": drill_visuals()},
        {"name": "gantt", "display_name": "Gantt", "width": 960, "height": 540, "visuals": gantt_visuals()},
        {"name": "dates", "display_name": "Dates", "width": 960, "height": 540, "visuals": date_visuals()},
        {"name": "numbers", "display_name": "Numbers", "width": 960, "height": 540, "visuals": number_visuals()},
    ]);
    json!({"version": 1, "model": {"culture": "en-US", "tables": []}, "report": {"theme": null, "pages": pages}})
}

/// Drilled visuals (the reader's drill state as the engine sends it): an axis expanded down a
/// level (concatenated labels), an axis drilled into one member (its children only), a matrix
/// with +/- icons, one member expanded.
fn drill_visuals() -> Value {
    let both = ok(
        cols(&[
            ("Category", "Category", "column"),
            ("Product", "Category", "column"),
            ("Total", "Y", "measure"),
        ]),
        json!([
            ["Toys", "Ball", 5.0],
            ["Toys", "Kite", 7.0],
            ["Wheels", "Bike", 150.5],
            ["Wheels", "Car", 300.0]
        ]),
    );
    let down = ok(
        cols(&[("Product", "Category", "column"), ("Total", "Y", "measure")]),
        json!([["Bike", 150.5], ["Car", 300.0]]),
    );
    let mut matrix = ok(
        cols(&[
            ("Category", "Rows", "column"),
            ("Product", "Rows", "column"),
            ("Total", "Values", "measure"),
        ]),
        json!([
            ["Toys", "Ball", 5.0],
            ["Toys", "Kite", 7.0],
            ["Wheels", "Bike", 150.5],
            ["Wheels", "Car", 300.0]
        ]),
    );
    matrix["row_expansion"] = json!({"open": [false, false], "toggled": [["Wheels"]]});
    let mut m = v("d3", "pivotTable", 10, 280, 400, 200, matrix);
    m["format"] = json!({"table": {"expand_buttons": true}});
    m["drill"] = json!({"role": "Rows", "matrix": true, "top": 0, "bottom": 1, "can_down": false});
    let mut d2 = v("d2", "clusteredColumnChart", 480, 10, 300, 250, down);
    d2["drill"] = json!({"role": "Category", "top": 1, "bottom": 1, "can_up": true, "path": [{"level": 0, "value": "Wheels"}]});
    json!([v("d1", "clusteredColumnChart", 10, 10, 450, 250, both), d2, m])
}

/// Charts on a date category (docs/render.md "Continuous date axes"): columns at irregular
/// `General Date` days on a continuous axis and the same set `Categorical`, a line whose rows
/// come out of time order, an area over hours, horizontal bars over quarters, a combo.
fn date_visuals() -> Value {
    let dated = |name: &str, fmt: &str, rows: Value| {
        let mut c = cols(&[(name, "Category", "column"), ("Total", "Y", "measure")]);
        c[0]["data_type"] = json!("dateTime");
        c[0]["format_string"] = json!(fmt);
        ok(c, rows)
    };
    let days = dated(
        "Day",
        "General Date",
        json!([
            ["2026-01-05T00:00:00", 120.0],
            ["2026-01-06T00:00:00", 90.0],
            ["2026-01-07T00:00:00", 140.0],
            ["2026-01-12T00:00:00", 60.0],
            ["2026-01-20T00:00:00", 110.0],
            ["2026-02-02T00:00:00", 80.0]
        ]),
    );
    let unsorted = dated(
        "Date",
        "Short Date",
        json!([
            ["2025-03-01", 30.0],
            ["2025-01-01", 10.0],
            ["2025-01-15", 25.0],
            ["2025-06-10", 18.0],
            ["2025-02-01", 40.0]
        ]),
    );
    let hours = dated(
        "Time",
        "General Date",
        json!([
            ["2026-01-05T00:00:00", 5.0],
            ["2026-01-05T03:00:00", 8.0],
            ["2026-01-05T06:00:00", 12.0],
            ["2026-01-05T15:00:00", 9.0],
            ["2026-01-05T18:00:00", 14.0],
            ["2026-01-06T06:00:00", 6.0]
        ]),
    );
    let quarters = dated(
        "Quarter",
        "Short Date",
        json!([
            ["2024-01-01", 50.0],
            ["2024-04-01", 70.0],
            ["2024-10-01", 65.0],
            ["2025-01-01", 90.0],
            ["2025-07-01", 75.0]
        ]),
    );
    let mut combo_cols = cols(&[
        ("Month", "Category", "column"),
        ("Total", "Y", "measure"),
        ("Cost", "Y2", "measure"),
    ]);
    combo_cols[0]["data_type"] = json!("dateTime");
    let combo = ok(
        combo_cols,
        json!([
            ["2025-01-01", 60.0, 30.0],
            ["2025-02-01", 75.0, 35.0],
            ["2025-04-01", 70.0, 42.0],
            ["2025-05-01", 95.0, 40.0],
            ["2025-08-01", 85.0, 50.0]
        ]),
    );
    let mut cat = v("t2", "clusteredColumnChart", 330, 10, 300, 250, days.clone());
    cat["format"] = json!({"x_axis": {"axis_type": "Categorical"}});
    json!([
        v("t1", "clusteredColumnChart", 10, 10, 310, 250, days),
        cat,
        v("t3", "lineChart", 640, 10, 310, 250, unsorted),
        v("t4", "areaChart", 10, 280, 310, 250, hours),
        v("t5", "clusteredBarChart", 330, 280, 300, 250, quarters),
        v("t6", "lineClusteredColumnComboChart", 640, 280, 310, 250, combo),
    ])
}

/// Continuous numeric axes and the `start` / `end` of continuous category axes
/// (docs/render.md "Continuous numeric axes"): columns on irregular whole numbers and the same
/// set `Categorical`, a line on decimals with a format string from unsorted rows, an area
/// widened by `start` / `end` (the last value dropped), horizontal bars on years, a date line cut
/// to January.
fn number_visuals() -> Value {
    let numbered = |name: &str, typ: &str, fmt: &str, rows: Value| {
        let mut c = cols(&[(name, "Category", "column"), ("Total", "Y", "measure")]);
        c[0]["data_type"] = json!(typ);
        c[0]["format_string"] = json!(fmt);
        ok(c, rows)
    };
    let sizes = numbered(
        "Size",
        "int64",
        "",
        json!([
            [1, 120.0],
            [2, 90.0],
            [3, 140.0],
            [8, 60.0],
            [10, 110.0],
            [16, 80.0]
        ]),
    );
    let weights = numbered(
        "Weight",
        "double",
        "#,0.0",
        json!([
            [1500.0, 30.0],
            [1000.0, 10.0],
            [4000.0, 25.0],
            [2250.0, 40.0],
            [2000.0, 18.0]
        ]),
    );
    let years = numbered(
        "Year",
        "int64",
        "0",
        json!([
            [2018, 50.0],
            [2019, 70.0],
            [2021, 65.0],
            [2024, 90.0],
            [2025, 75.0]
        ]),
    );
    let mut dc = cols(&[("Day", "Category", "column"), ("Total", "Y", "measure")]);
    dc[0]["data_type"] = json!("dateTime");
    dc[0]["format_string"] = json!("Short Date");
    let days = ok(
        dc,
        json!([
            ["2026-01-05T00:00:00", 12.0],
            ["2026-01-12T00:00:00", 18.0],
            ["2026-01-19T00:00:00", 9.0],
            ["2026-01-26T00:00:00", 15.0],
            ["2026-03-02T00:00:00", 30.0]
        ]),
    );
    let mut cat = v("n2", "clusteredColumnChart", 330, 10, 300, 250, sizes.clone());
    cat["format"] = json!({"x_axis": {"axis_type": "Categorical"}});
    let mut widened = v("n4", "areaChart", 10, 280, 310, 250, sizes.clone());
    widened["format"] = json!({"x_axis": {"min": 0, "max": 12}});
    let mut jan = v("n6", "lineChart", 640, 280, 310, 250, days);
    jan["format"] = json!({"x_axis": {"min": "2026-01-01T00:00:00", "max": "2026-02-01T00:00:00"}});
    json!([
        v("n1", "clusteredColumnChart", 10, 10, 310, 250, sizes),
        cat,
        v("n3", "lineChart", 640, 10, 310, 250, weights),
        widened,
        v("n5", "clusteredBarChart", 330, 280, 300, 250, years),
        jan,
    ])
}

/// Gantt visuals (docs/formats/gantt.md): phases with their tasks, a milestone, progress and a
/// detail column on an auto-scaled month axis; then the same plan collapsed to one phase, with
/// two fixed levels, shaded week ends and colours by category.
fn gantt_visuals() -> Value {
    let mut columns = cols(&[
        ("Phase", "Category", "column"),
        ("Task", "Task", "column"),
        ("Start", "StartDate", "aggregation"),
        ("End", "EndDate", "aggregation"),
        ("Done", "Progress", "measure"),
        ("Owner", "Columns", "aggregation"),
    ]);
    columns[2]["data_type"] = json!("dateTime");
    columns[3]["data_type"] = json!("dateTime");
    let plan = ok(
        columns,
        json!([
            ["Build", "Design", "2025-01-06", "2025-01-31", 1.0, "Ann"],
            ["Build", "Code", "2025-02-03", "2025-03-28", 0.4, "Bob"],
            ["Build", "Test", "2025-03-10", "2025-04-04", 0.1, "Dee"],
            ["Launch", "Review", "2025-04-01", null, null, "Ann"],
            ["Launch", "Ship", "2025-04-07", "2025-04-30", 0.0, "Cy"]
        ]),
    );
    let mut a = v("g1", "ganttChart", 10, 10, 940, 250, plan.clone());
    a["title"] = json!("Plan");
    let mut b = v("g2", "ganttChart", 10, 280, 940, 250, plan);
    b["format"] = json!({"title": {"show": false},
        "gantt": {"axis": {"auto_scale": false, "primary_unit": "month", "secondary_unit": "day", "shading_show": true},
                  "row": {"color_by_category": true}, "milestone": {"shape": "circle"}},
        "gantt_rows": [{"path": [{"c": 0, "v": "Build"}], "collapsed": true}]});
    json!([a, b])
}

/// Maps on the open basemap (Natural Earth), in a light or a dark report theme: bubbles at
/// latitude and longitude on five continents (the world, 1:110m), bubbles at cities named only
/// (a region: 1:50m countries, lakes, admin-1 boundaries, city names), a filled map of countries.
fn maps_snapshot(dark: bool) -> Value {
    let world = ok(
        cols(&[
            ("City", "Category", "column"),
            ("Continent", "Series", "column"),
            ("Lat", "Y", "aggregation"),
            ("Lon", "X", "aggregation"),
            ("Total", "Size", "measure"),
        ]),
        json!([
            ["Paris", "Europe", 48.86, 2.35, 120.0],
            ["New York", "America", 40.71, -74.0, 150.0],
            ["São Paulo", "America", -23.55, -46.63, 90.0],
            ["Lagos", "Africa", 6.45, 3.4, 60.0],
            ["Mumbai", "Asia", 19.08, 72.88, 110.0],
            ["Tokyo", "Asia", 35.68, 139.69, 140.0],
            ["Sydney", "Oceania", -33.87, 151.21, 70.0]
        ]),
    );
    let cities = ok(
        cols(&[("City", "Category", "column"), ("Total", "Size", "measure")]),
        json!([
            ["Chicago", 80.0],
            ["Houston", 60.0],
            ["Denver", 30.0],
            ["Seattle", 40.0],
            ["New York", 120.0],
            ["Atlanta", 50.0]
        ]),
    );
    let countries = ok(
        cols(&[("Country", "Category", "column"), ("Market", "Series", "column")]),
        json!([
            ["France", "Core"],
            ["Germany", "Core"],
            ["Spain", "Growth"],
            ["Italy", "Growth"],
            ["Poland", "New"]
        ]),
    );
    let mut m1 = v("m1", "map", 10, 10, 940, 250, world);
    m1["title"] = json!("World");
    let mut m2 = v("m2", "map", 10, 270, 465, 260, cities);
    m2["title"] = json!("Cities");
    let mut m3 = v("m3", "filledMap", 485, 270, 465, 260, countries);
    m3["title"] = json!("Markets");
    let (theme, bg) = if dark {
        (
            json!({"name": "Dark", "data_colors": ["#4AA8FF", "#F2C811", "#E66C37", "#6CD4A6", "#C47DE8"],
                   "foreground": "#F3F2F1", "background": "#1F1F1F"}),
            "#1F1F1F",
        )
    } else {
        (Value::Null, "#FFFFFF")
    };
    json!({"version": 1, "model": {"culture": "en-US", "tables": []}, "report": {"theme": theme, "pages": [
        {"name": "maps", "display_name": "Maps", "width": 960, "height": 540, "background": bg,
         "visuals": [m1, m2, m3]}
    ]}})
}

fn decode(png: &[u8]) -> tiny_skia::Pixmap {
    tiny_skia::Pixmap::decode_png(png).unwrap()
}

#[test]
fn golden_pages() {
    let snap = snapshot();
    let fonts = FontBook::bundled_only();
    let opts = RenderOptions {
        system_fonts: false,
        ..RenderOptions::default()
    };
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden");
    let bless = bicraft_env::var_os("BICRAFT_BLESS").is_some();
    let (maps, maps_dark) = (maps_snapshot(false), maps_snapshot(true));
    let pages = [
        (&snap, 0, "charts"),
        (&snap, 1, "others"),
        (&snap, 2, "drill"),
        (&snap, 3, "gantt"),
        (&snap, 4, "dates"),
        (&snap, 5, "numbers"),
        (&maps, 0, "maps"),
        (&maps_dark, 0, "maps-dark"),
    ];
    for (snap, i, name) in pages {
        let img = render_page(snap, i, &HashMap::<String, Vec<u8>>::new(), &fonts, &opts).unwrap();
        let path = dir.join(format!("{name}.png"));
        if bless {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(&path, &img.png).unwrap();
            continue;
        }
        let golden = decode(
            &std::fs::read(&path)
                .unwrap_or_else(|e| panic!("{}: {e} (BICRAFT_BLESS=1 writes it)", path.display())),
        );
        let ours = decode(&img.png);
        assert_eq!(
            (ours.width(), ours.height()),
            (golden.width(), golden.height()),
            "{name}"
        );
        // tolerant comparison: mean channel difference and share of clearly different pixels
        let (mut sum, mut bad) = (0u64, 0usize);
        for (a, b) in ours.pixels().iter().zip(golden.pixels()) {
            let d = [
                a.red().abs_diff(b.red()),
                a.green().abs_diff(b.green()),
                a.blue().abs_diff(b.blue()),
            ];
            sum += d.iter().map(|x| u64::from(*x)).sum::<u64>();
            if d.iter().any(|x| *x > 64) {
                bad += 1;
            }
        }
        let n = ours.pixels().len();
        let mean = sum as f64 / (n * 3) as f64;
        let share = bad as f64 / n as f64;
        assert!(
            mean < 1.0 && share < 0.005,
            "{name}: mean diff {mean:.3}, {:.3}% pixels differ",
            share * 100.0
        );
        let statuses: Vec<&str> = img.visuals.iter().map(|v| v.status).collect();
        if name == "others" {
            assert_eq!(statuses.last(), Some(&"placeholder"));
        } else {
            assert!(statuses.iter().all(|s| *s == "drawn"), "{statuses:?}");
        }
    }
}

/// Squared luminance steps between vertically (then horizontally) adjacent device pixels inside
/// the upright text runs' boxes, per unit of ink: how sharp the text's edges are.
fn text_edge_energy(img: &bicraft_render::PageImage) -> (f64, f64) {
    let pm = decode(&img.png);
    let (w, h) = (i64::from(pm.width()), i64::from(pm.height()));
    let lum = |x: i64, y: i64| {
        pm.pixel(x.clamp(0, w - 1) as u32, y.clamp(0, h - 1) as u32)
            .map_or(1.0, |p| f64::from(p.green()) / 255.0)
    };
    let mut seen = vec![false; (w * h) as usize];
    let (mut v, mut hz, mut ink) = (0.0, 0.0, 0.0);
    let s = f64::from(img.scale);
    for t in img.texts.iter().filter(|t| t.rotate == 0.0) {
        let (x0, y0) = (
            (f64::from(t.x) * s).floor() as i64,
            (f64::from(t.y) * s).floor() as i64,
        );
        let (x1, y1) = (
            (f64::from(t.x + t.w) * s).ceil() as i64,
            (f64::from(t.y + t.h) * s).ceil() as i64,
        );
        for y in y0.max(0)..y1.min(h - 1) {
            for x in x0.max(0)..x1.min(w - 1) {
                let i = (y * w + x) as usize;
                if std::mem::replace(&mut seen[i], true) {
                    continue;
                }
                let a = lum(x, y);
                ink += 1.0 - a;
                v += (lum(x, y + 1) - a).powi(2);
                hz += (lum(x + 1, y) - a).powi(2);
            }
        }
    }
    (v / ink.max(1e-9), hz / ink.max(1e-9))
}

/// Hinting sharpens the text of whole pages (fitted at 68 % and at 100 %) and changes no text
/// position, width or hit region.
#[test]
fn hinting_sharpens_pages_and_moves_nothing() {
    let snap = snapshot();
    let shots = bicraft_env::var_os("BICRAFT_HINT_SHOTS").map(PathBuf::from);
    for scale in [0.68f32, 1.0] {
        let opts = RenderOptions {
            system_fonts: false,
            scale,
            text_dump: true,
            hits: true,
            ..RenderOptions::default()
        };
        let (mut e0, mut e1) = ((0.0, 0.0), (0.0, 0.0));
        for (i, name) in ["charts", "others", "drill"].iter().enumerate() {
            let render = |hinting: bool| {
                let fonts = FontBook::bundled_only();
                fonts.set_hinting(hinting);
                render_page(&snap, i, &HashMap::<String, Vec<u8>>::new(), &fonts, &opts).unwrap()
            };
            let (off, on) = (render(false), render(true));
            assert_eq!(off.texts, on.texts, "{name} at {scale}: text runs");
            assert_eq!(off.hits, on.hits, "{name} at {scale}: hit map");
            assert_ne!(off.png, on.png, "{name} at {scale}: hinted");
            let (a, b) = (text_edge_energy(&off), text_edge_energy(&on));
            (e0.0, e0.1, e1.0, e1.1) = (e0.0 + a.0, e0.1 + a.1, e1.0 + b.0, e1.1 + b.1);
            if let Some(d) = &shots {
                std::fs::create_dir_all(d).unwrap();
                std::fs::write(d.join(format!("{name}-{scale}-unhinted.png")), &off.png).unwrap();
                std::fs::write(d.join(format!("{name}-{scale}-hinted.png")), &on.png).unwrap();
            }
        }
        println!(
            "scale {scale}: text vertical edge energy ×{:.2}, horizontal ×{:.2}",
            e1.0 / e0.0,
            e1.1 / e0.1
        );
        assert!(e1.0 > e0.0 * 1.05, "scale {scale}: {e0:?} → {e1:?}");
    }
}

/// Subpixel text (the app's pages at low DPI) colours the text of whole pages and changes no
/// text position, width or hit region; vertical text stays grayscale.
#[test]
fn lcd_text_colours_pages_and_moves_nothing() {
    let snap = snapshot();
    let fonts = FontBook::bundled_only();
    for scale in [0.6f32, 1.0] {
        for (i, name) in ["charts", "others", "drill"].iter().enumerate() {
            let render = |lcd_text: bool| {
                let opts = RenderOptions {
                    system_fonts: false,
                    scale,
                    text_dump: true,
                    hits: true,
                    lcd_text,
                    ..RenderOptions::default()
                };
                render_page(&snap, i, &HashMap::<String, Vec<u8>>::new(), &fonts, &opts).unwrap()
            };
            let (off, on) = (render(false), render(true));
            assert_eq!(off.texts, on.texts, "{name} at {scale}: text runs");
            assert_eq!(off.hits, on.hits, "{name} at {scale}: hit map");
            let (a, b) = (decode(&off.png), decode(&on.png));
            // pixels whose red and blue differ: chart marks have colours of their own, subpixel
            // text adds its fringes
            let coloured = |pm: &tiny_skia::Pixmap| {
                pm.pixels()
                    .iter()
                    .filter(|p| p.red().abs_diff(p.blue()) > 24)
                    .count()
            };
            assert!(
                coloured(&b) > coloured(&a) + 200,
                "{name} at {scale}: {} → {}",
                coloured(&a),
                coloured(&b)
            );
            // vertical text (axis titles) keeps its grayscale pixels, away from upright runs
            let s = f64::from(on.scale);
            let near_upright = |x: u32, y: u32| {
                on.texts.iter().any(|u| {
                    u.rotate == 0.0
                        && (f64::from(u.x) * s - 3.0..f64::from(u.x + u.w) * s + 3.0).contains(&f64::from(x))
                        && (f64::from(u.y) * s - 3.0..f64::from(u.y + u.h) * s + 3.0).contains(&f64::from(y))
                })
            };
            let mut checked = 0;
            for t in on.texts.iter().filter(|t| t.rotate != 0.0) {
                let (x0, y0) = ((f64::from(t.x) * s) as u32, (f64::from(t.y) * s) as u32);
                let (x1, y1) = (
                    (f64::from(t.x + t.w) * s).ceil() as u32,
                    (f64::from(t.y + t.h) * s).ceil() as u32,
                );
                for y in y0..y1.min(a.height()) {
                    for x in x0..x1.min(a.width()) {
                        if !near_upright(x, y) {
                            checked += 1;
                            assert_eq!(
                                a.pixel(x, y),
                                b.pixel(x, y),
                                "{name} at {scale}: rotated `{}`",
                                t.text
                            );
                        }
                    }
                }
            }
            if *name == "charts" {
                assert!(checked > 0, "{name} at {scale}: the fixture has vertical text");
            }
        }
    }
}

/// A page whose text asks for Calibri: with no Calibri installed it takes the bundled Carlito
/// subset (Calibri's advance widths), not Selawik (Segoe UI's). `BICRAFT_FONT_SHOTS=DIR` writes
/// the page as before (Selawik) and after (Carlito).
#[test]
fn calibri_text_takes_calibri_widths_on_a_page() {
    let page = |family: &str| {
        let run = |text: &str, bold: bool, italic: bool| json!({"text": text, "font_size": 16, "font_family": family, "bold": bold, "italic": italic});
        json!({"version": 1, "model": {"culture": "en-US", "tables": []}, "report": {"theme": null, "pages": [
        {"name": "p1", "display_name": "Calibri", "width": 640, "height": 240, "visuals": [
            {"name": "t1", "type": "textbox", "x": 20, "y": 20, "width": 600, "height": 200,
             "text": {"paragraphs": [
                {"runs": [run("Quarterly revenue by region and product line", false, false)]},
                {"runs": [run("Total Sales 2024 (€) — Région Île-de-France", true, false)]},
                {"runs": [run("Prices exclude VAT; figures in thousands", false, true)]},
                {"runs": [run("The quick brown fox jumps over the lazy dog 0123456789", true, true)]}
             ]}}]}]}})
    };
    let opts = RenderOptions {
        system_fonts: false,
        text_dump: true,
        ..RenderOptions::default()
    };
    let fonts = FontBook::bundled_only();
    let render = |family: &str| {
        render_page(
            &page(family),
            0,
            &HashMap::<String, Vec<u8>>::new(),
            &fonts,
            &opts,
        )
        .unwrap()
    };
    let (before, after) = (render("Selawik"), render("Calibri"));
    assert_eq!(before.texts.len(), after.texts.len());
    for (b, a) in before.texts.iter().zip(&after.texts) {
        let f = bicraft_render::Font::new(&a.family, a.size)
            .weight(a.weight)
            .italic(a.italic);
        assert!((a.w - fonts.measure(&f, &a.text)).abs() < 0.05, "{a:?}");
        // Calibri is narrower than Segoe UI at the same size
        assert!(a.w < b.w * 0.95, "{} : {} vs Selawik {}", a.text, a.w, b.w);
    }
    if let Some(d) = bicraft_env::var_os("BICRAFT_FONT_SHOTS").map(PathBuf::from) {
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("calibri-before-selawik.png"), &before.png).unwrap();
        std::fs::write(d.join("calibri-after-carlito.png"), &after.png).unwrap();
    }
}
