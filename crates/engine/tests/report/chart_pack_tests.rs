//! BIcraft's chart pack (docs/formats/chart-pack.md): the organization chart, Venn diagram,
//! Marimekko and calendar heatmap. The golden comparison of the two layouts: the renderer's
//! Rust (`bicraft_render::chart_pack`) against `chart_pack_golden.json`, and the visuals'
//! TypeScript (compiled by BIcraft's own bundler) against the Rust, run with Node when Node is on
//! the PATH (a development check; it prints a note and passes when Node is missing); each
//! packaged visual run under Node with a stand-in host and document; the packages'
//! capabilities against the API schema; and the visuals installed in a report, drawn by
//! `report.render` and PDF export, formatted, and kept through PBIP and PBIX round trips.
//! `BICRAFT_BLESS=1` rewrites the golden file from the Rust layout.

use std::path::{Path, PathBuf};

use bicraft_engine::Session;
use bicraft_engine::chart_pack::{VISUALS, find, install, layout_script, package_bytes};
use bicraft_render::chart_pack;
use serde_json::{Value, json};

use crate::tempdir::TempDir;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

pub(crate) fn has_node() -> bool {
    let node = std::process::Command::new("node").arg("--version").output();
    node.is_ok_and(|o| o.status.success())
}

// ---------------------------------------------------------------------------------------------
// Golden layouts
// ---------------------------------------------------------------------------------------------

fn org_rows() -> Value {
    json!([
        ["ceo", null, "Ada Lovelace", 120],
        ["cto", "ceo", "Grace Hopper", 45],
        ["cfo", "ceo", "Mary Somerville", 30],
        ["coo", "ceo", "Emmy Noether", 40.5],
        ["dev1", "cto", "Platform team with a very long name", 20],
        ["dev2", "cto", "Apps", 25],
        ["fin1", "cfo", "Accounting", 30],
        ["ops1", "coo", "Logistics", 22],
        ["ops2", "coo", "Support", 10],
        ["ops3", "coo", "Facilities", 8.5]
    ])
}

fn day(z: i64) -> String {
    // 2024-01-01 is day 19723
    let (y, m, d) = civil(z);
    format!("{y:04}-{m:02}-{d:02}")
}

fn civil(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// `rows` with `highlights` (another visual's selection highlighting the visual, docs/formats/
/// bicraft-visuals.md "Highlights"): the highlighted part of each row's measure (cell `k`) as
/// `part` gives it from the row's index and measure, null when the row is not highlighted.
fn highlighted(rows: &Value, k: usize, part: &dyn Fn(usize, f64) -> Option<f64>) -> Value {
    let list = rows.as_array().cloned().unwrap_or_default();
    let parts: Vec<Value> = list
        .iter()
        .enumerate()
        .map(|(i, r)| {
            r.get(k)
                .and_then(Value::as_f64)
                .and_then(|v| part(i, v))
                .map_or(Value::Null, |p| json!(p))
        })
        .collect();
    json!({"rows": list, "highlights": parts})
}

/// The cases: a kind, an input and a size.
pub(crate) fn cases() -> Vec<(&'static str, &'static str, Value, f64, f64)> {
    let mut wide = vec![json!(["root", null, "Root", 1])];
    for i in 0..40 {
        wide.push(json!([format!("n{i}"), "root", format!("Node number {i}"), i]));
    }
    let numeric = json!([
        [1, null, "One", 1],
        [2, 1, null, 2],
        [3, 1, "Three", null],
        [4.5, 2, true, 4]
    ]);
    let venn3 = json!([
        ["u1", "Email"],
        ["u2", "Email"],
        ["u3", "Email"],
        ["u4", "Email"],
        ["u5", "Email"],
        ["u3", "Phone"],
        ["u4", "Phone"],
        ["u6", "Phone"],
        ["u7", "Phone"],
        ["u4", "Store"],
        ["u5", "Store"],
        ["u7", "Store"],
        ["u8", "Store"],
        ["u9", "Store"],
        ["u4", "Store"]
    ]);
    let venn5 = json!([
        ["a", "S1"],
        ["b", "S1"],
        ["c", "S2"],
        ["a", "S2"],
        ["d", "S3"],
        ["a", "S3"],
        ["b", "S3"],
        ["e", "S4"],
        ["f", "S5"],
        ["g", "S5"],
        [null, "S1"],
        ["h", ""]
    ]);
    let mekko = json!([
        ["North", "Bikes", 120],
        ["North", "Cars", 300],
        ["North", "Boats", 40],
        ["South", "Bikes", 80],
        ["South", "Cars", 90],
        ["East", "Bikes", 20],
        ["East", "Cars", 260],
        ["East", "Boats", 120],
        ["West", "Boats", 15]
    ]);
    let many: Vec<Value> = (0..30)
        .flat_map(|i| {
            (0..3).map(move |k| json!([format!("Category {i}"), format!("S{k}"), (i * 7 + k * 13) % 23]))
        })
        .collect();
    let year: Vec<Value> = (0..366)
        .filter(|i| i % 3 != 1)
        .map(|i| json!([day(19_723 + i), (i * 17) % 41]))
        .collect();
    let two: Vec<Value> = (0..700)
        .step_by(5)
        .map(|i| {
            json!([
                format!("{}T08:30:00", day(19_600 + i)),
                (i % 9) as f64 * 1.5 - 2.0
            ])
        })
        .collect();
    // a narrow chain beside a wide row: compaction tucks the wide grandchildren under the chain
    let uneven = json!([
        ["r", null, "Root", 9],
        ["a", "r", "A", 1],
        ["a1", "a", "A1", 1],
        ["a11", "a1", "A11", 1],
        ["a12", "a1", "A12", 1],
        ["a13", "a1", "A13", 1],
        ["a14", "a1", "A14", 1],
        ["b", "r", "B", 1],
        ["b1", "b", "B1", 1],
        ["b2", "b", "B2", 1],
        ["b3", "b", "B3", 1],
        ["b4", "b", "B4", 1],
        ["x", null, "Second root", 2]
    ]);
    let spring: Vec<Value> = (0..72).map(|i| json!([day(19_793 + i), (i * 5) % 17])).collect();
    let winter: Vec<Value> = (0..110)
        .step_by(2)
        .map(|i| json!([day(19_662 + i), i % 7]))
        .collect();
    let venn_overlap: Vec<Value> = (1..=9)
        .map(|i| json!([format!("i{i}"), "A"]))
        .chain((7..=12).map(|i| json!([format!("i{i}"), "B"])))
        .collect();
    // energy-like flows over three columns, with a link that skips a column
    let sankey = json!([
        ["Coal", "Power plants", 120],
        ["Gas", "Power plants", 80],
        ["Gas", "Heating", 45],
        ["Solar", "Power plants", 30.5],
        ["Power plants", "Homes", 90],
        ["Power plants", "Industry with a long name", 110],
        ["Power plants", "Losses", 30.5],
        ["Heating", "Homes", 45],
        ["Coal", "Industry with a long name", 20],
        ["Gas", "Heating", 5]
    ]);
    let crossing = json!([
        ["A", "Z", 1],
        ["B", "Y", 5],
        ["A", "X", 5],
        ["C", "Z", 2],
        ["C", "Y", 1]
    ]);
    vec![
        // highlighted by another visual's selection: parts over dimmed marks, dimmed nodes
        (
            "hl_mekko",
            "mekko",
            highlighted(&mekko, 2, &|i, v| (i % 2 == 0).then_some(v / 2.0)),
            480.0,
            300.0,
        ),
        (
            "hl_calendar",
            "calendar",
            highlighted(&Value::Array(spring.clone()), 1, &|i, v| {
                (i % 3 == 0).then_some(v * 0.6)
            }),
            420.0,
            180.0,
        ),
        (
            "hl_calendar_minimal",
            "calendar",
            {
                let mut x = highlighted(&Value::Array(spring.clone()), 1, &|i, v| {
                    (i % 3 == 0).then_some(v)
                });
                x["options"] = json!({"stylePreset": "minimal"});
                x
            },
            420.0,
            180.0,
        ),
        (
            "hl_sankey",
            "sankey",
            // the coal rows wholly, a third of gas to heating, the rest not highlighted
            highlighted(&sankey, 2, &|i, v| match i {
                0 | 8 => Some(v),
                2 => Some(v / 3.0),
                _ => None,
            }),
            640.0,
            320.0,
        ),
        (
            "hl_org",
            "org",
            highlighted(&org_rows(), 3, &|i, v| [1, 4, 5].contains(&i).then_some(v)),
            640.0,
            320.0,
        ),
        // nothing of the visual highlighted: every mark dimmed, no part drawn
        (
            "hl_mekko_none",
            "mekko",
            highlighted(&mekko, 2, &|_, _| None),
            360.0,
            240.0,
        ),
        ("org_basic", "org", json!({"rows": org_rows()}), 640.0, 320.0),
        ("org_compact", "org", json!({"rows": uneven}), 420.0, 260.0),
        (
            "org_slots",
            "org",
            json!({"rows": uneven, "options": {"compact": false}}),
            420.0,
            260.0,
        ),
        (
            "org_toggled",
            "org",
            json!({"rows": org_rows(), "options": {"toggled": ["coo", "cto"]}}),
            640.0,
            320.0,
        ),
        (
            "org_levels",
            "org",
            json!({"rows": org_rows(), "options": {"levels": 2, "toggled": ["cfo"], "color": "#5a9e3a", "textColor": "#252423", "fontSize": 13, "showValues": false}}),
            500.0,
            300.0,
        ),
        // left to right (G109, 2026-11-24): levels across, the slots down, buttons on the right
        (
            "org_left_to_right",
            "org",
            json!({"rows": org_rows(), "options": {"leftToRight": true, "toggled": ["coo"]}}),
            560.0,
            360.0,
        ),
        (
            "org_left_to_right_slots_minimal",
            "org",
            json!({"rows": uneven, "options": {"leftToRight": true, "compact": false, "stylePreset": "minimal"}}),
            420.0,
            300.0,
        ),
        ("org_wide", "org", json!({"rows": wide}), 420.0, 200.0),
        ("org_numeric_ids", "org", json!({"rows": numeric}), 300.0, 260.0),
        ("org_tiny", "org", json!({"rows": org_rows()}), 30.0, 20.0),
        ("venn_three", "venn", json!({"rows": venn3}), 400.0, 300.0),
        (
            "venn_two",
            "venn",
            json!({"rows": [["1", "A"], ["2", "A"], ["2", "B"], ["3", "B"], ["4", "B"]]}),
            360.0,
            220.0,
        ),
        (
            "venn_proportional",
            "venn",
            json!({"rows": venn_overlap, "options": {"proportional": true}}),
            360.0,
            220.0,
        ),
        (
            "venn_proportional_inside",
            "venn",
            json!({"rows": [["1", "A"], ["2", "A"], ["3", "A"], ["4", "A"], ["1", "B"], ["2", "B"]], "options": {"proportional": true}}),
            300.0,
            200.0,
        ),
        (
            "venn_proportional_apart",
            "venn",
            json!({"rows": [["1", "A"], ["2", "A"], ["3", "A"], ["4", "B"]], "options": {"proportional": true}}),
            300.0,
            160.0,
        ),
        (
            "venn_one",
            "venn",
            json!({"rows": [["1", "Only"], ["2", "Only"]], "options": {"color1": "#C23B5A"}}),
            200.0,
            200.0,
        ),
        (
            "venn_more_fr",
            "venn",
            json!({"rows": venn5, "lang": "fr", "options": {"showCounts": true, "fontSize": 14}}),
            420.0,
            260.0,
        ),
        (
            "venn_no_counts",
            "venn",
            json!({"rows": venn3, "options": {"showCounts": false, "color2": "#112233"}}),
            300.0,
            300.0,
        ),
        ("mekko_basic", "mekko", json!({"rows": mekko}), 480.0, 300.0),
        // series colours (G109, 2026-11-24): the first and third set, the second the theme's
        // second colour, an invalid one ignored
        (
            "mekko_series_colors",
            "mekko",
            json!({"rows": mekko, "options": {"color1": "#112233", "color2": "red", "color3": "#aa5500"},
                   "theme": {"colors": ["#3B5BA9", "#C0504D", "#9BBB59"]}}),
            480.0,
            300.0,
        ),
        (
            "mekko_sorted_axis",
            "mekko",
            json!({"rows": mekko, "options": {"sortColumns": true, "widthAxis": true}}),
            480.0,
            300.0,
        ),
        (
            "mekko_one_series",
            "mekko",
            json!({"rows": [["A", null, 10], ["B", null, 30], ["C", null, -4]]}),
            300.0,
            200.0,
        ),
        (
            "mekko_no_legend",
            "mekko",
            json!({"rows": mekko, "options": {"showLegend": false, "fontSize": 12, "showLabels": true}}),
            360.0,
            240.0,
        ),
        (
            "mekko_many",
            "mekko",
            json!({"rows": many, "options": {"showLabels": false}}),
            400.0,
            220.0,
        ),
        (
            "mekko_zero",
            "mekko",
            json!({"rows": [["A", "x", 0], ["B", "x", null]]}),
            300.0,
            200.0,
        ),
        ("calendar_year", "calendar", json!({"rows": year}), 900.0, 200.0),
        (
            "calendar_two_years_fr",
            "calendar",
            json!({"rows": two, "lang": "fr"}),
            700.0,
            320.0,
        ),
        (
            "calendar_small_options",
            "calendar",
            json!({"rows": [["2023-06-15", 4], ["2023-06-16", 4]], "options": {"low": "#ffffff", "high": "#C23B5A", "empty": "#F5F5F5", "showLegend": false}}),
            260.0,
            90.0,
        ),
        (
            "calendar_months",
            "calendar",
            json!({"rows": spring, "options": {"monthsOnly": true}}),
            420.0,
            140.0,
        ),
        (
            "calendar_sunday_fr",
            "calendar",
            json!({"rows": spring, "lang": "fr", "options": {"sundayFirst": true}}),
            900.0,
            200.0,
        ),
        (
            "calendar_months_span",
            "calendar",
            json!({"rows": winter, "options": {"monthsOnly": true, "sundayFirst": true}}),
            420.0,
            260.0,
        ),
        // BIcraft's look (docs/formats/visual-look.md): presets, switches and the theme
        (
            "look_org_bold",
            "org",
            json!({"rows": [["a", null, "Alpha", 10], ["b", "a", "Beta", 4], ["c", "a", "Gamma", 6]], "options": {"stylePreset": "bold"}}),
            360.0,
            220.0,
        ),
        (
            "look_org_theme",
            "org",
            json!({"rows": [["a", null, "Alpha", 10], ["b", "a", "Beta", 4], ["c", "a", "Gamma", 6]], "theme": {"colors": ["#7030A0"]}}),
            360.0,
            220.0,
        ),
        (
            "look_venn_bold",
            "venn",
            json!({"rows": [["1", "A"], ["2", "A"], ["2", "B"], ["3", "B"], ["3", "C"]], "options": {"stylePreset": "bold"}}),
            320.0,
            240.0,
        ),
        (
            "look_mekko_theme",
            "mekko",
            json!({"rows": [["X", "s1", 3], ["X", "s2", 5], ["Y", "s1", 2], ["Y", "s2", 1]], "options": {"showLabels": true, "styleGradient": false}, "theme": {"colors": ["#FFE699", "#203864"]}}),
            360.0,
            240.0,
        ),
        (
            "look_calendar_bold",
            "calendar",
            json!({"rows": [["2024-03-01", 1], ["2024-03-05", 4], ["2024-04-10", 9]], "options": {"stylePreset": "bold", "monthsOnly": true}, "theme": {"colors": ["#2E7D32"]}}),
            420.0,
            200.0,
        ),
        // the look's second pass: a dark report background, value chips, no card
        (
            "look2_calendar_dark",
            "calendar",
            json!({"rows": [["2024-03-01", 1], ["2024-03-05", 4], ["2024-04-10", 9], ["2024-06-30", 6]], "options": {"monthsOnly": true}, "theme": {"colors": [], "background": "#1F1F1F"}}),
            520.0,
            260.0,
        ),
        (
            "look2_venn_pills",
            "venn",
            json!({"rows": [["1", "A"], ["2", "A"], ["2", "B"], ["3", "B"], ["3", "C"]], "options": {"stylePills": true}}),
            320.0,
            240.0,
        ),
        (
            "look2_org_dark",
            "org",
            json!({"rows": [["a", null, "Alpha", 10], ["b", "a", "Beta", 4], ["c", "a", "Gamma", 6]], "theme": {"colors": [], "background": "#202020"}}),
            360.0,
            220.0,
        ),
        (
            "look2_mekko_no_card",
            "mekko",
            json!({"rows": [["X", "s1", 3], ["X", "s2", 5], ["Y", "s1", 2], ["Y", "s2", 1]], "options": {"showLabels": true, "styleCard": false, "stylePreset": "bold"}}),
            360.0,
            240.0,
        ),
        // date cells written with their column's format string (G109, 2026-11-24); two dates
        // of one month stay two columns
        (
            "mekko_dates",
            "mekko",
            json!({"rows": [
                ["2024-01-02T00:00:00", "2024-03-05T14:07:00", 10],
                ["2024-01-20T00:00:00", "2024-03-05T14:07:00", 5],
                ["2024-02-01T00:00:00", "North", 8]
            ], "formats": ["mmm yyyy", "General Date", ""], "lang": "fr"}),
            420.0,
            260.0,
        ),
        (
            "org_dates",
            "org",
            json!({"rows": [
                ["a", null, "2024-01-31T00:00:00", 1],
                ["b", "a", "2024-12-30T00:00:00", 2],
                ["2024-11-25T00:00:00", "a", null, 3]
            ], "formats": ["dd/mm/yyyy", "", "Long Date", ""]}),
            480.0,
            220.0,
        ),
        (
            "venn_dates",
            "venn",
            json!({"rows": [
                ["x", "2024-01-31T00:00:00"], ["y", "2024-01-31T00:00:00"], ["y", "2024-02-29T00:00:00"]
            ], "formats": ["", "d mmm"], "lang": "fr"}),
            360.0,
            240.0,
        ),
        // BIcraft's look on the Sankey: Bold nodes and labels, the report theme's colours
        (
            "look_sankey_bold",
            "sankey",
            json!({"rows": sankey.clone(), "options": {"stylePreset": "bold", "linkColor": "destination"}}),
            640.0,
            320.0,
        ),
        (
            "look_sankey_theme",
            "sankey",
            json!({"rows": sankey.clone(), "options": {"styleGradient": false}, "theme": {"colors": ["#7030A0", "#00A3A3", "#E07B00"]}}),
            480.0,
            260.0,
        ),
        ("sankey_basic", "sankey", json!({"rows": sankey}), 640.0, 320.0),
        (
            "sankey_cycle_fr",
            "sankey",
            json!({"rows": [["A", "B", 3], ["B", "C", 2], ["C", "A", 1], ["C", "D", 1.5], ["S", "S", 4], [1, 2, 5], [2, "C", 2]], "lang": "fr"}),
            420.0,
            240.0,
        ),
        (
            "sankey_options",
            "sankey",
            json!({"rows": sankey, "options": {"linkColor": "destination", "nodeWidth": 20, "nodePadding": 4, "linkOpacity": 60, "fontSize": 13}}),
            500.0,
            260.0,
        ),
        // node colours by the order seen, links coloured by their source following (G113)
        (
            "sankey_node_colors",
            "sankey",
            json!({"rows": sankey.clone(), "options": {"nodeColor1": "#112233", "nodeColor4": "#C00000", "nodeColor13": "#00FF00"}}),
            500.0,
            260.0,
        ),
        (
            "sankey_fixed_no_labels",
            "sankey",
            json!({"rows": sankey, "options": {"linkColor": "fixed", "linkFill": "#5a9e3a", "showLabels": false}}),
            360.0,
            200.0,
        ),
        (
            "sankey_crossing",
            "sankey",
            json!({"rows": crossing}),
            300.0,
            220.0,
        ),
        // a narrow visual: a link that skips the middle column, the last column's labels whole
        ("sankey_narrow", "sankey", narrow_sankey(), NARROW.0, NARROW.1),
        // date nodes written with their column's format string; two dates of one day stay two
        (
            "sankey_dates_fr",
            "sankey",
            json!({"rows": [
                ["2024-01-31T00:00:00", "Paid", 4],
                ["2024-01-31T08:00:00", "Paid", 2],
                ["2024-02-29T00:00:00", "Open", 3],
                ["Paid", "2024-03-01T00:00:00", 5]
            ], "formats": ["d mmmm yyyy", "dd/mm", ""], "lang": "fr"}),
            420.0,
            220.0,
        ),
        ("empty_org", "org", json!({"rows": []}), 300.0, 200.0),
        (
            "empty_sankey",
            "sankey",
            json!({"rows": [["A", "", 1], ["B", "C", -2], [null, "C", 3], ["D", "E", "x"]]}),
            300.0,
            200.0,
        ),
        (
            "empty_calendar",
            "calendar",
            json!({"rows": [["not a date", 3]]}),
            300.0,
            200.0,
        ),
    ]
    .into_iter()
    .chain(scatter_cases())
    .chain(line_cases())
    .collect()
}

/// `n` scatter rows `[details, legend, x, y, size]`: three segments, x and y loosely related,
/// a few rows without a size.
pub(crate) fn scatter_rows(n: usize) -> Vec<Value> {
    (0..n)
        .map(|i| {
            let x = ((i * 37) % 101) as f64 + (i % 7) as f64 * 0.25;
            let y = x * 0.6 + ((i * 53) % 41) as f64 - 20.0;
            let size = if i % 9 == 4 {
                Value::Null
            } else {
                json!(((i * 13) % 50) + 1)
            };
            let segment = ["North", "South", "West"][i % 3];
            json!([format!("P{i}"), segment, x, y, size])
        })
        .collect()
}

const SCATTER_NAMES: [&str; 5] = ["Product", "Segment", "Price", "Margin", "Volume"];

/// Scatter+ cases: one per option (markers, density, marginals, trend, quadrants, labels,
/// legend, look, highlights, French) and hostile rows.
fn scatter_cases() -> Vec<(&'static str, &'static str, Value, f64, f64)> {
    let rows = scatter_rows(40);
    let dense = scatter_rows(600);
    let with = |o: Value| json!({"rows": rows.clone(), "names": SCATTER_NAMES, "options": o});
    vec![
        ("scatter_basic", "scatter", with(json!({})), 520.0, 340.0),
        (
            "scatter_border_only",
            "scatter",
            with(json!({"markerFill": false, "markerSize": 7})),
            520.0,
            340.0,
        ),
        (
            "scatter_fill_only",
            "scatter",
            with(json!({"markerBorder": false, "fillOpacity": 50, "showLegend": false})),
            420.0,
            300.0,
        ),
        (
            "scatter_density_auto",
            "scatter",
            json!({"rows": dense, "names": SCATTER_NAMES, "options": {"densityThreshold": 200, "hexSize": 12}}),
            520.0,
            340.0,
        ),
        (
            "scatter_density_on_fr",
            "scatter",
            json!({"rows": rows.clone(), "names": SCATTER_NAMES, "lang": "fr", "options": {"density": "on", "hexSize": 16}}),
            420.0,
            300.0,
        ),
        (
            "scatter_points_forced",
            "scatter",
            json!({"rows": scatter_rows(300), "options": {"density": "off", "densityThreshold": 100, "markerSize": 3}}),
            420.0,
            300.0,
        ),
        (
            "scatter_histograms_trend",
            "scatter",
            with(json!({"marginals": "histogram", "trendLine": true})),
            520.0,
            360.0,
        ),
        (
            "scatter_box_quadrants",
            "scatter",
            with(json!({"marginals": "box", "quadrants": true})),
            520.0,
            360.0,
        ),
        (
            "scatter_quadrant_values",
            "scatter",
            with(
                json!({"quadrants": true, "quadrantAt": "value", "quadrantX": 50, "quadrantY": 10,
                        "labelTopRight": "Stars", "labelTopLeft": "Niche", "labelBottomLeft": "Laggards",
                        "labelBottomRight": "Volume plays with a very long label"}),
            ),
            520.0,
            340.0,
        ),
        (
            "scatter_top_labels",
            "scatter",
            with(json!({"labelMode": "top", "labelCount": 4, "fontSize": 12})),
            520.0,
            340.0,
        ),
        (
            "scatter_selected_labels",
            "scatter",
            json!({"rows": rows.clone(), "names": SCATTER_NAMES, "selected": [1, 4, 7, 39, 1000, -1, 2.5, "x"],
                   "options": {"labelMode": "selected"}}),
            520.0,
            340.0,
        ),
        (
            "hl_scatter",
            "scatter",
            {
                let mut v = highlighted(&json!(rows.clone()), 3, &|i, y| (i % 4 == 0).then_some(y));
                v["names"] = json!(SCATTER_NAMES);
                v
            },
            520.0,
            340.0,
        ),
        (
            "hl_scatter_density",
            "scatter",
            {
                let mut v = highlighted(&json!(dense.clone()), 3, &|i, y| (i % 5 == 0).then_some(y));
                v["options"] = json!({"density": "on"});
                v
            },
            420.0,
            300.0,
        ),
        (
            "look_scatter_bold_theme",
            "scatter",
            json!({"rows": rows.clone(), "options": {"stylePreset": "bold", "labelMode": "top", "trendLine": true},
                   "theme": {"colors": ["#7030A0", "#00A3A3", "#E07B00"]}}),
            520.0,
            340.0,
        ),
        (
            "look_scatter_dark",
            "scatter",
            json!({"rows": rows.clone(), "names": SCATTER_NAMES, "options": {"marginals": "histogram"},
                   "theme": {"colors": [], "background": "#1B1A19"}}),
            520.0,
            340.0,
        ),
        (
            "scatter_one_point_small",
            "scatter",
            json!({"rows": [["Only", null, 3, 4, null]], "options": {"quadrants": true, "trendLine": true, "marginals": "box"}}),
            160.0,
            150.0,
        ),
        // the R² text clear of the top labels (Eve at the plot edge, G115)
        (
            "scatter_trend_labels",
            "scatter",
            json!({"rows": [
                ["Ann", null, 1, 10, null], ["Bob", null, 2, 19, null], ["Cy", null, 3, 31, null],
                ["Dee", null, 4, 39, null], ["Eve", null, 5, 52, null]
            ], "names": SCATTER_NAMES, "options": {"trendLine": true, "labelMode": "top", "labelCount": 5}}),
            360.0,
            240.0,
        ),
        (
            "empty_scatter",
            "scatter",
            json!({"rows": [["a", "b", "x", 1, 2], [null, null, null, null, null], ["c", 1, 2], "row", 7]}),
            300.0,
            200.0,
        ),
    ]
}

/// `months` months of two series from January 2024, `[date, series, value, value2]`: North
/// rising, South falling, the second value a target between them.
pub(crate) fn line_rows(months: usize) -> Vec<Value> {
    let mut out = Vec::new();
    for m in 0..months {
        let (y, mo) = (2024 + m / 12, m % 12 + 1);
        let date = format!("{y:04}-{mo:02}-01");
        let north = 100.0 + m as f64 * 6.0 + ((m * 7) % 5) as f64 * 4.0;
        let south = 220.0 - m as f64 * 5.0 + ((m * 3) % 4) as f64 * 6.0;
        out.push(json!([date, "North", north, 160]));
        out.push(json!([date, "South", south, 150]));
    }
    out
}

/// Line / Area+ cases: one per option (area, band, threshold, rule, labels, forecast, emphasis,
/// legend, axes, look, highlights, French) and hostile rows.
fn line_cases() -> Vec<(&'static str, &'static str, Value, f64, f64)> {
    let rows = line_rows(24);
    let fmts = json!(["yyyy-mm-dd", "", "0", "0"]);
    let with = |o: Value| json!({"rows": rows.clone(), "formats": fmts.clone(), "options": o});
    let one: Vec<Value> = line_rows(18)
        .into_iter()
        .filter(|r| r[1] == "North")
        .map(|r| json!([r[0], null, r[2], r[3]]))
        .collect();
    let single = |o: Value| json!({"rows": one.clone(), "options": o});
    let daily: Vec<Value> = (0..60)
        .map(|i| json!([day(20_000 + i), null, ((i * 17) % 23) as f64 - 4.0, null]))
        .collect();
    let hourly: Vec<Value> = (0..30)
        .map(|i| {
            json!([
                format!("2024-03-05T{:02}:{:02}:00", 6 + i / 2, (i % 2) * 30),
                "Load",
                (i * 7 % 11) as f64,
                null
            ])
        })
        .collect();
    let numeric: Vec<Value> = (0..25)
        .map(|i| {
            json!([
                i as f64 * 2.5,
                if i % 2 == 0 { "A" } else { "B" },
                (i * i) as f64 * 0.8,
                null
            ])
        })
        .collect();
    let gaps = json!([
        ["2024-01-01", null, 10, null],
        ["2024-02-01", null, 14, null],
        ["2024-03-01", null, null, null],
        ["2024-04-01", null, 9, null],
        ["2024-05-01", null, null, null],
        ["2024-06-01", null, 12, null],
        ["2024-07-01", null, 18, null],
        ["2024-07-01", null, 2, null]
    ]);
    let many: Vec<Value> = (0..2100)
        .flat_map(|i| {
            let d = format!("{}T00:00:00", day(18_000 + i));
            [
                json!([d.clone(), "X", (i % 97) as f64, null]),
                json!([d, "Y", (i % 61) as f64 + 30.0, null]),
            ]
        })
        .collect();
    vec![
        ("line_dates", "linearea", with(json!({})), 560.0, 320.0),
        (
            "line_area",
            "linearea",
            with(json!({"area": true, "areaFill": "#8a4fe0", "areaOpacity": 40, "lineWidth": 3})),
            560.0,
            320.0,
        ),
        (
            "line_band",
            "linearea",
            single(json!({"bandUp": "#1C8FCC", "bandOpacity": 45})),
            480.0,
            300.0,
        ),
        (
            "line_band_off",
            "linearea",
            single(json!({"band": false, "labelMode": "none"})),
            480.0,
            300.0,
        ),
        (
            "line_threshold_above",
            "linearea",
            with(json!({"threshold": true, "thresholdValue": 170})),
            560.0,
            320.0,
        ),
        (
            "line_threshold_below",
            "linearea",
            single(
                json!({"threshold": true, "thresholdValue": 150, "thresholdSide": "below", "thresholdColor": "#bf8700"}),
            ),
            480.0,
            300.0,
        ),
        (
            "line_rule",
            "linearea",
            with(json!({"colorRule": "above", "ruleValue": 160, "ruleColor": "#0e9f84"})),
            560.0,
            320.0,
        ),
        (
            "line_rule_below",
            "linearea",
            single(json!({"colorRule": "below", "ruleValue": 150})),
            480.0,
            300.0,
        ),
        (
            "line_labels_minmax",
            "linearea",
            with(json!({"labelMode": "minmax", "fontSize": 12})),
            560.0,
            320.0,
        ),
        (
            "line_labels_all",
            "linearea",
            single(json!({"labelMode": "all", "markers": true})),
            560.0,
            320.0,
        ),
        (
            "line_forecast",
            "linearea",
            with(json!({"forecastFrom": "2025-06-01", "area": true})),
            560.0,
            320.0,
        ),
        (
            "line_forecast_today_fr",
            "linearea",
            json!({"rows": rows.clone(), "lang": "fr", "today": 1_727_740_800_000.0_f64,
                   "options": {"forecastFrom": "today"}}),
            560.0,
            320.0,
        ),
        (
            "line_emphasis",
            "linearea",
            with(json!({"emphasize": "South", "labelMode": "none"})),
            560.0,
            320.0,
        ),
        (
            "line_legend",
            "linearea",
            with(json!({"endLabels": false})),
            560.0,
            320.0,
        ),
        (
            "line_category_axis",
            "linearea",
            with(json!({"axisType": "category", "yZero": true})),
            560.0,
            320.0,
        ),
        (
            "line_numeric_axis",
            "linearea",
            json!({"rows": numeric}),
            480.0,
            300.0,
        ),
        (
            "line_text_axis",
            "linearea",
            json!({"rows": [["Mon", null, 3, null], ["Tue", null, 5, null], ["Wed", null, 4, null], ["Thu", null, 8, null],
                            ["Fri", null, 6.5, null], ["Sat", null, 2, null], ["Sun", null, 1, null]]}),
            360.0,
            240.0,
        ),
        ("line_daily", "linearea", json!({"rows": daily}), 520.0, 260.0),
        (
            "line_hourly",
            "linearea",
            json!({"rows": hourly, "lang": "fr"}),
            520.0,
            260.0,
        ),
        (
            "line_gaps",
            "linearea",
            json!({"rows": gaps, "options": {"area": true}}),
            420.0,
            260.0,
        ),
        ("line_many", "linearea", json!({"rows": many}), 640.0, 300.0),
        // stacked areas: each series on the ones before it, labels with their own values (G116)
        (
            "line_stacked_area",
            "linearea",
            json!({"rows": rows.clone(), "formats": fmts.clone(),
                   "options": {"area": true, "stacked": true, "labelMode": "minmax"}}),
            560.0,
            320.0,
        ),
        // stacked lines with a gap, a fall and a second measure left out, in French
        (
            "line_stacked_lines_fr",
            "linearea",
            json!({"rows": [
                ["2024-01-01", "A", 10, 4], ["2024-02-01", "A", 12, 4], ["2024-03-01", "A", 9, 4],
                ["2024-01-01", "B", 5, 4], ["2024-02-01", "B", null, 4], ["2024-03-01", "B", 7, 4],
                ["2024-01-01", "C", -3, 4], ["2024-02-01", "C", 6, 4], ["2024-03-01", "C", 2, 4]
            ], "lang": "fr", "options": {"stacked": true, "markers": true, "labelMode": "all"}}),
            480.0,
            280.0,
        ),
        (
            "hl_linearea",
            "linearea",
            {
                let mut v = highlighted(&json!(rows.clone()), 2, &|i, y| (i % 3 != 0).then_some(y * 0.5));
                v["options"] = json!({"area": true});
                v
            },
            560.0,
            320.0,
        ),
        // one highlighted point: a dot over the muted line (Desktop 2.159.1231.0, 2026-11-25)
        (
            "hl_linearea_lone",
            "linearea",
            highlighted(&json!(rows.clone()), 2, &|i, y| (i == 4).then_some(y)),
            560.0,
            320.0,
        ),
        (
            "look_line_bold",
            "linearea",
            json!({"rows": rows.clone(), "options": {"stylePreset": "bold", "labelMode": "minmax"},
                   "theme": {"colors": ["#7030A0", "#00A3A3"]}}),
            560.0,
            320.0,
        ),
        (
            "look_line_minimal_single_color",
            "linearea",
            single(json!({"stylePreset": "minimal", "lineColor": "#e23d4f", "area": true})),
            480.0,
            300.0,
        ),
        (
            "empty_linearea",
            "linearea",
            json!({"rows": [["2024-01-01", "A", null, 3], [null, null, null, null], ["x", "y", "z", "w"], 5]}),
            300.0,
            200.0,
        ),
    ]
}

fn golden_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/report/chart_pack_golden.json")
}

/// The golden file's text: one primitive or hit per line, so a change shows as a short diff.
fn golden_text(got: &serde_json::Map<String, Value>) -> String {
    let list = |v: &Value| {
        let items: Vec<String> = v
            .as_array()
            .map(|l| l.iter().map(|x| format!("      {x}")).collect())
            .unwrap_or_default();
        if items.is_empty() {
            "[]".to_string()
        } else {
            format!("[\n{}\n    ]", items.join(",\n"))
        }
    };
    let cases: Vec<String> = got
        .iter()
        .map(|(k, v)| {
            format!(
                "  {}: {{\n    \"prims\": {},\n    \"hits\": {}\n  }}",
                json!(k),
                list(&v["prims"]),
                list(&v["hits"])
            )
        })
        .collect();
    format!("{{\n{}\n}}\n", cases.join(",\n"))
}

fn rust_results() -> serde_json::Map<String, Value> {
    let mut out = serde_json::Map::new();
    for (name, kind, input, w, h) in cases() {
        out.insert(name.into(), chart_pack::layout(kind, &input, w, h).json());
    }
    out
}

/// The same JSON, numbers within 0.011 (both round to hundredths).
fn same(a: &Value, b: &Value, at: &str) -> Result<(), String> {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            let (x, y) = (x.as_f64().unwrap_or(f64::NAN), y.as_f64().unwrap_or(f64::NAN));
            if (x - y).abs() <= 0.011 {
                Ok(())
            } else {
                Err(format!("{at}: {x} != {y}"))
            }
        }
        (Value::Array(x), Value::Array(y)) => {
            if x.len() != y.len() {
                return Err(format!("{at}: {} items != {}", x.len(), y.len()));
            }
            for (i, (p, q)) in x.iter().zip(y).enumerate() {
                same(p, q, &format!("{at}[{i}]"))?;
            }
            Ok(())
        }
        (Value::Object(x), Value::Object(y)) => {
            let mut kx: Vec<&String> = x.keys().collect();
            let mut ky: Vec<&String> = y.keys().collect();
            kx.sort();
            ky.sort();
            if kx != ky {
                return Err(format!("{at}: keys {kx:?} != {ky:?}"));
            }
            for k in kx {
                same(&x[k], &y[k], &format!("{at}.{k}"))?;
            }
            Ok(())
        }
        _ if a == b => Ok(()),
        _ => Err(format!("{at}: {a} != {b}")),
    }
}

#[test]
fn the_rust_layouts_match_the_golden_file() {
    let got = rust_results();
    let path = golden_path();
    if bicraft_env::var_os("BICRAFT_BLESS").is_some() {
        std::fs::write(&path, golden_text(&got)).unwrap();
    }
    let want: Value = serde_json::from_str(
        &std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{}: {e} (BICRAFT_BLESS=1 writes it)", path.display())),
    )
    .unwrap();
    same(&Value::Object(got.clone()), &want, "golden").unwrap();
    for (name, v) in &got {
        let n = v["prims"].as_array().unwrap().len();
        assert_eq!(
            n == 0,
            name.starts_with("empty") || name == "org_tiny",
            "{name}: {n} primitives"
        );
        // every hit names a drawn primitive
        for h in v["hits"].as_array().unwrap() {
            assert!(h["prim"].as_u64().unwrap() < n as u64, "{name}: {h}");
        }
    }
}

/// Every text the layouts write stays inside the visual (estimated width, as both drawers
/// estimate it).
#[test]
fn texts_stay_inside_the_visual() {
    let mut errors = Vec::new();
    for (name, kind, input, w, h) in cases() {
        for p in chart_pack::layout(kind, &input, w, h).prims {
            let p = p.json();
            if p["t"] != "text" {
                continue;
            }
            let x = p["x"].as_f64().unwrap();
            let y = p["y"].as_f64().unwrap();
            let size = p["size"].as_f64().unwrap();
            let wd = p["text"].as_str().unwrap().chars().count() as f64 * size * 0.55;
            let (l, r) = match p["anchor"].as_str().unwrap() {
                "start" => (x, x + wd),
                "end" => (x - wd, x),
                _ => (x - wd / 2.0, x + wd / 2.0),
            };
            if l < -0.01 || r > w + 0.01 || y - size / 2.0 < -0.01 || y + size / 2.0 > h + 0.01 {
                errors.push(format!("{name}: {p} outside {w}x{h}"));
            }
        }
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

/// The Rust layout of the golden case `name`.
fn case(name: &str) -> chart_pack::Out {
    let (_, kind, input, w, h) = cases().into_iter().find(|c| c.0 == name).unwrap();
    chart_pack::layout(kind, &input, w, h)
}

fn prims(name: &str) -> Vec<Value> {
    case(name).prims.iter().map(|p| p.json()).collect()
}

/// Scatter+'s R² text keeps clear of the point labels (G115).
#[test]
fn the_trend_text_keeps_clear_of_the_point_labels() {
    let boxed = |p: &Value| {
        let (x, y, size) = (
            p["x"].as_f64().unwrap(),
            p["y"].as_f64().unwrap(),
            p["size"].as_f64().unwrap(),
        );
        let wd = p["text"].as_str().unwrap().chars().count() as f64 * size * 0.55;
        let l = if p["anchor"] == "end" { x - wd } else { x };
        [l, y - size / 2.0, l + wd, y + size / 2.0]
    };
    let meets = |a: [f64; 4], b: [f64; 4]| a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3];
    let texts: Vec<Value> = prims("scatter_trend_labels")
        .into_iter()
        .filter(|p| p["t"] == "text")
        .collect();
    let r2 = texts
        .iter()
        .find(|p| p["text"].as_str().unwrap().starts_with("R²"))
        .unwrap();
    let names: Vec<&Value> = texts
        .iter()
        .filter(|p| ["Ann", "Bob", "Cy", "Dee", "Eve"].contains(&p["text"].as_str().unwrap()))
        .collect();
    assert_eq!(names.len(), 5, "{texts:?}");
    for n in names {
        assert!(!meets(boxed(n), boxed(r2)), "{n} meets {r2}");
    }
}

/// The Sankey's node colour options colour their nodes and the links coloured by them (G113):
/// Coal is the first node seen, Heating the fourth; a 13th colour is not read.
#[test]
fn sankey_node_colour_options_colour_nodes_and_their_links() {
    let out = case("sankey_node_colors");
    let fill = |prim: &Value| out.prims[prim.as_u64().unwrap() as usize].json()["fill"].clone();
    let node = |i: u64| out.hits.iter().find(|h| h["node"] == i).unwrap();
    assert_eq!(fill(&node(0)["prim"]), "#112233");
    assert_eq!(fill(&node(3)["prim"]), "#C00000");
    assert_ne!(fill(&node(1)["prim"]), "#112233");
    let from_coal: Vec<&Value> = out.hits.iter().filter(|h| h["source"] == "Coal").collect();
    assert_eq!(from_coal.len(), 2);
    assert!(from_coal.iter().all(|h| fill(&h["prim"]) == "#112233"));
}

/// Stacked Line / Area+ series stand on the ones before them, their areas down to the series
/// below, their labels with their own values (G116).
#[test]
fn stacked_series_stand_on_the_ones_before_them() {
    // stacked lines: in January B (5) on A (10) at 15, C (−3) below zero, so the markers' heights
    // are in the ratio of the values 10, 15 and −3
    let out = case("line_stacked_lines_fr");
    let y_of = |series: u64, point: u64| {
        let h = out
            .hits
            .iter()
            .find(|h| h["series"] == series && h["point"] == point)
            .unwrap();
        out.prims[h["prim"].as_u64().unwrap() as usize].json()["cy"]
            .as_f64()
            .unwrap()
    };
    let (a, b, c) = (y_of(0, 0), y_of(1, 0), y_of(2, 0));
    assert!(b < a && a < c, "{a} {b} {c}");
    assert!(((a - b) / (c - a) - 5.0 / 13.0).abs() < 0.01, "{a} {b} {c}");
    // the labels write each point's own value: B's 5, not its top 15
    let labels: Vec<String> = out
        .prims
        .iter()
        .map(|p| p.json())
        .filter(|p| p["t"] == "text" && p["weight"].as_f64().is_some_and(|w| w >= 600.0))
        .map(|p| p["text"].as_str().unwrap().to_string())
        .collect();
    assert!(
        labels.contains(&"5".to_string()) && !labels.contains(&"15".to_string()),
        "{labels:?}"
    );
    // stacked areas: South's area runs down along North's line
    let polys: Vec<Value> = prims("line_stacked_area")
        .into_iter()
        .filter(|p| p["t"] == "poly")
        .collect();
    assert_eq!(polys.len(), 2, "{polys:?}");
    let pts = |p: &Value| p["points"].as_array().unwrap().clone();
    let (north, south) = (pts(&polys[0]), pts(&polys[1]));
    let n = north.len() / 2;
    let mut under: Vec<Value> = south[n..].to_vec();
    under.reverse();
    assert_eq!(under, north[..n].to_vec());
}

/// The size of the narrow Sankey case (a visual as narrow as one reported drawn with a ribbon
/// over a node and cut labels).
const NARROW: (f64, f64) = (264.0, 360.0);

fn narrow_sankey() -> Value {
    json!({"rows": [
        ["Coal", "Power", 100],
        ["Coal", "Industry", 25],
        ["Gas", "Power", 50],
        ["Gas", "Heating", 30],
        ["Power", "Industry", 80],
        ["Power", "Homes", 70],
        ["Heating", "Homes", 30]
    ]})
}

/// What is wrong with a layout (`{prims, hits}`) of the narrow Sankey: a label cut, a node past
/// the right edge, a ribbon not starting on its source's right edge or not ending on its
/// destination's left edge, the link that skips a column not passing that column beside its
/// node.
fn narrow_sankey_errors(out: &Value, who: &str) -> Vec<String> {
    let mut errors = Vec::new();
    let prims = out["prims"].as_array().cloned().unwrap_or_default();
    let hits = out["hits"].as_array().cloned().unwrap_or_default();
    let f = |p: &Value, k: &str| p[k].as_f64().unwrap_or(f64::NAN);
    let node = |name: &str| -> Option<Value> {
        let h = hits.iter().find(|h| h["name"] == name)?;
        prims.get(h["prim"].as_u64()? as usize).cloned()
    };
    for t in ["Coal", "Gas", "Power", "Heating", "Industry", "Homes"] {
        if !prims.iter().any(|p| p["t"] == "text" && p["text"] == t) {
            errors.push(format!("{who}: the label `{t}` is not drawn whole"));
        }
        if let Some(r) = node(t)
            && f(&r, "x") + f(&r, "w") > NARROW.0 + 0.01
        {
            errors.push(format!("{who}: the node `{t}` passes the right edge: {r}"));
        }
    }
    let close = |a: f64, b: f64| (a - b).abs() < 0.011;
    for h in hits.iter().filter(|h| h.get("link").is_some()) {
        let (Some(s), Some(t)) = (
            node(h["source"].as_str().unwrap_or("")),
            node(h["target"].as_str().unwrap_or("")),
        ) else {
            errors.push(format!("{who}: no nodes for {h}"));
            continue;
        };
        let Some(p) = h["prim"].as_u64().and_then(|i| prims.get(i as usize)) else {
            errors.push(format!("{who}: no ribbon for {h}"));
            continue;
        };
        let pts: Vec<(f64, f64)> = p["points"]
            .as_array()
            .map(|l| {
                l.iter()
                    .map(|q| {
                        (
                            q[0].as_f64().unwrap_or(f64::NAN),
                            q[1].as_f64().unwrap_or(f64::NAN),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let x0 = pts.iter().map(|q| q.0).fold(f64::INFINITY, f64::min);
        let x1 = pts.iter().map(|q| q.0).fold(f64::NEG_INFINITY, f64::max);
        let (sx, tx) = (f(&s, "x") + f(&s, "w"), f(&t, "x"));
        if !close(x0, sx) || !close(x1, tx) {
            errors.push(format!(
                "{who}: the ribbon {h} runs from x {x0} to {x1}, its nodes' edges are {sx} and {tx}"
            ));
        }
        for (x, r) in [(x0, &s), (x1, &t)] {
            for q in pts.iter().filter(|q| close(q.0, x)) {
                if q.1 < f(r, "y") - 0.011 || q.1 > f(r, "y") + f(r, "h") + 0.011 {
                    errors.push(format!("{who}: the ribbon {h} meets {r} outside it at y {}", q.1));
                }
            }
        }
    }
    // Coal to Industry skips the middle column: it passes it above or below Power
    let link = hits
        .iter()
        .find(|h| h["source"] == "Coal" && h["target"] == "Industry" && h.get("link").is_some());
    match (
        link.and_then(|h| prims.get(h["prim"].as_u64()? as usize)),
        node("Power"),
    ) {
        (Some(p), Some(power)) => {
            let (top, bottom) = (f(&power, "y"), f(&power, "y") + f(&power, "h"));
            for x in [f(&power, "x"), f(&power, "x") + f(&power, "w")] {
                let ys: Vec<f64> = p["points"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|q| close(q[0].as_f64().unwrap_or(f64::NAN), x))
                    .filter_map(|q| q[1].as_f64())
                    .collect();
                let beside = ys.len() >= 2
                    && (ys.iter().all(|y| *y <= top + 0.011) || ys.iter().all(|y| *y >= bottom - 0.011));
                if !beside {
                    errors.push(format!(
                        "{who}: Coal to Industry crosses Power's column at x {x} through {ys:?}, Power spans {top}..{bottom}"
                    ));
                }
            }
        }
        _ => errors.push(format!("{who}: no Coal to Industry ribbon or no Power node")),
    }
    errors
}

/// The narrow Sankey (a regression): the Rust layout and, with Node, the TypeScript one.
#[test]
fn a_narrow_sankey_lines_up_and_keeps_its_labels_whole() {
    let mut errors = narrow_sankey_errors(
        &chart_pack::layout("sankey", &narrow_sankey(), NARROW.0, NARROW.1).json(),
        "rust",
    );
    if let Some(ts) = ts_results() {
        errors.extend(narrow_sankey_errors(&ts["sankey_narrow"], "typescript"));
    } else {
        eprintln!("note: node is not on the PATH; the TypeScript Sankey is not checked (development check)");
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

/// Every case laid out by the visuals' TypeScript under Node (`{name: {prims, hits}}`), `None`
/// without Node.
fn ts_results() -> Option<Value> {
    if !has_node() {
        return None;
    }
    let dir = TempDir::new("chart-pack-ts");
    let harness = format!(
        "var M = {}\n\
         var fs = require('fs');\n\
         var cases = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));\n\
         var out = {{}};\n\
         for (var i = 0; i < cases.length; i++) {{\n\
           var c = cases[i];\n\
           out[c.name] = M.layout(c.kind, c.input, c.w, c.h);\n\
         }}\n\
         fs.writeFileSync(process.argv[3], JSON.stringify(out));\n",
        layout_script().unwrap()
    );
    let script = dir.join("harness.js");
    std::fs::write(&script, harness).unwrap();
    let input: Vec<Value> = cases()
        .into_iter()
        .map(|(name, kind, input, w, h)| json!({"name": name, "kind": kind, "input": input, "w": w, "h": h}))
        .collect();
    let cases_path = dir.join("cases.json");
    std::fs::write(&cases_path, Value::Array(input).to_string()).unwrap();
    let out_path = dir.join("out.json");
    let o = std::process::Command::new("node")
        .arg(&script)
        .arg(&cases_path)
        .arg(&out_path)
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "node failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    Some(serde_json::from_str(&std::fs::read_to_string(&out_path).unwrap()).unwrap())
}

/// What is wrong with the ribbons of a Sankey layout (`{prims, hits}`): two ribbons sharing a
/// slot of a node's height (each one's span at the node's edge must be its own), or a ribbon
/// crossing a node that is not one of its ends (or a column it skips without a waypoint).
fn sankey_ribbon_errors(out: &Value, who: &str) -> Vec<String> {
    const EPS: f64 = 0.011;
    let mut errors = Vec::new();
    let prims = out["prims"].as_array().cloned().unwrap_or_default();
    let hits = out["hits"].as_array().cloned().unwrap_or_default();
    let num = |p: &Value, k: &str| p[k].as_f64().unwrap_or(f64::NAN);
    // each node: (name, x, y, w, h)
    let nodes: Vec<(String, f64, f64, f64, f64)> = hits
        .iter()
        .filter(|h| h.get("node").is_some())
        .filter_map(|h| {
            let r = prims.get(h["prim"].as_u64()? as usize)?;
            Some((
                h["name"].as_str()?.to_string(),
                num(r, "x"),
                num(r, "y"),
                num(r, "w"),
                num(r, "h"),
            ))
        })
        .collect();
    // each node's slots: (top, bottom, link)
    let mut slots: std::collections::HashMap<String, Vec<(f64, f64, String)>> = Default::default();
    // a link's highlighted part is a second ribbon inside its own (a later hit on the same link)
    let mut drawn = std::collections::HashSet::new();
    for h in hits.iter().filter(|h| h.get("link").is_some()) {
        let id = format!("{} -> {}", h["source"], h["target"]);
        if !drawn.insert(id.clone()) {
            continue;
        }
        let Some(pts) = h["prim"]
            .as_u64()
            .and_then(|i| prims.get(i as usize))
            .and_then(|p| p["points"].as_array())
        else {
            errors.push(format!("{who}: no ribbon for {id}"));
            continue;
        };
        let pts: Vec<(f64, f64)> = pts
            .iter()
            .map(|q| {
                (
                    q[0].as_f64().unwrap_or(f64::NAN),
                    q[1].as_f64().unwrap_or(f64::NAN),
                )
            })
            .collect();
        let at = |x: f64| -> Option<(f64, f64)> {
            let ys: Vec<f64> = pts
                .iter()
                .filter(|q| (q.0 - x).abs() < EPS)
                .map(|q| q.1)
                .collect();
            (!ys.is_empty()).then(|| {
                (
                    ys.iter().copied().fold(f64::INFINITY, f64::min),
                    ys.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                )
            })
        };
        let x0 = pts.iter().map(|q| q.0).fold(f64::INFINITY, f64::min);
        let x1 = pts.iter().map(|q| q.0).fold(f64::NEG_INFINITY, f64::max);
        for (name, nx, ny, nw, nh) in &nodes {
            let end = h["source"] == name.as_str() || h["target"] == name.as_str();
            if end {
                // its slot at this end: outgoing on the node's right edge, incoming on its left
                let (x, edge) = if h["source"] == name.as_str() {
                    (nx + nw, "right")
                } else {
                    (*nx, "left")
                };
                match at(x) {
                    Some((a, b)) => {
                        slots
                            .entry(format!("{name}'s {edge} edge"))
                            .or_default()
                            .push((a, b, id.clone()))
                    }
                    None => errors.push(format!("{who}: {id} does not reach {name}'s edge")),
                }
                continue;
            }
            // a node it is not an end of: the ribbon passes its column beside it, or not at all
            if *nx + nw <= x0 + EPS || *nx >= x1 - EPS {
                continue;
            }
            for x in [*nx, nx + nw] {
                match at(x) {
                    Some((a, b)) if b <= ny + EPS || a >= ny + nh - EPS => {}
                    Some((a, b)) => errors.push(format!(
                        "{who}: {id} crosses {name} ({a}..{b} over {ny}..{})",
                        ny + nh
                    )),
                    None => errors.push(format!("{who}: {id} passes {name}'s column without a waypoint")),
                }
            }
        }
    }
    for (name, list) in &slots {
        for (i, a) in list.iter().enumerate() {
            for b in list.iter().skip(i + 1) {
                if a.0 < b.1 - EPS && b.0 < a.1 - EPS {
                    errors.push(format!(
                        "{who}: at {name}, {} ({}..{}) and {} ({}..{}) share a slot",
                        a.2, a.0, a.1, b.2, b.0, b.1
                    ));
                }
            }
        }
    }
    errors
}

/// No two ribbons share a slot of a node, and no ribbon crosses a node it does not end at, in
/// every Sankey case, in the Rust layout and (with Node) the TypeScript one.
#[test]
fn sankey_ribbons_keep_their_own_slots_and_go_round_nodes() {
    let ts = ts_results();
    if ts.is_none() {
        eprintln!("note: node is not on the PATH; the TypeScript Sankey is not checked (development check)");
    }
    let mut errors = Vec::new();
    for (name, kind, input, w, h) in cases() {
        if kind != "sankey" {
            continue;
        }
        errors.extend(sankey_ribbon_errors(
            &chart_pack::layout(kind, &input, w, h).json(),
            &format!("rust {name}"),
        ));
        if let Some(ts) = &ts {
            errors.extend(sankey_ribbon_errors(&ts[name], &format!("typescript {name}")));
        }
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

/// The visuals' TypeScript, compiled by BIcraft's bundler and run by Node, lays out every case
/// as the renderer does.
#[test]
fn the_typescript_layouts_match_the_rust_ones() {
    let Some(ts) = ts_results() else {
        eprintln!(
            "note: node is not on the PATH; the chart pack's TypeScript layouts are not compared (development check)"
        );
        return;
    };
    let mut errors = Vec::new();
    for (name, want) in &rust_results() {
        if let Err(e) = same(&ts[name], want, name) {
            errors.push(e);
        }
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

// ---------------------------------------------------------------------------------------------
// The packages
// ---------------------------------------------------------------------------------------------

pub(crate) fn package_json(guid: &str, bytes: &[u8]) -> Value {
    let read = bicraft_pbix::pbiviz::read_pbiviz(bytes).unwrap();
    let name = format!("resources/{guid}.pbiviz.json");
    let (_, body) = read.files.iter().find(|(n, _)| *n == name).unwrap();
    serde_json::from_slice(body).unwrap()
}

#[test]
fn the_packages_capabilities_follow_the_api_schema() {
    let schema_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/report/powerbi-visuals-api/schema.capabilities.json");
    let schema: Value = serde_json::from_str(&std::fs::read_to_string(schema_path).unwrap()).unwrap();
    for d in VISUALS {
        let pkg = package_json(d.guid, &package_bytes(d).unwrap());
        let caps = &pkg["capabilities"];
        let issues = bicraft_pbir::JsonSchema::new(schema.clone()).validate(caps);
        assert!(issues.is_empty(), "{}: {issues:?}\n{caps}", d.kind);
        assert_eq!(caps["privileges"], json!([]), "{}", d.kind);
        assert_eq!(caps["supportsKeyboardFocus"], json!(true));
        assert_eq!(caps["supportsLandingPage"], json!(true));
        assert_eq!(pkg["visual"]["guid"], d.guid);
        assert_eq!(pkg["visual"]["version"], d.version);
        // every format property the layout reads is declared
        for o in d.objects {
            for p in o.props {
                assert!(
                    caps["objects"][o.name]["properties"][p.name].is_object(),
                    "{}: {}.{}",
                    d.kind,
                    o.name,
                    p.name
                );
            }
        }
        let js = pkg["content"]["js"].as_str().unwrap();
        assert!(js.starts_with(&format!("var {} =", d.guid)), "{}", d.kind);
    }
}

/// Each packaged visual, under Node with a stand-in host and document: the landing page without
/// fields, a message when a required role is missing, then its points drawn, tooltips, selection
/// (cross-filtering ids), the context menu, high contrast, the format pane, and for the
/// organization chart a node collapsed by its button and persisted.
#[test]
fn the_packaged_visuals_run_under_node() {
    if !has_node() {
        eprintln!("note: node is not on the PATH; the chart pack's visuals are not run (development check)");
        return;
    }
    let dir = TempDir::new("chart-pack-visuals");
    let mut script = String::from(DOM);
    for d in VISUALS {
        let pkg = package_json(d.guid, &package_bytes(d).unwrap());
        script.push_str(pkg["content"]["js"].as_str().unwrap());
        script.push_str(&format!("\nPLUGINS[{:?}] = {}.default;\n", d.kind, d.guid));
    }
    script.push_str(VISUAL_CASES);
    let path = dir.join("visuals.js");
    std::fs::write(&path, script).unwrap();
    let out_path = dir.join("out.json");
    let o = std::process::Command::new("node")
        .arg(&path)
        .arg(&out_path)
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "node failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    let got: Value = serde_json::from_str(&std::fs::read_to_string(&out_path).unwrap()).unwrap();
    let want = json!({
        "org": {
            "landing": "BIcraft Org Chart|Add fields to Id and Parent id to draw this visual.",
            "missing": "Ajoutez des champs dans Id du parent pour dessiner ce visuel.",
            "rects": 5, "targets": 4, "role": "listbox", "label": "Label: B, Id: b, Parent: a, Value: 2",
            "tip": ["show", 4], "selected": ["b|a|B"], "dimmed": 3, "menu": "b|a|B",
            "toggle": ["button", "true", "Collapse A"], "persisted": "[\"a\"]", "after_toggle": 2,
            "contrast": "#FFFF00", "cards": ["nodes", "tree", "style"]
        },
        "venn": {
            "landing": "BIcraft Venn|Add fields to Item and Set to draw this visual.",
            "missing": "Ajoutez des champs dans Ensemble pour dessiner ce visuel.",
            "rects": 3, "targets": 5, "role": "listbox", "label": "Set: Q, Items: 2",
            "tip": ["show", 2], "selected": ["2|Q", "3|Q"], "dimmed": 4, "menu": "2|Q",
            "region": "Region: P ∩ Q, Items: 1",
            "contrast": "#FFFF00", "cards": ["sets", "labels", "style"]
        },
        "mekko": {
            "landing": "BIcraft Marimekko|Add fields to Category and Value to draw this visual.",
            "missing": "Ajoutez des champs dans Valeur pour dessiner ce visuel.",
            "rects": 6, "targets": 3, "role": "listbox", "label": "Region: N, Kind: y, Amount: 3.0, Share of the category: 75.0%",
            "tip": ["show", 4], "selected": ["N|y"], "dimmed": 2, "menu": "N|y",
            "contrast": "#FFFF00", "cards": ["colors", "labels", "legend", "columns", "style"]
        },
        "calendar": {
            "landing": "BIcraft Calendar Heatmap|Add fields to Date and Value to draw this visual.",
            "missing": "Ajoutez des champs dans Valeur pour dessiner ce visuel.",
            "rects": 372, "targets": 2, "role": "listbox", "label": "Day: 2024-01-03, Count: 9",
            "tip": ["show", 2], "selected": ["2024-01-03"], "dimmed": 1, "menu": "2024-01-03",
            "contrast": "#FFFF00", "cards": ["colors", "legend", "range", "style"]
        },
        "sankey": {
            "landing": "BIcraft Sankey|Add fields to Source, Destination and Weight to draw this visual.",
            "missing": "Ajoutez des champs dans Destination et Poids pour dessiner ce visuel.",
            "rects": 4, "targets": 6, "role": "listbox", "label": "Node: b, Incoming: 1, Outgoing: 3",
            "tip": ["show", 3], "selected": ["a|b", "b|c"], "dimmed": 3, "menu": "a|b",
            "link": ["From: a, To: b, Flow: 1", 34], "link_selected": ["a|b"],
            "contrast": "#FFFF00", "cards": ["nodes", "colors", "links", "labels", "style"]
        },
        "scatter": {
            "landing": "BIcraft Scatter+|Add fields to Details, X axis and Y axis to draw this visual.",
            "missing": "Ajoutez des champs dans Axe X et Axe Y pour dessiner ce visuel.",
            "targets": 3, "role": "listbox", "label": "Name: q, Seg: s1, Price: 2, Margin: 1.0",
            "tip": ["show", 4], "selected": ["q|s1"], "dimmed": 2, "menu": "q|s1",
            // labels on the selected points: none before, the clicked point's after
            "labels": [0, ["q"]], "cells": 3,
            "contrast": "#FFFF00",
            "cards": ["markers", "density", "marginals", "trend", "quadrants", "labels", "legend", "style"]
        },
        "linearea": {
            "landing": "BIcraft Line / Area+|Add fields to Axis and Value to draw this visual.",
            "missing": "Ajoutez des champs dans Valeur pour dessiner ce visuel.",
            "targets": 3, "role": "listbox", "label": "Day: 2024-01-02, Sales: 7",
            "tip": ["show", 2], "selected": ["2024-01-02"], "dimmed": 2, "menu": "2024-01-02",
            "lines": 1, "contrast": "#FFFF00",
            "cards": ["lines", "area", "band", "threshold", "rule", "forecast", "labels", "axes", "legend", "style"]
        }
    });
    let mut errors = Vec::new();
    for k in [
        "org", "venn", "mekko", "calendar", "sankey", "scatter", "linearea",
    ] {
        for (key, v) in want[k].as_object().unwrap() {
            if &got[k][key] != v {
                errors.push(format!("{k}.{key}: got {}, want {v}", got[k][key]));
            }
        }
    }
    // the labels take the look's text size when the format pane sets none (Modern: names 12 px,
    // values 11 px), as `report.render` draws them; Power BI Desktop 2.159.1231.0 drew the
    // Sankey's labels at 11 px, the visuals passing the format pane's earlier default (2026-11-24)
    for (k, size) in [("org", 12), ("venn", 13), ("mekko", 11), ("sankey", 12)] {
        let sizes = got[k]["sizes"].as_array().cloned().unwrap_or_default();
        let has = |x: &Value| {
            x.as_str()
                .is_some_and(|x| x.trim_end_matches("px") == size.to_string())
        };
        if !sizes.iter().any(has) {
            errors.push(format!("{k}: no {size} px text: {sizes:?}"));
        }
        if got[k]["pane_size"] != json!(size) {
            errors.push(format!("{k}: the pane shows {} for {size}", got[k]["pane_size"]));
        }
    }
    assert!(errors.is_empty(), "{}\nall: {got}", errors.join("\n"));
}

/// A stand-in `document`, elements and host, enough for the visuals' update and interactions.
pub(crate) const DOM: &str = r##"
var PLUGINS = {};
function El(tag) {
  this.tagName = tag; this.attrs = {}; this.children = []; this.parentNode = null; this.listeners = {}; this.textContent = "";
  var self = this;
  this.style = { setProperty: function () {} };
  this.classList = { toggle: function () {}, add: function () {} };
}
El.prototype.setAttribute = function (k, v) { this.attrs[k] = String(v); };
El.prototype.getAttribute = function (k) { return k in this.attrs ? this.attrs[k] : null; };
El.prototype.removeAttribute = function (k) { delete this.attrs[k]; };
El.prototype.appendChild = function (c) { c.parentNode = this; this.children.push(c); return c; };
El.prototype.removeChild = function (c) { var i = this.children.indexOf(c); if (i >= 0) { this.children.splice(i, 1); } c.parentNode = null; return c; };
Object.defineProperty(El.prototype, "firstChild", { get: function () { return this.children.length ? this.children[0] : null; } });
El.prototype.addEventListener = function (t, f) { (this.listeners[t] = this.listeners[t] || []).push(f); };
El.prototype.closest = function (sel) { var e = this; while (e) { if (e.attrs && "data-row" in e.attrs) { return e; } e = e.parentNode; } return null; };
El.prototype.getBoundingClientRect = function () { return { left: 1, top: 2, width: 3, height: 4 }; };
El.prototype.focus = function () {};
El.prototype.all = function (tag, out) { out = out || []; for (var i = 0; i < this.children.length; i++) { var c = this.children[i]; if (c.tagName === tag) { out.push(c); } c.all(tag, out); } return out; };
var document = { createElement: function (t) { return new El(t); }, createElementNS: function (ns, t) { return new El(t); } };
var window = {};
function fire(target, type, extra) {
  var e = { type: type, target: target, clientX: 5, clientY: 6, ctrlKey: false, metaKey: false, shiftKey: false, stopped: false,
    stopPropagation: function () { e.stopped = true; }, preventDefault: function () {} };
  for (var k in extra || {}) { e[k] = extra[k]; }
  var n = target;
  while (n && !e.stopped) { (n.listeners[type] || []).forEach(function (f) { f(e); }); n = n.parentNode; }
  return e;
}
function fakeId(k) { return { k: k, equals: function (o) { return !!o && o.k === k; } }; }
function makeHost(locale, contrast) {
  var h = { locale: locale, log: [], persisted: null, sel: [] };
  // as Power BI Desktop 2.159.1231.0 does (seen 2026-11-24), the identity of row i is the row's:
  // every category column's value at i (`h.cats`, set by the case), whichever column is named
  function text(x) { return x instanceof Date ? x.getFullYear() + "-" + String(x.getMonth() + 1).padStart(2, "0") + "-" + String(x.getDate()).padStart(2, "0") : String(x); }
  h.createSelectionIdBuilder = function () {
    var parts = [];
    var b = { withCategory: function (c, i) { parts = (h.cats || [c]).map(function (k) { return text(k.values[i]); }); return b; }, createSelectionId: function () { return fakeId(parts.join("|")); } };
    return b;
  };
  h.createSelectionManager = function () {
    return { select: function (id, multi) { var ids = Array.isArray(id) ? id : [id]; h.sel = multi ? h.sel.concat(ids) : ids; return { then: function (f) { f(h.sel.slice()); } }; },
      clear: function () { h.sel = []; return { then: function () {} }; }, getSelectionIds: function () { return h.sel.slice(); },
      registerOnSelectCallback: function () {}, showContextMenu: function (id) { h.log.push(["menu", id.k]); } };
  };
  h.tooltipService = { enabled: function () { return true; }, show: function (o) { h.log.push(["show", o.dataItems.length]); }, move: function () {}, hide: function () {} };
  h.persistProperties = function (c) { h.persisted = c.merge[0].properties.toggled; };
  if (contrast) { h.colorPalette = { isHighContrast: true, foreground: { value: "#FFFF00" }, background: { value: "#000000" }, foregroundSelected: { value: "#00FFFF" } }; }
  return h;
}
function col(name, role, values, format) { var r = {}; r[role] = true; return { source: { displayName: name, roles: r, format: format || "" }, values: values }; }
function dataView(cats, vals, objects) {
  var cols = cats.concat(vals).map(function (c) { return c.source; });
  return { metadata: { columns: cols, objects: objects || {} }, categorical: { categories: cats, values: vals } };
}
"##;

/// One run per visual; the results are compared in Rust.
const VISUAL_CASES: &str = r##"
var CASES = {
  org: { cats: [col("Id", "id", ["a", "b", "c", "d"]), col("Parent", "parent", [null, "a", "a", "b"]), col("Label", "label", ["A", "B", "C", "D"])],
    vals: [col("Value", "value", [1, 2, 3, 4], "0")], missing: [col("Id", "id", ["a"])], target: 1 },
  venn: { cats: [col("Item", "item", ["1", "2", "2", "3"]), col("Set", "set", ["P", "P", "Q", "Q"])], vals: [],
    missing: [col("Item", "item", ["1"])], target: 1 },
  mekko: { cats: [col("Region", "category", ["N", "N", "S"]), col("Kind", "series", ["x", "y", "x"])], vals: [col("Amount", "value", [1, 3, 2], "0.0")],
    missing: [col("Region", "category", ["N"])], target: 1 },
  calendar: { cats: [col("Day", "date", [new Date(2024, 0, 2), new Date(2024, 0, 3)], "yyyy-MM-dd")], vals: [col("Count", "value", [7, 9], "0")],
    missing: [col("Day", "date", [new Date(2024, 0, 2)])], target: 1 },
  sankey: { cats: [col("From", "source", ["a", "a", "b"]), col("To", "destination", ["b", "c", "c"])], vals: [col("Flow", "weight", [1, 2, 3], "0")],
    missing: [col("From", "source", ["a"])], target: 1 },
  scatter: { cats: [col("Name", "details", ["p", "q", "r"]), col("Seg", "legend", ["s1", "s1", "s2"])],
    vals: [col("Price", "x", [1, 2, 3], "0"), col("Margin", "y", [3, 1, 2], "0.0")], missing: [col("Name", "details", ["p"])], target: 1 },
  linearea: { cats: [col("Day", "axis", [new Date(2024, 0, 1), new Date(2024, 0, 2), new Date(2024, 0, 3)], "yyyy-MM-dd")],
    vals: [col("Sales", "value", [5, 7, 6], "0")], missing: [col("Day", "axis", [new Date(2024, 0, 1)])], target: 1 }
};
var out = {};
Object.keys(CASES).forEach(function (kind) {
  var c = CASES[kind]; var r = {};
  var element = new El("div"); var host = makeHost("en-US");
  var v = PLUGINS[kind].create({ element: element, host: host });
  v.update({ dataViews: [{ metadata: { columns: [] } }], viewport: { width: 400, height: 300 } });
  var landing = element.all("p");
  r.landing = landing.map(function (p) { return p.textContent; }).join("|");
  var frEl = new El("div"); var fr = PLUGINS[kind].create({ element: frEl, host: makeHost("fr-FR") });
  fr.update({ dataViews: [dataView(c.missing, [])], viewport: { width: 400, height: 300 } });
  r.missing = frEl.all("p").map(function (p) { return p.textContent; }).join("|");
  host.cats = c.cats;
  v.update({ dataViews: [dataView(c.cats, c.vals)], viewport: { width: 600, height: 300 } });
  var svg = element.all("svg")[0];
  var points = svg.all("rect").concat(svg.all("circle"), svg.all("text"), svg.all("polygon")).filter(function (e) { return e.attrs.tabindex === "0" && e.attrs.role !== "button"; });
  r.rects = svg.all("rect").length;
  r.targets = points.length;
  r.role = svg.attrs.role;
  var t = points[c.target];
  r.label = t.attrs["aria-label"];
  fire(t, "mousemove");
  r.tip = host.log[host.log.length - 1];
  fire(t, "click");
  r.selected = host.sel.map(function (s) { return s.k; });
  r.dimmed = points.filter(function (p) { return p.attrs.opacity === "0.4"; }).length;
  fire(t, "contextmenu");
  r.menu = host.log[host.log.length - 1][1];
  if (kind === "venn") {
    var region = points.filter(function (p) { return p.tagName === "text"; })[2];
    r.region = region ? region.attrs["aria-label"] : null;
  }
  if (kind === "sankey") {
    var link = points.filter(function (p) { return p.tagName === "polygon"; })[0];
    r.link = [link.attrs["aria-label"], link.attrs.points.split(" ").length];
    fire(link, "click");
    r.link_selected = host.sel.map(function (s) { return s.k; });
  }
  if (kind === "scatter") {
    // labels on the selected points: a click redraws the visual with the point's label
    var lEl = new El("div"); var lh = makeHost("en-US"); lh.cats = c.cats;
    var lv = PLUGINS[kind].create({ element: lEl, host: lh });
    lv.update({ dataViews: [dataView(c.cats, c.vals, { labels: { mode: "selected" } })], viewport: { width: 600, height: 300 } });
    var names = function () { return lEl.all("svg")[0].all("text").map(function (t) { return t.textContent; }).filter(function (s) { return s === "p" || s === "q" || s === "r"; }); };
    var before = names();
    var dots = lEl.all("svg")[0].all("circle").filter(function (e) { return e.attrs.tabindex === "0"; });
    fire(dots[1], "click");
    r.labels = [before.length, names()];
    var dEl = new El("div"); var dv2 = PLUGINS[kind].create({ element: dEl, host: makeHost("en-US") });
    dv2.update({ dataViews: [dataView(c.cats, c.vals, { density: { mode: "on" } })], viewport: { width: 600, height: 300 } });
    r.cells = dEl.all("svg")[0].all("polygon").length;
  }
  if (kind === "linearea") {
    r.lines = svg.all("polyline").length;
  }
  if (kind === "org") {
    var btn = svg.all("circle").filter(function (e) { return e.attrs.role === "button"; })[0];
    r.toggle = [btn.attrs.role, btn.attrs["aria-expanded"], btn.attrs["aria-label"]];
    fire(btn, "click");
    r.persisted = host.persisted;
    r.after_toggle = element.all("svg")[0].all("rect").length;
  }
  var hcEl = new El("div"); var hc = PLUGINS[kind].create({ element: hcEl, host: makeHost("en-US", true) });
  hc.update({ dataViews: [dataView(c.cats, c.vals)], viewport: { width: 600, height: 300 } });
  var anyText = hcEl.all("svg")[0].all("text")[0];
  r.contrast = anyText ? anyText.attrs.fill : null;
  r.cards = v.getFormattingModel().cards.map(function (k) { return k.uid; });
  r.sizes = svg.all("text").map(function (e) { return e.attrs["font-size"]; }).filter(function (x, i, a) { return x && a.indexOf(x) === i; }).sort();
  var fm = JSON.stringify(v.getFormattingModel());
  var m = fm.match(/"propertyName":"fontSize"\}[^{}]*?"value":([0-9.]+)/) || fm.match(/"fontSize"[^{}]*?"value":([0-9.]+)/);
  r.pane_size = m ? +m[1] : null;
  out[kind] = r;
});
require("fs").writeFileSync(process.argv[2], JSON.stringify(out));
"##;

// ---------------------------------------------------------------------------------------------
// In a report
// ---------------------------------------------------------------------------------------------

fn session() -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Pack"}));
    run(
        &mut s,
        "table.import",
        json!({"name": "Org", "m": "let Source = #table(type table [Id = text, Parent = text, Name = text, Head = Int64.Type], {{\"a\", null, \"Alice\", 3}, {\"b\", \"a\", \"Bob\", 2}, {\"c\", \"a\", \"Cleo\", 1}}) in Source"}),
    );
    run(
        &mut s,
        "table.import",
        json!({"name": "M", "m": "let Source = #table(type table [Item = text, Grp = text, Amount = Int64.Type, Day = date], {{\"i1\", \"P\", 5, #date(2024, 1, 2)}, {\"i2\", \"P\", 7, #date(2024, 1, 3)}, {\"i2\", \"Q\", 3, #date(2024, 1, 3)}, {\"i3\", \"Q\", 4, #date(2024, 2, 9)}}) in Source"}),
    );
    run(
        &mut s,
        "measure.add",
        json!({"table": "M", "name": "Total", "expression": "SUM(M[Amount])"}),
    );
    run(
        &mut s,
        "measure.add",
        json!({"table": "Org", "name": "Heads", "expression": "SUM(Org[Head])"}),
    );
    run(&mut s, "report.new", json!({}));
    s
}

fn page(s: &Session) -> String {
    s.report.as_ref().unwrap().pages[0].name.clone()
}

fn add(s: &mut Session, kind: &str, y: f64, fields: Value) -> String {
    let d = find(kind).unwrap();
    assert!(install(s, kind).unwrap(), "{kind} embedded");
    assert!(!install(s, kind).unwrap(), "{kind} already embedded");
    let p = page(s);
    run(
        s,
        "visual.add",
        json!({"page": p, "visual_type": d.guid, "x": 10, "y": y, "width": 600, "height": 170, "fields": fields}),
    )["visual"]
        .as_str()
        .unwrap()
        .to_string()
}

fn add_all(s: &mut Session) -> Vec<(String, &'static str)> {
    vec![
        (
            add(
                s,
                "org",
                0.0,
                json!([{"role": "id", "field": "Org[Id]"}, {"role": "parent", "field": "Org[Parent]"}, {"role": "label", "field": "Org[Name]"}, {"role": "value", "field": "[Heads]"}]),
            ),
            "org",
        ),
        (
            add(
                s,
                "venn",
                180.0,
                json!([{"role": "item", "field": "M[Item]"}, {"role": "set", "field": "M[Grp]"}]),
            ),
            "venn",
        ),
        (
            add(
                s,
                "mekko",
                360.0,
                json!([{"role": "category", "field": "M[Grp]"}, {"role": "series", "field": "M[Item]"}, {"role": "value", "field": "[Total]"}]),
            ),
            "mekko",
        ),
        (
            add(
                s,
                "calendar",
                540.0,
                json!([{"role": "date", "field": "M[Day]"}, {"role": "value", "field": "[Total]"}]),
            ),
            "calendar",
        ),
        (
            add(
                s,
                "sankey",
                545.0,
                json!([{"role": "source", "field": "M[Grp]"}, {"role": "destination", "field": "M[Item]"}, {"role": "weight", "field": "[Total]"}]),
            ),
            "sankey",
        ),
        (
            add(
                s,
                "scatter",
                90.0,
                json!([{"role": "details", "field": "M[Item]"}, {"role": "legend", "field": "M[Grp]"}, {"role": "x", "field": "[Total]"}, {"role": "y", "field": "[Total]"}]),
            ),
            "scatter",
        ),
        (
            add(
                s,
                "linearea",
                270.0,
                json!([{"role": "axis", "field": "M[Day]"}, {"role": "series", "field": "M[Grp]"}, {"role": "value", "field": "[Total]"}]),
            ),
            "linearea",
        ),
    ]
}

fn snap_visual(s: &Session, name: &str) -> Value {
    s.snapshot()["report"]["pages"][0]["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == name)
        .cloned()
        .unwrap()
}

fn rendered(s: &mut Session) -> Value {
    let p = page(s);
    run(s, "report.render", json!({"page": p, "system_fonts": false}))
}

#[test]
fn the_visuals_install_and_the_renderer_draws_them_with_their_data() {
    let mut s = session();
    let added = add_all(&mut s);
    let listed = run(&mut s, "report.custom_visuals", json!({}));
    for d in VISUALS {
        assert!(
            listed.to_string().contains(d.guid),
            "{} not listed: {listed}",
            d.kind
        );
    }
    for (name, kind) in &added {
        let sv = snap_visual(&s, name);
        assert_eq!(sv["chart_pack"]["kind"], *kind, "{sv}");
        // the input the renderer lays out holds the data rows, cells in the roles' order
        assert_eq!(sv["data"]["status"], "ok", "{kind}: {}", sv["data"]);
    }
    let r = rendered(&mut s);
    for (name, kind) in &added {
        let v = r["pages"][0]["visuals"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["name"] == *name)
            .unwrap();
        assert_eq!(v["status"], "drawn", "{kind}: {v}");
    }
    // the renderer's input for each visual, from the snapshot (what the visual reads in Power BI)
    let org = snap_visual(&s, &added[0].0);
    let venn = snap_visual(&s, &added[1].0);
    let cal = snap_visual(&s, &added[3].0);
    let rows = |v: &Value| v["data"]["rows"].as_array().unwrap().len();
    assert_eq!(rows(&org), 3);
    assert_eq!(rows(&venn), 4);
    assert_eq!(rows(&cal), 3);
    let scatter = snap_visual(&s, &added[5].0);
    let line = snap_visual(&s, &added[6].0);
    assert_eq!(rows(&scatter), 4, "one point per item and segment");
    assert_eq!(rows(&line), 4);
    // Line / Area+ gets the preview's today (midnight UTC, epoch ms) for a forecast from today
    let today = line["chart_pack"]["today"].as_f64().unwrap();
    assert_eq!(today % 86_400_000.0, 0.0, "{today}");
    assert!(today > 1.7e12, "{today}");
    assert!(scatter["chart_pack"].get("today").is_none());
    let dir = TempDir::new("chart-pack-pdf");
    let pdf = dir.join("r.pdf");
    let r = run(
        &mut s,
        "report.export_pdf",
        json!({"out": pdf, "system_fonts": false}),
    );
    for (name, kind) in &added {
        let v = r["pages"][0]["visuals"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["name"] == *name)
            .unwrap();
        assert_eq!(v["status"], "drawn", "pdf {kind}: {v}");
    }
    assert!(std::fs::metadata(&pdf).unwrap().len() > 1000);
}

#[test]
fn format_options_reach_the_layout_and_survive_round_trips() {
    let mut s = session();
    let added = add_all(&mut s);
    let p = page(&s);
    let org = added[0].0.clone();
    run(
        &mut s,
        "visual.format",
        json!({"page": p, "visual": org, "objects": [
            {"object": "nodes", "property": "fill", "value": {"color": "#5A9E3A"}},
            {"object": "nodes", "property": "showValues", "value": false},
            {"object": "tree", "property": "levels", "value": 2},
            {"object": "tree", "property": "toggled", "value": "[\"b\",\"c\"]"},
            {"object": "tree", "property": "leftToRight", "value": true}
        ]}),
    );
    let want = json!({"color": "#5A9E3A", "showValues": false, "levels": 2.0, "toggled": ["b", "c"], "leftToRight": true});
    assert_eq!(snap_visual(&s, &org)["chart_pack"]["options"], want);
    let cal = added[3].0.clone();
    run(
        &mut s,
        "visual.format",
        json!({"page": p, "visual": cal, "objects": [{"object": "legend", "property": "show", "value": false}]}),
    );
    assert_eq!(
        snap_visual(&s, &cal)["chart_pack"]["options"],
        json!({"showLegend": false})
    );
    let mekko = added[2].0.clone();
    run(
        &mut s,
        "visual.format",
        json!({"page": p, "visual": mekko, "objects": [{"object": "colors", "property": "series2", "value": {"color": "#AA5500"}}]}),
    );
    assert_eq!(
        snap_visual(&s, &mekko)["chart_pack"]["options"],
        json!({"color2": "#AA5500"})
    );
    let sankey = added[4].0.clone();
    run(
        &mut s,
        "visual.format",
        json!({"page": p, "visual": sankey, "objects": [
            {"object": "links", "property": "colorBy", "value": "destination"},
            {"object": "links", "property": "opacity", "value": 70},
            {"object": "nodes", "property": "width", "value": 18}
        ]}),
    );
    let sankey_want = json!({"linkColor": "destination", "linkOpacity": 70.0, "nodeWidth": 18.0});
    assert_eq!(snap_visual(&s, &sankey)["chart_pack"]["options"], sankey_want);
    // a value the drop-down does not list is not an option
    run(
        &mut s,
        "visual.format",
        json!({"page": p, "visual": sankey, "objects": [{"object": "links", "property": "colorBy", "value": "rainbow"}]}),
    );
    assert_eq!(
        snap_visual(&s, &sankey)["chart_pack"]["options"],
        json!({"linkOpacity": 70.0, "nodeWidth": 18.0})
    );
    run(
        &mut s,
        "visual.format",
        json!({"page": p, "visual": sankey, "objects": [{"object": "links", "property": "colorBy", "value": "destination"}]}),
    );
    // PBIP and PBIX keep the packages, the types and the options
    let dir = TempDir::new("chart-pack-rt");
    for file in ["pack.pbip", "pack.pbix"] {
        let path = dir.join(file);
        run(&mut s, "file.save_as", json!({"path": path}));
        let mut t = Session::new();
        run(&mut t, "file.open", json!({"path": path}));
        assert_eq!(snap_visual(&t, &org)["chart_pack"]["options"], want, "{file}");
        assert_eq!(
            snap_visual(&t, &sankey)["chart_pack"]["options"],
            sankey_want,
            "{file}"
        );
        for (_, kind) in &added {
            assert!(!install(&mut t, kind).unwrap(), "{file}: {kind} was kept");
        }
        let r = rendered(&mut t);
        for (name, kind) in &added {
            let v = r["pages"][0]["visuals"]
                .as_array()
                .unwrap()
                .iter()
                .find(|v| v["name"] == *name)
                .unwrap();
            assert_eq!(v["status"], "drawn", "{file} {kind}: {v}");
        }
    }
}

#[test]
fn install_refuses_unknown_names_and_reports_without_a_report() {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "X"}));
    assert!(install(&mut s, "org").is_err());
    run(&mut s, "report.new", json!({}));
    let e = install(&mut s, "chord").unwrap_err();
    assert!(e.contains("org, venn, mekko, calendar, sankey"), "{e}");
    assert!(s.report.as_ref().unwrap().resources.is_empty());
}
