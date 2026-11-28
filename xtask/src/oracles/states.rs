//! `oracle-gen states`: is the formatting bicraft writes for stateful visual objects (new card,
//! new slicers, buttons, navigators, shapes) *applied* by Power BI Desktop, not only kept by its
//! save? Each test is one property set twice through `visual.format`: variant P as bicraft
//! writes it (the `default` state entry for stateful objects), variant L with `state: "none"`
//! (the entry without selector). Controls are objects Desktop reads without selector, where both
//! variants must apply. See docs/formats/visual-options.md, "State selectors".
//!
//! ```text
//! cargo xtask oracle-gen states DIR         -> DIR/cmds.json (bicraft-cli run --file: DIR/states.pbix)
//! capture.ps1 -File DIR/states.pbix -Out DIR/desktop -Pages <pages printed by the first step>
//! cargo xtask oracle-gen states DIR check [CAPTURE_DIR]
//!     -> per test: whether P and L show the target colour (pixels or text colour), and whether
//!        that matches the expectation
//! ```

use std::path::Path;

use serde_json::{Value, json};

use super::{Cmds, fields, join, pages_of, write_json};
use crate::rcmp::img::Rgb;

const RED: &str = "#FF0000";
const GREEN: &str = "#00B050";
const BLUE: &str = "#0000FF";
const MAG: &str = "#FF00FF";
const YEL: &str = "#FFFF00";
const CYAN: &str = "#00FFFF";

const PER_PAGE: usize = 3;

struct Test {
    name: &'static str,
    ty: &'static str,
    fields: &'static [(&'static str, &'static str)],
    /// (object, [(property, value)]); a "#RRGGBB" string value becomes `{"color": ...}`.
    object: &'static str,
    props: Vec<(&'static str, Value)>,
    colour: &'static str,
    stateful: bool,
}

const TOT: &[(&str, &str)] = &[("Data", "[Total]")];
const REG: &[(&str, &str)] = &[("Values", "Sales[Region]")];
const CAT: &[(&str, &str)] = &[("Values", "Sales[Category]")];

fn tests() -> Vec<Test> {
    let t = |name, ty, fields, object, props: Vec<(&'static str, Value)>, colour, stateful| Test {
        name,
        ty,
        fields,
        object,
        props,
        colour,
        stateful,
    };
    vec![
        t(
            "c_value",
            "cardVisual",
            TOT,
            "value",
            vec![("fontColor", json!(RED))],
            RED,
            true,
        ),
        t(
            "c_label",
            "cardVisual",
            TOT,
            "label",
            vec![("fontColor", json!(GREEN))],
            GREEN,
            true,
        ),
        t(
            "c_accent",
            "cardVisual",
            TOT,
            "accentBar",
            vec![
                ("show", json!(true)),
                ("color", json!(BLUE)),
                ("width", json!(12)),
            ],
            BLUE,
            true,
        ),
        t(
            "c_outline",
            "cardVisual",
            TOT,
            "outline",
            vec![
                ("show", json!(true)),
                ("lineColor", json!(MAG)),
                ("weight", json!(6)),
            ],
            MAG,
            true,
        ),
        t(
            "c_fill",
            "cardVisual",
            TOT,
            "fillCustom",
            vec![("show", json!(true)), ("fillColor", json!(YEL))],
            YEL,
            true,
        ),
        t(
            "c_callout",
            "cardVisual",
            TOT,
            "cardCalloutArea",
            vec![("show", json!(true)), ("backgroundFillColor", json!(CYAN))],
            CYAN,
            true,
        ),
        t(
            "l_value",
            "listSlicer",
            REG,
            "value",
            vec![("fontColor", json!(RED))],
            RED,
            true,
        ),
        t(
            "l_bg",
            "listSlicer",
            REG,
            "background",
            vec![("show", json!(true)), ("color", json!(YEL))],
            YEL,
            true,
        ),
        t(
            "a_value",
            "advancedSlicerVisual",
            CAT,
            "value",
            vec![("fontColor", json!(RED))],
            RED,
            true,
        ),
        t(
            "a_fill",
            "advancedSlicerVisual",
            CAT,
            "fillCustom",
            vec![("show", json!(true)), ("fillColor", json!(YEL))],
            YEL,
            true,
        ),
        t(
            "b_fill",
            "actionButton",
            &[],
            "fill",
            vec![
                ("show", json!(true)),
                ("fillColor", json!(RED)),
                ("transparency", json!(0)),
            ],
            RED,
            true,
        ),
        t(
            "b_text",
            "actionButton",
            &[],
            "text",
            vec![
                ("show", json!(true)),
                ("text", json!("Hello")),
                ("fontColor", json!(GREEN)),
            ],
            GREEN,
            true,
        ),
        t(
            "p_fill",
            "pageNavigator",
            &[],
            "fill",
            vec![
                ("show", json!(true)),
                ("fillColor", json!(RED)),
                ("transparency", json!(0)),
            ],
            RED,
            true,
        ),
        t(
            "s_fill",
            "shape",
            &[],
            "fill",
            vec![
                ("show", json!(true)),
                ("fillColor", json!(RED)),
                ("transparency", json!(0)),
            ],
            RED,
            true,
        ),
        t(
            "s_text",
            "shape",
            &[],
            "text",
            vec![
                ("show", json!(true)),
                ("text", json!("Hello")),
                ("fontColor", json!(GREEN)),
            ],
            GREEN,
            true,
        ),
        // controls: read without selector
        t(
            "t_box",
            "textSlicer",
            REG,
            "inputTextBox",
            vec![("backShow", json!(true)), ("backColor", json!(CYAN))],
            CYAN,
            false,
        ),
        t(
            "bs_fill",
            "basicShape",
            &[],
            "fill",
            vec![("show", json!(true)), ("fillColor", json!(RED))],
            RED,
            false,
        ),
    ]
}

/// One placed visual: (page, name, test index, x, y, variant).
struct Placed {
    page: String,
    name: String,
    test: usize,
    x: i64,
    y: i64,
    variant: char,
}

const W: i64 = 300;
const H: i64 = 200;

fn placed(tests: &[Test]) -> Vec<Placed> {
    let mut v = Vec::new();
    for (i, t) in tests.iter().enumerate() {
        let page = format!("S{}", i / PER_PAGE + 1);
        let y = 20 + (i % PER_PAGE) as i64 * 230;
        for (variant, x) in [('P', 20), ('L', 660)] {
            v.push(Placed {
                page: page.clone(),
                name: format!("{}_{variant}", t.name),
                test: i,
                x,
                y,
                variant,
            });
        }
    }
    v
}

fn csv() -> String {
    let regions = ["North", "South", "East", "West", "Central"];
    let cats = ["Bikes", "Clothing", "Accessories"];
    let mut rows = vec!["Region,Category,Month,Amount,Qty".to_string()];
    for (ri, r) in (0i64..).zip(regions) {
        for (ci, c) in (0i64..).zip(cats) {
            for m in 1..=12i64 {
                let amt = (1000 * (5 - ri)) as f64 * (1.0 + ci as f64 * 0.6) + (40 * m) as f64;
                rows.push(format!(
                    "{r},{c},2025-{m:02}-01,{amt:.2},{}",
                    (ri * 7 + m) % 23 + 3
                ));
            }
        }
    }
    rows.join("\n") + "\n"
}

pub fn gen_cmds(dir: &str) -> Result<(), String> {
    let tests = tests();
    let vis = placed(&tests);
    let pages = pages_of(vis.iter().map(|v| v.page.as_str()));
    let mut c = Cmds::default();
    c.c("model.new", json!({"name": "States", "culture": "en-US"}));
    c.c(
        "table.import",
        json!({"name": "Sales", "csv": csv(), "types": {"Month": "dateTime", "Amount": "double", "Qty": "int64"}}),
    );
    c.c(
        "measure.add",
        json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])", "format_string": "#,0"}),
    );
    c.c("report.new", json!({}));
    let spec_pages: Vec<Value> = pages
        .iter()
        .map(|pg| {
            let visuals: Vec<Value> = vis
                .iter()
                .filter(|v| v.page == *pg)
                .map(|v| {
                    let t = tests.get(v.test);
                    json!({"name": v.name, "type": t.map(|t| t.ty), "x": v.x, "y": v.y,
                           "width": W, "height": H,
                           "fields": fields(t.map(|t| t.fields).unwrap_or_default())})
                })
                .collect();
            json!({"key": pg, "display_name": pg, "width": 1280, "height": 720, "visuals": visuals})
        })
        .collect();
    c.c(
        "report.apply",
        json!({"spec": {"pages": spec_pages}, "prune": "all"}),
    );
    for v in &vis {
        let Some(t) = tests.get(v.test) else { continue };
        let objs: Vec<Value> = t
            .props
            .iter()
            .map(|(prop, val)| {
                let val = match val.as_str() {
                    Some(s) if s.starts_with('#') => json!({"color": s}),
                    _ => val.clone(),
                };
                // `target`: a slicer's `background` is also a container object name
                let mut item =
                    json!({"object": t.object, "property": prop, "value": val, "target": "visual"});
                if v.variant == 'L' && t.stateful {
                    item["state"] = json!("none");
                }
                item
            })
            .collect();
        c.c(
            "visual.format",
            json!({"page": v.page, "visual": v.name, "objects": objs}),
        );
    }
    c.c(
        "project.new",
        json!({"dir": join(dir, &["proj"]), "name": "States"}),
    );
    c.c(
        "file.save_as",
        json!({"path": join(dir, &["states.pbix"]), "overwrite": true}),
    );
    std::fs::create_dir_all(dir).map_err(|e| format!("{dir}: {e}"))?;
    write_json(&Path::new(dir).join("cmds.json"), &c.value(), 1)?;
    println!("pages: {}", pages.join(","));
    Ok(())
}

fn rgb(hex: &str) -> [u8; 3] {
    let h = |i: usize| {
        hex.get(i..i + 2)
            .and_then(|s| u8::from_str_radix(s, 16).ok())
            .unwrap_or(0)
    };
    [h(1), h(3), h(5)]
}

/// Pixels of `img` within the page rectangle (x, y, w, h) at `sc` image pixels per page pixel
/// whose summed channel distance to `c` is below 40.
fn colour_pixels(img: &Rgb, sc: f64, (x, y, w, h): (i64, i64, i64, i64), c: [u8; 3]) -> usize {
    // numpy slicing of `int(v * sc)` bounds, clamped to the image
    let b = |v: i64, max: usize| ((v as f64 * sc) as i64).clamp(0, max as i64) as usize;
    let (x0, x1, y0, y1) = (b(x, img.w), b(x + w, img.w), b(y, img.h), b(y + h, img.h));
    let mut n = 0;
    for yy in y0..y1 {
        for xx in x0..x1 {
            let p = img.get(xx, yy);
            let d: i32 = p
                .iter()
                .zip(c)
                .map(|(a, b)| (i32::from(*a) - i32::from(b)).abs())
                .sum();
            if d < 40 {
                n += 1;
            }
        }
    }
    n
}

/// Whether a captured text node (capture.mjs JSON) of colour `want` starts inside the rectangle.
fn text_hit(txt: &[Value], want: &str, (x, y, w, h): (i64, i64, i64, i64)) -> bool {
    txt.iter().any(|d| {
        let num = |k: &str| d.get(k).and_then(Value::as_f64);
        let (Some(dx), Some(dy), Some(z)) = (num("x"), num("y"), num("z")) else {
            return false;
        };
        if z == 0.0 || d.get("color").and_then(Value::as_str) != Some(want) {
            return false;
        }
        let (px, py) = (dx * 1280.0 / z, dy * 1280.0 / z);
        (x as f64) <= px && px < (x + w) as f64 && (y as f64) <= py && py < (y + h) as f64
    })
}

pub fn check(_dir: &str, cap: &Path) -> Result<(), String> {
    let tests = tests();
    let vis = placed(&tests);
    let pages = pages_of(vis.iter().map(|v| v.page.as_str()));
    let mut seen: std::collections::HashMap<String, bool> = Default::default();
    for (pi, pg) in pages.iter().enumerate() {
        let png = cap.join(format!("{:02}-{pg}.png", pi + 1));
        if !png.is_file() {
            println!("{pg} no capture");
            continue;
        }
        let img = Rgb::load_png(&png)?;
        let sc = img.w as f64 / 1280.0;
        let txt: Vec<Value> = super::read_json(&png.with_extension("json"))
            .ok()
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default();
        for v in vis.iter().filter(|v| v.page == *pg) {
            let Some(t) = tests.get(v.test) else { continue };
            let c = rgb(t.colour);
            let rect = (v.x, v.y, W, H);
            let px = colour_pixels(&img, sc, rect, c);
            let want = format!("rgb({}, {}, {})", c[0], c[1], c[2]);
            seen.insert(v.name.clone(), px > 50 || text_hit(&txt, &want, rect));
        }
    }
    let show = |b: Option<&bool>| match b {
        Some(true) => "True",
        Some(false) => "False",
        None => "None",
    };
    let mut bad = 0;
    for t in &tests {
        let p = seen.get(&format!("{}_P", t.name));
        let l = seen.get(&format!("{}_L", t.name));
        let ok = p == Some(&true) && l == Some(&!t.stateful);
        if !ok {
            bad += 1;
        }
        println!(
            "{:<10} {:<22} stateful={:<5} applied: bicraft={} stateless={} {}",
            t.name,
            t.ty,
            if t.stateful { "True" } else { "False" },
            show(p),
            show(l),
            if ok { "ok" } else { "UNEXPECTED" }
        );
    }
    println!("unexpected: {bad}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_check_counts_pixels_and_texts() {
        let Some(mut img) = Rgb::new(128, 72, [255; 3]) else {
            panic!("image");
        };
        // scale 0.1: the page rectangle (20, 20, 300, 200) is pixels 2..32 x 2..22
        for y in 0..10 {
            for x in 0..10 {
                img.px[y * 128 + x + 2 * 128 + 2] = [250, 5, 5];
            }
        }
        assert_eq!(colour_pixels(&img, 0.1, (20, 20, W, H), rgb(RED)), 100);
        assert_eq!(colour_pixels(&img, 0.1, (660, 20, W, H), rgb(RED)), 0);
        let txt = vec![json!({"color": "rgb(0, 176, 80)", "x": 700.0, "y": 30.0, "z": 1280.0})];
        assert!(text_hit(&txt, "rgb(0, 176, 80)", (660, 20, W, H)));
        assert!(!text_hit(&txt, "rgb(0, 176, 80)", (20, 20, W, H)));
    }
}
