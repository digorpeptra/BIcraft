//! Renderer unit tests: every visual family renders from a synthetic snapshot without panicking,
//! and hostile snapshots never crash.

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

fn cat_val() -> Value {
    data(
        json!([{"name": "Region", "role": "Category", "kind": "column", "data_type": "string"},
               {"name": "Total", "role": "Y", "kind": "measure"}]),
        json!([["North", 120.0], ["South", 80.0], ["East", 45.5], ["West", -10.0]]),
    )
}

fn opts() -> RenderOptions {
    RenderOptions {
        system_fonts: false,
        ..RenderOptions::default()
    }
}

fn render(s: &Value) -> PageImage {
    let fonts = FontBook::bundled_only();
    render_page(s, 0, &HashMap::<String, Vec<u8>>::new(), &fonts, &opts()).unwrap()
}

/// Share of pixels that are not white.
fn ink(img: &PageImage) -> f64 {
    let pm = tiny_skia::Pixmap::decode_png(&img.png).unwrap();
    let n = pm
        .pixels()
        .iter()
        .filter(|p| p.red() < 245 || p.green() < 245 || p.blue() < 245)
        .count();
    n as f64 / pm.pixels().len() as f64
}

#[test]
fn every_family_renders() {
    let types = [
        "card",
        "cardVisual",
        "multiRowCard",
        "clusteredColumnChart",
        "stackedColumnChart",
        "hundredPercentStackedColumnChart",
        "clusteredBarChart",
        "stackedBarChart",
        "hundredPercentStackedBarChart",
        "lineChart",
        "areaChart",
        "stackedAreaChart",
        "lineClusteredColumnComboChart",
        "lineStackedColumnComboChart",
        "pieChart",
        "donutChart",
        "tableEx",
        "pivotTable",
        "slicer",
        "listSlicer",
        "advancedSlicerVisual",
        "textSlicer",
        "gauge",
        "kpi",
        "funnel",
        "waterfallChart",
        "treemap",
        "ribbonChart",
    ];
    for t in types {
        let s = snap(
            json!([{"name": "v", "type": t, "x": 10, "y": 10, "z": 0, "width": 400, "height": 250, "title": "T",
                              "fields": [{"role": "Category", "kind": "column", "table": "Sales", "name": "Region"},
                                         {"role": "Y", "kind": "measure", "table": "Sales", "name": "Total"}],
                              "data": cat_val()}]),
        );
        let img = render(&s);
        assert_eq!((img.width, img.height), (640, 360), "{t}");
        assert_eq!(img.visuals.len(), 1);
        assert_eq!(img.visuals[0].status, "drawn", "{t}");
        assert!(ink(&img) > 0.001, "{t}: nothing drawn ({})", ink(&img));
    }
}

#[test]
fn scatter_text_shapes_buttons_images_and_unknown() {
    let s = snap(json!([
        {"name": "sc", "type": "scatterChart", "x": 0, "y": 0, "width": 300, "height": 200,
         "fields": [{"role": "Category", "kind": "column", "name": "Region"}],
         "data": data(json!([{"name": "Region", "role": "Category"}, {"name": "A", "role": "X", "kind": "measure"}, {"name": "B", "role": "Y", "kind": "measure"}]),
                      json!([["a", 1, 2], ["b", 3, 5], ["c", 4, 1]]))},
        {"name": "tx", "type": "textbox", "x": 310, "y": 0, "width": 300, "height": 100,
         "text": {"paragraphs": [{"align": "center", "runs": [{"text": "Hello world, this is a long line that wraps", "bold": true, "font_size": 14}]}]}},
        {"name": "bt", "type": "actionButton", "x": 310, "y": 110, "width": 120, "height": 40,
         "button": {"text": "Back", "icon": "back", "fill": "#E6E6E6", "show_fill": true}},
        {"name": "sh", "type": "shape", "x": 440, "y": 110, "width": 100, "height": 60},
        {"name": "im", "type": "image", "x": 310, "y": 180, "width": 100, "height": 100},
        {"name": "cv", "type": "someCustomVisual1234", "x": 420, "y": 200, "width": 200, "height": 100, "title": "Custom"},
        {"name": "nv", "type": "pageNavigator", "x": 0, "y": 300, "width": 300, "height": 40},
        {"name": "hd", "type": "card", "hidden": true, "x": 0, "y": 0, "width": 10, "height": 10},
        {"name": "gr", "type": null, "x": 0, "y": 0, "width": 10, "height": 10}
    ]));
    let img = render(&s);
    let st: Vec<(&str, &str)> = img.visuals.iter().map(|v| (v.name.as_str(), v.status)).collect();
    assert_eq!(
        st,
        vec![
            ("sc", "drawn"),
            ("tx", "drawn"),
            ("bt", "drawn"),
            ("sh", "drawn"),
            ("im", "drawn"),
            ("cv", "placeholder"),
            ("nv", "drawn")
        ]
    );
}

/// A custom visual (the snapshot's `custom_visual`) is a placeholder that shows the package's
/// icon above its display name; BIcraft never runs its code.
#[test]
fn a_custom_visual_is_its_icon_and_name() {
    // a 16×16 red PNG of our own
    let mut pm = tiny_skia::Pixmap::new(16, 16).unwrap();
    pm.fill(tiny_skia::Color::from_rgba8(220, 20, 20, 255));
    let png = pm.encode_png().unwrap();
    let id = "0123456789abcdef0123456789abcdef";
    let cv = json!({"name": "tinyGuid", "display_name": "Tiny",
                    "icon": {"id": id, "mime": "image/png", "width": 16, "height": 16}});
    let s = snap(json!([
        {"name": "cv", "type": "tinyGuid", "x": 0, "y": 0, "width": 200, "height": 160, "custom_visual": cv},
        {"name": "small", "type": "tinyGuid", "x": 300, "y": 0, "width": 40, "height": 20, "custom_visual": cv}
    ]));
    let fonts = FontBook::bundled_only();
    let assets: HashMap<String, Vec<u8>> = [(id.to_string(), png)].into();
    let img = render_page(&s, 0, &assets, &fonts, &opts()).unwrap();
    assert_eq!(img.visuals[0].status, "placeholder");
    assert!(
        img.visuals[0]
            .note
            .as_deref()
            .unwrap_or("")
            .contains("never runs")
    );
    let pm = tiny_skia::Pixmap::decode_png(&img.png).unwrap();
    let red = pm
        .pixels()
        .iter()
        .filter(|p| p.red() > 180 && p.green() < 80 && p.blue() < 80)
        .count();
    assert!(red > 100, "the icon is drawn ({red} red pixels)");
    // the name under the icon: dark text pixels between the icon's bottom and the box's
    let w = pm.width() as usize;
    let is_red = |p: &tiny_skia::PremultipliedColorU8| p.red() > 180 && p.green() < 80;
    let icon_bottom = (0..pm.height() as usize)
        .filter(|y| (0..200.min(w)).any(|x| is_red(&pm.pixels()[y * w + x])))
        .max()
        .unwrap();
    let text = (icon_bottom + 1..(icon_bottom + 60).min(pm.height() as usize))
        .flat_map(|y| (10..190.min(w)).map(move |x| y * w + x))
        .filter(|&i| {
            let p = pm.pixels()[i];
            p.alpha() > 0 && p.red().max(p.green()).max(p.blue()) < 170
        })
        .count();
    assert!(text > 20, "the name is drawn under the icon ({text} text pixels)");
    // without the asset, and in a box too small for the icon: the labelled box only
    let img = render(&s);
    assert_eq!(img.visuals.len(), 2);
}

#[test]
fn a_hidden_group_hides_its_members() {
    let s = snap(json!([
        {"name": "g", "type": null, "hidden": true, "x": 0, "y": 0, "width": 200, "height": 100},
        {"name": "c1", "type": "shape", "parent_group": "g", "x": 0, "y": 0, "width": 100, "height": 100},
        {"name": "inner", "type": null, "parent_group": "g", "x": 0, "y": 0, "width": 100, "height": 100},
        {"name": "c2", "type": "shape", "parent_group": "inner", "x": 100, "y": 0, "width": 100, "height": 100},
        {"name": "shown", "type": "shape", "x": 210, "y": 0, "width": 100, "height": 100}
    ]));
    let img = render(&s);
    let names: Vec<&str> = img.visuals.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, vec!["shown"]);
}

#[test]
fn no_data_is_a_faded_preview() {
    let s = snap(
        json!([{"name": "v", "type": "clusteredColumnChart", "x": 0, "y": 0, "width": 300, "height": 200,
        "fields": [{"role": "Category", "kind": "column", "table": "Sales", "name": "Region"}, {"role": "Y", "kind": "measure", "table": "Sales", "name": "Total"}],
        "data": {"status": "unavailable", "reason": "no table data"}}]),
    );
    let img = render(&s);
    assert_eq!(img.visuals[0].status, "preview");
    assert!(
        img.visuals[0]
            .note
            .as_deref()
            .unwrap_or("")
            .contains("no table data")
    );
}

#[test]
fn page_selection_and_scale() {
    let s = snap(json!([]));
    assert_eq!(find_page(&s, "p1"), Some(0));
    assert_eq!(find_page(&s, "page 1"), Some(0));
    assert_eq!(find_page(&s, "0"), Some(0));
    assert_eq!(find_page(&s, "1"), None);
    let fonts = FontBook::bundled_only();
    let o = RenderOptions { scale: 2.0, ..opts() };
    let img = render_page(&s, 0, &HashMap::<String, Vec<u8>>::new(), &fonts, &o).unwrap();
    assert_eq!((img.width, img.height), (1280, 720));
    let o = RenderOptions {
        scale: 100.0,
        max_pixels: 1_000_000,
        ..opts()
    };
    let img = render_page(&s, 0, &HashMap::<String, Vec<u8>>::new(), &fonts, &o).unwrap();
    assert!(u64::from(img.width) * u64::from(img.height) <= 1_000_000);
    assert!(matches!(
        render_page(&s, 3, &HashMap::<String, Vec<u8>>::new(), &fonts, &o),
        Err(RenderError::PageNotFound(_))
    ));
    assert_eq!(
        render_page(&json!({}), 0, &HashMap::<String, Vec<u8>>::new(), &fonts, &o).unwrap_err(),
        RenderError::NoReport
    );
}

#[test]
fn background_images_and_wallpaper() {
    let mut pm = tiny_skia::Pixmap::new(4, 4).unwrap();
    pm.fill(tiny_skia::Color::from_rgba8(0, 0, 255, 255));
    let id = "0123456789abcdef0123456789abcdef".to_string();
    let mut assets = HashMap::new();
    assets.insert(id.clone(), pm.encode_png().unwrap());
    let mut s = snap(json!([]));
    s["report"]["pages"][0]["background_image"] =
        json!({"image": {"id": id, "mime": "image/png", "width": 4, "height": 4}, "fit": "fit"});
    let fonts = FontBook::bundled_only();
    let img = render_page(&s, 0, &assets, &fonts, &opts()).unwrap();
    let out = tiny_skia::Pixmap::decode_png(&img.png).unwrap();
    let p = out.pixel(320, 180).unwrap();
    assert_eq!((p.red(), p.green(), p.blue()), (0, 0, 255));
}

#[test]
fn hostile_snapshots_do_not_crash() {
    let nasty = [
        json!({"report": {"pages": [{"width": 1e300, "height": -5, "visuals": [{"type": "card", "width": 1e12, "height": f64::MAX}]}]}}),
        json!({"report": {"pages": [{"visuals": [{"type": "tableEx", "x": 0, "y": 0, "width": 100, "height": 100,
            "fields": [{"role": "Values", "kind": "column", "name": "x"}],
            "data": {"status": "ok", "columns": [{"name": "x"}], "rows": [[{"deep": [1, 2]}], ["a"], [null], [true], [1e308]],
                     "cell_styles": [{"r": 0, "c": 0, "bg": "#zzz", "bar": {"from": -5, "to": 99, "color": "#f00"}, "icon": {"name": "../../x"}}]}}]}]}}),
        json!({"report": {"pages": [{"visuals": [{"type": "pieChart", "width": 50, "height": 50, "fields": [{"role": "Y"}],
            "data": {"status": "ok", "columns": [{"name": "v", "role": "Y", "kind": "measure"}], "rows": [[-1], [0], [f64::MIN_POSITIVE]]}}]}]}}),
        json!({"report": {"pages": [{"visuals": [{"type": "textbox", "width": 3, "height": 3, "text": {"paragraphs": [{"runs": [{"text": "x".repeat(50000), "font_size": 1e9}]}]}}]}]}}),
        json!({"report": {"pages": [{"visuals": [{"type": "lineChart", "width": 300, "height": 200, "format": {"y_axis": {"log": true, "min": -1, "max": 0}},
            "fields": [{"role": "Category"}], "data": {"status": "ok", "columns": [{"name": "c", "role": "Category"}, {"name": "v", "role": "Y", "kind": "measure"}],
            "rows": [["a", 0], ["b", -3], ["c", null]]}}]}]}}),
        json!({"report": {"pages": [{"visuals": [{"type": "clusteredColumnChart", "width": 300, "height": 200, "format": {"small_multiples": {"column": 7}},
            "fields": [{"role": "Category"}], "data": {"status": "ok", "columns": [{"name": "c", "role": "Category"}], "rows": [["a"]]}}]}]}}),
        json!({"report": {"pages": "nope"}}),
        json!(null),
    ];
    let fonts = FontBook::bundled_only();
    for s in nasty {
        let _ = render_page(&s, 0, &HashMap::<String, Vec<u8>>::new(), &fonts, &opts());
    }
}

/// Regression: a header variable once shadowed the visual's `y`, drawing every visual at the top.
#[test]
fn spotlight_fades_the_other_visuals() {
    let chart = |name: &str, x: i32| {
        json!({"name": name, "type": "clusteredColumnChart", "x": x, "y": 20, "width": 300, "height": 300,
               "fields": [{"role": "Category", "kind": "column", "name": "Region"}], "data": cat_val()})
    };
    let mut s = snap(json!([chart("a", 10), chart("b", 330)]));
    // the ink of the page's left and right halves
    let halves = |s: &Value| {
        let img = render(s);
        let pm = tiny_skia::Pixmap::decode_png(&img.png).unwrap();
        let half = pm.width() / 2;
        let ink = |x0: u32, x1: u32| {
            (0..pm.height())
                .flat_map(|y| (x0..x1).map(move |x| (x, y)))
                .filter(|(x, y)| pm.pixel(*x, *y).is_some_and(|p| p.red() < 200))
                .count()
        };
        (ink(0, half), ink(half, pm.width()))
    };
    let (a0, b0) = halves(&s);
    s["interaction"] = json!({"spotlight": {"page": "p1", "visual": "a"}});
    let (a1, b1) = halves(&s);
    assert_eq!(a1, a0, "the spotlight visual is drawn as before");
    assert!(b0 > 500 && b1 * 10 < b0, "{b0} {b1}");
    // another page's spotlight, or a visual that is not there, dims nothing
    for spot in [
        json!({"page": "p2", "visual": "a"}),
        json!({"page": "p1", "visual": "zz"}),
    ] {
        s["interaction"] = json!({"spotlight": spot});
        assert_eq!(halves(&s), (a0, b0));
    }
}

#[test]
fn visuals_are_drawn_at_their_position() {
    let s = snap(
        json!([{"name": "v", "type": "clusteredColumnChart", "x": 300, "y": 200, "width": 300, "height": 150,
        "title": "T", "fields": [{"role": "Category", "kind": "column", "name": "Region"}], "data": cat_val()}]),
    );
    let img = render(&s);
    let pm = tiny_skia::Pixmap::decode_png(&img.png).unwrap();
    let ink_in = |y0: u32, y1: u32| {
        (y0..y1)
            .flat_map(|y| (0..pm.width()).map(move |x| (x, y)))
            .filter(|(x, y)| pm.pixel(*x, *y).is_some_and(|p| p.red() < 200))
            .count()
    };
    assert_eq!(ink_in(0, 195), 0, "nothing above the visual");
    assert!(ink_in(200, 350) > 500);
}

#[test]
fn snapshot_strings_captions_and_formatting_keys() {
    let fonts = FontBook::bundled_only();
    let snap = json!({"ui": {"strings": {"blank": "(Vide)", "total": "Total FR", "aggregations": {"sum": "Somme de {0}"}}},
                      "report": {"theme": null, "pages": []}});
    let ctx = data::Ctx::new(&snap, &fonts, None);
    assert_eq!(ctx.blank(), "(Vide)");
    assert_eq!(ctx.t("total", "Total"), "Total FR");
    assert_eq!(ctx.t("search", "Search"), "Search");
    assert_eq!(ctx.agg_name("Sum", "Amount").as_deref(), Some("Somme de Amount"));
    let v = json!({"auto_title": "Ventes par Ville", "fields": [{"role": "Y", "name": "Total", "caption": "Ventes"}]});
    assert_eq!(data::auto_title(&v, &ctx.loc), "Ventes par Ville");
    // gauge axis from format.gauge, data labels without display units
    let s = snap_with(
        json!([{"name": "g", "type": "gauge", "x": 0, "y": 0, "width": 300, "height": 200,
        "format": {"gauge": {"min": 0, "max": 2, "target": 1}, "labels": {"display_units": 1}},
        "fields": [{"role": "Y", "kind": "measure", "name": "Total"}],
        "data": data(json!([{"name": "Total", "role": "Y", "kind": "measure"}]), json!([[0.5]]))}]),
    );
    assert_eq!(render(&s).visuals[0].status, "drawn");
    let f = data::vis_format(&json!({"format": {"labels": {"display_units": 1}}}), "col", &ctx);
    assert_eq!(charts::lbl(417_736.0, None, &f, &ctx), "417,736");
}

#[test]
fn a_custom_theme_without_a_base_theme_draws_classic_titles() {
    // Power BI Desktop 2.159.1231.0 (2026-11-24): a report with only a custom theme (no base
    // theme) keeps the classic look's light grey regular titles, with the custom data colours
    let fonts = FontBook::bundled_only();
    let theme = json!({"data_colors": ["#7B2CBF"], "foreground": "#252423", "background": "#FFFFFF", "classic": true});
    let snap = json!({"report": {"theme": theme, "pages": []}});
    let ctx = data::Ctx::new(&snap, &fonts, None);
    let f = data::vis_format(&json!({}), "col", &ctx);
    assert_eq!(f.title.color, data::CLASSIC_TITLE);
    assert_eq!(data::palette(&theme)[0], Rgb(0x7b, 0x2c, 0xbf));
    // a theme with a base theme keeps the themed titles
    let snap = json!({"report": {"theme": {"data_colors": ["#7B2CBF"]}, "pages": []}});
    let ctx = data::Ctx::new(&snap, &fonts, None);
    assert_ne!(
        data::vis_format(&json!({}), "col", &ctx).title.color,
        data::CLASSIC_TITLE
    );
}

fn snap_with(visuals: Value) -> Value {
    snap(visuals)
}

/// Pie / donut label styles and positions, gauge bounds (literal and field-bound), new-card
/// layouts (rows table, orientation, accent bar), multi-row overflow and KPI without a trend
/// axis: each draws (or, for the KPI, leaves the body empty) and never panics.
#[test]
fn parts_and_cards_formatting() {
    let pie_fields = json!([{"role": "Category", "kind": "column", "table": "Sales", "name": "Region"},
                            {"role": "Y", "kind": "measure", "table": "Sales", "name": "Total"}]);
    let mut vis = Vec::new();
    for (k, style) in [
        "Category",
        "Data",
        "Percent of total",
        "Both",
        "Category, percent of total",
        "Data value, percent of total",
        "Category, data value, percent of total",
        "bogus",
    ]
    .iter()
    .enumerate()
    {
        for (j, pos) in ["outside", "inside"].iter().enumerate() {
            vis.push(
                json!({"name": format!("p{k}{j}"), "type": if j == 0 { "pieChart" } else { "donutChart" },
                "x": 0, "y": 0, "width": 200, "height": 160, "fields": pie_fields, "data": cat_val(),
                "format": {"labels": {"label_style": style, "position": pos, "percent_precision": 99},
                           "pie": {"inner_radius": 250, "start_angle": -1e300}}}),
            );
        }
    }
    let gauge_cols = json!([{"name": "Total", "role": "Y", "kind": "measure"}, {"name": "Max", "role": "MaxValue", "kind": "measure"}]);
    for g in [
        json!({"min": 0, "max": 1000, "target": 750}),
        json!({"max": {"field": "[Max]", "column": 1}, "target": -5}),
        json!({"max": {"field": "x", "column": 1e9}, "callout": {"display_units": 1e6, "precision": 3, "font_size": 1e9}}),
        json!({"min": 5, "max": 5, "target_label": {"show": false}}),
    ] {
        vis.push(
            json!({"name": "g", "type": "gauge", "x": 0, "y": 0, "width": 300, "height": 90,
            "data": data(gauge_cols.clone(), json!([[420.0, 900.0]])), "format": {"gauge": g}}),
        );
    }
    let card_cols = json!([{"name": "Region", "role": "Rows", "kind": "column"}, {"name": "Total", "role": "Data", "kind": "measure"},
                           {"name": "Units", "role": "Data", "kind": "measure"}]);
    for card in [
        json!({}),
        json!({"layout": {"orientation": 2, "columns": 0}, "accent_bar": {"show": true, "width": 1e6, "position": "top"}}),
        json!({"label": {"position": "belowvalue", "alignment": "center", "font_size": -3}, "value": {"show": false}}),
    ] {
        vis.push(json!({"name": "c", "type": "cardVisual", "x": 0, "y": 0, "width": 300, "height": 200,
            "data": data(card_cols.clone(), json!([["North", 1.0, 2.0], ["South", null, 3.0], ["East", 1e15, -1.0], ["West", 0.0, 0.0]])),
            "format": {"card": card}}));
        vis.push(json!({"name": "c1", "type": "cardVisual", "x": 0, "y": 0, "width": 30, "height": 300,
            "data": data(json!([{"name": "Total", "role": "Data", "kind": "measure"}, {"name": "Units", "role": "Data", "kind": "measure"}]),
                         json!([[1.0, 2.0]])), "format": {"card": card}}));
    }
    vis.push(
        json!({"name": "m", "type": "multiRowCard", "x": 0, "y": 0, "width": 300, "height": 100,
        "data": data(card_cols.clone(), json!([["a", 1, 2], ["b", 3, 4], ["c", 5, 6], ["d", 7, 8]]))}),
    );
    for v in vis {
        let img = render(&snap(json!([v.clone()])));
        assert_eq!(img.visuals.len(), 1, "{v}");
        assert!(ink(&img) > 0.0005, "{v}: nothing drawn");
    }
    // Desktop draws a KPI without a trend axis as an empty box
    let kpi = json!([{"name": "k", "type": "kpi", "x": 0, "y": 0, "width": 300, "height": 200,
        "data": data(json!([{"name": "Total", "role": "Indicator", "kind": "measure"}]), json!([[5.0]]))}]);
    assert!(ink(&render(&snap(kpi))) < 0.0005);
}

fn texts(s: &Value) -> Vec<canvas::TextRun> {
    let fonts = FontBook::bundled_only();
    let o = RenderOptions {
        text_dump: true,
        ..opts()
    };
    render_page(s, 0, &HashMap::<String, Vec<u8>>::new(), &fonts, &o)
        .unwrap()
        .texts
}

/// Power BI Desktop 2.159.1231.0 (2026-11-23) shows its default title ("Total and Target by K")
/// over the BIcraft Spec visual, as over a built-in chart, and draws the spec below it.
#[test]
fn the_spec_visual_has_the_default_title_above_its_drawing() {
    let spec_visual = |title: Value| {
        let mut v = json!({"name": "s", "type": "bicraftSpecVisual7F3A9C2E5B1D4E6F8A0C2B4D",
            "x": 0, "y": 0, "width": 400, "height": 260, "auto_title": "Total by Region",
            "spec": {"version": 1, "marks": [{"type": "bar", "y": "value", "name": "Total"}]},
            "fields": [{"role": "category", "kind": "column", "name": "Region"},
                       {"role": "values", "kind": "measure", "name": "Total"}],
            "data": data(json!([{"name": "Region", "role": "category", "kind": "column", "data_type": "string"},
                                {"name": "Total", "role": "values", "kind": "measure"}]),
                         json!([["North", 120.0], ["South", 80.0]]))});
        if !title.is_null() {
            v["format"] = json!({"title": title});
        }
        snap(json!([v]))
    };
    let runs = texts(&spec_visual(Value::Null));
    let title = runs
        .iter()
        .find(|t| t.text == "Total by Region")
        .expect("the default title is drawn");
    // the drawing (its legend and ticks) lies below the title
    let below = runs
        .iter()
        .filter(|t| t.text != title.text)
        .map(|t| t.y)
        .fold(f32::MAX, f32::min);
    assert!(
        below >= title.y + title.h - 0.5,
        "{below} vs {}",
        title.y + title.h
    );
    // the title switched off: no title
    let off = texts(&spec_visual(json!({"show": false})));
    assert!(off.iter().all(|t| t.text != "Total by Region"));
}

/// Power BI Desktop 2.159.1231.0 (2026-11-24) draws its default title over every BIcraft visual,
/// the first pack's histogram, bullet, box plot and dumbbell included ("People by Score"), and
/// the visual below it.
#[test]
fn bicraft_chart_visuals_have_the_default_title_above_their_drawing() {
    let v = json!({"name": "h", "type": "bicraftHistogram4C8E1A7D3F9B2E6A0D5C",
        "x": 0, "y": 0, "width": 300, "height": 225, "auto_title": "People by Score",
        "bicraft_visual": {"kind": "histogram", "options": {}},
        "fields": [{"role": "values", "kind": "column", "name": "Score"},
                   {"role": "frequency", "kind": "measure", "name": "People"}],
        "data": data(json!([{"name": "Score", "role": "values", "kind": "column", "data_type": "int64"},
                            {"name": "People", "role": "frequency", "kind": "measure"}]),
                     json!([[40.0, 1.0], [55.0, 2.0], [90.0, 1.0]]))});
    let runs = texts(&snap(json!([v.clone()])));
    let title = runs
        .iter()
        .find(|t| t.text == "People by Score")
        .expect("the default title is drawn");
    let top = |runs: &[canvas::TextRun]| {
        runs.iter()
            .filter(|t| t.text != "People by Score")
            .map(|t| t.y)
            .fold(f32::MAX, f32::min)
    };
    let below = top(&runs);
    assert!(
        below >= title.y + title.h - 0.5,
        "{below} vs {}",
        title.y + title.h
    );
    // Desktop's frame sits 21 px down under the title, 5 px down without it: the drawing moves
    // 16 px, not the 20 px of the 8 px gap under other visuals' titles
    let mut off = v;
    off["format"] = json!({"title": {"show": false}});
    let untitled = top(&texts(&snap(json!([off]))));
    assert!((below - untitled - 16.0).abs() < 0.6, "{below} - {untitled}");
}

#[test]
fn outside_labels_of_thin_bars_are_drawn() {
    // eight bars in a short chart are thinner than a label: Desktop still labels them, in the
    // gap to the next bar
    let cats = ["a", "b", "c", "d", "e", "f", "g", "h"];
    let rows: Vec<Value> = cats
        .iter()
        .enumerate()
        .map(|(i, c)| json!([c, 1000 + 100 * i]))
        .collect();
    // (labels that are no axis tick: the ticks are multiples of 500)
    let odd = ["1,100", "1,200", "1,300", "1,400", "1,600", "1,700"];
    for h in (120..=320).step_by(20) {
        let v = json!([{"name": "b", "type": "clusteredBarChart", "x": 0, "y": 0, "width": 600, "height": h,
            "data": data(json!([{"name": "Region", "role": "Category", "kind": "column", "data_type": "string"},
                                {"name": "Total", "role": "Y", "kind": "measure"}]), Value::Array(rows.clone())),
            "format": {"labels": {"show": true, "font_size": 10.5}, "x_axis": {"inner_padding": 50}}}]);
        let t = texts(&snap(v));
        // every category shown (the others are behind the scroll bar) has its label
        let shown: Vec<&str> = t
            .iter()
            .map(|r| r.text.as_str())
            .filter(|x| cats.contains(x))
            .collect();
        let labels = t.iter().filter(|r| odd.contains(&r.text.as_str())).count();
        let ticks_among = shown.iter().filter(|c| **c == "a" || **c == "f").count();
        assert!(
            !shown.is_empty() && labels + ticks_among == shown.len(),
            "height {h}: {:?}",
            t.iter().map(|r| (&r.text, r.y)).collect::<Vec<_>>()
        );
    }
}

fn combo(line: &[f64]) -> Value {
    let rows: Vec<Value> = ["a", "b", "c", "d"]
        .iter()
        .zip([100.0, 50.0, 80.0, 60.0])
        .zip(line)
        .map(|((c, y), l)| json!([c, y, l]))
        .collect();
    json!({
        "version": 1,
        "model": {"culture": "en-US", "tables": [{"name": "Sales", "columns": [{"name": "Region", "data_type": "string"}],
                  "measures": [{"name": "Total", "format_string": "#,0"},
                               {"name": "Balance", "format_string": "+#,0;-#,0;0"}]}]},
        "report": {"theme": null, "pages": [{"name": "p1", "display_name": "Page 1", "width": 640, "height": 360, "visuals": [
            {"name": "c", "type": "lineClusteredColumnComboChart", "x": 0, "y": 0, "width": 600, "height": 300,
             "fields": [{"role": "Category", "kind": "column", "table": "Sales", "name": "Region"},
                        {"role": "Y", "kind": "measure", "table": "Sales", "name": "Total"},
                        {"role": "Y2", "kind": "measure", "table": "Sales", "name": "Balance"}],
             "data": data(json!([{"name": "Region", "role": "Category", "kind": "column", "data_type": "string"},
                                 {"name": "Total", "role": "Y", "kind": "measure"},
                                 {"name": "Balance", "role": "Y2", "kind": "measure"}]), Value::Array(rows))}]}]}
    })
}

#[test]
fn combo_line_of_the_same_magnitude_shares_the_value_axis() {
    // a signed balance next to amounts: one axis, extended below zero for the line
    let t = texts(&combo(&[40.0, -30.0, 20.0, 10.0]));
    let right: Vec<&str> = t
        .iter()
        .filter(|r| r.x > 560.0)
        .map(|r| r.text.as_str())
        .collect();
    assert!(right.is_empty(), "no secondary axis expected: {right:?}");
    // the axis reaches below zero for the line (Desktop labels no tick there): the zero tick is
    // well above the category labels
    let y = |s: &str| t.iter().find(|r| r.text == s).map(|r| r.y).unwrap();
    let (y0, y50, cat) = (y("0"), y("50"), y("a"));
    assert!(
        cat - y0 > 0.5 * (y0 - y50),
        "0 at {y0}, 50 at {y50}, categories at {cat}"
    );
}

#[test]
fn combo_line_of_another_magnitude_keeps_its_own_axis() {
    let t = texts(&combo(&[0.4, -0.3, 0.2, 0.1]));
    let right: Vec<&str> = t
        .iter()
        .filter(|r| r.x > 540.0)
        .map(|r| r.text.as_str())
        .collect();
    assert!(right.len() >= 3, "{right:?}");
    assert!(right.iter().any(|x| x.starts_with('-')), "{right:?}");
}

fn months_page(culture: &str, visuals: Value) -> Value {
    json!({
        "version": 1,
        "model": {"culture": culture, "tables": [{"name": "Cal", "columns": [{"name": "YM", "data_type": "string"}],
                  "measures": [{"name": "Spend", "format_string": "#,0"}, {"name": "Prior", "format_string": "#,0"},
                               {"name": "Rate", "format_string": "0.0"}]}]},
        "report": {"theme": {"data_colors": ["#118DFF", "#12239E"], "font_size": 10.5},
                   "pages": [{"name": "p1", "display_name": "Page 1", "width": 1920, "height": 1080, "visuals": visuals}]}
    })
}

fn months_chart(t: &str, y: f64, w: f64) -> Value {
    let rows: Vec<Value> = (0..21)
        .map(|i| {
            json!([
                format!("{}-{:02}", 2025 + i / 12, i % 12 + 1),
                1000 + 137 * i,
                900 + (i * 53) % 400
            ])
        })
        .collect();
    let second = if t == "lineChart" { "Y" } else { "Y2" };
    json!({"name": format!("{t}{y}"), "type": t, "x": 48, "y": y, "width": w, "height": 280,
        "data": data(json!([{"name": "YM", "role": "Category", "kind": "column", "data_type": "string"},
                            {"name": "Spend", "role": "Y", "kind": "measure"},
                            {"name": "Prior", "role": second, "kind": "measure"}]), Value::Array(rows))})
}

#[test]
fn numbers_follow_the_model_culture() {
    // pea (a): no formatting culture given → the model's (fr-FR) separators, not en-US
    let card = json!([{"name": "c", "type": "card", "x": 0, "y": 0, "width": 300, "height": 150,
        "data": data(json!([{"name": "Rate", "role": "Values", "kind": "measure"}]), json!([[12.5]]))}]);
    let fr = texts(&months_page("fr-FR", card.clone()));
    assert!(
        fr.iter().any(|r| r.text == "12,5"),
        "{:?}",
        fr.iter().map(|r| &r.text).collect::<Vec<_>>()
    );
    // an explicit formatting culture wins over the model's
    let fonts = FontBook::bundled_only();
    let o = RenderOptions {
        text_dump: true,
        culture: Some("en-US".into()),
        ..opts()
    };
    let en = render_page(
        &months_page("fr-FR", card),
        0,
        &HashMap::<String, Vec<u8>>::new(),
        &fonts,
        &o,
    )
    .unwrap()
    .texts;
    assert!(
        en.iter().any(|r| r.text == "12.5"),
        "{:?}",
        en.iter().map(|r| &r.text).collect::<Vec<_>>()
    );
}

#[test]
fn month_labels_are_whole_at_desktops_chart_size() {
    // pea (i, h): a theme whose label class is 10.5 pt still gets 9 pt (12 px) chart text, as in
    // Desktop 2.158; 21 "yyyy-mm" labels are turned whole under a wide line chart and flat,
    // without touching, under a combo chart
    let t = texts(&months_page(
        "en-US",
        json!([
            months_chart("lineChart", 20.0, 1054.0),
            months_chart("lineClusteredColumnComboChart", 400.0, 1208.0),
            months_chart("lineChart", 760.0, 592.0)
        ]),
    ));
    let labels = |lo: f32, hi: f32| -> Vec<&canvas::TextRun> {
        t.iter()
            .filter(|r| r.y >= lo && r.y < hi && (r.text.starts_with("2025") || r.text.starts_with("2026")))
            .collect()
    };
    let line = labels(0.0, 380.0);
    assert_eq!(
        line.len(),
        21,
        "{:?}",
        line.iter().map(|r| &r.text).collect::<Vec<_>>()
    );
    assert!(
        line.iter()
            .all(|r| r.text.len() == 7 && (r.size - 12.0).abs() < 0.01 && r.rotate != 0.0),
        "{:?}",
        line.iter()
            .map(|r| (&r.text, r.size, r.rotate))
            .collect::<Vec<_>>()
    );
    let combo = labels(380.0, 740.0);
    assert_eq!(
        combo.len(),
        21,
        "{:?}",
        combo.iter().map(|r| &r.text).collect::<Vec<_>>()
    );
    assert!(combo.iter().all(|r| r.text.len() == 7 && r.rotate == 0.0));
    for w in combo.windows(2) {
        assert!(
            w[0].x + w[0].w < w[1].x,
            "labels touch: {} {} / {}",
            w[0].text,
            w[0].x + w[0].w,
            w[1].x
        );
    }
    // about 24 px per category: still 35°, whole (Desktop 2.158)
    let narrow = labels(740.0, 1080.0);
    assert_eq!(narrow.len(), 21);
    assert!(
        narrow
            .iter()
            .all(|r| r.text.len() == 7 && (r.rotate + 35.0).abs() < 0.01),
        "{:?}",
        narrow.iter().map(|r| (&r.text, r.rotate)).collect::<Vec<_>>()
    );
}

#[test]
fn date_axis_labels_thin_out_and_scrolled_labels_turn_upright() {
    // rtri: 12 months whose labels fit their month but not with Desktop's gap between labels:
    // a tick every other month, flat (Desktop 2.158: "janv. 2025", "mars 2025"…), not
    // word-wrapped
    let fonts = FontBook::bundled_only();
    let s = snap(json!([]));
    let ctx = data::Ctx::new(&s, &fonts, None);
    let f = data::vis_format(&json!({}), "col", &ctx);
    let px = 32.0 / 3.0;
    let cd = data::ChartData {
        raw: (1..=12).map(|m| json!(format!("2025-{m:02}-01"))).collect(),
        cat_date: true,
        ..Default::default()
    };
    let ta = crate::timeaxis::TimeAxis::of(&cd, &[], &f).unwrap();
    let widest = charts::tw(&ctx, "Sep 2025", px);
    // a month (28 days at least) just wider than a label
    let s_day = f64::from(widest + px * 0.2) / 28.0;
    let ticks = ta.ticks(&ctx, 2000.0, 0.0, s_day, px, &|w| w + px * 2.3);
    let texts: Vec<&str> = ticks.iter().map(|t| t.1.as_str()).take(3).collect();
    assert_eq!(texts, ["Jan 2025", "Mar 2025", "May 2025"]);
    // 28 products on 560 px scroll at 8 pt: the labels turn 90°, as on Desktop
    let names: Vec<String> = (0..28).map(|i| format!("Product number {i}")).collect();
    let b = axes::Bands::new(28, 560.0, axes::INNER_PAD, axes::MIN_CATEGORY);
    assert!(b.scrolls() && b.step > px * 1.9, "{b:?}");
    let shown = names.get(..b.n).unwrap_or_default();
    let cl = axes::cat_labels(&ctx, shown, b.step, px, 300.0, &f);
    assert_eq!(cl.mode, axes::CatMode::Turn(90.0));
}

#[test]
fn pie_and_funnel_labels_are_12px_din_and_kpi_goal_keeps_the_culture_percent() {
    // rtri: Desktop 2.158 draws classic pie, donut and funnel labels at 12 px in
    // `wf_standard-font` (DIN); Segoe UI stands in at DIN's width
    let fields = json!([{"role": "Category", "kind": "column", "table": "Sales", "name": "Region"},
                        {"role": "Y", "kind": "measure", "table": "Sales", "name": "Total"}]);
    let pos = json!([["North", 120.0], ["South", 80.0], ["East", 45.5], ["West", 10.0]]);
    let d = data(
        json!([{"name": "Region", "role": "Category", "kind": "column", "data_type": "string"},
               {"name": "Total", "role": "Y", "kind": "measure"}]),
        pos,
    );
    let t = texts(&snap(json!([
        {"name": "p", "type": "pieChart", "x": 0, "y": 0, "width": 300, "height": 300, "fields": fields, "data": d},
        {"name": "f", "type": "funnel", "x": 320, "y": 0, "width": 300, "height": 300, "fields": fields, "data": d}
    ])));
    let labels: Vec<&canvas::TextRun> = t
        .iter()
        .filter(|r| ["North", "South", "East", "West", "100%"].contains(&r.text.as_str()))
        .collect();
    assert!(labels.iter().any(|r| r.x < 300.0) && labels.iter().any(|r| r.text == "100%"));
    assert!(
        labels
            .iter()
            .all(|r| r.size == 12.0 && r.family.starts_with("DIN")),
        "{:?}",
        labels
            .iter()
            .map(|r| (&r.text, r.size, &r.family))
            .collect::<Vec<_>>()
    );
    let fonts = FontBook::bundled_only();
    let seg = fonts.measure(&fonts::Font::new("Segoe UI", 12.0), "100%");
    let din = fonts.measure(&fonts::Font::new("DIN, Segoe UI", 12.0), "100%");
    assert!((din / seg - 0.92).abs() < 0.001, "{din} / {seg}");
    // the KPI goal's distance: "-33,33 %" with a French model (a value's "0.0%" format string
    // stays "38,0%")
    let kpi = json!([{"name": "k", "type": "kpi", "x": 0, "y": 0, "width": 400, "height": 300,
        "data": data(json!([{"name": "YM", "role": "TrendLine", "kind": "column", "data_type": "string"},
                            {"name": "Spend", "role": "Indicator", "kind": "measure"},
                            {"name": "Prior", "role": "Goal", "kind": "measure"}]),
                     json!([["2025-01", 50.0, 60.0], ["2025-02", 40.0, 60.0]]))}]);
    let t = texts(&months_page("fr-FR", kpi));
    assert!(
        t.iter().any(|r| r.text.ends_with("(-33,33\u{a0}%)")),
        "{:?}",
        t.iter().map(|r| &r.text).collect::<Vec<_>>()
    );
}

#[test]
fn scatter_axes_take_their_font_sizes() {
    // x_axis / y_axis font sizes apply to the scatter's tick labels, as on bar and line charts
    let rows = json!([["a", 10, 100], ["b", 20, 300], ["c", 35, 250]]);
    let cols = json!([{"name": "Region", "role": "Category", "kind": "column", "data_type": "string"},
                      {"name": "Units", "role": "X", "kind": "measure"},
                      {"name": "Total", "role": "Y", "kind": "measure"}]);
    let v = json!([{"name": "s", "type": "scatterChart", "x": 0, "y": 0, "width": 600, "height": 330,
        "data": data(cols, rows),
        "format": {"x_axis": {"font_size": 14}, "y_axis": {"font_size": 7}}}]);
    let t = texts(&snap(v));
    let size = |s: &str| t.iter().find(|r| r.text == s).map(|r| r.size);
    let px = |pt: f32| pt * crate::data::PT;
    // a y tick ("300") and an x tick ("20")
    assert!(
        size("300").is_some_and(|z| (z - px(7.0)).abs() < 0.05),
        "{:?}",
        size("300")
    );
    assert!(
        size("20").is_some_and(|z| (z - px(14.0)).abs() < 0.05),
        "{:?}",
        size("20")
    );
}

#[test]
fn small_multiples_keep_per_series_label_settings_on_their_series() {
    // the split column (1) sits before the series: `series_labels` of column 3 (Cost) must
    // follow it to column 2 of each panel, not land on another series
    let cols = json!([{"name": "Region", "role": "Category", "kind": "column", "data_type": "string"},
                      {"name": "Year", "role": "Rows", "kind": "column", "data_type": "string"},
                      {"name": "Total", "role": "Y", "kind": "measure"},
                      {"name": "Cost", "role": "Y", "kind": "measure"}]);
    let rows = json!([
        ["a", "Y1", 11, 77],
        ["b", "Y1", 13, 79],
        ["a", "Y2", 11, 77],
        ["b", "Y2", 13, 79]
    ]);
    let v = json!([{"name": "s", "type": "clusteredColumnChart", "x": 0, "y": 0, "width": 600, "height": 330,
        "data": data(cols, rows),
        "format": {"small_multiples": {"column": 1},
                   "series_labels": [{"column": 3, "show": true, "color": "#FF0000"}]}}]);
    let t = texts(&snap(v));
    let red = |s: &str| t.iter().filter(|r| r.text == s).count();
    assert!(
        red("77") >= 2,
        "{:?}",
        t.iter().map(|r| &r.text).collect::<Vec<_>>()
    );
    assert_eq!(
        red("11"),
        0,
        "{:?}",
        t.iter().map(|r| &r.text).collect::<Vec<_>>()
    );
    assert!(
        t.iter()
            .filter(|r| r.text == "77")
            .all(|r| r.color == crate::canvas::Rgb(0xff, 0, 0))
    );
}

/// M10.2 English oracle (Desktop 2.158 in English): an empty visual's prompt; a table names a
/// distinct count "Count of" (the field's label, not the engine's data column name); a decimal
/// column's aggregation shows two decimals; a `Long Date` column and a `General Date` measure
/// keep their formats; a KPI's goal uses its own format string.
#[test]
fn english_desktop_texts() {
    let t = |s: &Value| texts(s).into_iter().map(|r| r.text).collect::<Vec<_>>();
    let empty = json!([{"name": "e", "type": "clusteredColumnChart", "x": 0, "y": 0, "width": 400, "height": 300,
        "fields": [], "data": {"status": "ok", "columns": [], "rows": []}}]);
    let mut s = snap(empty);
    s["ui"] = json!({"strings_language": "en", "strings": {"empty_visual": "Add fields to show data in this visual"}});
    assert!(
        t(&s)
            .iter()
            .any(|x| x == "Add fields to show data in this visual"),
        "{:?}",
        t(&s)
    );

    let table = json!([{"name": "t", "type": "tableEx", "x": 0, "y": 0, "width": 600, "height": 300,
        "fields": [
            {"role": "Values", "kind": "column", "table": "Sales", "name": "Month"},
            {"role": "Values", "kind": "aggregation", "table": "Sales", "name": "Amount", "function": "Count", "label": "Count of Amount"},
            {"role": "Values", "kind": "aggregation", "table": "Sales", "name": "Amount", "function": "Avg", "label": "Average of Amount"},
            {"role": "Values", "kind": "measure", "table": "Sales", "name": "Last"}],
        "data": data(json!([
            {"name": "Month", "role": "Values", "kind": "column", "data_type": "dateTime"},
            {"name": "Count (Distinct) of Amount", "role": "Values", "kind": "aggregation", "data_type": "int64"},
            {"name": "Average of Amount", "role": "Values", "kind": "aggregation", "data_type": "double"},
            {"name": "Last", "role": "Values", "kind": "measure"}]),
            json!([["2025-01-01T00:00:00", 36, 3081.3, "2025-12-01T00:00:00"]]))}]);
    let mut s = snap(table);
    s["model"] = json!({"culture": "en-US", "tables": [{"name": "Sales",
        "columns": [{"name": "Month", "data_type": "dateTime", "format_string": "Long Date"},
                    {"name": "Amount", "data_type": "double"}],
        "measures": [{"name": "Last", "format_string": "General Date"}]}]});
    let got = t(&s);
    for want in [
        "Count of Amount",
        "3,081.30",
        "Wednesday, January 1, 2025",
        "12/1/2025 12:00:00 AM",
    ] {
        assert!(got.iter().any(|x| x == want), "{want} not in {got:?}");
    }
    assert!(!got.iter().any(|x| x.contains("Count (Distinct)")), "{got:?}");
}

/// A KPI's goal in the goal's own format string ("Goal: 74,360" for a `#,0` goal beside a
/// `#,0.00` indicator) and, in an English UI, "%" without a space even for a French culture
/// (Desktop 2.158 in English on a fr-FR locale: "(+5,26%)").
#[test]
fn kpi_goal_format_and_english_percent() {
    let kpi = json!([{"name": "k", "type": "kpi", "x": 0, "y": 0, "width": 400, "height": 300,
        "fields": [{"role": "Indicator", "kind": "measure", "table": "Sales", "name": "Total"},
                   {"role": "TrendLine", "kind": "column", "table": "Sales", "name": "Month"},
                   {"role": "Goal", "kind": "measure", "table": "Sales", "name": "Goal"}],
        "data": data(json!([{"name": "Month", "role": "TrendLine", "kind": "column", "data_type": "string"},
                            {"name": "Total", "role": "Indicator", "kind": "measure"},
                            {"name": "Goal", "role": "Goal", "kind": "measure"}]),
                     json!([["a", 70000.0, 70000.0], ["b", 78273.69, 74360.0055]]))}]);
    let mut s = snap(kpi);
    s["model"] = json!({"culture": "fr-FR", "tables": [{"name": "Sales", "columns": [{"name": "Month", "data_type": "string"}],
        "measures": [{"name": "Total", "format_string": "#,0.00"}, {"name": "Goal", "format_string": "#,0"}]}]});
    s["ui"] = json!({"strings_language": "en", "strings": {"goal": "Goal"}});
    let got: Vec<String> = texts(&s).into_iter().map(|r| r.text).collect();
    // two runs, as Desktop's two text nodes
    assert!(
        got.iter()
            .any(|x| x == "Goal: 74\u{a0}360" || x == "Goal: 74\u{202f}360"),
        "{got:?}"
    );
    assert!(got.iter().any(|x| x == "(+5,26%)"), "{got:?}");
}

fn render_hits(s: &Value) -> PageImage {
    let fonts = FontBook::bundled_only();
    let o = RenderOptions {
        hits: true,
        raw: true,
        ..opts()
    };
    render_page(s, 0, &HashMap::<String, Vec<u8>>::new(), &fonts, &o).unwrap()
}

fn vis(name: &str, ty: &str, x: f64, w: f64, d: Value) -> Value {
    json!({"name": name, "type": ty, "x": x, "y": 10, "width": w, "height": 200, "data": d,
           "fields": [{"role": "Category", "kind": "column", "table": "Sales", "name": "Region"}]})
}

/// The `view.interact` action of the topmost hit at (x, y), with Ctrl or not.
fn action_at(img: &PageImage, x: f32, y: f32, ctrl: bool) -> Option<Value> {
    hits::hit_at(&img.hits, x, y).and_then(|h| h.action_for(ctrl))
}

#[test]
fn hit_maps_follow_the_drawing() {
    let slicer_data = data(
        json!([{"name": "Region", "role": "Values", "kind": "column", "data_type": "string"}]),
        json!([["North"], ["South"]]),
    );
    let mut slicer = vis("s1", "slicer", 10.0, 150.0, slicer_data);
    slicer["slicer"] = json!({"mode": "list"});
    let s = snap(json!([
        slicer,
        vis("c1", "clusteredColumnChart", 170.0, 300.0, cat_val()),
        vis("t1", "tableEx", 480.0, 150.0, cat_val()),
    ]));
    // off by default: no hits, a PNG
    assert!(render(&s).hits.is_empty());
    let img = render_hits(&s);
    assert!(img.png.is_empty());
    assert_eq!(img.rgba.len(), (img.width * img.height * 4) as usize);
    // every visual catches clicks over its whole box
    for (name, x) in [("s1", 12.0), ("c1", 172.0), ("t1", 482.0)] {
        assert_eq!(
            hits::hit_at(&img.hits, x, 12.0).map(|h| h.visual.as_str()),
            Some(name)
        );
    }
    assert!(hits::hit_at(&img.hits, 5.0, 5.0).is_none());
    // the slicer's first item toggles North; Ctrl is passed through
    let item = img
        .hits
        .iter()
        .filter(|h| h.visual == "s1")
        .filter_map(|h| h.action_for(true))
        .find(|a| a["action"] == "slicer_toggle")
        .unwrap();
    assert_eq!(
        item,
        json!({"action": "slicer_toggle", "page": "p1", "visual": "s1", "value": "North", "ctrl": true})
    );
    // each column is a data point with its tooltip
    let cols: Vec<&hits::Hit> = img
        .hits
        .iter()
        .filter(|h| h.visual == "c1" && matches!(h.action, hits::HitAction::Send { .. }))
        .collect();
    assert_eq!(cols.len(), 4);
    let a = cols[0].action_for(false).unwrap();
    assert_eq!(a["action"], "select");
    assert_eq!(a["point"], json!([{"c": 0, "v": "North"}]));
    assert_eq!(a["ctrl"], false);
    assert_eq!(cols[0].tip[0], ("Region".to_string(), "North".to_string()));
    let (x, y, w, h) = cols[1].shape.bounds();
    assert_eq!(
        action_at(&img, x + w / 2.0, y + h / 2.0, false).unwrap()["point"][0]["v"],
        "South"
    );
    // table rows: the grouping columns only
    let row = img
        .hits
        .iter()
        .filter(|h| h.visual == "t1")
        .find_map(|h| h.action_for(false))
        .unwrap();
    assert_eq!(row["point"], json!([{"c": 0, "v": "North"}]));
}

#[test]
fn selections_dim_and_highlights_draw_parts() {
    let mut src = vis("c1", "clusteredColumnChart", 10.0, 300.0, cat_val());
    src["interaction"] = json!({"selected": {"points": [[{"c": 0, "v": "North"}]], "rows": [0]}});
    let mut other = vis("c2", "clusteredColumnChart", 320.0, 300.0, cat_val());
    other["interaction"] = json!({"filtered_by": "c1", "mode": "highlight"});
    other["data"]["highlights"] = json!([[null, 60.0], [null, null], [null, null], [null, null]]);
    let s = snap(json!([src, other]));
    let img = render_hits(&s);
    let px = |x: f32, y: f32| {
        let i = ((y.round() as u32 * img.width + x.round() as u32) * 4) as usize;
        [img.rgba[i], img.rgba[i + 1], img.rgba[i + 2]]
    };
    let centre = |vis: &str, k: usize| {
        let h = img
            .hits
            .iter()
            .filter(|h| h.visual == vis && h.action_for(false).is_some_and(|a| a["point"].is_array()))
            .nth(k)
            .unwrap();
        let (x, y, w, hh) = h.shape.bounds();
        (x + w / 2.0, y, hh)
    };
    // the selected column is opaque, the others faded (lighter)
    let (x0, y0, h0) = centre("c1", 0);
    let (x1, y1, h1) = centre("c1", 1);
    let lum = |p: [u8; 3]| p.iter().map(|c| u32::from(*c)).sum::<u32>();
    assert!(lum(px(x0, y0 + h0 / 2.0)) < lum(px(x1, y1 + h1 / 2.0)));
    // the highlighted visual: half of North's column opaque at the bottom, faded above
    let (x, y, h) = centre("c2", 0);
    assert!(lum(px(x, y + h * 0.9)) < lum(px(x, y + h * 0.1)));
    // a click on the source visual's background clears its selection
    let bg = action_at(&img, 12.0, 12.0, false).unwrap();
    assert_eq!(
        bg,
        json!({"action": "select", "page": "p1", "visual": "c1", "point": null})
    );
}

#[test]
fn decomposition_trees_draw_and_click() {
    let d = json!({"status": "ok",
        "columns": [{"name": "Total", "role": "Analyze", "kind": "measure"},
                    {"name": "Region", "role": "ExplainBy", "kind": "column", "data_type": "string"},
                    {"name": "Category", "role": "ExplainBy", "kind": "column", "data_type": "string"},
                    {"name": "Channel", "role": "ExplainBy", "kind": "column", "data_type": "string"}],
        "rows": [],
        "decomp": {"analyze": 0, "root": 300.0, "options": {},
                   "levels": [{"column": 1, "ai": null, "nodes": [["North", 200.0], ["South", 100.0]], "total": 2, "selected": 0, "auto": false},
                              {"column": 2, "ai": "high", "nodes": [["Bikes", 150.0], ["Clothing", 50.0]], "total": 2, "selected": 0, "auto": true}]}});
    let v = json!({"name": "dt", "type": "decompositionTreeVisual", "x": 0, "y": 0, "width": 640, "height": 360, "data": d,
        "fields": [{"role": "Analyze", "kind": "measure", "table": "Sales", "name": "Total"},
                   {"role": "ExplainBy", "kind": "column", "table": "Sales", "name": "Region"},
                   {"role": "ExplainBy", "kind": "column", "table": "Sales", "name": "Category"},
                   {"role": "ExplainBy", "kind": "column", "table": "Sales", "name": "Channel"}]});
    let s = snap(json!([v]));
    let img = render(&s);
    assert_eq!(img.visuals[0].status, "drawn");
    assert!(ink(&img) > 0.01);
    let got: Vec<String> = texts(&s).into_iter().map(|r| r.text).collect();
    for t in ["Total", "300", "North", "South", "Bikes", "Region", "Category"] {
        assert!(got.iter().any(|x| x == t), "{t} in {got:?}");
    }
    // the hit map: a path bar selects, another bar moves the path; the header cross collapses
    let img = render_hits(&s);
    let lay = decomp::layout(
        &s["report"]["pages"][0]["visuals"][0]["data"]["decomp"],
        630.0,
        350.0,
    );
    let (x1, north, south) = (
        lay.levels[1].x + 5.0 + 5.0,
        lay.levels[1].nodes[0].y + 5.0,
        lay.levels[1].nodes[1].y + 5.0,
    );
    let a = action_at(&img, x1, north, false).unwrap();
    assert_eq!(a["action"], "select");
    assert_eq!(a["point"], json!([{"c": 1, "v": "North"}]));
    let a = action_at(&img, x1, south, false).unwrap();
    assert_eq!(a["action"], "decomp_path");
    assert_eq!(
        (a["level"].clone(), a["value"].clone()),
        (json!(1), json!("South"))
    );
    let deep = action_at(
        &img,
        lay.levels[2].x + 10.0,
        lay.levels[2].nodes[0].y + 5.0,
        false,
    )
    .unwrap();
    assert_eq!(
        deep["point"],
        json!([{"c": 1, "v": "North"}, {"c": 2, "v": "Bikes"}])
    );
    let plus = hits::hit_at(
        &img.hits,
        lay.levels[2].x + 5.0 + 130.0 + 11.0,
        lay.levels[2].nodes[0].y + 5.0 - 2.4,
    )
    .unwrap();
    match &plus.action {
        hits::HitAction::Popup(p) => {
            assert_eq!(p["kind"], "decomp_expand");
            assert!(p["items"].as_array().unwrap().iter().any(|i| i["ai"] == "high"));
        }
        other => panic!("{other:?}"),
    }
    // a hostile tree never panics
    let mut bad = s.clone();
    bad["report"]["pages"][0]["visuals"][0]["data"]["decomp"] =
        json!({"analyze": 99, "root": "x", "levels": [{"column": 1e30, "nodes": [[{}, []]], "selected": 5}]});
    let _ = render(&bad);
    bad["report"]["pages"][0]["visuals"][0]["data"] = json!({"status": "unavailable"});
    assert_eq!(render(&bad).visuals[0].status, "preview");
}

/// A tree's scroll arrows and its Collapse click action (G34): the arrows send the window
/// before or after the one shown, a tree wider than the visual scrolls by levels, and with
/// `defaultClickAction: toggle` a click on an intermediate path member removes the levels below.
#[test]
fn decomposition_trees_scroll_and_collapse() {
    let tree = |width: f64, start: Value, shift: u64, options: Value| {
        let regions: Vec<Value> = (0..10)
            .map(|i| json!([format!("R{i}"), 100.0 - f64::from(i)]))
            .collect();
        let d = json!({"status": "ok",
            "columns": [{"name": "Total", "role": "Analyze", "kind": "measure"},
                        {"name": "Region", "role": "ExplainBy", "kind": "column", "data_type": "string"},
                        {"name": "Category", "role": "ExplainBy", "kind": "column", "data_type": "string"}],
            "rows": [],
            "decomp": {"analyze": 0, "root": 900.0, "options": options, "shift": shift,
                       "levels": [{"column": 1, "nodes": regions, "total": 10, "selected": 0, "start": start},
                                  {"column": 2, "nodes": [["Bikes", 60.0], ["Clothing", 40.0]], "total": 2, "selected": 0}]}});
        let v = json!({"name": "dt", "type": "decompositionTreeVisual", "x": 0, "y": 0, "width": width, "height": 360, "data": d,
            "fields": [{"role": "Analyze", "kind": "measure", "table": "Sales", "name": "Total"},
                       {"role": "ExplainBy", "kind": "column", "table": "Sales", "name": "Region"},
                       {"role": "ExplainBy", "kind": "column", "table": "Sales", "name": "Category"}]});
        snap(json!([v]))
    };
    let lay_of = |s: &Value, w: f32| {
        decomp::layout(
            &s["report"]["pages"][0]["visuals"][0]["data"]["decomp"],
            w - 10.0,
            350.0,
        )
    };
    // the first window: a down arrow under level 1 sends the next window
    let s = tree(640.0, Value::Null, 0, json!({}));
    let lay = lay_of(&s, 640.0);
    let l1 = &lay.levels[1];
    assert!(l1.down && !l1.up, "{l1:?}");
    let cx = l1.x + lay.bar_w / 2.0 - 2.0 + 5.0;
    let img = render_hits(&s);
    let a = action_at(&img, cx, lay.bottom + 57.0 + 5.0, false).unwrap();
    assert_eq!(a["action"], "decomp_scroll");
    assert_eq!(a["level"], 1);
    let next = a["start"].as_u64().unwrap() as usize;
    assert!(next > l1.start);
    // scrolled there: the window moved, an up arrow goes back, the level below stays shown
    // although its parent left the window
    let s = tree(640.0, json!(next), 0, json!({}));
    let lay = lay_of(&s, 640.0);
    let l1 = &lay.levels[1];
    assert!(l1.start > 0 && l1.up, "{l1:?}");
    assert!(l1.nodes.iter().all(|n| !n.on_path));
    assert_eq!(lay.levels.len(), 3);
    let img = render_hits(&s);
    let a = action_at(&img, cx, lay.top - 27.0 + 5.0, false).unwrap();
    assert_eq!(a["action"], "decomp_scroll");
    assert!(a["start"].as_u64().unwrap() < l1.start as u64);
    // a tree wider than the visual: an arrow on the right scrolls it by one level
    let s = tree(300.0, Value::Null, 0, json!({}));
    let lay = lay_of(&s, 300.0);
    assert!(lay.right && !lay.left);
    let mid = (lay.top + lay.bottom) / 2.0 + 5.0;
    let img = render_hits(&s);
    let a = action_at(&img, 300.0 - 5.0 - 9.0, mid, false).unwrap();
    assert_eq!(
        (a["action"].clone(), a["shift"].clone()),
        (json!("decomp_scroll"), json!(1))
    );
    // scrolled by two levels: an arrow on the left goes back, the hidden level 1 is not
    // clickable
    let s = tree(300.0, Value::Null, 2, json!({}));
    let lay = lay_of(&s, 300.0);
    assert!(
        lay.left && !lay.right && lay.levels[1].x + lay.bar_w < 0.0,
        "{lay:?}"
    );
    let img = render_hits(&s);
    let a = action_at(&img, 5.0 + 9.0, mid, false).unwrap();
    assert_eq!(a["shift"], 1);
    assert!(
        img.hits
            .iter()
            .all(|h| h.shape.bounds().0 + h.shape.bounds().2 > 0.0)
    );
    // Collapse: a click on the intermediate path member removes the levels below it; the last
    // level's member still selects
    let s = tree(
        640.0,
        Value::Null,
        0,
        json!({"tree": {"defaultClickAction": "toggle"}}),
    );
    let lay = lay_of(&s, 640.0);
    let img = render_hits(&s);
    let a = action_at(
        &img,
        lay.levels[1].x + 10.0,
        lay.levels[1].nodes[0].y + 5.0,
        false,
    )
    .unwrap();
    assert_eq!(
        (a["action"].clone(), a["level"].clone()),
        (json!("decomp_collapse"), json!(2))
    );
    let a = action_at(
        &img,
        lay.levels[2].x + 10.0,
        lay.levels[2].nodes[0].y + 5.0,
        false,
    )
    .unwrap();
    assert_eq!(a["action"], "select");
}

/// Bulleted and numbered paragraphs draw their marker before the text (M11.20).
#[test]
fn text_box_lists_draw_their_markers() {
    let tb = |paras: Value| {
        snap(
            json!([{"name": "tx", "type": "textbox", "x": 10, "y": 10, "width": 300, "height": 120,
                     "text": {"paragraphs": paras}}]),
        )
    };
    let t = texts(&tb(json!([
        {"list": "bullet", "runs": [{"text": "First"}]},
        {"list": "number", "runs": [{"text": "One"}]},
        {"list": "number", "runs": [{"text": "Two", "bold": true}]},
        {"runs": [{"text": "Plain"}]},
        {"list": "number", "runs": [{"text": "Again"}]},
        {"list": "teleport", "runs": [{"text": "Odd"}]}
    ])));
    let all: Vec<&str> = t.iter().map(|r| r.text.as_str()).collect();
    for m in ["\u{2022}", "1.", "2.", "First", "Two", "Plain", "Odd"] {
        assert!(all.iter().any(|x| x.contains(m)), "{m} missing from {all:?}");
    }
    // numbering restarts after a plain paragraph; an unknown list kind has no marker
    assert_eq!(all.iter().filter(|x| x.contains("1.")).count(), 2, "{all:?}");
    assert!(!all.iter().any(|x| x.contains("3.")), "{all:?}");
}

/// An image or a shape with an action catches clicks as a button does; one without does not.
#[test]
fn images_and_shapes_with_an_action_are_clickable() {
    let s = snap(json!([
        {"name": "im", "type": "image", "x": 10, "y": 10, "width": 100, "height": 100,
         "interaction": {"button": {"kind": "PageNavigation", "page": "p2"}}},
        {"name": "sh", "type": "shape", "x": 120, "y": 10, "width": 100, "height": 100,
         "interaction": {"button": {"kind": "WebUrl", "url": "https://example.com"}}},
        {"name": "plain", "type": "image", "x": 230, "y": 10, "width": 100, "height": 100}
    ]));
    let img = render_hits(&s);
    let a = action_at(&img, 50.0, 50.0, false).unwrap();
    assert_eq!(a, json!({"action": "button", "page": "p1", "visual": "im"}));
    let a = action_at(&img, 160.0, 50.0, false).unwrap();
    assert_eq!(a["visual"], "sh");
    assert!(action_at(&img, 260.0, 50.0, false).is_none());
}

/// A snapshot of three pages (the second hidden, the third a tooltip page) and three bookmarks,
/// two of them in group `g1`, the reader having applied `b2`.
fn navigators(visuals: Value) -> Value {
    let mut s = snap(visuals);
    s["report"]["pages"] = json!([
        s["report"]["pages"][0].clone(),
        {"name": "p2", "display_name": "Hidden", "hidden": true, "width": 640, "height": 360, "visuals": []},
        {"name": "p3", "display_name": "Tip", "page_type": "tooltip", "width": 320, "height": 240, "visuals": []}
    ]);
    s["bookmarks"] = json!({"bookmarks": [
        {"name": "b1", "display_name": "First", "group": null, "group_name": null},
        {"name": "b2", "display_name": "Second", "group": "Group 1", "group_name": "g1"},
        {"name": "b3", "display_name": "Third", "group": "Group 1", "group_name": "g1"}
    ]});
    s["interaction"] = json!({"bookmark": "b2"});
    s
}

/// The hits of visual `name` that send an action, in drawing order, with their boxes.
fn sent(img: &PageImage, name: &str) -> Vec<(Value, (f32, f32, f32, f32))> {
    img.hits
        .iter()
        .filter(|h| h.visual == name)
        .filter_map(|h| Some((h.action_for(false)?, h.shape.bounds())))
        .collect()
}

#[test]
fn bookmark_navigators_draw_their_group_and_click_a_bookmark() {
    let s = navigators(json!([
        {"name": "all", "type": "bookmarkNavigator", "x": 0, "y": 0, "width": 600, "height": 40},
        {"name": "grp", "type": "bookmarkNavigator", "x": 0, "y": 100, "width": 150, "height": 200,
         "navigator": {"orientation": "vertical", "bookmark_group": "g1"}},
        {"name": "none", "type": "bookmarkNavigator", "x": 300, "y": 100, "width": 150, "height": 60,
         "navigator": {"bookmark_group": "nothing"}}
    ]));
    let img = render_hits(&s);
    let st: Vec<(&str, &str)> = img.visuals.iter().map(|v| (v.name.as_str(), v.status)).collect();
    assert_eq!(st, vec![("all", "drawn"), ("grp", "drawn"), ("none", "drawn")]);
    // no group: every bookmark, in a row
    let all = sent(&img, "all");
    let names: Vec<&Value> = all.iter().map(|(a, _)| &a["bookmark"]).collect();
    assert_eq!(names, vec!["b1", "b2", "b3"]);
    assert_eq!(
        all[0].0,
        json!({"action": "bookmark", "bookmark": "b1", "page": "p1", "visual": "all"})
    );
    assert!(
        all[1].1.0 > all[0].1.0 && all[1].1.1 == all[0].1.1,
        "a row: {all:?}"
    );
    // its group, in a column
    let grp = sent(&img, "grp");
    let names: Vec<&Value> = grp.iter().map(|(a, _)| &a["bookmark"]).collect();
    assert_eq!(names, vec!["b2", "b3"]);
    assert!(
        grp[1].1.1 > grp[0].1.1 && grp[1].1.0 == grp[0].1.0,
        "a column: {grp:?}"
    );
    let a = action_at(&img, 20.0, grp[1].1.1 + 5.0, false).unwrap();
    assert_eq!(a["bookmark"], "b3");
    assert!(sent(&img, "none").is_empty());
    // the bookmark applied last is the highlighted one
    let runs = texts(&s);
    let weight = |t: &str| {
        runs.iter()
            .filter(|r| r.text == t)
            .map(|r| r.weight)
            .collect::<Vec<_>>()
    };
    assert_eq!(weight("Second"), vec![600, 600]);
    assert_eq!(weight("First"), vec![400]);
    assert_eq!(weight("Third"), vec![400, 400]);
    // none applied: the navigator's selected bookmark
    let mut s2 = s.clone();
    s2["interaction"] = json!({});
    s2["report"]["pages"][0]["visuals"][1]["navigator"]["selected_bookmark"] = json!("b3");
    let runs = texts(&s2);
    let w3: Vec<u16> = runs
        .iter()
        .filter(|r| r.text == "Third")
        .map(|r| r.weight)
        .collect();
    assert_eq!(w3, vec![400, 600]);
}

/// A navigator's styles (`navigator.styles`, the theme schema's `fill`, `outline`, `text`,
/// `accentBar` per state): the default fill and font colour on every item, the selected state's
/// over them on the selected one, the accent bar where its position says, the padding between.
#[test]
fn bookmark_navigators_draw_their_styles() {
    let s = navigators(json!([
        {"name": "nav", "type": "bookmarkNavigator", "x": 0, "y": 0, "width": 600, "height": 60,
         "navigator": {"padding": 20, "styles": {
            "default": {"fill": "#102030", "font_color": "#FFFFFF", "show_accent": false},
            "selected": {"fill": "#C04000", "font_color": "#00FF00", "bold": false,
                         "show_accent": true, "accent": "#0000FF", "accent_position": "top",
                         "accent_width": 6}}}}
    ]));
    let runs = texts(&s);
    let run = |t: &str| runs.iter().find(|r| r.text == t).cloned().unwrap();
    assert_eq!(run("First").color, Rgb(255, 255, 255));
    assert_eq!(run("Second").color, Rgb(0, 255, 0), "the selected item's colour");
    assert_eq!(run("Second").weight, 400, "bold off in the selected state");
    let img = render_hits(&s);
    let boxes: Vec<(f32, f32, f32, f32)> = sent(&img, "nav").into_iter().map(|(_, b)| b).collect();
    assert_eq!(boxes.len(), 3);
    assert!(
        (boxes[1].0 - (boxes[0].0 + boxes[0].2) - 20.0).abs() < 0.5,
        "20 px between items: {boxes:?}"
    );
    let img = render(&s);
    let pm = tiny_skia::Pixmap::decode_png(&img.png).unwrap();
    let px = |x: f32, y: f32| {
        let p = pm.pixel((x * img.scale) as u32, (y * img.scale) as u32).unwrap();
        (p.red(), p.green(), p.blue())
    };
    let (b0, b1) = (boxes[0], boxes[1]);
    assert_eq!(px(b0.0 + 4.0, b0.1 + 4.0), (0x10, 0x20, 0x30), "default fill");
    assert_eq!(
        px(b1.0 + 4.0, b1.1 + 2.0),
        (0, 0, 255),
        "the accent bar along the top"
    );
    assert_eq!(
        px(b1.0 + 4.0, b1.1 + b1.3 - 3.0),
        (0xC0, 0x40, 0x00),
        "selected fill"
    );
    assert_eq!(
        px(b0.0 + 4.0, b0.1 + 2.0),
        (0x10, 0x20, 0x30),
        "no bar on the others"
    );
}

/// A navigator's buttons take their tile shape (`navigator.shape`) and draw their shadow and
/// glow (`show_shadow`, `show_glow` of a state) around them, in the padding between buttons
/// (G55).
#[test]
fn navigator_buttons_draw_their_shape_shadow_and_glow() {
    let nav = |shape: Value, default: Value| {
        navigators(json!([
            {"name": "nav", "type": "bookmarkNavigator", "x": 0, "y": 0, "width": 600, "height": 60,
             "navigator": {"padding": 20, "shape": shape,
                           "styles": {"default": default, "selected": {}}}}
        ]))
    };
    let fill = json!({"fill": "#102030"});
    let px_of = |s: &Value| {
        let img = render(s);
        let pm = tiny_skia::Pixmap::decode_png(&img.png).unwrap();
        let scale = img.scale;
        move |x: f32, y: f32| {
            let p = pm.pixel((x * scale) as u32, (y * scale) as u32).unwrap();
            (p.red(), p.green(), p.blue())
        }
    };
    let plain = nav(Value::Null, fill.clone());
    let boxes: Vec<(f32, f32, f32, f32)> = sent(&render_hits(&plain), "nav")
        .into_iter()
        .map(|(_, b)| b)
        .collect();
    let b0 = boxes[0];
    let (mid, gap) = (b0.1 + b0.3 / 2.0, b0.0 + b0.2 + 3.0);
    let px = px_of(&plain);
    let navy = (0x10, 0x20, 0x30);
    assert_eq!(px(b0.0 + 1.0, b0.1 + 1.0), navy, "a rectangle fills its corner");
    let background = px(gap, mid);
    assert_ne!(background, navy);

    // a pill leaves its corners empty and keeps its middle
    let px = px_of(&nav(json!({"tile": "pill"}), fill.clone()));
    assert_ne!(px(b0.0 + 1.0, b0.1 + 1.0), navy, "the pill's corner");
    // inside its straight top edge, away from the text
    assert_eq!(px(b0.0 + b0.3 / 2.0 + 2.0, b0.1 + 3.0), navy);

    // a black shadow moved right falls in the padding
    let mut shadow = fill.clone();
    shadow["show_shadow"] = json!(true);
    shadow["shadow_angle"] = json!(0);
    shadow["shadow_distance"] = json!(6);
    shadow["shadow_transparency"] = json!(0);
    let (r, g, b) = px_of(&nav(Value::Null, shadow))(gap, mid);
    assert!(
        u16::from(r) + u16::from(g) + u16::from(b)
            < u16::from(background.0) + u16::from(background.1) + u16::from(background.2),
        "the shadow darkens the padding: {:?} vs {background:?}",
        (r, g, b)
    );

    // a green glow around the button
    let mut glow = fill;
    glow["show_glow"] = json!(true);
    glow["glow"] = json!("#00FF00");
    glow["glow_transparency"] = json!(0);
    let (r, g, b) = px_of(&nav(Value::Null, glow))(b0.0 + b0.2 + 1.5, mid);
    assert!(g > r && g > b, "the glow is green: {:?}", (r, g, b));
}

#[test]
fn page_navigators_list_the_pages_their_options_show() {
    let s = navigators(json!([
        {"name": "nv", "type": "pageNavigator", "x": 0, "y": 0, "width": 600, "height": 40},
        {"name": "all", "type": "pageNavigator", "x": 0, "y": 100, "width": 600, "height": 120,
         "navigator": {"orientation": "grid", "columns": 2, "show_hidden_pages": true, "show_tooltip_pages": true}}
    ]));
    let img = render_hits(&s);
    let nv = sent(&img, "nv");
    assert_eq!(
        nv.iter().map(|(a, _)| a.clone()).collect::<Vec<_>>(),
        vec![json!({"action": "page", "page": "p1", "visual": "nv"})],
        "hidden and tooltip pages are left out"
    );
    let all = sent(&img, "all");
    let pages: Vec<&Value> = all.iter().map(|(a, _)| &a["page"]).collect();
    assert_eq!(pages, vec!["p1", "p2", "p3"]);
    // two columns: the third page starts the second row
    assert_eq!(all[2].1.0, all[0].1.0);
    assert!(all[2].1.1 > all[0].1.1);
    assert!(all[1].1.0 > all[0].1.0);
    // the current page is highlighted
    let runs = texts(&s);
    let w: Vec<u16> = runs
        .iter()
        .filter(|r| r.text == "Page 1")
        .map(|r| r.weight)
        .collect();
    assert_eq!(w, vec![600, 600]);
}

/// High contrast (the app's view option): the white page turns black, the text white (on the
/// light button fill, now black, too), the column series bright on black; without the option
/// the same snapshot draws as before.
#[test]
fn high_contrast_rendering() {
    use crate::canvas::HIGH_CONTRAST_COLORS;
    let s = snap(json!([
        {"name": "c", "type": "clusteredColumnChart", "x": 10, "y": 10, "width": 300, "height": 200, "title": "Sales",
         "fields": [{"role": "Category", "kind": "column", "table": "Sales", "name": "Region"},
                    {"role": "Y", "kind": "measure", "table": "Sales", "name": "Total"}],
         "data": cat_val()},
        {"name": "bt", "type": "actionButton", "x": 330, "y": 10, "width": 120, "height": 40,
         "button": {"text": "Back", "fill": "#E6E6E6", "show_fill": true}}
    ]));
    let fonts = FontBook::bundled_only();
    let o = RenderOptions {
        text_dump: true,
        raw: true,
        ..opts()
    };
    let none = HashMap::<String, Vec<u8>>::new();
    let plain = render_page(&s, 0, &none, &fonts, &o).unwrap();
    let hc_opts = RenderOptions {
        high_contrast: true,
        ..o.clone()
    };
    let hc = render_page(&s, 0, &none, &fonts, &hc_opts).unwrap();
    let px = |img: &PageImage, x: u32, y: u32| {
        let i = ((y * img.width + x) * 4) as usize;
        (img.rgba[i], img.rgba[i + 1], img.rgba[i + 2])
    };
    assert_eq!(px(&plain, 630, 350), (255, 255, 255));
    assert_eq!(px(&hc, 630, 350), (0, 0, 0), "the page is black");
    for r in &hc.texts {
        assert_eq!(r.color, Rgb::WHITE, "{}", r.text);
    }
    assert!(hc.texts.iter().any(|r| r.text == "Back"));
    assert!(plain.texts.iter().any(|r| r.color != Rgb::WHITE));
    // the series: the classic palette's first colour (teal, no theme) is drawn bright cyan
    let cyan = HIGH_CONTRAST_COLORS[1];
    let bars = hc
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| (p[0], p[1], p[2]) == (cyan.0, cyan.1, cyan.2))
        .count();
    assert!(bars > 500, "{bars} bright cyan pixels");
    let blue = HIGH_CONTRAST_COLORS[6];
    // the palette reads at 7:1 or more on black and maps to itself
    for c in HIGH_CONTRAST_COLORS {
        assert!((c.lum() + 0.05) / 0.05 >= 7.0, "{c:?}");
        assert_eq!(c.high_contrast(), c, "{c:?} stays");
    }
    assert_eq!(Rgb::WHITE.high_contrast(), Rgb(0, 0, 0));
    assert_eq!(Rgb(0x25, 0x24, 0x23).high_contrast(), Rgb::WHITE);
    assert_eq!(Rgb(0xE6, 0xE6, 0xE6).high_contrast(), Rgb(0, 0, 0));
    assert_eq!(Rgb(0x11, 0x8D, 0xFF).high_contrast(), blue);
}

/// Data bars (conditional formatting): the bars' colours, the axis line in the axis colour, and
/// Show bar only, which leaves the value's text out.
#[test]
fn data_bars_draw_their_axis_and_show_bar_only() {
    let table = |only: bool| {
        json!([{"name": "t", "type": "tableEx", "x": 0, "y": 0, "width": 400, "height": 200,
            "fields": [{"role": "Values", "kind": "measure", "table": "Sales", "name": "Total"}],
            "data": {"status": "ok",
                "columns": [{"name": "Total", "role": "Values", "kind": "measure"}],
                "rows": [[120.0], [-40.0]],
                "cell_styles": [
                    {"r": 0, "c": 0, "bar": {"from": 0.5, "to": 1.0, "color": "#00FF00", "axis": 0.5,
                                             "axis_color": "#0000FF", "only": only}},
                    {"r": 1, "c": 0, "bar": {"from": 0.3, "to": 0.5, "color": "#FF0000", "axis": 0.5,
                                             "axis_color": "#0000FF", "only": only}}]}}])
    };
    let colours = |s: &Value| {
        let img = render(s);
        let pm = tiny_skia::Pixmap::decode_png(&img.png).unwrap();
        let has = |r: u8, g: u8, b: u8| {
            pm.pixels()
                .iter()
                .any(|p| (p.red(), p.green(), p.blue()) == (r, g, b))
        };
        (has(0, 255, 0), has(255, 0, 0), has(0, 0, 255))
    };
    let t = |s: &Value| texts(s).into_iter().map(|r| r.text).collect::<Vec<_>>();
    let shown = snap(table(false));
    assert_eq!(
        colours(&shown),
        (true, true, true),
        "positive, negative bars and the axis"
    );
    assert!(t(&shown).iter().any(|x| x == "120"), "{:?}", t(&shown));
    let only = snap(table(true));
    assert_eq!(colours(&only), (true, true, true));
    assert!(
        !t(&only).iter().any(|x| x == "120" || x == "-40"),
        "{:?}",
        t(&only)
    );
    // without an axis colour no axis is drawn
    let mut plain = snap(table(false));
    plain["report"]["pages"][0]["visuals"][0]["data"]["cell_styles"][0]["bar"]["axis_color"] = Value::Null;
    plain["report"]["pages"][0]["visuals"][0]["data"]["cell_styles"][1]["bar"]["axis_color"] = Value::Null;
    assert_eq!(colours(&plain), (true, true, false));
}

/// The colour at report pixel (x, y) of `s` drawn with the pointer at `pointer`.
fn pixel_with(s: &Value, pointer: Option<states::Pointer>, x: f32, y: f32) -> (u8, u8, u8) {
    let fonts = FontBook::bundled_only();
    let o = RenderOptions { pointer, ..opts() };
    let img = render_page(s, 0, &HashMap::<String, Vec<u8>>::new(), &fonts, &o).unwrap();
    let pm = tiny_skia::Pixmap::decode_png(&img.png).unwrap();
    let p = pm.pixel((x * img.scale) as u32, (y * img.scale) as u32).unwrap();
    (p.red(), p.green(), p.blue())
}

fn over(visual: &str, item: Option<&str>, press: bool) -> Option<states::Pointer> {
    Some(states::Pointer {
        visual: visual.into(),
        item: item.map(str::to_string),
        press,
    })
}

/// A button is drawn in its hover and press states under the pointer, its default state
/// elsewhere (Microsoft Learn, "Create buttons in Power BI reports": Default, On hover, On press,
/// Disabled); a shape's press state too.
#[test]
fn buttons_and_shapes_draw_the_state_under_the_pointer() {
    let s = snap(json!([
        {"name": "bt", "type": "actionButton", "x": 10, "y": 10, "width": 100, "height": 40,
         "button": {"fill": "#102030", "show_fill": true, "states": {
            "hover": {"fill": "#405060"}, "press": {"fill": "#708090"}}},
         "interaction": {"button": {"kind": "PageNavigation", "page": "p2", "enabled": true}}},
        {"name": "sh", "type": "shape", "x": 200, "y": 10, "width": 100, "height": 60,
         "shape": {"fill": "#00AA00", "show_line": false, "states": {"press": {"fill": "#AA0000"}}}}
    ]));
    assert_eq!(pixel_with(&s, None, 20.0, 20.0), (0x10, 0x20, 0x30));
    assert_eq!(
        pixel_with(&s, over("bt", None, false), 20.0, 20.0),
        (0x40, 0x50, 0x60)
    );
    assert_eq!(
        pixel_with(&s, over("bt", None, true), 20.0, 20.0),
        (0x70, 0x80, 0x90)
    );
    // the pointer over another visual leaves the button alone
    assert_eq!(
        pixel_with(&s, over("sh", None, false), 20.0, 20.0),
        (0x10, 0x20, 0x30)
    );
    assert_eq!(
        pixel_with(&s, over("sh", None, false), 250.0, 40.0),
        (0x00, 0xAA, 0x00),
        "no hover look"
    );
    assert_eq!(
        pixel_with(&s, over("sh", None, true), 250.0, 40.0),
        (0xAA, 0x00, 0x00)
    );
}

/// A disabled button (a drillthrough button without a selection) is drawn in its disabled state,
/// or dimmed when it sets none, takes no click and shows its disabled tooltip.
#[test]
fn disabled_buttons_take_no_click_and_show_their_tooltip() {
    let s = snap(json!([
        {"name": "own", "type": "actionButton", "x": 10, "y": 10, "width": 100, "height": 40,
         "button": {"fill": "#102030", "show_fill": true, "states": {"disabled": {"fill": "#C8C8C8"}}},
         "interaction": {"button": {"kind": "Drillthrough", "page": "p2", "enabled": false, "tooltip": "Pick a region"}}},
        {"name": "dim", "type": "actionButton", "x": 200, "y": 10, "width": 100, "height": 40,
         "button": {"fill": "#000000", "show_fill": true},
         "interaction": {"button": {"kind": "Drillthrough", "page": "p2", "enabled": false}}},
        {"name": "on", "type": "actionButton", "x": 400, "y": 10, "width": 100, "height": 40,
         "button": {"fill": "#000000", "show_fill": true},
         "interaction": {"button": {"kind": "Drillthrough", "page": "p2", "enabled": true, "tooltip": "Go"}}}
    ]));
    assert_eq!(
        pixel_with(&s, None, 20.0, 20.0),
        (0xC8, 0xC8, 0xC8),
        "its disabled look"
    );
    let dim = pixel_with(&s, None, 210.0, 20.0);
    assert!(dim.0 > 0x60 && dim.0 == dim.1, "dimmed: {dim:?}");
    assert_eq!(pixel_with(&s, None, 410.0, 20.0), (0, 0, 0));
    let img = render_hits(&s);
    assert!(
        action_at(&img, 20.0, 20.0, false).is_none(),
        "a disabled button takes no click"
    );
    let tip = hits::hit_at(&img.hits, 20.0, 20.0)
        .map(|h| h.tip.clone())
        .unwrap();
    assert_eq!(tip, vec![(String::new(), "Pick a region".to_string())]);
    assert!(action_at(&img, 210.0, 20.0, false).is_none());
    assert_eq!(action_at(&img, 410.0, 20.0, false).unwrap()["action"], "button");
}

/// A conditional (fx) destination: the click sends the destination the snapshot evaluated.
#[test]
fn conditional_destinations_are_sent_with_the_click() {
    let s = snap(json!([
        {"name": "go", "type": "actionButton", "x": 10, "y": 10, "width": 100, "height": 40,
         "button": {"text": "Go"},
         "interaction": {"button": {"kind": "PageNavigation", "page": "p2", "conditional": true,
                                    "destination": "p2", "enabled": true}}}
    ]));
    let img = render_hits(&s);
    assert_eq!(
        action_at(&img, 20.0, 20.0, false).unwrap(),
        json!({"action": "button", "page": "p1", "visual": "go", "destination": "p2"})
    );
}

/// A navigator button under the pointer takes the hover (then press) state over the selected
/// one; the others keep theirs.
#[test]
fn navigator_buttons_draw_hover_and_press() {
    let s = navigators(json!([
        {"name": "nav", "type": "bookmarkNavigator", "x": 0, "y": 0, "width": 600, "height": 60,
         "navigator": {"padding": 0, "styles": {
            "default": {"fill": "#102030", "show_accent": false},
            "selected": {"fill": "#C04000", "show_accent": false},
            "hover": {"fill": "#00C000"}, "press": {"fill": "#0000C0"}}}}
    ]));
    // b1 at the left, b2 (selected) in the middle (pixels away from the centred text)
    assert_eq!(pixel_with(&s, None, 10.0, 30.0), (0x10, 0x20, 0x30));
    assert_eq!(
        pixel_with(&s, over("nav", Some("b1"), false), 10.0, 30.0),
        (0x00, 0xC0, 0x00)
    );
    assert_eq!(
        pixel_with(&s, over("nav", Some("b1"), true), 10.0, 30.0),
        (0x00, 0x00, 0xC0)
    );
    assert_eq!(
        pixel_with(&s, over("nav", Some("b1"), false), 215.0, 12.0),
        (0xC0, 0x40, 0x00)
    );
    assert_eq!(
        pixel_with(&s, over("nav", Some("b2"), false), 215.0, 12.0),
        (0x00, 0xC0, 0x00)
    );
    // the navigator's buttons name themselves in their hits (the visual's own hit names none)
    let img = render_hits(&s);
    let items: Vec<String> = img
        .hits
        .iter()
        .filter(|h| h.visual == "nav")
        .filter_map(|h| h.item.clone())
        .collect();
    assert_eq!(items, vec!["b1", "b2", "b3"]);
}

/// Allow deselection: a click on the selected button launches the deselection bookmark (or,
/// without one, deselects); Hide deselection bookmark leaves it out of the navigator.
#[test]
fn bookmark_navigators_allow_deselection() {
    let s = navigators(json!([
        {"name": "launch", "type": "bookmarkNavigator", "x": 0, "y": 0, "width": 600, "height": 40,
         "navigator": {"allow_deselection": true, "deselection_bookmark": "b3", "hide_deselection_bookmark": true}},
        {"name": "plain", "type": "bookmarkNavigator", "x": 0, "y": 100, "width": 600, "height": 40,
         "navigator": {"allow_deselection": true}},
        {"name": "off", "type": "bookmarkNavigator", "x": 0, "y": 200, "width": 600, "height": 40}
    ]));
    let img = render_hits(&s);
    let launch = sent(&img, "launch");
    let names: Vec<&Value> = launch.iter().map(|(a, _)| &a["bookmark"]).collect();
    assert_eq!(
        names,
        vec!["b1", "b3"],
        "b2 (selected) launches b3, b3 hidden: {launch:?}"
    );
    assert_eq!(launch.len(), 2);
    let plain = sent(&img, "plain");
    assert_eq!(
        plain[1].0,
        json!({"action": "bookmark_deselect", "page": "p1", "visual": "plain"})
    );
    assert_eq!(plain[0].0["bookmark"], "b1");
    let off = sent(&img, "off");
    assert_eq!(
        off[1].0["bookmark"], "b2",
        "without Allow deselection the selected one applies again"
    );
}

/// A shape's text is drawn in it, aligned and coloured as it says.
#[test]
fn shapes_draw_their_text() {
    let s = snap(json!([
        {"name": "sh", "type": "shape", "x": 10, "y": 10, "width": 200, "height": 80,
         "shape": {"tile": "rectangle", "text": {"text": "Hello\nWorld", "font_color": "#FF0000", "font_size": 12, "bold": true}}},
        {"name": "off", "type": "shape", "x": 300, "y": 10, "width": 200, "height": 80,
         "shape": {"text": {"text": "Hidden", "show_text": false}}}
    ]));
    let runs = texts(&s);
    let hello = runs.iter().find(|r| r.text == "Hello").expect("Hello drawn");
    assert_eq!(hello.color, Rgb(255, 0, 0));
    assert_eq!(hello.weight, 700);
    let world = runs.iter().find(|r| r.text == "World").expect("World drawn");
    assert!(world.y > hello.y, "two lines");
    assert!(!runs.iter().any(|r| r.text == "Hidden"));
}

/// G54: the whole-visual colours the producer evaluated (`data.visual_styles`) colour a chart's
/// axis labels and titles and its legend, a gauge's callout and a KPI's value, goal and distance.
#[test]
fn whole_visual_conditional_colours_are_drawn() {
    let rgb = |h: u32| canvas::Rgb((h >> 16) as u8, (h >> 8) as u8, h as u8);
    let mut chart = json!({"name": "c", "type": "clusteredColumnChart", "x": 0, "y": 0,
        "width": 600, "height": 340,
        "format": {"x_axis": {"title": true}, "y_axis": {"title": true}},
        "data": data(
            json!([{"name": "Region", "role": "Category", "kind": "column", "data_type": "string"},
                   {"name": "Kind", "role": "Series", "kind": "column", "data_type": "string"},
                   {"name": "Total", "role": "Y", "kind": "measure"}]),
            json!([["North", "Bikes", 120.0], ["North", "Cars", 30.0], ["South", "Bikes", 80.0]]))});
    let plain = texts(&snap(json!([chart.clone()])));
    let run = |runs: &[canvas::TextRun], t: &str| runs.iter().find(|r| r.text == t).map(|r| r.color);
    assert_ne!(run(&plain, "North"), Some(rgb(0x00AA00)));
    chart["data"]["visual_styles"] = json!({"category_axis_color": "#00AA00",
        "category_title_color": "#0000AA", "value_axis_color": "#AA0000",
        "value_title_color": "#AA00AA", "legend_color": "#00AAAA"});
    let runs = texts(&snap(json!([chart])));
    assert_eq!(run(&runs, "North"), Some(rgb(0x00AA00)));
    assert_eq!(run(&runs, "Region"), Some(rgb(0x0000AA)));
    assert_eq!(run(&runs, "Total"), Some(rgb(0xAA00AA)));
    assert_eq!(run(&runs, "Bikes"), Some(rgb(0x00AAAA)));
    let tick = runs
        .iter()
        .find(|r| r.text == "100")
        .expect("a value tick is drawn");
    assert_eq!(tick.color, rgb(0xAA0000));
    // a gauge's callout value
    let gauge = json!({"name": "g", "type": "gauge", "x": 0, "y": 0, "width": 400, "height": 300,
        "data": {"status": "ok", "visual_styles": {"callout_color": "#123456"},
                 "columns": [{"name": "Total", "role": "Y", "kind": "measure"}], "rows": [[50.0]]}});
    let runs = texts(&snap(json!([gauge])));
    assert_eq!(
        run(&runs, "50"),
        Some(rgb(0x123456)),
        "{:?}",
        runs.iter().map(|r| &r.text).collect::<Vec<_>>()
    );
    // a KPI's value (the good status), goal and distance
    let kpi = json!({"name": "k", "type": "kpi", "x": 0, "y": 0, "width": 500, "height": 300,
        "data": {"status": "ok",
                 "visual_styles": {"good_color": "#123456", "goal_color": "#AA0000", "distance_color": "#00AA00"},
                 "columns": [{"name": "Month", "role": "TrendLine", "kind": "column", "data_type": "string"},
                             {"name": "Total", "role": "Indicator", "kind": "measure"},
                             {"name": "Target", "role": "Goal", "kind": "measure"}],
                 "rows": [["Jan", 5.0, 4.0], ["Feb", 8.0, 4.0]]}});
    let runs = texts(&snap(json!([kpi])));
    let names: Vec<&String> = runs.iter().map(|r| &r.text).collect();
    assert_eq!(run(&runs, "8"), Some(rgb(0x123456)), "{names:?}");
    let goal = runs
        .iter()
        .find(|r| r.text.starts_with("Goal"))
        .expect("the goal is drawn");
    assert_eq!(goal.color, rgb(0xAA0000), "{names:?}");
    let dist = runs
        .iter()
        .find(|r| r.text.starts_with("(+"))
        .expect("the distance is drawn");
    assert_eq!(dist.color, rgb(0x00AA00), "{names:?}");
}

#[test]
fn the_classic_card_applies_its_display_units_and_decimals() {
    // `labels.labelDisplayUnits` / `labelPrecision` of Power BI's classic card: auto keeps the
    // abbreviated value, set units scale it, decimals apply to the shown value
    let card = |format: Value| {
        json!([{"name": "c", "type": "card", "x": 0, "y": 0, "width": 300, "height": 150, "format": format,
            "data": data(json!([{"name": "Revenue", "role": "Values", "kind": "measure"}]), json!([[1_234_567.0]]))}])
    };
    let shown =
        |format: Value| -> Vec<String> { texts(&snap(card(format))).into_iter().map(|r| r.text).collect() };
    let auto = shown(json!({}));
    let cases = [
        (
            json!({"labels": {"display_units": 1e3, "precision": 0}}),
            "1,235K",
        ),
        (
            json!({"labels": {"display_units": 1e6, "precision": 3}}),
            "1.235M",
        ),
        (
            json!({"labels": {"display_units": 1e9, "precision": 3}}),
            "0.001bn",
        ),
        (
            json!({"labels": {"display_units": 1, "precision": 1}}),
            "1,234,567.0",
        ),
        (json!({"labels": {"precision": 1}}), "1.2M"),
    ];
    for (format, want) in cases {
        let got = shown(format.clone());
        assert!(got.iter().any(|t| t == want), "{format}: {got:?}");
        assert_ne!(got, auto, "{format} changed nothing");
    }
    // auto (0) is the default
    assert_eq!(shown(json!({"labels": {"display_units": 0}})), auto);
}
