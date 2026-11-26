//! `oracle-gen synth`: the render oracle's main synthetic report (set `synth`), one page per
//! visual family plus stress, data-label, combo and axis-end pages. Data from an M literal.
//!
//! `cargo xtask oracle-gen synth OUT.pbix CMDS.json LAYOUT.json`, then
//! `bicraft run --file CMDS.json` writes OUT.pbix; LAYOUT.json lists every visual's page and
//! rectangle for `cargo xtask render-compare`.

use std::path::Path;

use serde_json::{Value, json};

use super::{Cmds, fields, merged, py_float, pyjson::round2, stable_name, wave_amount, write_json};

/// (type, x, y, w, h, fields, extra commands; `None` = no `key`).
type V = (&'static str, i64, i64, i64, i64, Value, Option<Vec<Value>>);

const R: &str = "Sales[Region]";
const CA: &str = "Sales[Category]";
const M: &str = "Sales[Month]";
const SG: &str = "Sales[Segment]";
const PR: &str = "Sales[Product]";

fn csv() -> String {
    let regions = ["North", "South", "East", "West", "Central"];
    let cats = ["Bikes", "Clothing", "Accessories"];
    let segments = [
        "Small and Medium Business",
        "Enterprise Accounts",
        "Public Sector Organizations",
        "Consumer Retail",
        "Education",
    ];
    let mut products = Vec::new();
    for a in ["Road", "Mountain", "Touring", "Urban", "Kids", "Cargo"] {
        for b in [
            "Frame Set",
            "Helmet Pro",
            "Gloves",
            "Saddle Comfort",
            "Pedals",
            "Lights Kit",
        ] {
            products.push(format!("{a} {b}"));
        }
    }
    let mut rows = vec!["Region,Category,Month,Amount,Cost,Qty,Segment,Product".to_string()];
    let mut k: i64 = 0;
    for (ri, r) in (0i64..).zip(regions) {
        for (ci, c) in (0i64..).zip(cats) {
            for m in 1..=12i64 {
                k += 1;
                let amt = wave_amount(ri, ci, m);
                let seg = segments.get(ri as usize).copied().unwrap_or_default();
                let prod = products
                    .get(((k * 5) as usize) % products.len())
                    .map(String::as_str)
                    .unwrap_or_default();
                rows.push(format!(
                    "{r},{c},2025-{m:02}-01,{},{},{},{seg},{prod}",
                    py_float(amt),
                    py_float(round2(amt * 0.62)),
                    (k * 7) % 23 + 3
                ));
            }
        }
    }
    rows.join("\n") + "\n"
}

/// `lab(pos, **more)`: a `visual.format` showing data labels.
fn lab(pos: Option<&str>, more: Value) -> Value {
    let mut o = vec![json!({"object": "labels", "property": "show", "value": true})];
    if let Some(p) = pos {
        o.push(json!({"object": "labels", "property": "labelPosition", "value": p}));
    }
    if let Value::Object(m) = more {
        for (k, v) in m {
            o.push(json!({"object": "labels", "property": k, "value": v}));
        }
    }
    json!(["visual.format", {"objects": o}])
}

fn v(t: &'static str, x: i64, y: i64, w: i64, h: i64, f: &[(&str, &str)]) -> V {
    (t, x, y, w, h, fields(f), None)
}

fn vx(t: &'static str, x: i64, y: i64, w: i64, h: i64, f: &[(&str, &str)], extra: Vec<Value>) -> V {
    (t, x, y, w, h, fields(f), Some(extra))
}

fn pages() -> Vec<(&'static str, Vec<V>)> {
    let mut p = Vec::new();
    p.push((
        "Cards",
        vec![
            v("card", 20, 20, 280, 150, &[("Values", "[Total]")]),
            v(
                "cardVisual",
                320,
                20,
                600,
                150,
                &[("Data", "[Total]"), ("Data", "[Units]"), ("Data", "[Margin %]")],
            ),
            v(
                "multiRowCard",
                940,
                20,
                320,
                300,
                &[("Values", R), ("Values", "[Total]"), ("Values", "[Units]")],
            ),
            v(
                "kpi",
                20,
                200,
                440,
                250,
                &[("Indicator", "[Total]"), ("TrendLine", M), ("Goal", "[Goal]")],
            ),
            v("gauge", 480, 200, 440, 250, &[("Y", "[Margin %]")]),
        ],
    ));
    let rs = [("Category", R), ("Series", CA), ("Y", "[Total]")];
    p.push((
        "Columns",
        vec![
            v(
                "clusteredColumnChart",
                20,
                20,
                400,
                330,
                &[("Category", R), ("Y", "[Total]")],
            ),
            v("columnChart", 440, 20, 400, 330, &rs),
            v("hundredPercentStackedColumnChart", 860, 20, 400, 330, &rs),
            v(
                "clusteredBarChart",
                20,
                370,
                400,
                330,
                &[("Category", R), ("Y", "[Total]"), ("Y", "[Cost Total]")],
            ),
            v("barChart", 440, 370, 400, 330, &rs),
            v("hundredPercentStackedBarChart", 860, 370, 400, 330, &rs),
        ],
    ));
    p.push((
        "Lines",
        vec![
            v(
                "lineChart",
                20,
                20,
                610,
                330,
                &[("Category", M), ("Y", "[Total]"), ("Y", "[Cost Total]")],
            ),
            v(
                "areaChart",
                650,
                20,
                610,
                330,
                &[("Category", M), ("Y", "[Total]")],
            ),
            v(
                "stackedAreaChart",
                20,
                370,
                610,
                330,
                &[("Category", M), ("Series", CA), ("Y", "[Total]")],
            ),
            v(
                "lineClusteredColumnComboChart",
                650,
                370,
                610,
                330,
                &[("Category", R), ("Y", "[Total]"), ("Y2", "[Margin %]")],
            ),
        ],
    ));
    p.push((
        "Parts",
        vec![
            v("pieChart", 20, 20, 400, 330, &[("Category", R), ("Y", "[Total]")]),
            v(
                "donutChart",
                440,
                20,
                400,
                330,
                &[("Category", CA), ("Y", "[Total]")],
            ),
            v(
                "treemap",
                860,
                20,
                400,
                330,
                &[("Group", R), ("Values", "[Total]")],
            ),
            v("funnel", 20, 370, 400, 330, &[("Category", R), ("Y", "[Total]")]),
            v(
                "waterfallChart",
                440,
                370,
                400,
                330,
                &[("Category", R), ("Y", "[Total]")],
            ),
            v(
                "scatterChart",
                860,
                370,
                400,
                330,
                &[("Category", R), ("X", "[Units]"), ("Y", "[Total]")],
            ),
        ],
    ));
    p.push((
        "Tables",
        vec![
            v(
                "tableEx",
                20,
                20,
                600,
                300,
                &[
                    ("Values", R),
                    ("Values", "[Total]"),
                    ("Values", "[Units]"),
                    ("Values", "[Margin %]"),
                ],
            ),
            v(
                "pivotTable",
                640,
                20,
                620,
                300,
                &[("Rows", R), ("Columns", CA), ("Values", "[Total]")],
            ),
            v("slicer", 20, 340, 250, 200, &[("Values", CA)]),
            v("textbox", 290, 340, 400, 120, &[]),
            v("shape", 710, 340, 200, 120, &[]),
            v("actionButton", 930, 340, 200, 60, &[]),
        ],
    ));
    // stress pages: many categories, long labels, data labels in each position, combo with two
    // axes
    let pr = [("Category", PR), ("Y", "[Total]")];
    let rt = [("Category", R), ("Y", "[Total]")];
    let rtc = [("Category", R), ("Y", "[Total]"), ("Y", "[Cost Total]")];
    let rct = [("Category", R), ("Series", CA), ("Y", "[Total]")];
    p.push((
        "Stress",
        vec![
            vx("clusteredColumnChart", 20, 20, 610, 330, &pr, vec![]),
            vx(
                "clusteredColumnChart",
                650,
                20,
                300,
                330,
                &pr,
                vec![json!(["visual.filter", {"field": PR, "top": 10, "by": "[Total]"}])],
            ),
            vx("clusteredBarChart", 970, 20, 290, 330, &pr, vec![]),
            vx(
                "clusteredColumnChart",
                20,
                370,
                610,
                330,
                &[("Category", SG), ("Y", "[Total]")],
                vec![],
            ),
            vx(
                "columnChart",
                650,
                370,
                610,
                330,
                &[("Category", SG), ("Series", CA), ("Y", "[Total]")],
                vec![],
            ),
        ],
    ));
    p.push((
        "Labels",
        vec![
            vx(
                "clusteredColumnChart",
                20,
                20,
                400,
                330,
                &rtc,
                vec![lab(Some("InsideEnd"), json!({}))],
            ),
            vx(
                "clusteredColumnChart",
                440,
                20,
                400,
                330,
                &rt,
                vec![lab(
                    Some("OutsideEnd"),
                    json!({"labelDisplayUnits": 1000, "labelPrecision": 1}),
                )],
            ),
            vx(
                "clusteredColumnChart",
                860,
                20,
                400,
                330,
                &rt,
                vec![lab(
                    Some("InsideCenter"),
                    json!({"enableBackground": true, "backgroundColor": {"color": "#FFFF00"},
                           "backgroundTransparency": 50}),
                )],
            ),
            vx(
                "clusteredColumnChart",
                20,
                370,
                400,
                330,
                &rt,
                vec![lab(Some("InsideBase"), json!({}))],
            ),
            vx(
                "columnChart",
                440,
                370,
                400,
                330,
                &rct,
                vec![lab(None, json!({}))],
            ),
            vx(
                "clusteredBarChart",
                860,
                370,
                400,
                330,
                &rtc,
                vec![
                    lab(None, json!({})),
                    json!(["visual.series", {"target": "[Cost Total]", "labels": {
                        "show": true, "color": "#FD625E", "display_units": 1000, "precision": 2}}]),
                ],
            ),
        ],
    ));
    p.push((
        "Combo",
        vec![
            vx(
                "lineClusteredColumnComboChart",
                20,
                20,
                610,
                330,
                &[("Category", R), ("Y", "[Total]"), ("Y2", "[Units]")],
                vec![lab(None, json!({}))],
            ),
            vx(
                "lineStackedColumnComboChart",
                650,
                20,
                610,
                330,
                &[
                    ("Category", R),
                    ("Series", CA),
                    ("Y", "[Total]"),
                    ("Y2", "[Margin %]"),
                ],
                vec![],
            ),
            vx(
                "lineChart",
                20,
                370,
                400,
                330,
                &[("Category", M), ("Y", "[Total]"), ("Y", "[Cost Total]")],
                vec![json!(["visual.series", {"series": [
                    {"target": "[Total]", "line": {"style": "dashed", "show_marker": true, "marker_size": 6}},
                    {"target": "[Cost Total]", "line": {"style": "dotted", "width": 3}}]}])],
            ),
            vx(
                "hundredPercentStackedColumnChart",
                440,
                370,
                400,
                330,
                &rct,
                vec![lab(None, json!({}))],
            ),
            vx(
                "clusteredColumnChart",
                860,
                370,
                400,
                330,
                &[("Category", M), ("Y", "[Total]")],
                vec![],
            ),
        ],
    ));
    // value axis ends: the same chart with maxima at different distances from the next tick
    let axes = [
        ("clusteredColumnChart", 52),
        ("clusteredColumnChart", 62),
        ("clusteredColumnChart", 75),
        ("clusteredColumnChart", 90),
        ("clusteredColumnChart", 58),
        ("clusteredColumnChart", 70),
        ("clusteredColumnChart", 84),
        ("clusteredBarChart", 66),
    ];
    let axes = (0i64..)
        .zip(axes)
        .map(|(i, (t, k))| {
            let y = format!("[S{k}]");
            vx(
                t,
                20 + i % 4 * 315,
                20 + i / 4 * 350,
                300,
                if i < 4 { 330 } else { 200 },
                &[("Category", R), ("Y", &y)],
                vec![],
            )
        })
        .collect();
    p.push(("Axes", axes));
    p
}

pub fn run(out: &str, cmds_path: &Path, layout_path: &Path) -> Result<(), String> {
    let mut c = Cmds::default();
    c.c("model.new", json!({"name": "Synth"}));
    c.c(
        "table.import",
        json!({"name": "Sales", "csv": csv(), "types": {
            "Month": "dateTime", "Amount": "double", "Cost": "double", "Qty": "int64"}}),
    );
    for (n, e, f) in [
        ("Total", "SUM(Sales[Amount])", "#,0"),
        ("Cost Total", "SUM(Sales[Cost])", "#,0"),
        ("Margin %", "DIVIDE([Total]-[Cost Total],[Total])", "0.0%"),
        ("Units", "SUM(Sales[Qty])", "#,0"),
        ("Goal", "[Total]*0.95", "#,0"),
    ] {
        c.c(
            "measure.add",
            json!({"table": "Sales", "name": n, "expression": e, "format_string": f}),
        );
    }
    for k in [52, 58, 62, 66, 70, 75, 84, 90] {
        c.c(
            "measure.add",
            json!({"table": "Sales", "name": format!("S{k}"),
                   "expression": format!("[Total]*{}", py_float(f64::from(k) / 100.0)),
                   "format_string": "#,0"}),
        );
    }
    c.c("report.new", json!({}));
    let pages = pages();
    for (i, (name, vis)) in pages.iter().enumerate() {
        if i == 0 {
            c.c("page.rename", json!({"page": "Page 1", "display_name": name}));
        } else {
            c.c(
                "page.add",
                json!({"display_name": name, "width": 1280, "height": 720, "key": name}),
            );
        }
        for (k, (t, x, y, w, h, fl, extra)) in vis.iter().enumerate() {
            let mut p = json!({"page": name, "visual_type": t, "x": x, "y": y,
                               "width": w, "height": h});
            if fl.as_array().is_some_and(|a| !a.is_empty()) {
                p["fields"] = fl.clone();
            }
            let keyed = extra.as_ref().filter(|_| i > 0);
            if keyed.is_some() {
                p["key"] = json!(format!("v{k}"));
            }
            c.c("visual.add", p);
            if let Some(extra) = keyed {
                let vn = stable_name(&format!("{}/v{k}", stable_name(name)));
                for e in extra {
                    let cmd = e.get(0).and_then(Value::as_str).unwrap_or_default();
                    let params = e.get(1).cloned().unwrap_or_else(|| json!({}));
                    c.c(cmd, merged(json!({"page": name, "visual": vn}), params));
                }
            }
        }
    }
    c.c("file.save_as", json!({"path": out, "overwrite": true}));
    write_json(cmds_path, &c.value(), 1)?;
    let layout: Vec<Value> = pages
        .iter()
        .enumerate()
        .flat_map(|(i, (name, vis))| {
            vis.iter().map(move |(t, x, y, w, h, ..)| {
                json!({"page": i, "page_name": name, "type": t, "x": x, "y": y,
                       "width": w, "height": h})
            })
        })
        .collect();
    write_json(layout_path, &Value::Array(layout), 1)
}
