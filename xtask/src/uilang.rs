//! `cargo xtask oracle-gen uilang OUT.pbix COMMANDS.json LAYOUT.json [CULTURE]`: the UI-language oracle
//! report (M10.2). It writes the `bicraft-cli run` commands of a synthetic report whose visuals show
//! the text Power BI Desktop generates in its own UI language: auto titles ("Sum of Amount by
//! Region and Category"), the nine implicit-measure names, "(Blank)" (a blank Region row),
//! "Total" rows and columns, the waterfall legend, a gauge's and a KPI's titles, empty visuals,
//! and number / date formats. Write it twice (en-US and fr-FR models) and capture both under one
//! UI language to see which culture the numbers follow (`tools/render-oracle/README.md`). Slicer
//! phrases (All, Select all, Multiple selections) come from the slicer oracle report.

use serde_json::{Value, json};

const REGIONS: [&str; 5] = ["North", "South", "East", "West", ""];
const CATS: [&str; 3] = ["Bikes", "Clothing", "Accessories"];

type Vis = (&'static str, u32, u32, u32, u32, Vec<(&'static str, String)>);

fn csv() -> String {
    let mut out = String::from("Region,Category,Month,Amount,Qty\n");
    let mut k = 0u32;
    for (ri, r) in REGIONS.iter().enumerate() {
        for ci in 0..CATS.len() {
            for m in 1..=12u32 {
                k += 1;
                let (rf, cf, mf) = (ri as f64, ci as f64, f64::from(m));
                let amt =
                    1000.0 * (5.0 - rf) * (1.0 + cf * 0.6) + 300.0 * (mf * 0.7 + rf).sin() + 40.0 * mf + 0.25;
                let amt = (amt * 100.0).round() / 100.0;
                let cat = CATS.get(ci).copied().unwrap_or("");
                out.push_str(&format!("{r},{cat},2025-{m:02}-01,{amt},{}\n", (k * 7) % 23 + 3));
            }
        }
    }
    out
}

fn pages() -> Vec<(&'static str, Vec<Vis>)> {
    let (r, ca, m) = (
        "Sales[Region]".to_string(),
        "Sales[Category]".to_string(),
        "Sales[Month]".to_string(),
    );
    let a = |f: &str| format!("{f}(Sales[Amount])");
    let s = |x: &str| x.to_string();
    vec![
        (
            "Titles",
            vec![
                (
                    "clusteredColumnChart",
                    20,
                    20,
                    400,
                    330,
                    vec![("Category", r.clone()), ("Y", a("Sum"))],
                ),
                (
                    "columnChart",
                    440,
                    20,
                    400,
                    330,
                    vec![
                        ("Category", r.clone()),
                        ("Series", ca.clone()),
                        ("Y", s("[Total]")),
                    ],
                ),
                (
                    "clusteredBarChart",
                    860,
                    20,
                    400,
                    330,
                    vec![("Category", r.clone()), ("Y", a("Sum")), ("Y", a("Max"))],
                ),
                (
                    "lineChart",
                    20,
                    370,
                    400,
                    330,
                    vec![
                        ("Category", m.clone()),
                        ("Y", a("Avg")),
                        ("Y", a("Min")),
                        ("Y", a("Max")),
                    ],
                ),
                (
                    "gauge",
                    440,
                    370,
                    400,
                    330,
                    vec![
                        ("Y", s("[Total]")),
                        ("MinValue", s("[Low]")),
                        ("MaxValue", s("[High]")),
                        ("TargetValue", s("[Goal]")),
                    ],
                ),
                (
                    "kpi",
                    860,
                    370,
                    400,
                    330,
                    vec![
                        ("Indicator", s("[Total]")),
                        ("TrendLine", m.clone()),
                        ("Goal", s("[Goal]")),
                    ],
                ),
            ],
        ),
        (
            "Aggs",
            vec![
                (
                    "tableEx",
                    20,
                    20,
                    1240,
                    300,
                    vec![
                        ("Values", r.clone()),
                        ("Values", a("Sum")),
                        ("Values", a("Avg")),
                        ("Values", a("Min")),
                        ("Values", a("Max")),
                        ("Values", a("CountNonNull")),
                        ("Values", a("Count")),
                        ("Values", a("Median")),
                        ("Values", a("StandardDeviation")),
                        ("Values", a("Variance")),
                    ],
                ),
                ("card", 20, 340, 300, 150, vec![("Values", a("Count"))]),
                ("card", 340, 340, 300, 150, vec![("Values", a("CountNonNull"))]),
                (
                    "pieChart",
                    660,
                    340,
                    600,
                    360,
                    vec![
                        ("Category", r.clone()),
                        ("Series", ca.clone()),
                        ("Y", s("[Total]")),
                    ],
                ),
            ],
        ),
        (
            "Blanks",
            vec![
                (
                    "clusteredColumnChart",
                    20,
                    20,
                    400,
                    330,
                    vec![("Category", r.clone()), ("Y", s("[Total]"))],
                ),
                (
                    "pivotTable",
                    440,
                    20,
                    820,
                    330,
                    vec![
                        ("Rows", r.clone()),
                        ("Columns", ca.clone()),
                        ("Values", s("[Total]")),
                    ],
                ),
                (
                    "waterfallChart",
                    20,
                    370,
                    400,
                    330,
                    vec![("Category", r.clone()), ("Y", s("[Total]"))],
                ),
                ("slicer", 440, 370, 240, 330, vec![("Values", r.clone())]),
                (
                    "donutChart",
                    700,
                    370,
                    560,
                    330,
                    vec![("Category", r.clone()), ("Y", s("[Total]"))],
                ),
            ],
        ),
        (
            "Empty",
            vec![
                ("clusteredColumnChart", 20, 20, 400, 330, vec![]),
                ("tableEx", 440, 20, 400, 330, vec![]),
                ("card", 860, 20, 400, 200, vec![]),
                ("slicer", 20, 370, 400, 330, vec![]),
                ("lineChart", 440, 370, 400, 330, vec![]),
                ("pieChart", 860, 370, 400, 330, vec![]),
            ],
        ),
        (
            "Formats",
            vec![
                (
                    "tableEx",
                    20,
                    20,
                    820,
                    680,
                    vec![
                        ("Values", m),
                        ("Values", r),
                        ("Values", s("[Total]")),
                        ("Values", s("[Share]")),
                        ("Values", s("[Big]")),
                        ("Values", a("Sum")),
                        ("Values", a("Avg")),
                    ],
                ),
                ("card", 860, 20, 400, 150, vec![("Values", s("[Total]"))]),
                ("card", 860, 190, 400, 150, vec![("Values", s("[Big]"))]),
                ("card", 860, 360, 400, 150, vec![("Values", s("[Last Month]"))]),
                ("card", 860, 530, 400, 150, vec![("Values", s("[Share]"))]),
            ],
        ),
    ]
}

/// The commands and the layout of the report written to `out` with model culture `culture`.
pub fn report(out: &str, culture: &str) -> (Value, Value) {
    let mut c: Vec<Value> = Vec::new();
    let mut cmd = |name: &str, p: Value| c.push(json!({"command": name, "params": p}));
    cmd("model.new", json!({"name": "UiLang", "culture": culture}));
    cmd(
        "table.import",
        json!({"name": "Sales", "csv": csv(),
        "types": {"Month": "dateTime", "Amount": "double", "Qty": "int64"}}),
    );
    for (name, expr, fmt) in [
        ("Total", "SUM(Sales[Amount])", "#,0.00"),
        ("Goal", "[Total]*0.95", "#,0"),
        ("Low", "[Total]*0.5", "#,0"),
        ("High", "[Total]*1.5", "#,0"),
        ("Share", "DIVIDE([Total],CALCULATE([Total],ALL(Sales)))", "0.0%"),
        ("Big", "[Total]*1000", "#,0"),
        ("Last Month", "MAX(Sales[Month])", "General Date"),
    ] {
        cmd(
            "measure.add",
            json!({"table": "Sales", "name": name, "expression": expr, "format_string": fmt}),
        );
    }
    cmd("report.new", json!({}));
    let mut layout = Vec::new();
    for (i, (name, vis)) in pages().into_iter().enumerate() {
        if i == 0 {
            cmd("page.rename", json!({"page": "Page 1", "display_name": name}));
        } else {
            cmd(
                "page.add",
                json!({"display_name": name, "width": 1280, "height": 720, "key": name}),
            );
        }
        for (t, x, y, w, h, fields) in vis {
            let mut p = json!({"page": name, "visual_type": t, "x": x, "y": y, "width": w, "height": h});
            if !fields.is_empty() {
                p["fields"] = fields
                    .iter()
                    .map(|(role, f)| json!({"role": role, "field": f}))
                    .collect();
            }
            cmd("visual.add", p);
            layout.push(
                json!({"page": i, "page_name": name, "type": t, "x": x, "y": y, "width": w, "height": h}),
            );
        }
    }
    cmd("file.save_as", json!({"path": out, "overwrite": true}));
    (Value::Array(c), Value::Array(layout))
}

pub fn cmd(args: &[String]) -> Result<(), String> {
    let [out, cmds, layout, rest @ ..] = args else {
        return Err(
            "usage: cargo xtask oracle-gen uilang OUT.pbix COMMANDS.json LAYOUT.json [CULTURE]".into(),
        );
    };
    let culture = rest.first().map_or("en-US", String::as_str);
    let (c, l) = report(out, culture);
    let write = |path: &str, v: &Value| {
        let text = serde_json::to_string_pretty(v).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| format!("{path}: {e}"))
    };
    write(cmds, &c)?;
    write(layout, &l)?;
    println!(
        "pages: {}",
        pages().iter().map(|p| p.0).collect::<Vec<_>>().join(",")
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_has_every_page_and_a_blank_region() {
        let (c, l) = report("x.pbix", "fr-FR");
        let cmds = c.as_array().map_or(0, Vec::len);
        assert!(cmds > 30);
        assert_eq!(l.as_array().map_or(0, Vec::len), 26);
        assert_eq!(c[0]["params"]["culture"], "fr-FR");
        // the blank Region rows that make "(Blank)" appear
        assert!(
            c[1]["params"]["csv"]
                .as_str()
                .is_some_and(|s| s.contains("\n,Bikes,"))
        );
    }
}
