//! `oracle-gen slicers`: the slicer oracle (set `slicers`), one page per slicer mode family
//! (list, select all, single select, search box, hierarchy, tiles, button and new list slicers,
//! dropdown, between / before / after on numbers and dates, relative date, text slicer). Modes and
//! selections are slicer objects Desktop writes (`data.mode`, `general.filter`, `selection.*`,
//! `data.numericStart`...), patched into the PBIR.
//!
//! ```text
//! cargo xtask oracle-gen slicers DIR        -> DIR/cmds1.json, DIR/cmds2.json, DIR/layout.json
//! bicraft run --file DIR/cmds1.json        -> DIR/proj/Synth.pbip
//! cargo xtask oracle-gen slicers DIR patch  -> writes the slicer objects into the project
//! bicraft run --file DIR/cmds2.json        -> DIR/slicers.pbix
//! ```
//! then render, capture (pages printed by the first step) and compare as in
//! tools/render-oracle/README.md. The model culture is fr-FR to match a French Desktop UI.

use serde_json::{Value, json};

use super::{Cmds, Vis, merged, py_float, two_step_gen, two_step_patch, wave_amount};

fn csv() -> String {
    let regions = ["North", "South", "East", "West", "Central"];
    let cats = ["Bikes", "Clothing", "Accessories"];
    let chans = ["Online", "Store", "Partner"];
    let colors = ["Red", "Blue", "Black", "Silver"];
    let mut rows = vec!["Region,Category,Channel,Color,Month,Amount,Qty".to_string()];
    let mut k: i64 = 0;
    for (ri, r) in (0i64..).zip(regions) {
        for (ci, c) in (0i64..).zip(cats) {
            for m in 1..=12i64 {
                k += 1;
                let amt = wave_amount(ri, ci, m);
                let ch = chans.get(((m - 1) % 3) as usize).copied().unwrap_or_default();
                let co = colors.get(((m - 1) % 4) as usize).copied().unwrap_or_default();
                rows.push(format!(
                    "{r},{c},{ch},{co},2025-{m:02}-01,{},{}",
                    py_float(amt),
                    (k * 7) % 23 + 3
                ));
            }
        }
    }
    rows.join("\n") + "\n"
}

fn col(p: &str) -> Value {
    json!({"Column": {"Expression": {"SourceRef": {"Source": "s"}}, "Property": p}})
}

fn lit(v: &str) -> Value {
    json!({"Literal": {"Value": v}})
}

fn filter(cond: Value) -> Value {
    json!({"filter": {"Version": 2, "From": [{"Name": "s", "Entity": "Sales", "Type": 0}],
                      "Where": [{"Condition": cond}]}})
}

fn in_(p: &str, vals: &[&str]) -> Value {
    let vals: Vec<Value> = vals.iter().map(|v| json!([lit(&format!("'{v}'"))])).collect();
    json!({"In": {"Expressions": [col(p)], "Values": vals}})
}

fn cmp(p: &str, kind: i64, v: &str) -> Value {
    json!({"Comparison": {"ComparisonKind": kind, "Left": col(p), "Right": lit(v)}})
}

fn e(v: &str) -> Value {
    json!({"expr": {"Literal": {"Value": v}}})
}

fn mode_o(m: &str) -> Value {
    json!({"data": [{"properties": {"mode": e(&format!("'{m}'"))}}]})
}

/// `general.filter` with `cond`, then `extra` (whose keys replace or follow it).
fn sel(cond: Value, extra: Value) -> Value {
    merged(
        json!({"general": [{"properties": {"filter": filter(cond)}}]}),
        extra,
    )
}

fn f(cs: &[&str]) -> Value {
    Value::Array(
        cs.iter()
            .map(|c| json!({"role": "Values", "field": format!("Sales[{c}]")}))
            .collect(),
    )
}

/// A relative-date slicer.
fn rel(rng: &str, dur: Option<i64>, per: &str) -> Value {
    let mut p = json!({"mode": e("'Relative'"), "relativeRange": e(&format!("'{rng}'")),
                       "relativePeriod": e(&format!("'{per}'"))});
    if let Some(d) = dur {
        p["relativeDuration"] = e(&format!("{d}D"));
    }
    json!({"data": [{"properties": p}]})
}

fn and(l: Value, r: Value) -> Value {
    json!({"And": {"Left": l, "Right": r}})
}

fn dt(d: &str) -> String {
    format!("datetime'{d}T00:00:00'")
}

fn visuals() -> Vec<Vis> {
    let mut out = Vec::new();
    #[allow(clippy::too_many_arguments)]
    let mut v = |page: &'static str,
                 name: &'static str,
                 ty: &'static str,
                 x: i64,
                 y: i64,
                 w: i64,
                 h: i64,
                 fields: Value,
                 objects: Value| {
        out.push(Vis {
            page,
            name,
            ty,
            x,
            y,
            w,
            h,
            fields,
            objects,
        });
    };
    let no = || json!({});
    let sel_all = json!([{"properties": {"selectAllCheckboxEnabled": e("true")}}]);
    let search = json!([{"properties": {"selfFilterEnabled": e("true")}}]);

    v(
        "SlList",
        "lst_plain",
        "slicer",
        20,
        20,
        240,
        250,
        f(&["Region"]),
        no(),
    );
    v(
        "SlList",
        "lst_selall",
        "slicer",
        280,
        20,
        240,
        180,
        f(&["Category"]),
        sel(in_("Category", &["Bikes"]), json!({"selection": sel_all})),
    );
    v(
        "SlList",
        "lst_single",
        "slicer",
        540,
        20,
        240,
        180,
        f(&["Channel"]),
        sel(
            in_("Channel", &["Store"]),
            json!({"selection": [{"properties": {"singleSelect": e("true")}}]}),
        ),
    );
    v(
        "SlList",
        "lst_search",
        "slicer",
        800,
        20,
        240,
        250,
        f(&["Color"]),
        json!({"general": search}),
    );
    v(
        "SlList2",
        "lst_multi",
        "slicer",
        20,
        20,
        240,
        250,
        f(&["Region"]),
        sel(in_("Region", &["North", "West"]), no()),
    );
    v(
        "SlList2",
        "lst_selall0",
        "slicer",
        280,
        20,
        240,
        180,
        f(&["Category"]),
        json!({"selection": sel_all}),
    );
    v(
        "SlList2",
        "lst_inv",
        "slicer",
        540,
        20,
        240,
        250,
        f(&["Color"]),
        json!({"general": [{"properties": {"filter": filter(json!({"Not": {"Expression": in_("Color", &["Blue"])}}))}}],
               "data": [{"properties": {"isInvertedSelectionMode": e("true")}}],
               "selection": sel_all}),
    );
    v(
        "SlList2",
        "lst_hier",
        "slicer",
        800,
        20,
        240,
        250,
        f(&["Channel", "Category"]),
        no(),
    );
    v(
        "SlTiles",
        "tile_h",
        "slicer",
        20,
        20,
        600,
        80,
        f(&["Region"]),
        sel(
            in_("Region", &["North"]),
            json!({"general": [{"properties": {"orientation": e("1D"),
                                               "filter": filter(in_("Region", &["North"]))}}]}),
        ),
    );
    v(
        "SlTiles",
        "tile_btn",
        "advancedSlicerVisual",
        20,
        140,
        600,
        200,
        f(&["Category"]),
        no(),
    );
    v(
        "SlTiles",
        "tile_list",
        "listSlicer",
        660,
        20,
        280,
        300,
        f(&["Channel"]),
        no(),
    );
    v(
        "SlTiles",
        "tile_h0",
        "slicer",
        660,
        360,
        600,
        60,
        f(&["Color"]),
        json!({"general": [{"properties": {"orientation": e("1D")}}]}),
    );
    v(
        "SlDrop",
        "dd_all",
        "slicer",
        20,
        20,
        240,
        70,
        f(&["Region"]),
        mode_o("Dropdown"),
    );
    v(
        "SlDrop",
        "dd_one",
        "slicer",
        280,
        20,
        240,
        70,
        f(&["Category"]),
        sel(in_("Category", &["Bikes"]), mode_o("Dropdown")),
    );
    v(
        "SlDrop",
        "dd_multi",
        "slicer",
        540,
        20,
        240,
        70,
        f(&["Color"]),
        sel(in_("Color", &["Red", "Blue"]), mode_o("Dropdown")),
    );
    v(
        "SlRange",
        "rg_num",
        "slicer",
        20,
        20,
        300,
        120,
        f(&["Amount"]),
        mode_o("Between"),
    );
    v(
        "SlRange",
        "rg_le",
        "slicer",
        360,
        20,
        300,
        120,
        f(&["Qty"]),
        mode_o("Before"),
    );
    v(
        "SlRange",
        "rg_ge",
        "slicer",
        700,
        20,
        300,
        120,
        f(&["Qty"]),
        mode_o("After"),
    );
    v(
        "SlRange",
        "rg_date",
        "slicer",
        20,
        180,
        300,
        120,
        f(&["Month"]),
        mode_o("Between"),
    );
    v(
        "SlRange",
        "rg_before",
        "slicer",
        360,
        180,
        300,
        120,
        f(&["Month"]),
        mode_o("Before"),
    );
    v(
        "SlRange",
        "rg_after",
        "slicer",
        700,
        180,
        300,
        120,
        f(&["Month"]),
        mode_o("After"),
    );
    v(
        "SlRange",
        "rg_rel",
        "slicer",
        20,
        340,
        400,
        120,
        f(&["Month"]),
        mode_o("Relative"),
    );
    v(
        "SlRange2",
        "rg_num_sel",
        "slicer",
        20,
        20,
        300,
        120,
        f(&["Amount"]),
        sel(
            and(cmp("Amount", 2, "2000D"), cmp("Amount", 4, "6000D")),
            json!({"data": [{"properties": {"mode": e("'Between'"), "numericStart": e("2000D"),
                                            "numericEnd": e("6000D")}}]}),
        ),
    );
    v(
        "SlRange2",
        "rg_date_sel",
        "slicer",
        360,
        20,
        300,
        120,
        f(&["Month"]),
        sel(
            and(
                cmp("Month", 2, &dt("2025-03-01")),
                cmp("Month", 3, &dt("2025-10-01")),
            ),
            json!({"data": [{"properties": {"mode": e("'Between'"), "startDate": e(&dt("2025-03-01")),
                                            "endDate": e(&dt("2025-09-30"))}}]}),
        ),
    );
    v(
        "SlText",
        "txt_empty",
        "textSlicer",
        20,
        20,
        300,
        80,
        f(&["Region"]),
        no(),
    );
    v(
        "SlText",
        "txt_typed",
        "textSlicer",
        360,
        20,
        300,
        80,
        f(&["Category"]),
        json!({"general": [{"properties": {"filter": filter(
            json!({"Contains": {"Left": col("Category"), "Right": lit("'ike'")}}))}}]}),
    );

    // displayed range different from the filter, strict bounds, filter-only ranges
    v(
        "SlRange3",
        "rg_num_disp",
        "slicer",
        20,
        20,
        300,
        120,
        f(&["Amount"]),
        sel(
            and(cmp("Amount", 2, "1000D"), cmp("Amount", 4, "8000D")),
            json!({"data": [{"properties": {"mode": e("'Between'"), "numericStart": e("2000D"),
                                            "numericEnd": e("6000D")}}]}),
        ),
    );
    v(
        "SlRange3",
        "rg_num_excl",
        "slicer",
        360,
        20,
        300,
        120,
        f(&["Amount"]),
        sel(
            and(cmp("Amount", 1, "2000D"), cmp("Amount", 3, "6000D")),
            mode_o("Between"),
        ),
    );
    v(
        "SlRange3",
        "rg_date_fonly",
        "slicer",
        700,
        20,
        300,
        120,
        f(&["Month"]),
        sel(
            and(
                cmp("Month", 2, &dt("2025-04-01")),
                cmp("Month", 3, &dt("2025-09-01")),
            ),
            mode_o("Between"),
        ),
    );
    v(
        "SlRange3",
        "rg_le_sel",
        "slicer",
        20,
        180,
        300,
        120,
        f(&["Qty"]),
        sel(
            cmp("Qty", 4, "15L"),
            json!({"data": [{"properties": {"mode": e("'Before'"), "numericEnd": e("15L")}}]}),
        ),
    );
    v(
        "SlRange3",
        "rg_ge_sel",
        "slicer",
        360,
        180,
        300,
        120,
        f(&["Month"]),
        sel(
            cmp("Month", 2, &dt("2025-06-01")),
            json!({"data": [{"properties": {"mode": e("'After'"), "startDate": e(&dt("2025-06-01"))}}]}),
        ),
    );
    // relative-date slicers with their settings
    v(
        "SlRel",
        "rel_last3m",
        "slicer",
        20,
        20,
        400,
        120,
        f(&["Month"]),
        rel("Last", Some(3), "Months"),
    );
    v(
        "SlRel",
        "rel_next7d",
        "slicer",
        460,
        20,
        400,
        120,
        f(&["Month"]),
        rel("Next", Some(7), "Days"),
    );
    v(
        "SlRel",
        "rel_this_w",
        "slicer",
        460,
        340,
        400,
        120,
        f(&["Month"]),
        rel("This", None, "Weeks"),
    );
    v(
        "SlRel",
        "rel_this_m",
        "slicer",
        20,
        180,
        400,
        120,
        f(&["Month"]),
        rel("This", None, "Months"),
    );
    v(
        "SlRel",
        "rel_last7d",
        "slicer",
        460,
        180,
        400,
        120,
        f(&["Month"]),
        rel("Last", Some(7), "Days"),
    );
    // header off, search box on
    let hdr_off = json!([{"properties": {"show": e("false")}}]);
    v(
        "SlHead",
        "hdr_off_list",
        "slicer",
        20,
        20,
        240,
        250,
        f(&["Region"]),
        json!({"header": hdr_off}),
    );
    v(
        "SlHead",
        "hdr_off_search",
        "slicer",
        280,
        20,
        240,
        250,
        f(&["Color"]),
        json!({"header": hdr_off, "general": search}),
    );
    v(
        "SlHead",
        "search_sel",
        "slicer",
        540,
        20,
        240,
        250,
        f(&["Region"]),
        json!({"general": [{"properties": {"selfFilterEnabled": e("true"),
                                           "filter": filter(in_("Region", &["South"]))}}]}),
    );
    v(
        "SlHead",
        "hdr_off_drop",
        "slicer",
        800,
        20,
        240,
        70,
        f(&["Category"]),
        merged(json!({"header": hdr_off}), mode_o("Dropdown")),
    );
    v(
        "SlHead",
        "hdr_off_rng",
        "slicer",
        800,
        120,
        300,
        100,
        f(&["Qty"]),
        merged(json!({"header": hdr_off}), mode_o("Between")),
    );
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
        json!({"name": "Sales", "csv": csv(), "types": {"Month": "dateTime", "Amount": "double", "Qty": "int64"}}),
    );
    c.c(
        "measure.add",
        json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])", "format_string": "#,0"}),
    );
    c.c("report.new", json!({}));
    two_step_gen(dir, c, &vis, "slicers.pbix")
}
