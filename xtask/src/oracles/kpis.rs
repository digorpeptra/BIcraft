//! `oracle-gen kpis`: the KPI indicator's display units (set `kpis`): one KPI per measure
//! format string (`#,0`, `#,0.00`, `0`, `0.00`, none, `#,0.0`, currency, percent) and
//! magnitude (950 to 78 million), plus the `indicator.indicatorDisplayUnits` /
//! `indicatorPrecision` options. Each measure is a constant over the four quarters of the
//! trend axis, so the indicator shows that constant.
//!
//! ```text
//! cargo xtask oracle-gen kpis DIR          -> DIR/cmds1.json, DIR/cmds2.json, DIR/layout.json
//! bicraft-cli run --file DIR/cmds1.json       -> DIR/proj/Synth.pbip
//! cargo xtask oracle-gen kpis DIR patch    -> writes the indicator options
//! bicraft-cli run --file DIR/cmds2.json       -> DIR/kpis.pbix
//! ```
//! Model culture fr-FR (render with `{"ui_language": "fr-FR"}`).

use serde_json::{Value, json};

use super::{Cmds, Vis, fields, grids, two_step_gen, two_step_patch};

/// The magnitudes, as DAX literals.
const MAGS: [&str; 6] = ["950.5", "7827.4", "78270.25", "782702.5", "7827025", "78270250"];

/// Format strings per page (an empty one: no format string).
const FORMATS: [(&str, [&str; 4]); 2] = [
    ("KpiA", ["#,0", "#,0.00", "0", "0.00"]),
    ("KpiB", ["", "#,0.0", "#,0.00 €", "0.0%"]),
];

/// Page `KpiC`: (format index on page A, magnitude index, displayUnits, precision).
const OPTIONS: [(usize, usize, Option<i64>, Option<i64>); 12] = [
    (1, 2, Some(1), None),
    (1, 2, Some(0), None),
    (1, 2, Some(1000), None),
    (1, 2, None, Some(0)),
    (1, 4, Some(1), None),
    (1, 4, None, Some(1)),
    (0, 2, Some(0), None),
    (0, 2, Some(1000), None),
    (0, 2, None, Some(2)),
    (0, 4, Some(1_000_000), None),
    (0, 4, Some(1000), Some(1)),
    (0, 5, None, Some(3)),
];

/// Page `KpiD`: more format strings (decimals, grouping, sections, suffixes) at two magnitudes.
const MORE: [&str; 12] = [
    "#,0.000",
    "0.0",
    "0.000",
    "#,0.0000",
    "#,##0.00",
    "#,0.00;-#,0.00",
    "\\$#,0.0",
    "#,0.0 €",
    "#,0.00 \"u\"",
    "0.00;(0.00)",
    "#,0.###",
    "#,0.##",
];
/// The magnitudes of page `KpiD` (indices into `MAGS`).
const MORE_MAGS: [usize; 2] = [2, 4];

fn measure(fi: usize, page: usize, mi: usize) -> String {
    format!("K{page}{fi}{mi}")
}

/// The visual names (`'static` for `Vis`).
fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn lit(v: &str) -> Value {
    json!({"expr": {"Literal": {"Value": v}}})
}

fn visuals() -> Vec<Vis> {
    let mut out = Vec::new();
    for (pi, (page, _)) in FORMATS.iter().enumerate() {
        for fi in 0..4usize {
            for mi in 0..MAGS.len() {
                let m = measure(fi, pi, mi);
                out.push(Vis {
                    page,
                    name: leak(format!("k{pi}{fi}{mi}")),
                    ty: "kpi",
                    x: 10 + 210 * mi as i64,
                    y: 10 + 175 * fi as i64,
                    w: 200,
                    h: 165,
                    fields: fields(&[("Indicator", &format!("[{m}]")), ("TrendLine", "Sales[Quarter]")]),
                    objects: json!({}),
                });
            }
        }
    }
    for (i, (fi, mi, units, precision)) in OPTIONS.iter().enumerate() {
        let mut p = serde_json::Map::new();
        if let Some(u) = units {
            p.insert("indicatorDisplayUnits".into(), lit(&format!("{u}D")));
        }
        if let Some(d) = precision {
            p.insert("indicatorPrecision".into(), lit(&format!("{d}L")));
        }
        let m = measure(*fi, 0, *mi);
        out.push(Vis {
            page: "KpiC",
            name: leak(format!("o{i:02}")),
            ty: "kpi",
            x: 10 + 210 * (i % 6) as i64,
            y: 10 + 175 * (i / 6) as i64,
            w: 200,
            h: 165,
            fields: fields(&[("Indicator", &format!("[{m}]")), ("TrendLine", "Sales[Quarter]")]),
            objects: json!({"indicator": [{"properties": Value::Object(p)}]}),
        });
    }
    for fi in 0..MORE.len() {
        for (k, mi) in MORE_MAGS.iter().enumerate() {
            let slot = fi * 2 + k;
            out.push(Vis {
                page: "KpiD",
                name: leak(format!("d{fi:02}{mi}")),
                ty: "kpi",
                x: 10 + 210 * (slot % 6) as i64,
                y: 10 + 175 * (slot / 6) as i64,
                w: 200,
                h: 165,
                fields: fields(&[
                    ("Indicator", &format!("[D{fi:02}{mi}]")),
                    ("TrendLine", "Sales[Quarter]"),
                ]),
                objects: json!({}),
            });
        }
    }
    out
}

pub fn run(dir: &str, patch: bool) -> Result<(), String> {
    let vis = visuals();
    if patch {
        return two_step_patch(dir, &vis, false);
    }
    let mut c = Cmds::default();
    c.c("model.new", json!({"name": "Synth", "culture": "fr-FR"}));
    c.c(
        "table.import",
        json!({"name": "Sales", "csv": grids::sales_csv(), "types": {"Amount": "double", "Cost": "double"}}),
    );
    for (pi, (_, fmts)) in FORMATS.iter().enumerate() {
        for (fi, f) in fmts.iter().enumerate() {
            for (mi, v) in MAGS.iter().enumerate() {
                let mut p = json!({"table": "Sales", "name": measure(fi, pi, mi),
                    "expression": format!("{v} * DIVIDE(COUNTROWS(Sales), COUNTROWS(Sales))")});
                if !f.is_empty() {
                    p["format_string"] = json!(f);
                }
                c.c("measure.add", p);
            }
        }
    }
    for (fi, f) in MORE.iter().enumerate() {
        for mi in MORE_MAGS {
            let v = MAGS.get(mi).copied().unwrap_or("1");
            c.c(
                "measure.add",
                json!({"table": "Sales", "name": format!("D{fi:02}{mi}"), "format_string": f,
                    "expression": format!("{v} * DIVIDE(COUNTROWS(Sales), COUNTROWS(Sales))")}),
            );
        }
    }
    c.c("report.new", json!({}));
    two_step_gen(dir, c, &vis, "kpis.pbix")
}
