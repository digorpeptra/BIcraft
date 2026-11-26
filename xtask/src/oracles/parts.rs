//! `oracle-gen parts`: the pie / donut / gauge / card / KPI oracle (set `parts`), one visual per
//! formatting variant (pie label styles and positions, donut hole and start angle, gauge axis
//! literals and fields, new card layouts and accent bars, multi-row cards, KPI variants, funnel
//! and treemap labels). The variants are visual objects Desktop writes (`labels.labelStyle`,
//! `slices.innerRadiusRatio`, `axis.max`, `accentBar.show`...), patched into the PBIR.
//!
//! ```text
//! cargo xtask oracle-gen parts DIR          -> DIR/cmds1.json, DIR/cmds2.json, DIR/layout.json
//! bicraft run --file DIR/cmds1.json        -> DIR/proj/Synth.pbip
//! cargo xtask oracle-gen parts DIR patch    -> writes the visual objects into the project
//! bicraft run --file DIR/cmds2.json        -> DIR/parts.pbix
//! ```
//! then render, capture (pages printed by the first step) and compare as in
//! tools/render-oracle/README.md. The model culture is fr-FR to match a French Desktop UI
//! (render with `--lang fr`).

use serde_json::{Value, json};

use super::{Cmds, Vis, fields, py_float, two_step_gen, two_step_patch, wave_amount};

const R: &str = "Sales[Region]";
const CA: &str = "Sales[Category]";
const P: &str = "Sales[Product]";
const M: &str = "Sales[Month]";

const PRODS: [&str; 12] = [
    "Alpha", "Bravo", "Charlie", "Delta", "Echo", "Foxtrot", "Golf", "Hotel", "India", "Juliet", "Kilo",
    "Lima",
];

fn csv() -> String {
    let regions = ["North", "South", "East", "West", "Central"];
    let cats = ["Bikes", "Clothing", "Accessories"];
    let mut rows = vec!["Region,Category,Product,Month,Amount,Qty".to_string()];
    let mut k: i64 = 0;
    for (ri, r) in (0i64..).zip(regions) {
        for (ci, c) in (0i64..).zip(cats) {
            for m in 1..=12i64 {
                k += 1;
                let amt = wave_amount(ri, ci, m);
                let p = PRODS.get(((k * 5) % 12) as usize).copied().unwrap_or_default();
                rows.push(format!(
                    "{r},{c},{p},2025-{m:02}-01,{},{}",
                    py_float(amt),
                    (k * 7) % 23 + 3
                ));
            }
        }
    }
    // a long tail of small products for label collisions
    for (i, p) in (0i64..).zip(PRODS.iter().skip(6)) {
        rows.push(format!("North,Bikes,{p}x,2025-01-01,{},1", 150 + 40 * i));
    }
    rows.join("\n") + "\n"
}

/// A literal expression.
fn e(v: &str) -> Value {
    json!({"expr": {"Literal": {"Value": v}}})
}

/// A string literal expression.
fn s(v: &str) -> Value {
    e(&format!("'{v}'"))
}

fn col(c: &str) -> Value {
    json!({"solid": {"color": s(c)}})
}

fn props(kw: Value) -> Value {
    json!([{"properties": kw}])
}

const G6: [(i64, i64); 6] = [(20, 20), (440, 20), (860, 20), (20, 370), (440, 370), (860, 370)];

fn g6(i: usize) -> (i64, i64) {
    G6.get(i).copied().unwrap_or((20, 20))
}

fn visuals() -> Vec<Vis> {
    let mut out = Vec::new();
    #[allow(clippy::too_many_arguments)]
    let mut v = |page: &'static str,
                 name: &'static str,
                 ty: &'static str,
                 (x, y): (i64, i64),
                 w: i64,
                 h: i64,
                 f: &[(&str, &str)],
                 objects: Value| {
        out.push(Vis {
            page,
            name,
            ty,
            x,
            y,
            w,
            h,
            fields: fields(f),
            objects,
        });
    };
    let no = || json!({});
    let rt = [("Category", R), ("Y", "[Total]")];
    let ct = [("Category", CA), ("Y", "[Total]")];
    let pt = [("Category", P), ("Y", "[Total]")];

    const STYLES: [&str; 7] = [
        "Category",
        "Data",
        "Percent of total",
        "Both",
        "Category, percent of total",
        "Data value, percent of total",
        "Category, data value, percent of total",
    ];
    const PIE_NAMES: [&str; 7] = ["pie_0", "pie_1", "pie_2", "pie_3", "pie_4", "pie_5", "pie_6"];
    for (i, (st, name)) in STYLES.iter().zip(PIE_NAMES).enumerate() {
        let (pg, slot) = if i < 4 { ("PieA", i) } else { ("PieB", i - 4) };
        v(
            pg,
            name,
            "pieChart",
            g6(slot),
            400,
            330,
            &rt,
            json!({"labels": props(json!({"labelStyle": s(st)}))}),
        );
    }
    v("PieA", "pie_default_cat", "pieChart", g6(4), 400, 330, &ct, no());
    v("PieA", "pie_many", "pieChart", g6(5), 400, 330, &pt, no());
    v(
        "PieB",
        "pie_inside",
        "pieChart",
        g6(3),
        400,
        330,
        &rt,
        json!({"labels": props(json!({"position": s("inside"), "labelStyle": s("Percent of total")}))}),
    );
    v(
        "PieB",
        "pie_legend",
        "pieChart",
        g6(4),
        400,
        330,
        &rt,
        json!({"legend": props(json!({"show": e("true")}))}),
    );
    v(
        "PieB",
        "pie_start90",
        "pieChart",
        g6(5),
        400,
        330,
        &rt,
        json!({"slices": props(json!({"startAngle": e("90L")}))}),
    );
    v("Donut", "donut_default", "donutChart", g6(0), 400, 330, &rt, no());
    v(
        "Donut",
        "donut_r30",
        "donutChart",
        g6(1),
        400,
        330,
        &rt,
        json!({"slices": props(json!({"innerRadiusRatio": e("30L")}))}),
    );
    v(
        "Donut",
        "donut_inside",
        "donutChart",
        g6(2),
        400,
        330,
        &ct,
        json!({"labels": props(json!({"position": s("inside")}))}),
    );
    v(
        "Donut",
        "donut_pref_in",
        "donutChart",
        g6(3),
        400,
        330,
        &rt,
        json!({"labels": props(json!({"position": s("preferInside"),
                                      "labelStyle": s("Data value, percent of total")}))}),
    );
    v("Donut", "donut_many", "donutChart", g6(4), 400, 330, &pt, no());
    v(
        "Donut",
        "donut_nolabels",
        "donutChart",
        g6(5),
        400,
        330,
        &rt,
        json!({"labels": props(json!({"show": e("false")})), "legend": props(json!({"show": e("true")}))}),
    );

    v(
        "Gauges",
        "g_default",
        "gauge",
        g6(0),
        400,
        330,
        &[("Y", "[Total]")],
        no(),
    );
    v(
        "Gauges",
        "g_literal",
        "gauge",
        g6(1),
        400,
        330,
        &[("Y", "[Total]")],
        json!({"axis": props(json!({"min": e("0D"), "max": e("1200000D"), "target": e("1000000D")}))}),
    );
    v(
        "Gauges",
        "g_fields",
        "gauge",
        g6(2),
        400,
        330,
        &[
            ("Y", "[Total]"),
            ("MinValue", "[Low]"),
            ("MaxValue", "[High]"),
            ("TargetValue", "[Goal]"),
        ],
        no(),
    );
    v(
        "Gauges",
        "g_pct",
        "gauge",
        g6(3),
        400,
        330,
        &[("Y", "[Margin %]")],
        no(),
    );
    v(
        "Gauges",
        "g_pct_target",
        "gauge",
        g6(4),
        400,
        330,
        &[("Y", "[Margin %]")],
        json!({"axis": props(json!({"target": e("0.5D")}))}),
    );
    v(
        "Gauges",
        "g_wide",
        "gauge",
        (860, 370),
        400,
        180,
        &[("Y", "[Total]"), ("TargetValue", "[Goal]")],
        no(),
    );

    let one = [("Data", "[Total]")];
    let three = [("Data", "[Total]"), ("Data", "[Units]"), ("Data", "[Margin %]")];
    v("NCards", "nc_one", "cardVisual", (20, 20), 300, 150, &one, no());
    v(
        "NCards",
        "nc_three",
        "cardVisual",
        (340, 20),
        600,
        150,
        &three,
        no(),
    );
    v(
        "NCards",
        "nc_accent",
        "cardVisual",
        (960, 20),
        300,
        150,
        &one,
        json!({"accentBar": props(json!({"show": e("true")}))}),
    );
    v(
        "NCards",
        "nc_rows",
        "cardVisual",
        (20, 190),
        620,
        250,
        &[("Data", "[Total]"), ("Rows", R)],
        no(),
    );
    v(
        "NCards",
        "nc_grid",
        "cardVisual",
        (660, 190),
        600,
        250,
        &[
            ("Data", "[Total]"),
            ("Data", "[Units]"),
            ("Data", "[Margin %]"),
            ("Data", "[Goal]"),
        ],
        json!({"layout": props(json!({"columnCount": e("2L"), "rowCount": e("2L"), "autoGrid": e("false")}))}),
    );
    v(
        "NCards",
        "nc_vert",
        "cardVisual",
        (20, 460),
        300,
        240,
        &[("Data", "[Total]"), ("Data", "[Units]")],
        json!({"layout": props(json!({"orientation": e("1D")}))}),
    );
    v(
        "NCards",
        "nc_above",
        "cardVisual",
        (340, 460),
        300,
        150,
        &one,
        json!({"label": props(json!({"position": s("belowValue")})),
               "accentBar": props(json!({"show": e("true"), "position": s("Top"),
                                         "color": col("#E66C37"), "width": e("6D")}))}),
    );
    v(
        "NCards",
        "nc_center",
        "cardVisual",
        (660, 460),
        600,
        150,
        &[("Data", "[Total]"), ("Data", "[Units]")],
        json!({"value": props(json!({"horizontalAlignment": s("center")})),
               "label": props(json!({"horizontalAlignment": s("center")}))}),
    );

    v(
        "NCards2",
        "nc_rows_cards",
        "cardVisual",
        (20, 20),
        620,
        330,
        &[("Data", "[Total]"), ("Rows", R)],
        json!({"layout": props(json!({"style": s("Cards")}))}),
    );
    v(
        "NCards2",
        "nc_six",
        "cardVisual",
        (660, 20),
        600,
        330,
        &[
            ("Data", "[Total]"),
            ("Data", "[Units]"),
            ("Data", "[Margin %]"),
            ("Data", "[Goal]"),
            ("Data", "[Low]"),
            ("Data", "[High]"),
        ],
        no(),
    );
    v(
        "NCards2",
        "nc_accent_red",
        "cardVisual",
        (20, 370),
        300,
        150,
        &one,
        json!({"accentBar": props(json!({"show": e("true"), "color": col("#FD625E"), "width": e("8D")}))}),
    );
    v(
        "NCards2",
        "nc_rows_two",
        "cardVisual",
        (340, 370),
        620,
        330,
        &[("Data", "[Total]"), ("Data", "[Units]"), ("Rows", CA)],
        no(),
    );

    v(
        "MRKpi",
        "mr_cat",
        "multiRowCard",
        (20, 20),
        400,
        330,
        &[("Values", R), ("Values", "[Total]"), ("Values", "[Units]")],
        no(),
    );
    v(
        "MRKpi",
        "mr_meas",
        "multiRowCard",
        (440, 20),
        400,
        150,
        &[
            ("Values", "[Total]"),
            ("Values", "[Units]"),
            ("Values", "[Margin %]"),
        ],
        no(),
    );
    v(
        "MRKpi",
        "mr_four",
        "multiRowCard",
        (860, 20),
        400,
        330,
        &[
            ("Values", CA),
            ("Values", "[Total]"),
            ("Values", "[Units]"),
            ("Values", "[Margin %]"),
            ("Values", "[Goal]"),
        ],
        no(),
    );
    v(
        "MRKpi",
        "kpi_ind",
        "kpi",
        (440, 190),
        400,
        160,
        &[("Indicator", "[Total]")],
        no(),
    );
    v(
        "MRKpi",
        "kpi_trend",
        "kpi",
        (20, 370),
        400,
        330,
        &[("Indicator", "[Total]"), ("TrendLine", M)],
        no(),
    );
    v(
        "MRKpi",
        "kpi_goal",
        "kpi",
        (440, 370),
        400,
        330,
        &[("Indicator", "[Total]"), ("TrendLine", M), ("Goal", "[Goal]")],
        no(),
    );
    v(
        "MRKpi",
        "kpi_bad",
        "kpi",
        (860, 370),
        400,
        330,
        &[("Indicator", "[Total]"), ("TrendLine", M), ("Goal", "[High]")],
        no(),
    );

    v("FunTree", "fun_labels", "funnel", g6(0), 400, 330, &pt, no());
    v("FunTree", "fun_cat", "funnel", g6(1), 400, 330, &ct, no());
    v(
        "FunTree",
        "tree_labels",
        "treemap",
        g6(2),
        400,
        330,
        &[("Group", R), ("Values", "[Total]")],
        json!({"labels": props(json!({"show": e("true")}))}),
    );
    v(
        "FunTree",
        "tree_many",
        "treemap",
        g6(3),
        400,
        330,
        &[("Group", P), ("Values", "[Total]")],
        no(),
    );
    v(
        "FunTree",
        "tree_cat",
        "treemap",
        g6(4),
        400,
        330,
        &[("Group", CA), ("Values", "[Total]")],
        no(),
    );
    v(
        "FunTree",
        "pie_small",
        "pieChart",
        (860, 370),
        250,
        200,
        &rt,
        no(),
    );
    out
}

pub fn run(dir: &str, patch: bool) -> Result<(), String> {
    let vis = visuals();
    if patch {
        return two_step_patch(dir, &vis, true);
    }
    let mut c = Cmds::default();
    c.c("model.new", json!({"name": "Synth", "culture": "fr-FR"}));
    c.c(
        "table.import",
        json!({"name": "Sales", "csv": csv(), "types": {"Month": "dateTime", "Amount": "double", "Qty": "int64"}}),
    );
    for (n, ex, fs) in [
        ("Total", "SUM(Sales[Amount])", "#,0"),
        ("Units", "SUM(Sales[Qty])", "#,0"),
        (
            "Margin %",
            "DIVIDE(SUM(Sales[Amount]) - SUM(Sales[Amount]) * 0.62, SUM(Sales[Amount]))",
            "0.0%",
        ),
        ("Goal", "[Total]*0.95", "#,0"),
        ("Low", "[Total]*0.5", "#,0"),
        ("High", "[Total]*1.5", "#,0"),
    ] {
        c.c(
            "measure.add",
            json!({"table": "Sales", "name": n, "expression": ex, "format_string": fs}),
        );
    }
    c.c("report.new", json!({}));
    two_step_gen(dir, c, &vis, "parts.pbix")
}
