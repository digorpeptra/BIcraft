//! `oracle-gen grids`: the tables and matrices report of the render oracle (set `grids`):
//! vertical scroll thumbs at several row counts and visual heights, grids wider than their
//! visual (horizontal scroll), matrix subtotals and grand totals with the `subTotals` options,
//! and a matrix with two column fields. Data from M literals; model culture fr-FR to match a
//! French Desktop UI (render with `{"ui_language": "fr-FR"}`).
//!
//! `cargo xtask oracle-gen grids OUT.pbix CMDS.json LAYOUT.json`, then
//! `bicraft run --file CMDS.json` writes OUT.pbix.

use std::fmt::Write;
use std::path::Path;

use serde_json::{Value, json};

use super::{Cmds, fields, merged, stable_name, write_json};

/// (name, type, x, y, w, h, fields, [[cmd, params]]).
type V = (String, &'static str, i64, i64, i64, i64, Value, Vec<Value>);

const R: &str = "Sales[Region]";
const CA: &str = "Sales[Category]";
const CH: &str = "Sales[Channel]";
const P: &str = "Sales[Product]";
const NM: &str = "Items[Name]";

pub(super) fn sales_csv() -> String {
    let regions = ["North", "South", "East", "West", "Central"];
    let cats = ["Bikes", "Clothing", "Accessories"];
    let chans = ["Online", "Store"];
    let words = ["Frame", "Helmet", "Gloves", "Saddle", "Pedals", "Lights"];
    let prods: Vec<String> = (0..24usize)
        .map(|i| {
            format!(
                "P{i:02} {}",
                words.get(i % words.len()).copied().unwrap_or_default()
            )
        })
        .collect();
    let mut rows = vec!["Region,Category,Channel,Product,Quarter,Amount,Cost".to_string()];
    let mut k: usize = 0;
    for (ri, r) in (0i64..).zip(regions) {
        for (ci, c) in (0i64..).zip(cats) {
            for (hi, h) in (0i64..).zip(chans) {
                for q in 1..=4i64 {
                    k += 1;
                    let mut amt = (1000 * (5 - ri)) as f64 * (1.0 + ci as f64 * 0.6);
                    amt *= if hi != 0 { 1.3 } else { 1.0 };
                    amt += (37 * q * (ri + 1)) as f64;
                    let cost = amt * (0.45 + 0.07 * ((ri + 2 * ci + hi) % 5) as f64);
                    let p = prods.get(k % prods.len()).map(String::as_str).unwrap_or_default();
                    rows.push(format!("{r},{c},{h},{p},Q{q},{amt:.2},{cost:.2}"));
                }
            }
        }
    }
    rows.join("\n") + "\n"
}

fn items_csv() -> String {
    let mut s = String::from("Name,Val\n");
    for i in 1..=1000i64 {
        let _ = writeln!(s, "Item {i:04},{}", 100000 - 7 * i);
    }
    s
}

/// `obj(group={prop: value})`: one `visual.format` with those objects.
fn obj(groups: Value) -> Value {
    let mut o = Vec::new();
    for (g, props) in groups.as_object().into_iter().flatten() {
        for (k, v) in props.as_object().into_iter().flatten() {
            o.push(json!({"object": g, "property": k, "value": v}));
        }
    }
    json!(["visual.format", {"objects": o}])
}

/// A table of the `Items` names, top `n` (all 1000 without filter); `wide` adds ten measures.
#[allow(clippy::too_many_arguments)]
fn tv(name: &str, n: i64, x: i64, y: i64, w: i64, h: i64, wide: bool, extra: &[Value]) -> V {
    let mut fl = vec![
        ("Values".to_string(), NM.to_string()),
        ("Values".into(), "[Val Total]".into()),
    ];
    if wide {
        for i in 1..=10 {
            fl.push(("Values".into(), format!("[Value measure {i:02}]")));
        }
    }
    let pairs: Vec<(&str, &str)> = fl.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let mut cmds = Vec::new();
    if n < 1000 {
        cmds.push(json!(["visual.filter", {"field": NM, "top": n, "by": "[Val Total]"}]));
    }
    cmds.extend(extra.iter().cloned());
    (name.to_string(), "tableEx", x, y, w, h, fields(&pairs), cmds)
}

#[allow(clippy::too_many_arguments)]
fn mx(name: &str, x: i64, y: i64, w: i64, h: i64, f: &[(&str, &str)], cmds: Vec<Value>) -> V {
    (name.to_string(), "pivotTable", x, y, w, h, fields(f), cmds)
}

fn pages() -> Vec<(&'static str, Vec<V>)> {
    let mut pages = Vec::new();
    let mut a: Vec<V> = [20, 40, 59, 100, 200, 500]
        .iter()
        .zip(0i64..)
        .map(|(n, i)| tv(&format!("t{n}_300"), *n, 20 + 210 * i, 20, 200, 300, false, &[]))
        .collect();
    a.extend([
        tv("t1000_360", 1000, 20, 340, 200, 360, false, &[]),
        tv("t59_360", 59, 230, 340, 200, 360, false, &[]),
        tv("t59_200", 59, 440, 340, 200, 200, false, &[]),
        tv("t300_200", 300, 650, 340, 200, 200, false, &[]),
        tv("t30_200", 30, 860, 340, 200, 200, false, &[]),
        tv("t1000_150", 1000, 1070, 340, 200, 150, false, &[]),
    ]);
    pages.push(("ScrollA", a));
    let pad = [obj(
        json!({"grid": {"rowPadding": 6}, "values": {"fontSize": 12}}),
    )];
    let rows_nm = [("Rows", NM), ("Values", "[Val Total]")];
    pages.push((
        "ScrollB",
        vec![
            tv("p59_300", 59, 20, 20, 250, 300, false, &pad),
            tv("p200_300", 200, 290, 20, 250, 300, false, &pad),
            tv("p20_300", 20, 560, 20, 250, 300, false, &pad),
            mx(
                "m59_300",
                830,
                20,
                200,
                300,
                &rows_nm,
                vec![json!(["visual.filter", {"field": NM, "top": 59, "by": "[Val Total]"}])],
            ),
            mx("m1000_300", 1050, 20, 200, 300, &rows_nm, vec![]),
            tv("p1000_340", 1000, 20, 360, 250, 340, false, &pad),
            tv("p100_200", 100, 290, 360, 250, 200, false, &pad),
        ],
    ));
    pages.push((
        "HScroll",
        vec![
            tv("w10_300", 10, 20, 20, 400, 300, true, &[]),
            tv("w59_250", 59, 440, 20, 400, 250, true, &[]),
            mx(
                "mw_250",
                860,
                20,
                400,
                250,
                &[("Rows", R), ("Columns", P), ("Values", "[Total]")],
                vec![],
            ),
            mx(
                "mw2_300",
                20,
                360,
                600,
                300,
                &[
                    ("Rows", R),
                    ("Columns", CA),
                    ("Values", "[Total]"),
                    ("Values", "[Margin %]"),
                ],
                vec![],
            ),
            tv("w5_200", 5, 640, 360, 620, 200, true, &[]),
        ],
    ));
    let m3 = [("Rows", R), ("Rows", CA), ("Columns", CH), ("Values", "[Total]")];
    pages.push((
        "Subtot",
        vec![
            mx(
                "s_rows2",
                20,
                20,
                400,
                330,
                &[
                    ("Rows", R),
                    ("Rows", CA),
                    ("Values", "[Total]"),
                    ("Values", "[Margin %]"),
                ],
                vec![],
            ),
            mx(
                "s_cols",
                440,
                20,
                400,
                330,
                &[
                    ("Rows", R),
                    ("Columns", CH),
                    ("Values", "[Total]"),
                    ("Values", "[Margin %]"),
                ],
                vec![],
            ),
            mx("s_both", 860, 20, 400, 330, &m3, vec![]),
            mx(
                "s_norow",
                20,
                370,
                400,
                330,
                &m3,
                vec![obj(json!({"subTotals": {"rowSubtotals": false}}))],
            ),
            mx(
                "s_nocol",
                440,
                370,
                400,
                330,
                &m3,
                vec![obj(json!({"subTotals": {"columnSubtotals": false}}))],
            ),
            mx(
                "s_bottom",
                860,
                370,
                400,
                330,
                &m3,
                vec![obj(json!({"subTotals": {"rowSubtotalsPosition": "Bottom"}}))],
            ),
        ],
    ));
    pages.push((
        "Subtot2",
        vec![
            mx(
                "s_cols2",
                20,
                20,
                820,
                250,
                &[
                    ("Rows", R),
                    ("Columns", CH),
                    ("Columns", CA),
                    ("Values", "[Total]"),
                ],
                vec![],
            ),
            mx(
                "s_none",
                860,
                20,
                400,
                330,
                &m3,
                vec![obj(
                    json!({"subTotals": {"rowSubtotals": false, "columnSubtotals": false}}),
                )],
            ),
            mx(
                "s_pct",
                20,
                290,
                400,
                330,
                &[("Rows", R), ("Columns", CA), ("Values", "[Margin %]")],
                vec![],
            ),
            mx(
                "s_rows2b",
                440,
                370,
                400,
                330,
                &[("Rows", R), ("Rows", CA), ("Values", "[Total]")],
                vec![obj(json!({"subTotals": {"rowSubtotalsPosition": "Bottom"}}))],
            ),
        ],
    ));
    pages
}

pub fn run(out: &str, cmds_path: &Path, layout_path: &Path) -> Result<(), String> {
    let mut c = Cmds::default();
    c.c("model.new", json!({"name": "Grids", "culture": "fr-FR"}));
    c.c(
        "table.import",
        json!({"name": "Sales", "csv": sales_csv(), "types": {"Amount": "double", "Cost": "double"}}),
    );
    c.c(
        "table.import",
        json!({"name": "Items", "csv": items_csv(), "types": {"Val": "int64"}}),
    );
    c.c(
        "measure.add",
        json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])",
               "format_string": "#,0"}),
    );
    c.c(
        "measure.add",
        json!({"table": "Sales", "name": "Margin %",
               "expression": "DIVIDE(SUM(Sales[Amount])-SUM(Sales[Cost]),SUM(Sales[Amount]))",
               "format_string": "0.0%"}),
    );
    c.c(
        "measure.add",
        json!({"table": "Items", "name": "Val Total", "expression": "SUM(Items[Val])",
               "format_string": "#,0"}),
    );
    for i in 1..=10 {
        c.c(
            "measure.add",
            json!({"table": "Items", "name": format!("Value measure {i:02}"),
                   "expression": format!("[Val Total]*{i}.25"), "format_string": "#,0.00"}),
        );
    }
    c.c("report.new", json!({}));
    let pages = pages();
    for (name, vis) in &pages {
        c.c(
            "page.add",
            json!({"display_name": name, "width": 1280, "height": 720, "key": name}),
        );
        for (vn, t, x, y, w, h, fl, cmds) in vis {
            c.c(
                "visual.add",
                json!({"page": name, "visual_type": t, "x": x, "y": y, "width": w, "height": h,
                       "fields": fl, "key": vn}),
            );
            let id = stable_name(&format!("{}/{vn}", stable_name(name)));
            for e in cmds {
                let cmd = e.get(0).and_then(Value::as_str).unwrap_or_default();
                let params = e.get(1).cloned().unwrap_or_else(|| json!({}));
                c.c(cmd, merged(json!({"page": name, "visual": id}), params));
            }
        }
    }
    c.c("page.remove", json!({"page": "Page 1"}));
    c.c("file.save_as", json!({"path": out, "overwrite": true}));
    write_json(cmds_path, &c.value(), 1)?;
    let layout: Vec<Value> = pages
        .iter()
        .enumerate()
        .flat_map(|(i, (name, vis))| {
            vis.iter().map(move |(vn, _, x, y, w, h, ..)| {
                json!({"page": i, "page_name": name, "type": vn, "x": x, "y": y,
                       "width": w, "height": h})
            })
        })
        .collect();
    write_json(layout_path, &Value::Array(layout), 1)?;
    let names: Vec<&str> = pages.iter().map(|(n, _)| *n).collect();
    println!("pages: {}", names.join(","));
    Ok(())
}
