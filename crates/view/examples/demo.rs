//! Manual check of the live viewer: publishes a synthetic, evolving session snapshot
//! (docs/view-snapshot.md) every second, including the M8.2 additions (theme, page background,
//! visual format, evaluated data) so the rendering can be checked without the engine.
//!
//! ```text
//! cargo run -p bicraft-view --example demo -- [--port 8765] [--big]
//! ```
//!
//! `--big` adds a stress page (800 visuals with data, 10% of them changing every second) and
//! 140 extra tables to exercise the drawing caps.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

const CYCLE: u64 = 30;

fn main() {
    let mut port = 0u16;
    let mut big = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" => port = args.next().and_then(|p| p.parse().ok()).unwrap_or(0),
            "--big" => big = true,
            other => eprintln!("ignoring unknown argument {other:?}"),
        }
    }
    let view = match bicraft_view::start(([127, 0, 0, 1], port).into()) {
        Ok(view) => view,
        Err(e) => {
            eprintln!("cannot start the viewer: {e}");
            std::process::exit(1);
        }
    };
    println!("bicraft view demo at {}  (Ctrl+C to stop)", view.url());
    let mut step = 0u64;
    loop {
        view.publish(snapshot(step, big));
        step += 1;
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn vname(i: u64) -> String {
    format!("{:020x}", i.wrapping_mul(0x9E37_79B9_7F4A_7C15))
}

fn field(role: &str, kind: &str, table: &str, name: &str, function: Option<&str>) -> Value {
    let text = match (kind, function) {
        ("measure", _) => format!("[{name}]"),
        (_, Some(f)) => format!("{f}({table}[{name}])"),
        _ => format!("{table}[{name}]"),
    };
    json!({ "role": role, "kind": kind, "table": table, "name": name, "function": function, "text": text })
}

#[allow(clippy::too_many_arguments)]
fn visual(
    id: u64,
    ty: Option<&str>,
    (x, y, w, h): (i64, i64, i64, i64),
    z: i64,
    title: Option<&str>,
    fields: Vec<Value>,
    issues: Vec<Value>,
) -> Value {
    json!({
        "name": vname(id), "type": ty, "x": x, "y": y, "z": z, "width": w, "height": h, "tab_order": z,
        "title": title, "parent_group": null, "hidden": false, "fields": fields, "issues": issues,
    })
}

/// Sets the M8.2 `data` and `format` of a visual.
fn with(mut v: Value, data: Value, format: Value) -> Value {
    v["data"] = data;
    v["format"] = format;
    v
}

fn issue(severity: &str, message: &str) -> Value {
    json!({ "severity": severity, "message": message })
}

/// An evaluated result: `(name, role, kind, data_type)` per column.
fn data_ok(columns: &[(&str, &str, &str, &str)], rows: Vec<Value>) -> Value {
    let columns: Vec<Value> = columns
        .iter()
        .map(|(name, role, kind, ty)| json!({ "name": name, "role": role, "kind": kind, "data_type": ty }))
        .collect();
    json!({ "status": "ok", "reason": null, "columns": columns, "rows": rows, "truncated": false })
}

fn data_off(status: &str, reason: &str) -> Value {
    json!({ "status": status, "reason": reason, "columns": [], "rows": [], "truncated": false })
}

/// A white tile with rounded corners, the "modern" Desktop look.
fn tile() -> Value {
    json!({ "background": { "show": true, "color": "#FFFFFF", "transparency": 0 },
            "border": { "show": false, "color": "#E1DFDD", "radius": 8 } })
}

fn theme(custom: bool) -> Value {
    if custom {
        json!({ "name": "Forêt (custom)",
                "data_colors": ["#2D6A4F", "#E9C46A", "#E76F51", "#40916C", "#264653", "#F4A261", "#8AB17D", "#B56576"],
                "background": "#FFFFFF", "foreground": "#1D3557", "table_accent": "#2D6A4F",
                "font_family": "Segoe UI", "font_size": 10 })
    } else {
        json!({ "name": "CY24SU10",
                "data_colors": ["#118DFF", "#12239E", "#E66C37", "#6B007B", "#E044A7", "#744EC2",
                                "#D9B300", "#D64550", "#197278", "#1AAB40", "#15C6F4", "#4092FF"],
                "background": "#FFFFFF", "foreground": "#252423", "table_accent": "#118DFF",
                "font_family": "Segoe UI", "font_size": 9 })
    }
}

const CATS: [(&str, f64); 5] = [
    ("Vélos", 482_000.0),
    ("Composants", 241_900.0),
    ("Accessoires", 126_500.0),
    ("Vêtements", 88_200.0),
    ("Pièces", 37_400.0),
];
const PRODUCTS: [(&str, &str, f64); 8] = [
    ("Vélo route", "Vélos", 291_000.0),
    ("VTT", "Vélos", 191_000.0),
    ("Dérailleur", "Composants", 141_900.0),
    ("Freins", "Composants", 100_000.0),
    ("Casque", "Accessoires", 74_500.0),
    ("Gants", "Accessoires", 52_000.0),
    ("Maillot", "Vêtements", 88_200.0),
    ("Chaîne", "Pièces", 37_400.0),
];
const MONTHS: [&str; 12] = [
    "janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.", "déc.",
];
const CHANNELS: [(&str, f64); 3] = [("Web", 0.46), ("Magasin", 0.38), ("Revendeur", 0.16)];

fn round(x: f64) -> f64 {
    x.round()
}

fn monthly(i: usize, growth: f64) -> f64 {
    round((70_000.0 + 25_000.0 * (i as f64 * 0.6).sin() + 1_500.0 * i as f64) * growth)
}

fn snapshot(step: u64, big: bool) -> Value {
    let s = step % CYCLE;
    let at = |n: u64| s >= n;
    // Values move every 3 seconds, so most snapshots only change a few visuals.
    let tick = (s / 3) as f64;
    let growth = 1.0 + tick * 0.02;
    let custom_theme = (24..30).contains(&s);

    // ---- model
    let mut ventes_measures = vec![];
    if at(1) {
        ventes_measures.push(json!({ "name": "Total Ventes", "expression": "SUM ( Ventes[Montant] )", "format_string": "#,0 €", "hidden": false, "display_folder": null, "valid": true }));
        ventes_measures.push(json!({ "name": "Objectif", "expression": "1000000", "format_string": "#,0 €", "hidden": false, "display_folder": null, "valid": true }));
    }
    if at(3) {
        ventes_measures.push(json!({ "name": "Quantité", "expression": "SUM ( Ventes[Quantité] )", "format_string": "#,0", "hidden": false, "display_folder": null, "valid": true }));
    }
    if at(5) {
        ventes_measures.push(json!({ "name": "Marge %", "expression": "DIVIDE ( SUM ( Ventes[Marge] ), [Total Ventes] )", "format_string": "0.0 %", "hidden": false, "display_folder": "Ratios", "valid": true }));
    }
    let n1_invalid = (8..12).contains(&s);
    if at(8) {
        let expr = if n1_invalid {
            "CALCULATE ( [Total Ventes], SAMEPERIODLASTYEAR ( 'Date'[Jour] ) )"
        } else {
            "CALCULATE ( [Total Ventes], SAMEPERIODLASTYEAR ( 'Date'[Date] ) )"
        };
        ventes_measures.push(json!({ "name": "Ventes N-1", "expression": expr, "format_string": "#,0 €", "hidden": false, "display_folder": "Time intelligence", "valid": !n1_invalid }));
    }
    let col =
        |name: &str, ty: &str| json!({ "name": name, "data_type": ty, "calculated": false, "hidden": false });
    let mut tables = vec![
        json!({ "name": "Ventes", "hidden": false, "columns": [
            col("Date", "dateTime"), col("ProduitID", "int64"), col("ClientID", "int64"),
            col("Montant", "double"), col("Quantité", "int64"), col("Canal", "string"),
            { "name": "Marge", "data_type": "double", "calculated": true, "hidden": false, "expression": "Ventes[Montant] * 0.3" },
        ], "measures": ventes_measures }),
        json!({ "name": "Produit", "hidden": false, "columns": [
            { "name": "ProduitID", "data_type": "int64", "calculated": false, "hidden": true },
            col("Nom", "string"), col("Catégorie", "string"), col("Prix", "decimal"),
        ], "measures": [] }),
        json!({ "name": "Date", "hidden": false, "columns": [
            col("Date", "dateTime"), col("Année", "int64"), col("Mois", "string"), col("Week-end", "boolean"),
        ], "measures": [] }),
    ];
    let mut relationships = vec![
        json!({ "from_table": "Ventes", "from_column": "ProduitID", "to_table": "Produit", "to_column": "ProduitID", "active": true, "cross_filter": "single", "from_cardinality": "many", "to_cardinality": "one" }),
        json!({ "from_table": "Ventes", "from_column": "Date", "to_table": "Date", "to_column": "Date", "active": true, "cross_filter": "single", "from_cardinality": "many", "to_cardinality": "one" }),
    ];
    if at(6) {
        tables.push(json!({ "name": "Client", "hidden": false, "columns": [
            col("ClientID", "int64"), col("Nom", "string"), col("Pays", "string"),
        ], "measures": [] }));
    }
    if at(7) {
        relationships.push(json!({ "from_table": "Ventes", "from_column": "ClientID", "to_table": "Client", "to_column": "ClientID", "active": true, "cross_filter": "both", "from_cardinality": "many", "to_cardinality": "one" }));
    }
    if at(9) {
        tables.push(json!({ "name": "Région", "hidden": true, "columns": [ col("Pays", "string"), col("Zone", "string") ], "measures": [] }));
        relationships.push(json!({ "from_table": "Client", "from_column": "Pays", "to_table": "Région", "to_column": "Pays", "active": false, "cross_filter": "single", "from_cardinality": "many", "to_cardinality": "one" }));
    }
    if big {
        for i in 0..140u64 {
            let name = format!("Dim{i:03}");
            let columns: Vec<Value> = (0..30)
                .map(|c| col(&format!("Attr{c}"), if c % 3 == 0 { "int64" } else { "string" }))
                .collect();
            tables.push(json!({ "name": name, "hidden": false, "columns": columns, "measures": [] }));
            relationships.push(json!({ "from_table": "Ventes", "from_column": "ProduitID", "to_table": name, "to_column": "Attr0", "active": i % 4 != 0, "cross_filter": "single", "from_cardinality": "many", "to_cardinality": "one" }));
        }
    }

    // ---- data shared by several visuals
    let total: f64 = CATS.iter().map(|(_, v)| round(v * growth)).sum();
    let by_cat: Vec<Value> = CATS.iter().map(|(c, v)| json!([c, round(v * growth)])).collect();
    let months: Vec<(usize, &str)> = MONTHS.iter().copied().enumerate().collect();

    // ---- report
    let mut overview = vec![];
    if at(2) {
        overview.push(with(
            visual(
                1,
                Some("card"),
                (20, 20, 280, 120),
                0,
                Some("Total Ventes"),
                vec![field("Values", "measure", "Ventes", "Total Ventes", None)],
                vec![],
            ),
            data_ok(
                &[("Total Ventes", "Values", "measure", "double")],
                vec![json!([total])],
            ),
            json!({ "background": { "show": true, "color": "#FFFFFF", "transparency": 0 },
                    "border": { "show": true, "color": "#E1DFDD", "radius": 6 } }),
        ));
    }
    if at(3) {
        let data = if (12..18).contains(&s) {
            data_off(
                "unavailable",
                "the PBIP has no cache.abf: no table data to evaluate",
            )
        } else {
            data_ok(
                &[("Quantité", "Values", "measure", "int64")],
                vec![json!([12_840.0 + tick * 37.0])],
            )
        };
        overview.push(with(
            visual(
                2,
                Some("card"),
                (320, 20, 280, 120),
                1,
                Some("Quantité"),
                vec![field("Values", "measure", "Ventes", "Quantité", None)],
                vec![],
            ),
            data,
            json!({ "border": { "show": true, "color": "#E1DFDD", "radius": 6 } }),
        ));
    }
    if at(4) {
        let shift = if (24..29).contains(&s) {
            (s as i64 - 23) * 30
        } else {
            0
        };
        overview.push(with(
            visual(
                3,
                Some("clusteredColumnChart"),
                (20 + shift, 160, 580, 300),
                2,
                Some("Ventes par catégorie"),
                vec![
                    field("Category", "column", "Produit", "Catégorie", None),
                    field("Y", "measure", "Ventes", "Total Ventes", None),
                ],
                vec![],
            ),
            data_ok(
                &[
                    ("Catégorie", "Category", "column", "string"),
                    ("Total Ventes", "Y", "measure", "double"),
                ],
                by_cat.clone(),
            ),
            json!({ "title": { "show": true, "font_size": 14, "color": "#252423", "alignment": "left" },
                    "data_labels": true }),
        ));
    }
    if at(6) {
        let data = if n1_invalid {
            data_off("error", "Ventes N-1 does not validate")
        } else {
            data_ok(
                &[
                    ("Mois", "Category", "column", "string"),
                    ("Total Ventes", "Y", "measure", "double"),
                    ("Ventes N-1", "Y", "measure", "double"),
                ],
                months
                    .iter()
                    .map(|&(i, m)| json!([m, monthly(i, growth), round(monthly(i, 1.0) * 0.88)]))
                    .collect(),
            )
        };
        overview.push(with(
            visual(
                4,
                Some("lineChart"),
                (620, 160, 640, 300),
                3,
                Some("Évolution mensuelle"),
                vec![
                    field("Category", "column", "Date", "Mois", None),
                    field("Y", "measure", "Ventes", "Total Ventes", None),
                    field("Y", "measure", "Ventes", "Ventes N-1", None),
                ],
                if n1_invalid {
                    vec![issue("warning", "Ventes N-1 does not validate")]
                } else {
                    vec![]
                },
            ),
            data,
            json!({ "title": { "show": true, "font_size": 14 }, "legend": { "show": true, "position": "top" } }),
        ));
    }
    if at(7) {
        overview.push(with(
            visual(
                5,
                Some("slicer"),
                (620, 20, 300, 120),
                4,
                Some("Année"),
                vec![field("Values", "column", "Date", "Année", None)],
                vec![],
            ),
            data_ok(
                &[("Année", "Values", "column", "int64")],
                vec![json!([2022]), json!([2023]), json!([2024]), json!([2025])],
            ),
            Value::Null,
        ));
    }
    if at(9) {
        let issues = if s < 14 {
            vec![issue(
                "warning",
                "Table has more than 10 columns of the same field",
            )]
        } else {
            vec![]
        };
        overview.push(with(
            visual(
                6,
                Some("tableEx"),
                (20, 480, 580, 220),
                5,
                None,
                vec![
                    field("Values", "column", "Produit", "Nom", None),
                    field("Values", "aggregation", "Ventes", "Montant", Some("Sum")),
                    field("Values", "measure", "Ventes", "Marge %", None),
                ],
                issues,
            ),
            data_ok(
                &[
                    ("Nom", "Values", "column", "string"),
                    ("Montant", "Values", "aggregation", "double"),
                    ("Marge %", "Values", "measure", "double"),
                ],
                PRODUCTS
                    .iter()
                    .enumerate()
                    .map(|(i, (n, _, v))| json!([n, round(v * growth), 0.22 + i as f64 * 0.017]))
                    .collect(),
            ),
            Value::Null,
        ));
    }
    if at(11) {
        overview.push(visual(
            7,
            None,
            (940, 10, 330, 140),
            6,
            Some("KPIs"),
            vec![],
            vec![],
        ));
        let mut a = with(
            visual(
                8,
                Some("card"),
                (950, 20, 150, 120),
                7,
                Some("Marge %"),
                vec![field("Values", "measure", "Ventes", "Marge %", None)],
                vec![],
            ),
            data_ok(
                &[("Marge %", "Values", "measure", "double")],
                vec![json!([0.312])],
            ),
            Value::Null,
        );
        a["parent_group"] = json!(vname(7));
        let mut cumul = 0.0;
        let kpi_rows: Vec<Value> = months
            .iter()
            .map(|&(i, m)| {
                cumul += monthly(i, growth);
                json!([m, cumul, 1_000_000.0])
            })
            .collect();
        let mut b = with(
            visual(
                9,
                Some("kpi"),
                (1110, 20, 150, 120),
                8,
                Some("Objectif"),
                vec![
                    field("Indicator", "measure", "Ventes", "Total Ventes", None),
                    field("TrendLine", "column", "Date", "Mois", None),
                    field("Goal", "measure", "Ventes", "Objectif", None),
                ],
                vec![],
            ),
            data_ok(
                &[
                    ("Mois", "TrendLine", "column", "string"),
                    ("Total Ventes", "Indicator", "measure", "double"),
                    ("Objectif", "Goal", "measure", "double"),
                ],
                kpi_rows,
            ),
            Value::Null,
        );
        b["parent_group"] = json!(vname(7));
        overview.push(a);
        overview.push(b);
    }
    if at(13) {
        let mut t = visual(
            10,
            Some("textbox"),
            (620, 480, 300, 60),
            9,
            Some("Note (hidden)"),
            vec![],
            vec![],
        );
        t["hidden"] = json!(true);
        t["text"] = json!("Chiffres provisoires, mis à jour chaque nuit.");
        overview.push(t);
    }
    if at(15) {
        overview.push(with(
            visual(
                11,
                Some("map"),
                (940, 480, 320, 220),
                10,
                Some("Ventes par pays"),
                vec![
                    field("Category", "column", "Client", "Ville", None),
                    field("Size", "measure", "Ventes", "Total Ventes", None),
                ],
                vec![issue("error", "Client[Ville] does not exist in the model")],
            ),
            data_off("unsupported", "map visuals are not evaluated"),
            Value::Null,
        ));
    }
    let overview_bg = if custom_theme {
        json!("#EEF3EF")
    } else {
        Value::Null
    };
    let mut pages = vec![
        json!({ "name": vname(100), "display_name": "Overview", "width": 1280, "height": 720, "hidden": false, "background": overview_bg, "visuals": overview }),
    ];
    if at(10) {
        let mut pivot_rows = vec![];
        for (n, c, v) in PRODUCTS.iter().take(6) {
            for (k, year) in [2023, 2024, 2025].iter().enumerate() {
                pivot_rows.push(json!([c, n, year, round(v * (0.8 + k as f64 * 0.1))]));
            }
        }
        pages.push(json!({ "name": vname(101), "display_name": "Détails", "width": 1280, "height": 720, "hidden": false, "visuals": [
            with(visual(20, Some("pivotTable"), (20, 20, 760, 680), 0, Some("Détail par produit"), vec![
                field("Rows", "column", "Produit", "Catégorie", None), field("Rows", "column", "Produit", "Nom", None),
                field("Columns", "column", "Date", "Année", None), field("Values", "measure", "Ventes", "Total Ventes", None),
            ], vec![]),
            data_ok(&[("Catégorie", "Rows", "column", "string"), ("Nom", "Rows", "column", "string"),
                      ("Année", "Columns", "column", "int64"), ("Total Ventes", "Values", "measure", "double")], pivot_rows),
            json!({ "title": { "show": true, "font_size": 14 } })),
            with(visual(21, Some("scatterChart"), (800, 20, 460, 330), 1, Some("Prix vs quantité"), vec![
                field("X", "aggregation", "Produit", "Prix", Some("Avg")), field("Y", "measure", "Ventes", "Quantité", None),
            ], vec![]), data_off("unsupported", "scatter charts are not evaluated yet"), Value::Null),
            with(visual(22, Some("donutChart"), (800, 370, 460, 330), 2, Some("Part par catégorie"), vec![
                field("Category", "column", "Produit", "Catégorie", None), field("Y", "measure", "Ventes", "Total Ventes", None),
            ], vec![]), data_ok(&[("Catégorie", "Category", "column", "string"), ("Total Ventes", "Y", "measure", "double")], by_cat.clone()), Value::Null),
        ] }));
    }
    if at(12) {
        pages.push(gallery(growth, total));
    }
    if at(20) {
        pages.push(json!({ "name": vname(102), "display_name": "Tooltip", "width": 320, "height": 240, "hidden": true, "visuals": [
            visual(30, Some("card"), (10, 10, 300, 220), 0, None, vec![field("Values", "measure", "Ventes", "Total Ventes", None)], vec![]),
        ] }));
    }
    if big {
        pages.push(stress_page(step));
    }
    let active = if (10..16).contains(&s) {
        vname(101)
    } else if (16..22).contains(&s) {
        vname(104)
    } else {
        vname(100)
    };

    // ---- validation
    let mut objects = vec![];
    if n1_invalid {
        objects.push(json!({ "table": "Ventes", "kind": "measure", "name": "Ventes N-1", "diagnostics": [
            { "severity": "error", "message": "Column 'Jour' was not found in table 'Date'", "code": "DAX0001" },
        ] }));
    }
    let mut report_issues = vec![];
    for p in &pages {
        for v in p["visuals"].as_array().into_iter().flatten() {
            for i in v["issues"].as_array().into_iter().flatten() {
                report_issues.push(json!({ "severity": i["severity"], "message": i["message"], "page": p["display_name"], "visual": v["name"] }));
            }
        }
    }
    let errors = report_issues.iter().filter(|i| i["severity"] == "error").count();
    let warnings = report_issues.len() - errors;

    let commands = [
        ("project.new", true, None),
        ("measure.add", true, None),
        ("visual.add", true, None),
        ("measure.add", true, None),
        ("visual.add", true, None),
        (
            "measure.add",
            false,
            Some("a measure named 'Marge %' already exists"),
        ),
        ("table.add", true, None),
        ("relationship.add", true, None),
        ("measure.add", true, None),
        ("visual.add", true, None),
    ];
    let (id, ok, error) = commands[(s % commands.len() as u64) as usize];
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    json!({
        "version": 1,
        "revision": step + 1,
        "source": { "path": "C:/demo/Ventes.pbip", "kind": "project", "format": "pbip-tmdl" },
        "dirty": { "model": !s.is_multiple_of(7), "report": !s.is_multiple_of(5) },
        "last_command": { "id": id, "ok": ok, "error": error, "at_ms": now },
        "model": { "name": "Ventes", "culture": "fr-FR", "tables": tables, "relationships": relationships },
        "report": { "active_page": active, "theme": theme(custom_theme), "pages": pages },
        "validation": {
            "model": { "valid": objects.is_empty(), "invalid_count": objects.len(), "objects": objects },
            "report": { "valid": errors == 0, "error_count": errors, "warning_count": warnings, "issues": report_issues },
        },
    })
}

/// One page with every visual type the viewer renders, some with data, some without.
fn gallery(growth: f64, total: f64) -> Value {
    let m = |name: &str, role: &str| field(role, "measure", "Ventes", name, None);
    let cat = |role: &str| field(role, "column", "Produit", "Catégorie", None);
    let canal = |role: &str| field(role, "column", "Ventes", "Canal", None);
    let mois = field("Category", "column", "Date", "Mois", None);
    let cat_col = ("Catégorie", "Category", "column", "string");
    let ventes_y = ("Total Ventes", "Y", "measure", "double");

    let mut by_cat_year = vec![];
    for (c, v) in CATS {
        for (k, year) in [2023, 2024, 2025].iter().enumerate() {
            by_cat_year.push(json!([c, year, round(v * growth * (0.27 + k as f64 * 0.05))]));
        }
    }
    let mut by_cat_canal = vec![];
    for (i, (c, v)) in CATS.iter().enumerate() {
        for (k, (ch, share)) in CHANNELS.iter().enumerate() {
            let tilt = 1.0 + ((i + k) % 3) as f64 * 0.25;
            by_cat_canal.push(json!([c, ch, round(v * share * tilt)]));
        }
    }
    let top: Vec<Value> = PRODUCTS
        .iter()
        .take(6)
        .map(|(n, _, v)| json!([n, round(v * growth), round(v * 0.87)]))
        .collect();
    let combo: Vec<Value> = MONTHS
        .iter()
        .enumerate()
        .map(|(i, mo)| json!([mo, monthly(i, growth), 0.27 + 0.05 * (i as f64 * 0.8).cos()]))
        .collect();
    let canal_rows: Vec<Value> = CHANNELS
        .iter()
        .map(|(ch, share)| json!([ch, round(total * share)]))
        .collect();
    let multi: Vec<Value> = CATS
        .iter()
        .take(3)
        .enumerate()
        .map(|(i, (c, v))| json!([c, round(v * growth), 4_200 - i as i64 * 1_100]))
        .collect();

    let visuals = vec![
        with(
            visual(
                40,
                Some("columnChart"),
                (10, 10, 305, 220),
                0,
                Some("Ventes par catégorie et année"),
                vec![
                    cat("Category"),
                    field("Series", "column", "Date", "Année", None),
                    m("Total Ventes", "Y"),
                ],
                vec![],
            ),
            data_ok(
                &[cat_col, ("Année", "Series", "column", "int64"), ventes_y],
                by_cat_year,
            ),
            tile(),
        ),
        with(
            visual(
                41,
                Some("hundredPercentStackedBarChart"),
                (325, 10, 305, 220),
                1,
                Some("Mix par canal"),
                vec![cat("Category"), canal("Series"), m("Total Ventes", "Y")],
                vec![],
            ),
            data_ok(
                &[cat_col, ("Canal", "Series", "column", "string"), ventes_y],
                by_cat_canal,
            ),
            tile(),
        ),
        with(
            visual(
                42,
                Some("clusteredBarChart"),
                (640, 10, 305, 220),
                2,
                Some("Top produits"),
                vec![
                    field("Category", "column", "Produit", "Nom", None),
                    m("Total Ventes", "Y"),
                    m("Ventes N-1", "Y"),
                ],
                vec![],
            ),
            data_ok(
                &[
                    ("Nom", "Category", "column", "string"),
                    ventes_y,
                    ("Ventes N-1", "Y", "measure", "double"),
                ],
                top,
            ),
            {
                let mut f = tile();
                f["data_labels"] = json!(true);
                f["legend"] = json!({ "show": true, "position": "bottom" });
                f
            },
        ),
        with(
            visual(
                43,
                Some("areaChart"),
                (955, 10, 315, 220),
                3,
                Some("Ventes cumulées"),
                vec![mois.clone(), m("Total Ventes", "Y")],
                vec![],
            ),
            data_off("unsupported", "running totals are not evaluated yet"),
            tile(),
        ),
        with(
            visual(
                44,
                Some("lineClusteredColumnComboChart"),
                (10, 240, 460, 220),
                4,
                Some("Ventes et marge"),
                vec![mois, m("Total Ventes", "Y"), m("Marge %", "Y2")],
                vec![],
            ),
            data_ok(
                &[
                    ("Mois", "Category", "column", "string"),
                    ventes_y,
                    ("Marge %", "Y2", "measure", "double"),
                ],
                combo,
            ),
            tile(),
        ),
        with(
            visual(
                45,
                Some("pieChart"),
                (480, 240, 300, 220),
                5,
                Some("Part par canal"),
                vec![canal("Category"), m("Total Ventes", "Y")],
                vec![],
            ),
            data_ok(
                &[("Canal", "Category", "column", "string"), ventes_y],
                canal_rows.clone(),
            ),
            tile(),
        ),
        with(
            visual(
                46,
                Some("gauge"),
                (790, 240, 230, 220),
                6,
                Some("Objectif annuel"),
                vec![
                    m("Total Ventes", "Y"),
                    m("Objectif", "MaxValue"),
                    m("Objectif", "TargetValue"),
                ],
                vec![],
            ),
            data_ok(
                &[
                    ventes_y,
                    ("Objectif", "MaxValue", "measure", "double"),
                    ("Objectif", "TargetValue", "measure", "double"),
                ],
                vec![json!([total, 1_250_000.0, 1_000_000.0])],
            ),
            tile(),
        ),
        with(
            visual(
                47,
                Some("multiRowCard"),
                (1030, 240, 240, 220),
                7,
                None,
                vec![
                    cat("Values"),
                    m("Total Ventes", "Values"),
                    m("Quantité", "Values"),
                ],
                vec![],
            ),
            data_ok(
                &[
                    ("Catégorie", "Values", "column", "string"),
                    ("Total Ventes", "Values", "measure", "double"),
                    ("Quantité", "Values", "measure", "int64"),
                ],
                multi,
            ),
            tile(),
        ),
        with(
            visual(
                48,
                Some("cardVisual"),
                (10, 470, 460, 110),
                8,
                None,
                vec![
                    m("Total Ventes", "Data"),
                    m("Quantité", "Data"),
                    m("Marge %", "Data"),
                ],
                vec![],
            ),
            data_ok(
                &[
                    ("Total Ventes", "Data", "measure", "double"),
                    ("Quantité", "Data", "measure", "int64"),
                    ("Marge %", "Data", "measure", "double"),
                ],
                vec![json!([total, 12_840, 0.312])],
            ),
            Value::Null,
        ),
        with(
            visual(
                49,
                Some("slicer"),
                (10, 590, 220, 56),
                9,
                None,
                vec![canal("Values")],
                vec![],
            ),
            data_ok(
                &[("Canal", "Values", "column", "string")],
                CHANNELS.iter().map(|(c, _)| json!([c])).collect(),
            ),
            tile(),
        ),
        with(
            visual(
                50,
                Some("actionButton"),
                (240, 590, 110, 40),
                10,
                Some("Retour"),
                vec![],
                vec![],
            ),
            Value::Null,
            Value::Null,
        ),
        with(
            visual(
                51,
                Some("basicShape"),
                (360, 590, 110, 120),
                11,
                None,
                vec![],
                vec![],
            ),
            Value::Null,
            Value::Null,
        ),
        with(
            visual(
                52,
                Some("slicer"),
                (480, 470, 200, 240),
                12,
                None,
                vec![cat("Values")],
                vec![],
            ),
            data_ok(
                &[("Catégorie", "Values", "column", "string")],
                CATS.iter().map(|(c, _)| json!([c])).collect(),
            ),
            tile(),
        ),
        {
            let mut t = visual(53, Some("textbox"), (690, 470, 290, 70), 13, None, vec![], vec![]);
            t["text"] = json!("Galerie : chaque type de visuel rendu par bicraft view.");
            t
        },
        with(
            visual(54, Some("image"), (690, 550, 290, 160), 14, None, vec![], vec![]),
            Value::Null,
            Value::Null,
        ),
        with(
            visual(
                55,
                Some("tableEx"),
                (990, 470, 280, 240),
                15,
                Some("Clients"),
                vec![
                    field("Values", "column", "Client", "Nom", None),
                    field("Values", "column", "Client", "Pays", None),
                    m("Total Ventes", "Values"),
                ],
                vec![],
            ),
            data_off("unavailable", "PBIT: the template carries no table data"),
            {
                let mut f = tile();
                f["title"] = json!({ "show": true });
                f
            },
        ),
    ];
    json!({ "name": vname(104), "display_name": "Galerie", "width": 1280, "height": 720, "hidden": false,
            "background": "#F3F2F1", "visuals": visuals })
}

/// 800 small visuals with data; every second a different 10% of them change.
fn stress_page(step: u64) -> Value {
    let visuals: Vec<Value> = (0..800u64)
        .map(|i| {
            let (cx, cy) = ((i % 40) as i64, (i / 40) as i64);
            let issues = if i % 97 == 0 {
                vec![issue("warning", "stress warning")]
            } else {
                vec![]
            };
            let bump = if i % 10 == step % 10 { step as f64 } else { 0.0 };
            let val = |k: u64| 1_000.0 + ((i * 37 + k * 101) % 900) as f64 * 10.0 + bump;
            let total = field("Y", "measure", "Ventes", "Total Ventes", None);
            let (ty, fields, data) = match i % 6 {
                0 => (
                    "card",
                    vec![field("Values", "measure", "Ventes", "Total Ventes", None)],
                    data_ok(
                        &[("Total Ventes", "Values", "measure", "double")],
                        vec![json!([val(0) * 100.0])],
                    ),
                ),
                1 => (
                    "clusteredColumnChart",
                    vec![field("Category", "column", "Produit", "Catégorie", None), total],
                    data_ok(
                        &[
                            ("Catégorie", "Category", "column", "string"),
                            ("Total Ventes", "Y", "measure", "double"),
                        ],
                        (0..4).map(|k| json!([CATS[k as usize].0, val(k)])).collect(),
                    ),
                ),
                2 => (
                    "lineChart",
                    vec![field("Category", "column", "Date", "Mois", None), total],
                    data_ok(
                        &[
                            ("Mois", "Category", "column", "string"),
                            ("Total Ventes", "Y", "measure", "double"),
                        ],
                        (0..6).map(|k| json!([MONTHS[k as usize], val(k)])).collect(),
                    ),
                ),
                3 => (
                    "slicer",
                    vec![field("Values", "column", "Produit", "Catégorie", None)],
                    data_ok(
                        &[("Catégorie", "Values", "column", "string")],
                        CATS.iter().map(|(c, _)| json!([c])).collect(),
                    ),
                ),
                4 => (
                    "pieChart",
                    vec![field("Category", "column", "Produit", "Catégorie", None), total],
                    data_ok(
                        &[
                            ("Catégorie", "Category", "column", "string"),
                            ("Total Ventes", "Y", "measure", "double"),
                        ],
                        (0..4).map(|k| json!([CATS[k as usize].0, val(k)])).collect(),
                    ),
                ),
                _ => (
                    "tableEx",
                    vec![
                        field("Values", "column", "Produit", "Nom", None),
                        field("Values", "measure", "Ventes", "Total Ventes", None),
                    ],
                    data_ok(
                        &[
                            ("Nom", "Values", "column", "string"),
                            ("Total Ventes", "Values", "measure", "double"),
                        ],
                        (0..4).map(|k| json!([PRODUCTS[k as usize].0, val(k)])).collect(),
                    ),
                ),
            };
            with(
                visual(
                    1000 + i,
                    Some(ty),
                    (cx * 48, cy * 54, 46, 52),
                    i as i64,
                    Some(&format!("V{i}")),
                    fields,
                    issues,
                ),
                data,
                Value::Null,
            )
        })
        .collect();
    json!({ "name": vname(103), "display_name": "Stress", "width": 1920, "height": 1080, "hidden": false, "visuals": visuals })
}
